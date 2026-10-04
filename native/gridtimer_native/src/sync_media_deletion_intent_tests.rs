// v1.0.3.7 Windows - Ignore only pre-deletion historical media references.
// v0.0.0.2 - Exercise durable note detach, immutable identity, replay and shared ownership.
// v0.0.0.1 - Keep shared attachment references readable when one owner deletes its copy.

fn shared_attachment_workspace(note_ids: &[&str], now: i64) -> String {
    let mut raw = app_data::default_app_data_json(now);
    for id in note_ids {
        raw = app_data::upsert_note_app_data_json(
            &raw,
            &json!({
                "id": id,
                "title": id,
                "content": "Synthetic attachment ownership fixture",
                "updatedAtEpochMillis": now,
                "attachments": [{"id":"shared-image", "mimeType":"image/png",
                    "sha256":hex_bytes(&Sha256::digest(b"data")), "sizeBytes":4, "updatedAtEpochMillis":now}],
                "document": {"blocks":[{"id":format!("{id}-image"),"type":"IMAGE",
                    "attachmentId":"shared-image"}]}
            })
            .to_string(),
            now,
        )
        .unwrap();
    }
    raw
}

fn note_keeps_shared_attachment(raw: &str, note_id: &str) -> bool {
    let data: Value = serde_json::from_str(raw).unwrap();
    data["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == note_id)
        .is_some_and(|note| {
            note["attachments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|attachment| attachment["id"] == "shared-image")
        })
}

#[test]
fn media_deletion_intent_stale_note_delete_preserves_other_device_shared_owner() {
    let stale = shared_attachment_workspace(&["page-a"], 100);
    let remote = shared_attachment_workspace(&["page-a", "page-b"], 100);
    assert!(note_keeps_shared_attachment(&stale, "page-a"));
    assert!(note_keeps_shared_attachment(&remote, "page-b"));
    let removed = app_data::delete_note_permanently_app_data_json(&stale, "page-a", 300).unwrap();
    let merged = merge_sync_app_data_json(&remote, &removed, 400).unwrap();
    let merged_value: Value = serde_json::from_str(&merged).unwrap();
    assert!(!merged_value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note["id"] == "page-a"));
    assert!(
        note_keeps_shared_attachment(&merged, "page-b"),
        "deleting a stale copy of page A must not erase page B's surviving shared reference"
    );
    let replay = merge_sync_app_data_json(&merged, &removed, 500).unwrap();
    assert!(note_keeps_shared_attachment(&replay, "page-b"));
}

#[test]
fn media_deletion_intent_detaching_from_one_note_preserves_another_current_owner() {
    let shared = shared_attachment_workspace(&["page-a", "page-b"], 100);
    assert!(note_keeps_shared_attachment(&shared, "page-a"));
    assert!(note_keeps_shared_attachment(&shared, "page-b"));
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    assert!(!note_keeps_shared_attachment(&detached, "page-a"));
    assert!(
        note_keeps_shared_attachment(&detached, "page-b"),
        "removing an image from page A must not detach the same image from page B"
    );
    let merged = merge_sync_app_data_json(&shared, &detached, 400).unwrap();
    assert!(
        !note_keeps_shared_attachment(&merged, "page-a"),
        "a protected shared reference must not silently reattach to its removed owner"
    );
    assert!(note_keeps_shared_attachment(&merged, "page-b"));
}

#[test]
fn media_deletion_intent_preserves_unseen_concurrent_attachment_and_recovery_history() {
    let shared = shared_attachment_workspace(&["page-a", "page-b"], 100);
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    let mut concurrent: Value = serde_json::from_str(&shared).unwrap();
    let note = concurrent["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    note["attachments"].as_array_mut().unwrap().push(json!({
        "id":"unseen-addition", "sha256":"b".repeat(64), "sizeBytes":7,
        "mimeType":"application/octet-stream", "updatedAtEpochMillis":250
    }));
    note["updatedAtEpochMillis"] = json!(250);
    let concurrent = concurrent.to_string();
    for (left, right) in [(&concurrent, &detached), (&detached, &concurrent)] {
        let merged = merge_sync_app_data_json(left, right, 400).unwrap();
        assert!(!note_keeps_shared_attachment(&merged, "page-a"));
        assert!(note_keeps_shared_attachment(&merged, "page-b"));
        let merged: Value = serde_json::from_str(&merged).unwrap();
        let note = merged["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|note| note["id"] == "page-a")
            .unwrap();
        assert!(note["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "unseen-addition"));
        assert!(note["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|revision| revision["attachments"]
                .as_array()
                .is_some_and(|attachments| attachments
                    .iter()
                    .any(|item| item["id"] == "shared-image"))));
    }
}

#[test]
fn media_deletion_intent_scoped_cleanup_keeps_sealed_and_historical_owners() {
    for opaque in [false, true] {
        let stale = shared_attachment_workspace(&["page-a"], 100);
        let mut remote: Value =
            serde_json::from_str(&shared_attachment_workspace(&["page-a", "page-b"], 100)).unwrap();
        let note = remote["notes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|note| note["id"] == "page-b")
            .unwrap();
        if opaque {
            let fixture: Value =
                serde_json::from_str(&private_media_fixture("c2hhcmVkLXByaXZhdGU=").0).unwrap();
            *note = fixture["notes"][0].clone();
            note["id"] = json!("page-b");
        } else {
            let historical = json!({
                "id":"retained-page-b", "title":note["title"],
                "content":note["content"], "kind":note["kind"],
                "document":note["document"], "attachments":note["attachments"],
                "attachmentIds":["shared-image"],
                "capturedAtEpochMillis":100, "updatedAtEpochMillis":100
            });
            note["attachments"] = json!([]);
            note["document"]["blocks"] = json!([]);
            note["revisions"] = json!([historical]);
        }
        let removed =
            app_data::delete_note_permanently_app_data_json(&stale, "page-a", 300).unwrap();
        let plan = merge_sync_app_data_json_with_media_intents(
            &remote.to_string(),
            &removed,
            400,
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            plan.normalization.cleanup_candidates.get("shared-image"),
            Some(&300)
        );
        let merged: Value = serde_json::from_str(&plan.app_data_json).unwrap();
        assert!(!merged["tombstones"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tombstone| tombstone["entityId"] == "shared-image"));
        let survivor = merged["notes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|note| note["id"] == "page-b")
            .unwrap();
        if opaque {
            assert!(survivor["encryption"].is_object());
        } else {
            assert!(survivor["revisions"][0]["attachments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == "shared-image"));
        }
    }
}

#[test]
fn legacy_media_delete_does_not_block_on_pre_tombstone_history() {
    fn history_snapshot(tombstone_at: i64) -> String {
        let mut root: Value =
            serde_json::from_str(&shared_attachment_workspace(&["page-a"], 100)).unwrap();
        let note = root["notes"].as_array_mut().unwrap().first_mut().unwrap();
        let old_attachment = note["attachments"][0].clone();
        note["content"] = json!("current note text");
        note["attachments"] = json!([]);
        note["document"]["blocks"] = json!([{
            "id":"page-a-current-text", "type":"TEXT", "text":"current note text"
        }]);
        note["updatedAtEpochMillis"] = json!(200);
        note["revisions"] = json!([{
            "id":"old-revision", "content":"old note text with an image",
            "attachmentIds":["shared-image"], "attachments":[old_attachment],
            "document":{"blocks":[{"id":"old-image","type":"IMAGE","attachmentId":"shared-image"}]},
            "capturedAtEpochMillis":100, "updatedAtEpochMillis":100
        }]);
        note["versions"] = json!([
            {
                "id":"old-version", "noteId":"page-a", "sequence":1,
                "title":"page-a", "content":"old note text with an image", "kind":"STICKY",
                "attachmentIds":["shared-image"], "attachments":[old_attachment],
                "document":{"blocks":[{"id":"old-version-image","type":"IMAGE","attachmentId":"shared-image"}]},
                "createdAtEpochMillis":100, "updatedAtEpochMillis":100, "isLatest":false
            },
            {
                "id":"current-version", "noteId":"page-a", "sequence":2,
                "title":"page-a", "content":"current note text", "kind":"STICKY",
                "attachmentIds":[], "attachments":[],
                "document":{"blocks":[{"id":"page-a-current-text","type":"TEXT","text":"current note text"}]},
                "createdAtEpochMillis":200, "updatedAtEpochMillis":200, "isLatest":true
            }
        ]);
        note["latestVersionId"] = json!("current-version");
        root["tombstones"] = json!([{
            "entityType":"noteMedia", "entityId":"shared-image",
            "deletedAtEpochMillis":tombstone_at
        }]);
        root.to_string()
    }

    let old_only = history_snapshot(300);
    let plan = merge_sync_app_data_json_with_media_intents(
        &old_only,
        &old_only,
        400,
        &Default::default(),
        &Default::default(),
    )
    .expect("an old historical-only attachment reference is covered by the tombstone");
    assert!(plan.normalization.unresolved_ids.is_empty());
    assert!(!plan
        .normalization
        .cleanup_candidates
        .contains_key("shared-image"));
    let merged: Value = serde_json::from_str(&plan.app_data_json).unwrap();
    let note = merged["notes"].as_array().unwrap().first().unwrap();
    assert!(note["attachments"].as_array().unwrap().is_empty());
    assert!(note["revisions"][0]["attachments"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(note["revisions"][0]["attachmentIds"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(note["versions"][0]["attachments"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(note["versions"][0]["attachmentIds"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(merged["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["entityId"] == "shared-image"));
}

#[test]
fn media_deletion_intent_floor_rejects_alias_and_plain_timestamp_but_allows_new_attachment_revision(
) {
    use crate::note_media_intent::{
        infer_snapshot_detachments, infer_transition_detachments, merge_detachments,
        project_current_detachments,
    };
    let shared = shared_attachment_workspace(&["page-a", "page-b"], 100);
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    let floors = infer_snapshot_detachments(&serde_json::from_str(&detached).unwrap()).unwrap();
    let mut later_observed = floors.clone();
    later_observed
        .get_mut("page-a")
        .unwrap()
        .get_mut("shared-image")
        .unwrap()
        .observed_attachment_revision = 200;
    let mut left_then_right = floors.clone();
    merge_detachments(&mut left_then_right, &later_observed).unwrap();
    let mut right_then_left = later_observed.clone();
    merge_detachments(&mut right_then_left, &floors).unwrap();
    assert_eq!(
        left_then_right, right_then_left,
        "equal detach floors must retain the same latest observed revision in either merge order"
    );
    assert_eq!(left_then_right, later_observed);
    let mut before_note_delete: Value = serde_json::from_str(&shared).unwrap();
    let preimage = before_note_delete["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    let mut later_attachment = preimage["attachments"][0].clone();
    later_attachment["updatedAtEpochMillis"] = json!(200);
    preimage["revisions"] = json!([{"attachments":[later_attachment]}]);
    let inferred = infer_transition_detachments(&before_note_delete,
        &json!({"tombstones":[{"entityType":"note", "entityId":"page-a", "deletedAtEpochMillis":300}]}))
        .unwrap();
    assert_eq!(
        inferred["page-a"]["shared-image"].observed_attachment_revision, 200,
        "multiple retained pre-images must select the same latest observed revision"
    );
    let detached_value: Value = serde_json::from_str(&detached).unwrap();
    let removed_note = detached_value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    let revision_id = removed_note["revisions"][0]["id"].as_str().unwrap();
    let clock_rollback_restore =
        app_data::restore_note_revision_app_data_json(&detached, "page-a", revision_id, 200)
            .unwrap();
    let mut clock_rollback_restore: Value = serde_json::from_str(&clock_rollback_restore).unwrap();
    assert!(!project_current_detachments(&mut clock_rollback_restore, &floors).unwrap());
    assert!(note_keeps_shared_attachment(
        &clock_rollback_restore.to_string(),
        "page-a"
    ));
    let mut replay: Value = serde_json::from_str(&shared).unwrap();
    for note in replay["notes"].as_array_mut().unwrap() {
        note["revisions"] = json!([]);
        if note["id"] == "page-a" {
            note["updatedAtEpochMillis"] = json!(9000);
        }
    }
    assert!(project_current_detachments(&mut replay, &floors).unwrap());
    assert!(!note_keeps_shared_attachment(&replay.to_string(), "page-a"));
    assert!(note_keeps_shared_attachment(&replay.to_string(), "page-b"));
    let mut restored: Value = serde_json::from_str(&shared).unwrap();
    let note = restored["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    note["attachments"][0]["updatedAtEpochMillis"] = json!(301);
    assert!(!project_current_detachments(&mut restored, &floors).unwrap());
    assert!(note_keeps_shared_attachment(
        &restored.to_string(),
        "page-a"
    ));
    for field in ["sha256", "sizeBytes"] {
        let mut incomplete = restored.clone();
        let note = incomplete["notes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|note| note["id"] == "page-a")
            .unwrap();
        note["attachments"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            project_current_detachments(&mut incomplete, &floors).is_err(),
            "a later attachment revision without {field} cannot bypass a known detach identity"
        );
    }
    let mut incomplete_stale: Value = serde_json::from_str(&shared).unwrap();
    let stale_note = incomplete_stale["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    stale_note["attachments"][0]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    assert!(project_current_detachments(&mut incomplete_stale, &floors).unwrap());
    assert!(!note_keeps_shared_attachment(
        &incomplete_stale.to_string(),
        "page-a"
    ));
    let note = restored["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap();
    note["attachments"][0]["sha256"] = json!("b".repeat(64));
    assert!(project_current_detachments(&mut restored, &floors).is_err());
}

#[test]
fn media_deletion_intent_ambiguous_marker_cannot_remove_current_or_opaque_owner() {
    for opaque in [false, true] {
        let current = if opaque {
            private_media_fixture("dW5rbm93bg==").0
        } else {
            shared_attachment_workspace(&["page-b"], 100)
        };
        let mut incoming: Value =
            serde_json::from_str(&app_data::default_app_data_json(300)).unwrap();
        incoming["tombstones"] = json!([{"entityType":"noteMedia", "entityId":"shared-image", "deletedAtEpochMillis":300}]);
        assert!(merge_sync_app_data_json(&current, &incoming.to_string(), 400).is_none());
    }
}

fn media_intent_request(
    token: &str,
    request_id: &str,
    raw: &str,
    force_download: bool,
) -> HttpRequest {
    HttpRequest {
        headers: vec![("Authorization".into(), format!("Bearer {token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: request_id.into(),
            app_data_json: raw.into(),
            client_updated_at_epoch_millis: 300,
            acknowledged_generation: 0,
            device_name: "synthetic intent fixture".into(),
            force_download,
            ..Default::default()
        })
        .unwrap(),
        ..Default::default()
    }
}

fn media_intent_server_state() -> String {
    shared_attachment_workspace(&["page-a", "page-b"], 100)
}

fn media_intent_store_with_bytes(token: &str, raw: &str) -> TestSqliteStore {
    let fixture = account_test_store(token, raw.to_owned());
    fixture
        .store
        .upsert_media(
            "user-1",
            "shared-image",
            &hex_bytes(&Sha256::digest(b"data")),
            "image/png",
            4,
            b"data",
            100,
        )
        .unwrap();
    fixture
}

#[test]
fn media_deletion_intent_http_replay_and_reopen_preserve_floor_after_history_pruning() {
    let token = "media-intent-durable-token";
    let shared = media_intent_server_state();
    let fixture = media_intent_store_with_bytes(token, &shared);
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    let request = media_intent_request(token, "media-intent-first", &detached, false);
    let first = handle_sync(&request, &fixture.store);
    assert_eq!(first.0, 200, "{}", first.1.message);
    let head = fixture.store.read_account("user-1").unwrap();
    assert_eq!(
        first.1.app_data_json.as_deref(),
        Some(head.app_data_json.as_str())
    );
    assert!(!note_keeps_shared_attachment(&head.app_data_json, "page-a"));
    assert!(note_keeps_shared_attachment(&head.app_data_json, "page-b"));
    let second = handle_sync(&request, &fixture.store);
    assert_eq!(second.0, 200, "{}", second.1.message);
    assert_eq!(encode_result(&first.1), encode_result(&second.1));
    assert_eq!(
        fixture.store.read_account("user-1").unwrap().revision,
        head.revision
    );
    let mut pruned: Value = serde_json::from_str(&head.app_data_json).unwrap();
    for note in pruned["notes"].as_array_mut().unwrap() {
        note["revisions"] = json!([]);
        note["versions"] = json!([]);
    }
    fixture
        .store
        .compare_and_swap_account("user-1", head.revision, &pruned.to_string(), 500)
        .unwrap();
    let reopened = SqliteServerStore::open(fixture.store.database_path(), None).unwrap();
    let stale = handle_sync(
        &media_intent_request(token, "media-intent-after-reopen", &shared, false),
        &reopened,
    );
    assert_eq!(stale.0, 200, "{}", stale.1.message);
    let current = reopened.read_account("user-1").unwrap();
    assert!(!note_keeps_shared_attachment(
        &current.app_data_json,
        "page-a"
    ));
    assert!(note_keeps_shared_attachment(
        &current.app_data_json,
        "page-b"
    ));
    assert!(reopened
        .note_media_intent_policy("user-1")
        .unwrap()
        .note_attachment_detachments()
        .get("page-a")
        .unwrap()
        .contains_key("shared-image"));
    assert_eq!(
        reopened
            .read_media("user-1", "shared-image")
            .unwrap()
            .unwrap()
            .content
            .as_slice(),
        b"data"
    );
}

#[test]
fn media_deletion_intent_equal_revision_merge_history_is_not_observed_removal() {
    let mut snapshot: Value =
        serde_json::from_str(&shared_attachment_workspace(&["page-a"], 100)).unwrap();
    let note = &mut snapshot["notes"][0];
    note["attachments"][0]["updatedAtEpochMillis"] = json!(50);
    let losing = note_current_as_revision(note).unwrap();
    note["attachments"] = json!([]);
    note["document"]["blocks"] = json!([]);
    note["revisions"] = json!([losing]);
    assert!(
        crate::note_media_intent::infer_snapshot_detachments(&snapshot)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn media_deletion_intent_cas_reject_is_atomic_and_raw_evidence_is_request_bound() {
    let token = "media-intent-cas-token";
    let shared = media_intent_server_state();
    let fixture = media_intent_store_with_bytes(token, &shared);
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    let plan = merge_sync_app_data_json_with_media_intents(
        &shared,
        &detached,
        400,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let account = fixture.store.read_account("user-1").unwrap();
    let policy = fixture.store.note_media_intent_policy("user-1").unwrap();
    let token_id = match fixture
        .store
        .authenticate_token(token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(token) => token.token_id,
        other => panic!("unexpected token state: {other:?}"),
    };
    let apply = |expected, raw: &str| {
        fixture
            .store
            .apply_sync_request_for_generation_with_pending_activation_and_media_intents(
                "user-1",
                token_id,
                0,
                "",
                "media-intent-atomic",
                "{\"operation\":\"synthetic-intent\"}",
                expected,
                &plan.app_data_json,
                "{}",
                now_millis(),
                None,
                raw,
            )
    };
    assert!(matches!(
        apply(account.revision + 1, &detached),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert_eq!(fixture.store.read_account("user-1").unwrap(), account);
    assert_eq!(
        fixture.store.note_media_intent_policy("user-1").unwrap(),
        policy
    );
    assert!(matches!(
        apply(account.revision, &detached).unwrap(),
        SyncRequestOutcome::Applied(_)
    ));
    let committed = fixture.store.read_account("user-1").unwrap();
    let committed_policy = fixture.store.note_media_intent_policy("user-1").unwrap();
    let mut changed_evidence: Value = serde_json::from_str(&detached).unwrap();
    changed_evidence["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|note| note["id"] == "page-a")
        .unwrap()["revisions"][0]["capturedAtEpochMillis"] = json!(299);
    assert!(apply(account.revision, &changed_evidence.to_string()).is_err());
    assert_eq!(fixture.store.read_account("user-1").unwrap(), committed);
    assert_eq!(
        fixture.store.note_media_intent_policy("user-1").unwrap(),
        committed_policy
    );
    assert!(matches!(
        apply(account.revision, &detached).unwrap(),
        SyncRequestOutcome::Replayed(_)
    ));
}

#[test]
fn media_deletion_intent_download_does_not_observe_incoming_removal() {
    let token = "media-intent-download-token";
    let shared = media_intent_server_state();
    let fixture = media_intent_store_with_bytes(token, &shared);
    let detached =
        app_data::delete_note_attachment_app_data_json(&shared, "page-a", "shared-image", 300)
            .unwrap();
    let before = fixture.store.read_account("user-1").unwrap();
    let policy = fixture.store.note_media_intent_policy("user-1").unwrap();
    let response = handle_sync(
        &media_intent_request(token, "media-intent-download", &detached, true),
        &fixture.store,
    );
    assert_eq!(response.0, 200, "{}", response.1.message);
    assert_eq!(fixture.store.read_account("user-1").unwrap(), before);
    assert_eq!(
        fixture.store.note_media_intent_policy("user-1").unwrap(),
        policy
    );
    assert!(note_keeps_shared_attachment(
        response.1.app_data_json.as_deref().unwrap(),
        "page-a"
    ));
}
