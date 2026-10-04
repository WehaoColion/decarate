// v0.0.3 - Restore blocking accepted sockets before the synchronous HTTP test handler.
// v0.0.2 - Check content conflicts, missing bytes and retained deletion intent.
// v0.0.1 - Reproduce real encrypted attachment upload and download omissions.
#[test]
fn encrypted_private_bytes_respect_local_attachment_deletion_before_upload() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    for kind in ["noteAttachment", "noteMedia"] {
        let (snapshot, policy, bytes, sha) = encrypted_byte_fixture();
        let mut snapshot: Value = serde_json::from_str(&snapshot).unwrap();
        snapshot["tombstones"] =
            json!([{"entityType":kind,"entityId":"cipher-file","deletedAtEpochMillis":300}]);
        let token = "private-local-deletion";
        let account = account_test_store(token, app_data::default_app_data_json(100));
        let identity = account.store.server_account_identity("user-1").unwrap();
        let media = DesktopNoteMediaStore::new(account.directory.join("retained-local")).unwrap();
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
        let server = EncryptedByteServer::start(&account.store);
        let result = sync(
            &media,
            &snapshot.to_string(),
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
        assert_eq!(
            result.uploaded, 0,
            "{kind}: deletion must not revive bytes: {result:?}"
        );
        assert_eq!(result.conflicts, 1, "{kind}: {result:?}");
        assert_eq!(
            result.deferred_reference_deletions,
            usize::from(kind == "noteMedia")
        );
        assert!(result.remote_deleted_at_by_attachment_id.is_empty());
        assert!(account
            .store
            .list_media_metadata("user-1", true)
            .unwrap()
            .is_empty());
        assert_eq!(
            media
                .read_blob("cipher-file", &sha, bytes.len() as i64)
                .unwrap(),
            bytes
        );
    }
}

#[test]
fn encrypted_private_bytes_missing_or_conflicting_remote_content_stays_incomplete() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    for wrong_content in [false, true] {
        let (snapshot, policy, mut bytes, sha) = encrypted_byte_fixture();
        let token = "private-remote-content";
        let account = account_test_store(token, snapshot.clone());
        let identity = account.store.server_account_identity("user-1").unwrap();
        let server = EncryptedByteServer::start(&account.store);
        if wrong_content {
            bytes[54] ^= 0xff;
            let uploaded = desktop_media_upload(
                &server.url,
                "user-1",
                token,
                &identity.server_instance_id,
                &identity.account_namespace,
                0,
                "",
                "cipher-file",
                &hex_bytes(&Sha256::digest(&bytes)),
                "image/bmp",
                bytes.len() as i64,
                100,
                &bytes,
                false,
            );
            assert!(uploaded.ok, "{}", uploaded.message);
        }
        let published = desktop_private_media_exchange(
            &server.url,
            "user-1",
            token,
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            policy.private_media_queries(&snapshot, true).unwrap(),
        );
        assert!(published.result.ok, "{}", published.result.message);
        let media = DesktopNoteMediaStore::new(account.directory.join("receiver")).unwrap();
        let result = sync(
            &media,
            &snapshot,
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
        assert_eq!(result.downloaded + result.uploaded, 0);
        assert_eq!(result.failures, usize::from(!wrong_content), "{result:?}");
        assert_eq!(result.conflicts, usize::from(wrong_content), "{result:?}");
        assert_eq!(result.legacy_missing, 0);
        assert!(result.resolved_sha256_by_attachment_id.is_empty());
        assert!(media
            .read_blob("cipher-file", &sha, bytes.len() as i64)
            .is_err());
    }
}

#[test]
fn encrypted_private_bytes_reject_same_id_with_different_local_content() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    let (snapshot, policy, mut bytes, _) = encrypted_byte_fixture();
    bytes[54] ^= 0xff;
    let wrong_hash = hex_bytes(&Sha256::digest(&bytes));
    let token = "encrypted-byte-mismatch";
    let account = account_test_store(token, app_data::default_app_data_json(100));
    let identity = account.store.server_account_identity("user-1").unwrap();
    let media = DesktopNoteMediaStore::new(account.directory.join("mismatched-local")).unwrap();
    media
        .write_download_blob(
            "cipher-file",
            &wrong_hash,
            bytes.len() as i64,
            "image/bmp",
            100,
            &bytes,
        )
        .unwrap();
    let server = EncryptedByteServer::start(&account.store);
    let result = sync(
        &media,
        &snapshot,
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
    assert_eq!(result.uploaded, 0);
    assert_eq!(
        result.conflicts, 1,
        "same ID must not authorize another file: {result:?}"
    );
    assert!(account
        .store
        .list_media_metadata("user-1", true)
        .unwrap()
        .is_empty());
    assert_eq!(
        media
            .read_blob("cipher-file", &wrong_hash, bytes.len() as i64)
            .unwrap(),
        bytes
    );
}

fn encrypted_byte_fixture() -> (
    String,
    crate::desktop_state_store::DesktopPrivacyPolicy,
    Vec<u8>,
    String,
) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&58_u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    bytes.resize(58, 0x7f);
    let sha = hex_bytes(&Sha256::digest(&bytes));
    let plain = app_data::upsert_note_app_data_json(&app_data::default_app_data_json(100),
        &json!({"id":"private-byte-note","title":"Encrypted fixture","content":"Private body",
            "attachments":[{"id":"cipher-file","kind":"IMAGE","mimeType":"image/bmp","sizeBytes":bytes.len(),
                "sha256":sha,"width":1,"height":1,"updatedAtEpochMillis":100}],
            "updatedAtEpochMillis":100}).to_string(),100).unwrap();
    let note = serde_json::from_str::<Value>(&plain).unwrap()["notes"][0].to_string();
    let (sealed, token) = crate::note_crypto::encrypt_note(&note, "private-byte-password").unwrap();
    let declaration = serde_json::from_str(
        &crate::note_crypto::session_media_references_json(&sealed, &token).unwrap(),
    )
    .unwrap();
    crate::note_crypto::close_session(&token);
    let note: Value = serde_json::from_str(&sealed).unwrap();
    assert_eq!(note["attachments"], json!([]));
    let policy = crate::desktop_state_store::DesktopPrivacyPolicy::default()
        .including_sealed_media(&note, declaration)
        .unwrap();
    let snapshot = app_data::upsert_note_app_data_json(&plain, &sealed, 200).unwrap();
    (snapshot, policy, bytes, sha)
}

struct EncryptedByteServer {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl EncryptedByteServer {
    fn start(store: &SqliteServerStore) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let store = Arc::new(store.clone());
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        handle_connection(
                            stream,
                            store.clone(),
                            Arc::new(Mutex::new(ServerRuntimeInfo::default())),
                            Arc::new(Mutex::new(LoginRateLimiter::default())),
                        )
                        .unwrap();
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("encrypted byte fixture: {error}"),
                }
            }
        });
        Self {
            url,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for EncryptedByteServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

#[test]
fn encrypted_private_bytes_upload_before_first_note_commit_and_download_to_another_device() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    let (snapshot, policy, bytes, sha) = encrypted_byte_fixture();
    let token = "encrypted-byte-upload";
    let account = account_test_store(token, app_data::default_app_data_json(100));
    let identity = account.store.server_account_identity("user-1").unwrap();
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
    let server = EncryptedByteServer::start(&account.store);
    let uploaded = sync(
        &sender,
        &snapshot,
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
    assert_eq!(
        uploaded.uploaded, 1,
        "the sealed note must upload its real attachment before publishing AppData: {uploaded:?}"
    );
    assert_eq!(uploaded.failures, 0, "{:?}", uploaded.failure_messages);
    account
        .store
        .compare_and_swap_account("user-1", 0, &snapshot, 200)
        .unwrap();
    let published = sync(
        &sender,
        &snapshot,
        &server.url,
        token,
        "user-1",
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        Direction::Bidirectional,
        Some(&policy),
    );
    assert_eq!(published.failures, 0, "{:?}", published.failure_messages);
    let receiver = DesktopNoteMediaStore::new(account.directory.join("receiver")).unwrap();
    let downloaded = sync(
        &receiver,
        &snapshot,
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
    assert_eq!(downloaded.downloaded, 1, "{downloaded:?}");
    assert_eq!(
        receiver
            .read_blob("cipher-file", &sha, bytes.len() as i64)
            .unwrap(),
        bytes
    );
    assert_eq!(downloaded.failures + downloaded.conflicts, 0);
}

#[test]
fn encrypted_private_bytes_download_uses_authenticated_ciphertext_content_binding() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    let (snapshot, policy, bytes, sha) = encrypted_byte_fixture();
    let token = "encrypted-byte-download";
    let account = account_test_store(token, snapshot.clone());
    let identity = account.store.server_account_identity("user-1").unwrap();
    let server = EncryptedByteServer::start(&account.store);
    let uploaded = desktop_media_upload(
        &server.url,
        "user-1",
        token,
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        "cipher-file",
        &sha,
        "image/bmp",
        bytes.len() as i64,
        100,
        &bytes,
        false,
    );
    assert!(uploaded.ok, "{}", uploaded.message);
    let published = desktop_private_media_exchange(
        &server.url,
        "user-1",
        token,
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        policy.private_media_queries(&snapshot, true).unwrap(),
    );
    assert!(published.result.ok, "{}", published.result.message);
    let receiver = DesktopNoteMediaStore::new(account.directory.join("receiver")).unwrap();
    let downloaded = sync(
        &receiver,
        &snapshot,
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
    assert_eq!(
        downloaded.downloaded, 1,
        "a locked receiving device must fetch the matching content: {downloaded:?}"
    );
    assert_eq!(
        receiver
            .read_blob("cipher-file", &sha, bytes.len() as i64)
            .unwrap(),
        bytes
    );
}

#[test]
fn encrypted_private_bytes_remote_deletion_neither_revives_nor_erases_retained_local_file() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_private_references as sync,
        DesktopNoteMediaSyncDirection as Direction,
    };
    let (snapshot, policy, bytes, sha) = encrypted_byte_fixture();
    let token = "private-remote-tombstone";
    let account = account_test_store(token, app_data::default_app_data_json(100));
    let identity = account.store.server_account_identity("user-1").unwrap();
    let server = EncryptedByteServer::start(&account.store);
    let uploaded = desktop_media_upload(
        &server.url,
        "user-1",
        token,
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        "cipher-file",
        &sha,
        "image/bmp",
        bytes.len() as i64,
        100,
        &bytes,
        false,
    );
    assert!(uploaded.ok, "{}", uploaded.message);
    let deleted = desktop_media_delete(
        &server.url,
        "user-1",
        token,
        &identity.server_instance_id,
        &identity.account_namespace,
        0,
        "",
        "cipher-file",
        300,
    );
    assert!(deleted.ok, "{}", deleted.message);
    let media = DesktopNoteMediaStore::new(account.directory.join("retained-local")).unwrap();
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
    for direction in [
        Direction::UploadOnly,
        Direction::DownloadOnly,
        Direction::Bidirectional,
    ] {
        let result = sync(
            &media,
            &snapshot,
            &server.url,
            token,
            "user-1",
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            direction,
            Some(&policy),
        );
        assert_eq!(result.conflicts, 1, "{direction:?}: {result:?}");
        assert_eq!(result.uploaded + result.downloaded, 0);
        assert!(result.remote_deleted_at_by_attachment_id.is_empty());
        assert_eq!(
            media
                .read_blob("cipher-file", &sha, bytes.len() as i64)
                .unwrap(),
            bytes
        );
        let remote = account.store.list_media_metadata("user-1", true).unwrap();
        assert_eq!(remote[0].deleted_at_epoch_millis, Some(300));
    }
}
