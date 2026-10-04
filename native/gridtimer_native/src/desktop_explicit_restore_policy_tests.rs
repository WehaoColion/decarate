#[test]
fn explicit_restore_clock_uses_only_selected_proven_floors_without_mutating_policy() {
    let mut value = serde_json::to_value(DesktopPrivacyPolicy::default()).unwrap();
    value["media_deletions"] = json!({"selected":700, "unselected":i64::MAX});
    value["media_candidates"] = json!({"selected":i64::MAX});
    value["opaque_media_deletions"] = json!({"selected":i64::MAX});
    value["note_attachment_detachments"] = json!({
        "note-a":{"selected":{"detached_at_epoch_millis":800,"observed_attachment_revision":100}},
        "note-b":{"selected":{"detached_at_epoch_millis":i64::MAX,"observed_attachment_revision":100}}
    });
    let policy: DesktopPrivacyPolicy = serde_json::from_value(value).unwrap();
    let before = policy.clone();
    assert_eq!(
        policy
            .next_explicit_attachment_restore_revision("note-a", &["selected".into()], 100)
            .unwrap(),
        801
    );
    assert_eq!(
        policy
            .next_explicit_attachment_restore_revision("other-note", &["selected".into()], 100)
            .unwrap(),
        701
    );
    assert_eq!(
        policy
            .next_explicit_attachment_restore_revision("note-b", &["unrelated".into()], 100)
            .unwrap(),
        100
    );
    assert!(policy
        .next_explicit_attachment_restore_revision("note-b", &["selected".into()], 100)
        .is_err());
    assert!(policy
        .next_explicit_attachment_restore_revision("note-a", &["unselected".into()], 100)
        .is_err());
    assert_eq!(policy, before);
}
