// v2.22.35 - Finance restore and complete HTML export regression coverage.

fn transfer_finance_fixture() -> (String, String) {
    let initial = app_state_with_note("backup-note", "keep this note", "keep this body", None);
    let current = json!({"dailyLedgers":{"2026-09-08":{
        "incomes":[{"id":"same-row","name":"current salary","kind":"ACTIVE","amount":100}],
        "expenses":[],"note":"current day"
    }}});
    let initial =
        app_data::update_finance_profile_app_data_json(&initial, &current.to_string(), 100)
            .unwrap();
    let imported = json!({
        "cashReserve":500,
        "dailyLedgers":{
            "2026-09-08":{"incomes":[
                {"id":"same-row","name":"older salary","kind":"ACTIVE","amount":900},
                {"id":"backup-only","name":"side income","kind":"ACTIVE","amount":300}
            ],"expenses":[]},
            "2026-09-09":{"incomes":[],"expenses":[],"note":"new day"}
        },
        "monthlySnapshots":{"2026-09":{"assets":[],"liabilities":[],"note":"zero snapshot"}}
    });
    let backup = gridtimer_native::encode_finance_backup_json(
        &imported.to_string(),
        WINDOWS_CLIENT_VERSION,
        200,
    )
    .unwrap();
    (initial, backup)
}

#[test]
fn transfer_finance_shared_roundtrip_keeps_current_ids_and_other_domains() {
    let (initial, backup) = transfer_finance_fixture();
    let imported = validate_finance_import(&backup).unwrap();
    let (_, summary) = finance_import_preview(&initial, &imported, 300).unwrap();
    assert_eq!(summary.added_days, 1);
    assert_eq!(summary.added_months, 1);
    assert_eq!(summary.added_rows, 1);
    let merged = app_data::merge_finance_profile_app_data_json(&initial, &imported, 300).unwrap();
    let before: Value = serde_json::from_str(&initial).unwrap();
    let after: Value = serde_json::from_str(&merged).unwrap();
    for key in [
        "notes",
        "slots",
        "sessions",
        "categories",
        "themeMode",
        "archivedTasks",
    ] {
        assert_eq!(before.get(key), after.get(key), "restore changed {key}");
    }
    let rows = after
        .pointer("/financeProfile/dailyLedgers/2026-09-08/incomes")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        rows.iter().find(|row| row["id"] == "same-row").unwrap()["amount"],
        100
    );
    assert_eq!(
        rows.iter().find(|row| row["id"] == "backup-only").unwrap()["amount"],
        300
    );
    assert_eq!(
        after
            .pointer("/financeProfile/monthlySnapshots/2026-09/note")
            .unwrap(),
        "zero snapshot"
    );
    let exported = gridtimer_native::encode_finance_backup_json(
        &after["financeProfile"].to_string(),
        WINDOWS_CLIENT_VERSION,
        400,
    )
    .unwrap();
    let decoded: Value =
        serde_json::from_str(&validate_finance_import(&exported).unwrap()).unwrap();
    assert_eq!(decoded, after["financeProfile"]);
}

#[test]
fn transfer_finance_import_rejects_missing_payload_invalid_dates_and_limits() {
    assert!(validate_finance_import(r#"{"schemaVersion":1,"appVersionName":"2"}"#).is_err());
    assert!(validate_finance_import(r#"{"schemaVersion":999,"financeProfile":{}}"#).is_err());
    assert!(validate_finance_import(r#"{"dailyLedgers":{"2026-02-29":{}}}"#).is_err());
    assert!(validate_finance_import(r#"{"monthlySnapshots":{"2026-13":{}}}"#).is_err());
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(validate_finance_import(&deep).is_err());
    assert!(validate_finance_import(&" ".repeat(FINANCE_IMPORT_MAX_BYTES + 1)).is_err());
    let too_many = json!({"dailyLedgers":{"2026-09-08":{"incomes":vec![json!({});50_001]}}});
    assert!(validate_finance_import(&too_many.to_string()).is_err());
}

fn transfer_prepare_finance_preview(
    client: &TimerWindowsClient,
    profile_json: String,
) -> FinanceImportPreview {
    let (current_profile, summary) =
        finance_import_preview(&client.state_json, &profile_json, now_millis()).unwrap();
    FinanceImportPreview {
        source_path: PathBuf::from("finance_backup.json"),
        workspace_path: client.state_path.clone(),
        profile_json,
        current_profile,
        base_version: client.data_version,
        summary,
    }
}

#[test]
fn transfer_finance_confirmation_saves_verified_recovery_copy_before_merge() {
    let dir = temp_test_dir("finance_transfer_confirm");
    let (initial, backup) = transfer_finance_fixture();
    let mut client = test_client_for_account_scope(&dir, "transfer", initial.clone());
    let imported = validate_finance_import(&backup).unwrap();
    client.transfers.pending_finance = Some(transfer_prepare_finance_preview(&client, imported));
    client.confirm_finance_import();
    assert!(
        client.transfers.pending_finance.is_none(),
        "{}",
        client.status
    );
    let saved_backup = client.transfers.last_finance_export.as_ref().unwrap();
    let recovered: Value =
        serde_json::from_str(&read_finance_import(saved_backup).unwrap()).unwrap();
    let before: Value = serde_json::from_str(&initial).unwrap();
    assert_eq!(recovered, before["financeProfile"]);
    assert_eq!(client.data.notes[0].title, "keep this note");
    assert_eq!(
        finance_period_count(
            &serde_json::to_value(&client.data.finance_profile).unwrap(),
            "dailyLedgers"
        ),
        2
    );
    assert_eq!(
        fs::read_to_string(&client.state_path).unwrap(),
        client.state_json
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn transfer_finance_confirmation_rechecks_newer_draft_before_writing() {
    let dir = temp_test_dir("finance_transfer_changed_preview");
    let (initial, backup) = transfer_finance_fixture();
    let mut client = test_client_for_account_scope(&dir, "transfer", initial);
    let imported = validate_finance_import(&backup).unwrap();
    client.transfers.pending_finance = Some(transfer_prepare_finance_preview(&client, imported));
    client.finance_draft.cash_reserve = 777;
    client.mark_finance_dirty();
    client.confirm_finance_import();
    assert!(client.transfers.pending_finance.is_some());
    assert!(client.transfers.last_finance_export.is_none());
    assert_eq!(client.data.finance_profile.cash_reserve, 777);
    assert_eq!(
        finance_period_count(
            &serde_json::to_value(&client.data.finance_profile).unwrap(),
            "dailyLedgers"
        ),
        1
    );
    client.confirm_finance_import();
    assert!(
        client.transfers.pending_finance.is_none(),
        "{}",
        client.status
    );
    assert_eq!(client.data.finance_profile.cash_reserve, 777);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn transfer_finance_backup_failure_never_merges_or_changes_other_data() {
    let dir = temp_test_dir("finance_transfer_backup_failure");
    let (initial, backup) = transfer_finance_fixture();
    let mut client = test_client_for_account_scope(&dir, "transfer", initial.clone());
    let blocker = client.state_path.parent().unwrap().join("finance_backups");
    fs::write(&blocker, b"not a directory").unwrap();
    let imported = validate_finance_import(&backup).unwrap();
    client.transfers.pending_finance = Some(transfer_prepare_finance_preview(&client, imported));
    client.confirm_finance_import();
    assert!(client.transfers.pending_finance.is_some());
    assert_eq!(client.state_json, initial);
    assert_eq!(fs::read_to_string(&client.state_path).unwrap(), initial);
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn transfer_note_html_preserves_text_contact_call_and_inline_images() {
    use gridtimer_native::desktop_note_media::{
        DesktopNoteMediaStore, ExistingBoundNoteMediaProbe, ReadOnlyDesktopNoteMediaStore,
    };
    let dir = temp_test_dir("complete_note_html");
    let image = test_one_pixel_bmp();
    let (state, attachment) = test_note_snapshot_with_media("export-image", &image);
    let mut note = decode_data(&state).notes.remove(0);
    note.document.rich_text_enabled = true;
    note.document.blocks.insert(0, DesktopNoteBlock {
        block_type: "TEXT".into(),
        text: "<p><strong>formatted body</strong></p><img src=\"note-image://export-image\"><script>unsafe()</script>".into(),
        ..Default::default()
    });
    note.document.blocks.push(DesktopNoteBlock {
        block_type: "CONTACT".into(),
        contact_name: "Export Contact".into(),
        contact_organization: "Contact Company".into(),
        contact_phones: vec![json!({"label":"work","number":"123456"})],
        ..Default::default()
    });
    note.document.blocks.push(DesktopNoteBlock {
        block_type: "CALL".into(),
        call_contact_name: "Call Contact".into(),
        call_phone_number: "987654".into(),
        call_direction: "INCOMING".into(),
        text: "call notes".into(),
        ..Default::default()
    });
    let store = DesktopNoteMediaStore::new_bound(&dir, "export-workspace").unwrap();
    store
        .write_download_blob(
            &attachment.id,
            &attachment.sha256,
            attachment.size_bytes,
            &attachment.mime_type,
            100,
            &image,
        )
        .unwrap();
    let ExistingBoundNoteMediaProbe::Present(reader) =
        ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&dir, "export-workspace").unwrap()
    else {
        panic!("missing store")
    };
    let html = complete_note_html(&note, "Folder", |item| {
        reader
            .read_blob(&item.id, &item.sha256, item.size_bytes)
            .map_err(|error| error.to_string())
    })
    .unwrap();
    for text in [
        "<strong>formatted body</strong>",
        "Export Contact",
        "Contact Company",
        "123456",
        "Call Contact",
        "987654",
        "call notes",
        "data:image/bmp;base64,",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(!html.contains("<script"));
    assert!(!html.contains("src=\"note-image://"));
    assert!(html.contains("default-src 'none'"));
    assert!(
        ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&dir, "another-workspace").is_err()
    );
    drop(reader);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn transfer_note_html_rejects_missing_corrupt_and_unsafe_image_references() {
    let image = test_one_pixel_bmp();
    let (state, _) = test_note_snapshot_with_media("export-image", &image);
    let mut note = decode_data(&state).notes.remove(0);
    assert!(complete_note_html(&note, "", |_| Err("missing image".into())).is_err());
    assert!(complete_note_html(&note, "", |_| Ok(vec![0; image.len()])).is_err());
    note.document.blocks = vec![DesktopNoteBlock {
        block_type: "IMAGE".into(),
        attachment_id: Some("../../secret".into()),
        ..Default::default()
    }];
    assert!(complete_note_html(&note, "", |_| panic!("unsafe path reached reader")).is_err());
}

#[test]
fn transfer_note_html_embeds_prefix_ids_without_rewriting_other_images() {
    let image = test_one_pixel_bmp();
    let (state, _) =
        test_note_snapshot_with_media_items(&[("image", &image), ("image-long", &image)]);
    let note = decode_data(&state).notes.remove(0);
    let html = complete_note_html(&note, "", |_| Ok(image.clone())).unwrap();
    assert_eq!(html.matches("src=\"data:image/bmp;base64,").count(), 2);
    assert!(!html.contains("src=\"note-image://"));
    assert!(!html.contains("-long\""));
}

#[cfg(target_os = "windows")]
#[test]
fn transfer_verified_export_handles_long_unicode_windows_paths() {
    let dir = temp_test_dir("long_export");
    let path = dir
        .join("a".repeat(110))
        .join("b".repeat(110))
        .join("report_记录.json");
    assert!(path.as_os_str().encode_wide().count() > 280);
    write_transfer_text_verified(&path, "{\"value\":1}").unwrap();
    write_transfer_text_verified(&path, "{\"value\":2}").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"value\":2}");
    fs::remove_dir_all(dir).unwrap();
}
