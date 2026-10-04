// Critical state coverage for the Windows UploadOnly attachment preflight.

#[test]
fn upload_preflight_restore_requires_current_reference_above_every_delete_floor() {
    let sha = hash('a');
    // current revision, history revision, attachment floor, media floor,
    // server floor, expected restore authorization.
    for (current, history, attachment_floor, media_floor, server_floor, allowed) in [
        (101, 0, 90, 100, 99, true),
        (101, 0, 101, 100, 99, false),
        (101, 0, 90, 101, 99, false),
        (101, 0, 90, 100, 101, false),
        (100, 101, 90, 100, 99, false),
        (0, 101, 90, 100, 99, false),
    ] {
        let app_data = MediaAppData {
            notes: vec![MediaNote {
                attachments: (current > 0)
                    .then(|| attachment("restored", &sha, 0, current))
                    .into_iter()
                    .collect(),
                revisions: (history > 0)
                    .then(|| HistoricalAttachments {
                        attachments: vec![attachment("restored", &sha, 0, history)],
                    })
                    .into_iter()
                    .collect(),
                ..MediaNote::default()
            }],
            tombstones: vec![
                MediaTombstone {
                    entity_type: NOTE_ATTACHMENT_TOMBSTONE.into(),
                    entity_id: "restored".into(),
                    deleted_at_epoch_millis: attachment_floor,
                },
                MediaTombstone {
                    entity_type: NOTE_MEDIA_TOMBSTONE.into(),
                    entity_id: "restored".into(),
                    deleted_at_epoch_millis: media_floor,
                },
            ],
        };
        let server = BTreeMap::from([(
            "restored".into(),
            server("restored", &sha, 50, server_floor),
        )]);
        let plan = build_reconcile_plan(&app_data, &server);
        let explicit = plan.explicitly_restored_attachment_ids.contains("restored");
        assert_eq!(allowed, explicit);
        // A newer cache write is not evidence of a user restoration.
        let cached = local("restored", &sha, 1_000_000);
        let action = decide_attachment_action(
            DesktopNoteMediaSyncDirection::UploadOnly,
            &plan.attachments[0],
            Some(&cached),
            server.get("restored"),
            plan.media_deletion_by_attachment_id
                .get("restored")
                .copied(),
            explicit,
        );
        assert_eq!(
            allowed,
            matches!(action, AttachmentAction::Upload { restore_deleted: true }),
            "current={current}, history={history}, floors={attachment_floor}/{media_floor}/{server_floor}: {action:?}"
        );
        if !allowed {
            assert!(!matches!(action, AttachmentAction::Upload { .. }));
        }
    }
}

/// A bounded synthetic transport peer. `None` closes the connection after
/// reading its request, exercising an interrupted upload without a replay.
fn restore_preflight_peer(
    responses: Vec<Option<SyncClientResult>>,
) -> (
    String,
    std::thread::JoinHandle<Vec<(String, serde_json::Value)>>,
) {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let worker = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("preflight peer did not accept request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header_end, content_length) = loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
                assert!(bytes.len() < 65_536, "oversized fixture request header");
                if bytes.ends_with(b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes.clone()).unwrap();
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (bytes.len(), content_length);
                }
            };
            assert!(content_length < 65_536, "oversized fixture request body");
            bytes.resize(header_end + content_length, 0);
            stream.read_exact(&mut bytes[header_end..]).unwrap();
            let path = String::from_utf8_lossy(&bytes[..header_end])
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            let body = serde_json::from_slice(&bytes[header_end..]).unwrap();
            requests.push((path, body));
            if let Some(response) = response {
                let body = serde_json::to_vec(&response).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        }
        requests
    });
    (url, worker)
}

#[test]
fn upload_preflight_restore_retries_after_transport_failure_then_reconciles() {
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!(
        "tenrate_media_restore_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = DesktopNoteMediaStore::new(&root).unwrap();
    let content = BASE64_STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jhAAAAABJRU5ErkJggg==").unwrap();
    let sha = format!("{:x}", Sha256::digest(&content));
    store
        .write_download_blob(
            "restored",
            &sha,
            content.len() as i64,
            "image/png",
            1_000_000,
            &content,
        )
        .unwrap();
    let app_data = serde_json::json!({
        "notes": [{"attachments": [{
            "id": "restored", "sha256": sha, "mimeType": "image/png",
            "sizeBytes": content.len(), "updatedAtEpochMillis": 101
        }]}],
        "tombstones": [
            {"entityType": "noteAttachment", "entityId": "restored", "deletedAtEpochMillis": 90},
            {"entityType": "noteMedia", "entityId": "restored", "deletedAtEpochMillis": 100}
        ]
    })
    .to_string();
    let identity = SyncClientResult {
        ok: true,
        user_id: "restore-user".into(),
        token_id: crate::sync_core::token_identifier("restore-token"),
        server_instance_id: "1".repeat(64),
        account_namespace: "2".repeat(64),
        ..SyncClientResult::default()
    };
    let mut deleted = server("restored", &sha, 50, 99);
    deleted.size_bytes = content.len() as i64;
    let mut active = deleted.clone();
    active.updated_at_epoch_millis = 101;
    active.deleted_at_epoch_millis = 0;
    let manifest = SyncClientResult {
        mode: "media_manifest".into(),
        media_items: vec![deleted],
        ..identity.clone()
    };
    let (url, worker) = restore_preflight_peer(vec![
        Some(manifest.clone()),
        None,
        Some(manifest),
        Some(SyncClientResult {
            mode: "media_upload".into(),
            media_item: Some(active.clone()),
            ..identity.clone()
        }),
        Some(SyncClientResult {
            mode: "media_manifest".into(),
            media_items: vec![active],
            ..identity.clone()
        }),
    ]);
    let reconcile = |direction| {
        sync_desktop_note_media_with_direction(
            &store,
            &app_data,
            &url,
            "restore-token",
            &identity.user_id,
            &identity.server_instance_id,
            &identity.account_namespace,
            0,
            "",
            direction,
        )
    };
    let failed = reconcile(DesktopNoteMediaSyncDirection::UploadOnly);
    assert_eq!(1, failed.failures, "{failed:?}");
    assert_eq!(0, failed.uploaded);
    assert!(failed.resolved_sha256_by_attachment_id.is_empty());
    assert!(failed.remote_deleted_at_by_attachment_id.is_empty());
    assert_eq!(
        content,
        store
            .read_blob("restored", &sha, content.len() as i64)
            .unwrap()
    );

    let retry = reconcile(DesktopNoteMediaSyncDirection::UploadOnly);
    assert_eq!(0, retry.failures, "{retry:?}");
    assert_eq!(0, retry.conflicts, "{retry:?}");
    assert_eq!(1, retry.uploaded);
    assert_eq!(
        Some(&sha),
        retry.resolved_sha256_by_attachment_id.get("restored")
    );
    assert!(retry.remote_deleted_at_by_attachment_id.is_empty());
    // Mirrors the media phase after the unchanged AppData can be published.
    let followup = reconcile(DesktopNoteMediaSyncDirection::Bidirectional);
    assert_eq!(0, followup.failures, "{followup:?}");
    assert_eq!(0, followup.conflicts, "{followup:?}");
    assert_eq!(0, followup.uploaded);
    assert_eq!(
        Some(&sha),
        followup.resolved_sha256_by_attachment_id.get("restored")
    );
    let requests = worker.join().unwrap();
    assert_eq!(5, requests.len(), "transport must not replay a write");
    for (index, (path, body)) in requests.iter().enumerate() {
        if index == 1 || index == 3 {
            assert_eq!("/v1/media/upload", path);
            assert_eq!(true, body["restoreDeleted"]);
            assert_eq!(101, body["updatedAtEpochMillis"]);
            assert_eq!("restored", body["attachmentId"]);
        } else {
            assert_eq!("/v1/media/manifest", path);
        }
    }
    assert_eq!(
        content,
        store
            .read_blob("restored", &sha, content.len() as i64)
            .unwrap()
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
