// v1.1.0.2 Windows - Check cached content survives unrelated saves and changes with its source.
#[test]
fn content_domain_receipts_keep_notes_hot_and_invalidate_edited_content() {
    let dir = temp_test_dir("content_domain_receipts");
    let mut client = knowledge_test_client(&dir);
    client.rebuild_view_cache();
    let notes = Arc::clone(&client.view_cache.notes);
    let navigation = client.knowledge_navigation();
    let index = client.knowledge_note_indices();
    let finance_version = client.finance_cache_version();
    client.ensure_selected_slot_draft();
    client.slot_title_draft = "updated timer title".into();
    client.mark_slot_dirty();
    client.submit_draft_snapshot(&egui::Context::default(), true);
    drain_draft_writer(&mut client);
    assert!(!client.slot_dirty, "{}", client.status);
    assert!(Arc::ptr_eq(&notes, &client.view_cache.notes));
    assert!(Arc::ptr_eq(&navigation, &client.knowledge_navigation()));
    assert!(Arc::ptr_eq(&index, &client.knowledge_note_indices()));
    assert_eq!(finance_version, client.finance_cache_version());

    // Use the production apply entry point. A changed note must rebuild both
    // navigation labels and canvas IDs, while finance retains its generation.
    let mut value: Value = serde_json::from_str(&client.state_json).unwrap();
    value["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["id"] == "doc-a")
        .unwrap()["title"] = json!("changed page");
    assert!(client.replace_state(Some(value.to_string()), "updated"));
    let new_navigation = client.knowledge_navigation();
    assert!(!Arc::ptr_eq(&navigation, &new_navigation));
    assert!(!Arc::ptr_eq(&index, &client.knowledge_note_indices()));
    assert!(new_navigation
        .pages
        .iter()
        .any(|p| p.id == "doc-a" && p.title == "changed page"));
    assert_eq!(finance_version, client.finance_cache_version());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn content_domains_include_privacy_directories_and_complete_financial_values() {
    let before = decode_data(&knowledge_fixture_state());
    let mut after = before.clone();
    after.note_folders.push(DesktopNoteFolder {
        id: "new-directory".into(),
        ..Default::default()
    });
    assert!(DesktopContentChanges::between(&before, &after).notes);
    after = before.clone();
    after.notes[0].encryption = Some(DesktopNoteEncryptionEnvelope::default());
    assert!(DesktopContentChanges::between(&before, &after).notes);
    after = before.clone();
    after
        .finance_profile
        .extra
        .insert("futureFinancialField".into(), json!({"balance": 7}));
    let changes = DesktopContentChanges::between(&before, &after);
    assert!(changes.finance);
    assert!(!changes.notes);
    let mut versions = DesktopContentVersions::default();
    versions.record(1, changes);
    assert_eq!(versions.finance, 1);
    assert_eq!(versions.notes, 0);
    assert_eq!(
        versions.domain(2, versions.notes),
        2,
        "unclassified replacement must invalidate conservatively"
    );
    versions.reset(3);
    assert_eq!(
        (versions.notes, versions.finance, versions.timers),
        (3, 3, 3)
    );
}
