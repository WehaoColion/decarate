// Windows - Keep independent document work available while money input is unfinished.

fn document_money_boundary_client(root: &Path) -> TimerWindowsClient {
    let mut client = money_test_client(root);
    let now = now_millis();
    let note = desktop_note_save_value(
        "document-money-boundary",
        DesktopNoteKind::Document,
        "Document before edit",
        "Document body",
        None,
        now,
    );
    assert!(client.replace_state(
        app_data::upsert_note_app_data_json(&client.state_json, &note.to_string(), now),
        "fixture",
    ));
    let ctx = workspace_test_context();
    money_test_frame(&mut client, &ctx, vec![]);
    money_test_replace(&mut client, &ctx, &money_test_field(false), "58.901");
    client.finance_draft.extra.get_mut("dailyLedgers").unwrap()["2026-10-02"]["expenses"][0]
        ["name"] = json!("Unfinished finance draft");
    client.mark_finance_dirty();
    client.switch_tab(AppTab::Knowledge);
    client.select_note_by_id("document-money-boundary");
    client
}

fn assert_document_boundary_retains_money(client: &mut TimerWindowsClient) {
    assert!(!client.finance_money_inputs_ready());
    assert_eq!(
        client.finance_workbench.money_drafts.fields[&money_test_field(false)].text,
        "58.901"
    );
    assert!(client.finance_dirty);
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        money_test_field(false).source(&disk.finance_profile),
        Some((58, Some(5890)))
    );
    assert_eq!(
        disk.finance_profile.extra["dailyLedgers"]["2026-10-02"]["expenses"][0]["name"],
        "subscription"
    );
    assert!(client.flush_all_pending_saves().is_err());
}

#[test]
fn document_money_boundary_opens_reading_and_rich_editors_and_exports_saved_document() {
    let root = temp_test_dir("document_money_boundary_read_edit");
    let mut client = document_money_boundary_client(&root);
    client.note_title_draft = "Document saved with unfinished finance".into();
    client.mark_note_dirty();

    let exported = client.note_sharing_snapshot().unwrap();
    assert_eq!(exported.title, "Document saved with unfinished finance");
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        disk.notes
            .iter()
            .find(|note| note.id == exported.id)
            .unwrap()
            .title,
        exported.title
    );
    assert_document_boundary_retains_money(&mut client);

    client.open_knowledge_reading_view(None);
    assert!(client.rich_editor.active);
    assert!(client.rich_editor.read_only);
    assert_eq!(client.rich_editor.note_id, exported.id);
    client.rich_editor = DesktopRichEditor::default();
    client.open_rich_editor();
    assert!(client.rich_editor.active);
    assert!(!client.rich_editor.read_only);
    assert_eq!(client.rich_editor.note_id, exported.id);
    client.rich_editor = DesktopRichEditor::default();
    assert_document_boundary_retains_money(&mut client);

    // Restoring the valid amount still persists the revoked completeness check.
    client.finance_workbench.money_drafts.cancel_invalid();
    client.flush_finance_draft().unwrap();
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    assert_eq!(
        disk.finance_profile.extra["dailyLedgers"]["2026-10-02"]["confirmedAtEpochMillis"],
        0
    );
    assert_eq!(
        money_test_field(false).source(&disk.finance_profile),
        Some((58, Some(5890)))
    );
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn document_money_boundary_saves_transcript_without_committing_unfinished_finance() {
    let root = temp_test_dir("document_money_boundary_transcript");
    let mut client = document_money_boundary_client(&root);
    client.speech_transcription.preview = Some(SpeechTranscriptionPreview {
        origin: client.ai_workspace_identity(),
        title: "Reviewed transcript".into(),
        text: "Reviewed words to preserve".into(),
        duration_millis: 5_000,
    });
    client.speech_transcription.preview_open = true;
    assert!(client.save_speech_transcription_preview());
    assert!(client.speech_transcription.preview.is_none());
    let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
    let saved = disk
        .notes
        .iter()
        .find(|note| note.title == "Reviewed transcript")
        .unwrap();
    assert!(saved.content.contains("Reviewed words to preserve"));
    assert_document_boundary_retains_money(&mut client);
    client.close_with_save_barrier(&workspace_test_context());
    assert!(matches!(
        client.shutdown_state,
        ClientShutdownState::SaveFailed { .. }
    ));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn document_money_boundary_restores_documents_and_sticky_notes_without_finance_commit() {
    for document in [true, false] {
        let root = temp_test_dir("document_money_boundary_restore");
        let mut client = document_money_boundary_client(&root);
        let id = if document {
            "document-money-boundary"
        } else {
            "money-note"
        };
        client.select_note_by_id(id);
        client.delete_note();
        assert!(client
            .data
            .notes
            .iter()
            .find(|note| note.id == id)
            .unwrap()
            .deleted_at_epoch_millis
            .is_some());
        if document {
            client.restore_knowledge_note(id);
        } else {
            client.restore_sticky_note(id);
        }
        let disk = decode_data(&fs::read_to_string(&client.state_path).unwrap());
        assert!(disk
            .notes
            .iter()
            .find(|note| note.id == id)
            .unwrap()
            .deleted_at_epoch_millis
            .is_none());
        assert_document_boundary_retains_money(&mut client);
        drop(client);
        fs::remove_dir_all(root).unwrap();
    }
}
