// Real restore transactions: byte corruption stays atomic; deduplicated content
// still restores every attachment identity independently.
fn media_restore_json(entries: &[(&str, &[u8])]) -> String {
    let attachments = entries
        .iter()
        .map(|(id, content)| {
            json!({
                "id": id,
                "sha256": sha256_hex(content),
                "mimeType": "application/octet-stream",
                "sizeBytes": content.len(),
                "updatedAtEpochMillis": 100
            })
        })
        .collect::<Vec<_>>();
    json!({"notes": [{"id": "restore-note", "attachments": attachments}]}).to_string()
}

fn media_restore_upload(store: &SqliteServerStore, id: &str, content: &[u8]) {
    assert!(matches!(
        store
            .upsert_media(
                "user-a",
                id,
                &sha256_hex(content),
                "application/octet-stream",
                content.len() as i64,
                content,
                101,
            )
            .unwrap(),
        MediaUpsertOutcome::Stored(_)
    ));
}

#[test]
fn media_restore_late_corrupt_blob_preserves_account_tokens_and_live_bytes() {
    let directory = TestDirectory::new("media-restore-late-corruption");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let first = b"valid-first-historical-content";
    let last = b"valid-last-historical-content";
    let historical = media_restore_json(&[("a-first", first), ("z-last", last)]);
    store
        .compare_and_swap_account("user-a", 0, &historical, 100)
        .unwrap();
    media_restore_upload(&store, "a-first", first);
    media_restore_upload(&store, "z-last", last);
    let current_bytes = b"current-content-must-survive-a-failed-restore";
    let current = json_with_attachment("current-file", current_bytes);
    store
        .compare_and_swap_account("user-a", 1, &current, 200)
        .unwrap();
    media_restore_upload(&store, "current-file", current_bytes);
    for id in ["a-first", "z-last"] {
        store.delete_media("user-a", id, 300, 300).unwrap();
    }
    assert_eq!(
        2,
        store
            .prune_deleted_media_content(301 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    let token = store
        .issue_token("user-a", "streamed-restore", "test-device", 400, 10_000)
        .unwrap();
    let receipt = store.ensure_restore_receipt("user-a", token.id).unwrap();
    store
        .acknowledge_restore_generation(
            "user-a",
            token.id,
            receipt.current_generation,
            &receipt.receipt,
        )
        .unwrap();
    let connection = store.open_connection(false).unwrap();
    // Prove the actual retained revision is complete before damaging its last
    // sorted attachment, so NotFound or an incomplete fixture cannot pass.
    load_complete_media_snapshot(&connection, "user-a", 1, &historical).unwrap();
    let mut damaged = last.to_vec();
    damaged[0] ^= 1;
    assert_eq!(
        1,
        connection
            .execute(
                "UPDATE media_snapshot_contents SET content=?1 WHERE sha256=?2",
                params![damaged, sha256_hex(last)],
            )
            .unwrap()
    );
    drop(connection);
    let account_before = store.read_account("user-a").unwrap();
    let state_before = opaque_restore_atomic_state(&store);
    let storage_before = store.snapshot_storage_stats("user-a").unwrap();
    let live_before = store.read_media("user-a", "current-file").unwrap();
    let metadata_before = store.list_media_metadata("user-a", true).unwrap();
    assert!(matches!(
        store.restore_account_snapshot("user-a", 1, 2, 500),
        Err(StoreError::Integrity(_))
    ));
    assert_eq!(account_before, store.read_account("user-a").unwrap());
    assert_eq!(state_before, opaque_restore_atomic_state(&store));
    assert_eq!(
        storage_before,
        store.snapshot_storage_stats("user-a").unwrap()
    );
    assert_eq!(
        live_before,
        store.read_media("user-a", "current-file").unwrap()
    );
    assert_eq!(
        metadata_before,
        store.list_media_metadata("user-a", true).unwrap()
    );
    for id in ["a-first", "z-last"] {
        assert!(store.read_media("user-a", id).unwrap().is_none());
    }

    // Repair the byte evidence, then inject a later read failure after the
    // first attachment is written. The transaction must roll back that write
    // as well as account/generation changes and the injected history change.
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "UPDATE media_snapshot_contents SET content=?1 WHERE sha256=?2",
            params![last.as_slice(), sha256_hex(last)],
        )
        .unwrap();
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER damage_late_restore_source AFTER INSERT ON note_media
             WHEN NEW.user_id='user-a' AND NEW.attachment_id='a-first'
             BEGIN
               UPDATE media_snapshot_contents SET content=zeroblob(size_bytes)
               WHERE sha256='{}';
             END;",
            sha256_hex(last)
        ))
        .unwrap();
    drop(connection);
    assert!(matches!(
        store.restore_account_snapshot("user-a", 1, 2, 501),
        Err(StoreError::Integrity(_))
    ));
    assert_eq!(account_before, store.read_account("user-a").unwrap());
    assert_eq!(state_before, opaque_restore_atomic_state(&store));
    assert_eq!(
        storage_before,
        store.snapshot_storage_stats("user-a").unwrap()
    );
    assert_eq!(
        live_before,
        store.read_media("user-a", "current-file").unwrap()
    );
    assert_eq!(
        metadata_before,
        store.list_media_metadata("user-a", true).unwrap()
    );
    let connection = store.open_connection(false).unwrap();
    assert_eq!(
        last.as_slice(),
        connection
            .query_row(
                "SELECT content FROM media_snapshot_contents WHERE sha256=?1",
                [sha256_hex(last)],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap()
    );
    connection
        .execute_batch("DROP TRIGGER damage_late_restore_source;")
        .unwrap();
    drop(connection);

    // Remove only the injected failure. The same restore must now work.
    let restored = store.restore_account_snapshot("user-a", 1, 2, 501).unwrap();
    assert_eq!(historical, restored.app_data_json);
    for (id, bytes) in [("a-first", first.as_slice()), ("z-last", last.as_slice())] {
        assert_eq!(
            bytes,
            store.read_media("user-a", id).unwrap().unwrap().content
        );
    }
    assert!(store
        .read_media("user-a", "current-file")
        .unwrap()
        .is_none());
    store.validate_integrity().unwrap();
}

fn media_restore_shared_hash_case(item_count: usize, content_size: usize, profile: bool) {
    let directory = TestDirectory::new("media-restore-shared-hash");
    let store = open_empty(&directory);
    store
        .create_user(new_user("user-a", "a@example.test"))
        .unwrap();
    let content = vec![0x5a; content_size];
    let ids = (0..item_count)
        .map(|index| format!("shared-{index:03}"))
        .collect::<Vec<_>>();
    let references = ids
        .iter()
        .map(|id| (id.as_str(), content.as_slice()))
        .collect::<Vec<_>>();
    let historical = media_restore_json(&references);
    store
        .compare_and_swap_account("user-a", 0, &historical, 100)
        .unwrap();
    for id in &ids {
        media_restore_upload(&store, id, &content);
    }
    store
        .compare_and_swap_account("user-a", 1, r#"{"notes":[]}"#, 200)
        .unwrap();
    for id in &ids {
        store.delete_media("user-a", id, 300, 300).unwrap();
    }
    assert_eq!(
        item_count,
        store
            .prune_deleted_media_content(301 + DELETED_MEDIA_CONTENT_RETENTION_MILLIS)
            .unwrap()
    );
    let connection = store.open_connection(false).unwrap();
    assert_eq!(
        (item_count as i64, 1_i64),
        connection
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT content_sha256) FROM account_snapshot_media_history WHERE user_id='user-a' AND account_revision=1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap()
    );
    drop(connection);
    for id in &ids {
        assert!(store.read_media("user-a", id).unwrap().is_none());
    }
    let reopened = SqliteServerStore::open(store.database_path(), None).unwrap();
    if profile {
        eprintln!(
            "MEDIA_RESTORE_MEASUREMENT_BEGIN pid={} items={} content_bytes={} logical_bytes={}",
            std::process::id(),
            item_count,
            content_size,
            item_count * content_size
        );
    }
    let started = std::time::Instant::now();
    let restored = reopened
        .restore_account_snapshot("user-a", 1, 2, 500)
        .unwrap();
    if profile {
        eprintln!(
            "MEDIA_RESTORE_MEASUREMENT_END elapsed_millis={}",
            started.elapsed().as_millis()
        );
    }
    assert_eq!(historical, restored.app_data_json);
    assert_eq!(
        item_count,
        reopened.list_media_metadata("user-a", false).unwrap().len()
    );
    for id in &ids {
        let media = reopened.read_media("user-a", id).unwrap().unwrap();
        assert_eq!(id, &media.metadata.attachment_id);
        assert_eq!(content, media.content);
        assert_eq!(100, media.metadata.updated_at_epoch_millis);
        assert!(media.metadata.deleted_at_epoch_millis.is_none());
    }
    reopened.validate_integrity().unwrap();
}

#[test]
fn media_restore_shared_hash_restores_each_identity_after_gc_and_reopen() {
    media_restore_shared_hash_case(8, 16 * 1024, false);
}

#[test]
#[ignore = "Run alone with external process-memory sampling and --nocapture"]
fn media_restore_shared_hash_memory_profile() {
    media_restore_shared_hash_case(32, 2 * 1024 * 1024, true);
}
