#[test]
fn note_list_cache_selection_keeps_complete_document_and_recovery_history() {
    let root = temp_test_dir("note_list_full_source");
    let mut client = knowledge_test_client(&root);
    let note = client
        .data
        .notes
        .iter_mut()
        .find(|n| n.id == "doc-b")
        .unwrap();
    note.document.blocks = (0..20)
        .map(|n| new_desktop_text_block(&format!("正文 {n} {}", "内容".repeat(100))))
        .collect();
    note.revisions = (0..6)
        .map(|n| DesktopNoteRevisionSnapshot {
            id: format!("recovery-{n}"),
            document: note.document.clone(),
            ..Default::default()
        })
        .collect();
    let expected_blocks = note.document.blocks.clone();
    client.data.note_preferences.selected_folder_id = None;
    client.data_version += 1;
    client.rebuild_view_cache();
    let row = client
        .view_cache
        .notes
        .iter()
        .find(|n| n.id == "doc-b")
        .unwrap();
    assert!(row.metadata_prefix.contains("6"));
    assert!(row.body_preview.chars().count() <= 55);
    let id = row.id.clone();
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == id)
        .unwrap()
        .title = "Updated source title".into();
    client.select_note_by_id(&id);
    assert_eq!(client.note_title_draft, "Updated source title");
    assert_eq!(client.note_blocks_draft, expected_blocks);
    assert_eq!(client.selected_note_ref().unwrap().revisions.len(), 6);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn note_list_cache_locked_content_is_not_searchable_or_retained_in_row() {
    let root = temp_test_dir("note_list_locked");
    let mut client = knowledge_test_client(&root);
    client.clear_note_draft_without_flush();
    let note = client
        .data
        .notes
        .iter_mut()
        .find(|n| n.id == "doc-a")
        .unwrap();
    note.content = "private-token".into();
    note.document.blocks = vec![new_desktop_text_block("private-token")];
    note.encryption = Some(Default::default());
    client.note_unlocked_record = None;
    client.data.note_preferences.selected_folder_id = None;
    client.data_version += 1;
    client.note_search_draft = "private-token".into();
    client.rebuild_view_cache();
    assert!(client.view_cache.notes.is_empty());
    client.note_search_draft.clear();
    client.rebuild_view_cache();
    let row = client
        .view_cache
        .notes
        .iter()
        .find(|n| n.id == "doc-a")
        .unwrap();
    assert!(row.encrypted);
    assert!(row.body_preview.is_empty());
    assert!(row.search_snippet.is_empty());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn note_list_cache_reuses_rows_until_source_or_search_changes() {
    let root = temp_test_dir("note_list_invalidation");
    let mut client = knowledge_test_client(&root);
    client.data.note_preferences.selected_folder_id = None;
    client.rebuild_view_cache();
    let initial = client.view_cache.notes.clone();
    client.rebuild_view_cache();
    assert!(Arc::ptr_eq(&initial, &client.view_cache.notes));
    client.note_search_draft = "第二份资料".into();
    client.rebuild_view_cache();
    assert_eq!(client.view_cache.notes.len(), 1);
    assert_eq!(client.view_cache.notes[0].id, "doc-b");
    assert!(client.view_cache.notes[0]
        .search_snippet
        .contains("第二份资料"));
    client
        .data
        .notes
        .iter_mut()
        .find(|n| n.id == "doc-b")
        .unwrap()
        .document
        .blocks[0]
        .text = "改后的正文".into();
    client.data_version += 1;
    client.rebuild_view_cache();
    assert!(client.view_cache.notes.is_empty());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn note_list_cache_unlock_and_relock_change_visible_content() {
    let root = temp_test_dir("note_list_unlock_relock");
    let mut client = knowledge_test_client(&root);
    client.clear_note_draft_without_flush();
    client.data.note_preferences.selected_folder_id = None;
    let note = client
        .data
        .notes
        .iter_mut()
        .find(|n| n.id == "doc-a")
        .unwrap();
    note.encryption = Some(Default::default());
    let mut unlocked = note.clone();
    unlocked.title = "解锁后的标题".into();
    unlocked.document.blocks = vec![new_desktop_text_block("unlocked-token")];
    client.note_unlocked_record = Some(unlocked);
    client.note_search_draft = "unlocked-token".into();
    client.data_version += 1;
    client.rebuild_view_cache();
    assert_eq!(client.view_cache.notes.len(), 1);
    assert_eq!(client.view_cache.notes[0].row_title, "解锁后的标题");
    assert!(client.view_cache.notes[0]
        .body_preview
        .contains("unlocked-token"));
    client.close_note_crypto_session();
    client.rebuild_view_cache();
    assert!(client.view_cache.notes.is_empty());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}
