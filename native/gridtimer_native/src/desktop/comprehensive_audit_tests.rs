// v1.0.1 - Prepare trusted peer test metadata before starting the network timeout.
// v0.0.1 - Reproduce privacy and credential boundaries using isolated data.

fn audit_sealed_state(raw: &str, id: &str) -> String {
    let value: Value = serde_json::from_str(raw).unwrap();
    let note = value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| note["id"] == id)
        .unwrap();
    let (sealed, session) =
        gridtimer_native::encrypt_desktop_note_json(&note.to_string(), "audit-only-password")
            .unwrap();
    gridtimer_native::close_desktop_note_session(&session);
    app_data::upsert_note_app_data_json(raw, &sealed, 1000).unwrap()
}

fn audit_owner(client: &TimerWindowsClient) -> String {
    desktop_state_owner(
        &client.sync.server_instance_id,
        &client.sync.account_namespace,
        &client.sync.user_id,
    )
    .unwrap()
}

#[test]
fn comprehensive_audit_privacy_preserves_other_notes_accounts_and_safe_recovery() {
    let dir = temp_test_dir("audit_privacy_preservation");
    let marker = "PRIVATE_RECOVERY_73915";
    let original = add_note_to_state(
        &app_state_with_note("private", "Private", marker, None),
        "ordinary",
        "Ordinary",
        "EARLIER_OTHER_CONTENT",
        110,
    );
    let mut client = test_client_for_account_scope(&dir, "owner-a", original.clone());
    client.save_state().unwrap();
    let other = test_client_for_account_scope(
        &dir,
        "owner-b",
        app_state_with_note("private", "Other account", "OTHER_ACCOUNT_CONTENT", None),
    );
    other.save_state().unwrap();
    let other_bytes = fs::read(&other.state_path).unwrap();
    let mut changed: Value = serde_json::from_str(&original).unwrap();
    let private = changed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "private")
        .unwrap()
        .clone();
    changed["syncConflictHistory"] = json!([{"id":"private-conflict", "entityType":"note", "entityId":"private", "losingRevisionEpochMillis":100, "capturedAtEpochMillis":120, "payload":private}]);
    assert!(client.replace_state(Some(changed.to_string()), "audit"));
    fs::write(
        client
            .state_path
            .with_file_name("timer_state.json.invalid-legacy"),
        &original,
    )
    .unwrap();
    let note = client
        .data
        .notes
        .iter()
        .find(|n| n.id == "private")
        .unwrap()
        .clone();
    client.load_note_draft_without_flush(&note);
    client.note_crypto_password_draft = "audit-only-password".into();
    client.enable_note_encryption();
    assert!(
        client
            .data
            .notes
            .iter()
            .find(|n| n.id == "private")
            .unwrap()
            .encryption
            .is_some(),
        "{}",
        client.status
    );
    assert!(audit_plaintext_files(&dir, marker).is_empty());
    assert_eq!(other_bytes, fs::read(&other.state_path).unwrap());
    assert!(client.state_json.contains("EARLIER_OTHER_CONTENT"));
    let store = open_desktop_state_store(&dir).unwrap();
    let owner = audit_owner(&client);
    let head = store.latest_valid(&owner, 2000).unwrap().unwrap();
    let db = rusqlite::Connection::open(store.database_path()).unwrap();
    db.execute(
        "UPDATE desktop_state_snapshots SET raw_sha256=?1 WHERE id=?2",
        rusqlite::params!["0".repeat(64), head.id],
    )
    .unwrap();
    drop(db);
    let (recovered, _) =
        load_state_snapshot_with_store(&dir, &client.state_path, &owner, 2000).unwrap();
    assert!(!recovered.value.contains(marker));
    assert!(recovered.value.contains("EARLIER_OTHER_CONTENT"));
    let recovered_data = decode_data(&recovered.value);
    assert!(recovered_data
        .notes
        .iter()
        .find(|n| n.id == "private")
        .unwrap()
        .encryption
        .is_some());
    drop(client);
    drop(other);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn comprehensive_audit_privacy_retries_after_commit_before_mirror_cleanup() {
    let dir = temp_test_dir("audit_privacy_crash");
    let marker = "CRASH_PRIVATE_CONTENT_62964";
    let raw = app_state_with_note("private", "Private", marker, None);
    let client = test_client_for_account_scope(&dir, "owner", raw.clone());
    client.save_state().unwrap();
    for suffix in [".bak", ".invalid-crash", ".tmp-previous"] {
        fs::write(
            client
                .state_path
                .with_file_name(format!("timer_state.json{suffix}")),
            &raw,
        )
        .unwrap();
    }
    let sealed = audit_sealed_state(&raw, "private");
    let owner = audit_owner(&client);
    let store = open_desktop_state_store(&dir).unwrap();
    store.record(&owner, &sealed, 2000, "local_save").unwrap();
    assert!(store.privacy_policy(&owner).unwrap().1);
    assert!(!audit_plaintext_files(&dir, marker).is_empty());
    // Simulate reopening after the durable transaction, without running the
    // JSON cleanup or refreshing external evidence in the interrupted writer.
    let (loaded, _) =
        load_state_snapshot_with_store(&dir, &client.state_path, &owner, 3000).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&loaded.value).unwrap(),
        serde_json::from_str::<Value>(&sealed).unwrap()
    );
    assert!(audit_plaintext_files(&dir, marker).is_empty());
    assert!(!store.privacy_policy(&owner).unwrap().1);
    assert!(
        store
            .record(&owner, &raw, 4000, "explicit_restore")
            .is_err(),
        "a restore cannot erase a privacy transition"
    );
    drop(client);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn comprehensive_audit_privacy_legacy_encrypted_primary_cleans_old_backup() {
    let dir = temp_test_dir("audit_privacy_legacy_backup");
    let raw = app_state_with_note("private", "Private", "LEGACY_PLAINTEXT_83296", None);
    let sealed = audit_sealed_state(&raw, "private");
    let state_path = state_path_for_user(&dir, "");
    fs::write(&state_path, &sealed).unwrap();
    fs::write(backup_path(&state_path), &raw).unwrap();
    let (loaded, _) = load_state_snapshot_with_store(&dir, &state_path, "guest-v1", 2000).unwrap();
    assert_eq!(decode_data(&loaded.value).notes.len(), 1);
    assert!(decode_data(&loaded.value).notes[0].encryption.is_some());
    assert!(audit_plaintext_files(&dir, "LEGACY_PLAINTEXT_83296").is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(windows)]
#[test]
fn comprehensive_audit_privacy_locked_backup_reports_pending_and_recovers() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = temp_test_dir("audit_privacy_locked_backup");
    let raw = app_state_with_note("private", "Private", "LOCKED_PRIVATE_CONTENT_1796", None);
    let client = test_client_for_account_scope(&dir, "owner", raw.clone());
    client.save_state().unwrap();
    let sealed = audit_sealed_state(&raw, "private");
    let owner = audit_owner(&client);
    let mut lock = None;
    let outcome = save_state_snapshot_with_store_and_evidence_refresh(
        &dir,
        &client.state_path,
        &owner,
        &sealed,
        &client.sync,
        2000,
        "local_save",
        |root, store| {
            verify_and_refresh_desktop_state_evidence(root, store)?;
            lock = Some(
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .open(backup_path(&client.state_path))?,
            );
            Ok(())
        },
    )
    .unwrap();
    assert!(matches!(
        outcome,
        StateSnapshotSaveOutcome::CommittedPrivacyPending(_)
    ));
    let store = open_desktop_state_store(&dir).unwrap();
    assert!(store.privacy_policy(&owner).unwrap().1);
    assert_eq!(
        store
            .latest_valid(&owner, 2000)
            .unwrap()
            .unwrap()
            .app_data_json,
        sealed
    );
    drop(lock);
    load_state_snapshot_with_store(&dir, &client.state_path, &owner, 3000).unwrap();
    assert!(audit_plaintext_files(&dir, "LOCKED_PRIVATE_CONTENT_1796").is_empty());
    assert!(!store.privacy_policy(&owner).unwrap().1);
    drop(client);
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn comprehensive_audit_privacy_allows_intentional_unseal_and_reseal() {
    let dir = temp_test_dir("audit_privacy_generations");
    let mut client = test_client_for_account_scope(
        &dir,
        "owner",
        app_state_with_note("private", "Private", "INITIAL_PRIVATE_8342", None),
    );
    client.save_state().unwrap();
    let note = client.data.notes[0].clone();
    client.load_note_draft_without_flush(&note);
    client.note_crypto_password_draft = "audit-only-password".into();
    client.enable_note_encryption();
    assert!(
        client.data.notes[0].encryption.is_some(),
        "{}",
        client.status
    );
    client.disable_note_encryption();
    assert!(
        client.data.notes[0].encryption.is_none(),
        "{}",
        client.status
    );
    assert!(client.state_json.contains("INITIAL_PRIVATE_8342"));
    client.note_crypto_password_draft = "audit-new-password".into();
    client.enable_note_encryption();
    assert!(
        client.data.notes[0].encryption.is_some(),
        "{}",
        client.status
    );
    assert!(audit_plaintext_files(&dir, "INITIAL_PRIVATE_8342").is_empty());
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

fn audit_plaintext_files(root: &Path, marker: &str) -> Vec<String> {
    let mut pending = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(entry.path());
            }
            if kind.is_file()
                && fs::read(entry.path())
                    .unwrap()
                    .windows(marker.len())
                    .any(|bytes| bytes == marker.as_bytes())
            {
                matches.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    matches.sort();
    matches
}

#[test]
fn comprehensive_audit_encryption_removes_plaintext_from_managed_recovery_files() {
    let dir = temp_test_dir("audit_encrypt_recovery");
    let marker = "TENRATE_AUDIT_PRIVATE_CONTENT_73651";
    let mut client = test_client_for_account_scope(
        &dir,
        "privacy-account",
        app_state_with_note("privacy-note", "Private", marker, None),
    );
    client.save_state().unwrap();
    let note = client.data.notes[0].clone();
    client.load_note_draft_without_flush(&note);
    client.note_crypto_password_draft = "test-only-password-73651".into();
    client.enable_note_encryption();
    assert!(
        client.data.notes[0].encryption.is_some(),
        "encryption transition failed: {}",
        client.status
    );
    client.close_note_crypto_session();
    let leaked = audit_plaintext_files(&dir, marker);
    assert!(
        leaked.is_empty(),
        "encrypted content remains in managed recovery files: {leaked:?}"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn comprehensive_audit_permanent_delete_removes_content_from_managed_recovery_files() {
    let dir = temp_test_dir("audit_delete_recovery");
    let marker = "TENRATE_AUDIT_DELETED_CONTENT_82641";
    let mut client = test_client_for_account_scope(
        &dir,
        "privacy-account",
        app_state_with_note("privacy-note", "Private", marker, None),
    );
    client.save_state().unwrap();
    let note = client.data.notes[0].clone();
    client.load_note_draft_without_flush(&note);
    client.delete_note();
    assert!(client
        .data
        .notes
        .iter()
        .any(|note| note.id == "privacy-note" && note.deleted_at_epoch_millis.is_some()));
    client.permanently_delete_sticky_note("privacy-note");
    assert!(
        !client
            .data
            .notes
            .iter()
            .any(|note| note.id == "privacy-note"),
        "permanent deletion failed: {}",
        client.status
    );
    let leaked = audit_plaintext_files(&dir, marker);
    assert!(
        leaked.is_empty(),
        "permanently deleted content remains in managed recovery files: {leaked:?}"
    );
    drop(client);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn comprehensive_audit_untrusted_loopback_receives_no_login_credentials() {
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let capture = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok((mut peer, _)) = listener.accept() {
                peer.set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 8192];
                while let Ok(count) = peer.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.len() > 65536 {
                        break;
                    }
                }
                let body = r#"{"ok":false,"message":"isolated fake endpoint"}"#;
                let response = format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                let _ = peer.write_all(response.as_bytes());
                return request;
            }
            if std::time::Instant::now() >= deadline {
                return Vec::new();
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
    let _result = run_sync_task(
        SyncTaskKind::Login,
        endpoint,
        "audit@example.test".into(),
        "AUDIT_PASSWORD_MUST_NOT_LEAVE_6235".into(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "audit-device".into(),
        0,
        String::new(),
        app_data::default_app_data_json(100),
        false,
    );
    let request = String::from_utf8_lossy(&capture.join().unwrap()).into_owned();
    assert!(
        !request.contains("AUDIT_PASSWORD_MUST_NOT_LEAVE_6235"),
        "an unverified loopback process received the synthetic login password"
    );
}

#[test]
fn comprehensive_audit_untrusted_loopback_logout_rejects_mixed_case_http() {
    use std::net::TcpListener;
    for scheme in ["http", "HTTP"] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = format!("{scheme}://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let capture = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .unwrap();
                    let mut buffer = [0; 8192];
                    let count = stream.read(&mut buffer).unwrap_or(0);
                    let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                    return buffer[..count].to_vec();
                }
                thread::sleep(Duration::from_millis(10));
            }
            Vec::new()
        });
        assert!(!revoke_token_once(
            &endpoint,
            "AUDIT_PRIVATE_LOGOUT_TOKEN_1361"
        ));
        let request = capture.join().unwrap();
        assert!(
            request.is_empty(),
            "untrusted logout peer received request bytes"
        );
    }
}

#[cfg(windows)]
#[test]
fn comprehensive_audit_socket_identity_checks_ipv4_ipv6_and_image_bytes() {
    use std::net::{TcpListener, TcpStream};
    let executable = std::env::current_exe().unwrap();
    let bytes = fs::read(&executable).unwrap();
    let expected_sha = format!("{:x}", Sha256::digest(&bytes));
    for address in ["127.0.0.1", "::1"] {
        let listener = TcpListener::bind((address, 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_accepted, _) = listener.accept().unwrap();
        let pin =
            verify_loopback_peer_image(&stream, &executable, bytes.len() as u64, &expected_sha)
                .expect("actual peer image must authenticate");
        assert!(
            verify_loopback_peer_image(&stream, &executable, bytes.len() as u64, &"0".repeat(64))
                .is_err(),
            "tampered identity must be rejected"
        );
        assert!(verify_loopback_peer_image(
            &stream,
            &executable,
            bytes.len() as u64 + 1,
            &expected_sha
        )
        .is_err());
        drop(pin);
    }
}

#[cfg(windows)]
fn audit_self_peer_identity() -> &'static (PathBuf, u64, String) {
    static IDENTITY: std::sync::OnceLock<(PathBuf, u64, String)> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(|| {
        let executable = std::env::current_exe().expect("test executable");
        let bytes = fs::read(&executable).expect("trusted test image");
        (
            executable,
            bytes.len() as u64,
            format!("{:x}", Sha256::digest(&bytes)),
        )
    })
}

#[cfg(windows)]
fn audit_verify_self_peer(stream: &std::net::TcpStream) -> io::Result<File> {
    let (executable, size, sha256) = audit_self_peer_identity();
    verify_loopback_peer_image(stream, executable, *size, sha256)
}

#[cfg(windows)]
#[test]
fn comprehensive_audit_verified_loopback_preserves_login_and_logout_semantics() {
    use std::net::TcpListener;
    let _expected = audit_self_peer_identity();
    for status in [200_u16, 401] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let capture = thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            // Match the real service header deadline; authentication must finish before credentials are read.
            peer.set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut buffer = [0_u8; 8192];
            let count = peer.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]).into_owned();
            let body = r#"{"ok":true,"message":"synthetic trusted peer"}"#;
            write!(peer, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            request
        });
        let _verification = sync_core::LocalHttpPeerVerification::enforce(audit_verify_self_peer);
        if status == 200 {
            let response: Value = serde_json::from_str(&sync_core::login_account_json(
                &endpoint,
                "audit@example.test",
                "known-test-password",
                "audit",
            ))
            .unwrap();
            assert_eq!(Some(true), response["ok"].as_bool());
            assert!(capture.join().unwrap().contains("known-test-password"));
        } else {
            assert!(
                sync_core::revoke_loopback_token_once(&endpoint, "known-test-token"),
                "already revoked token must settle its pending job"
            );
            assert!(capture.join().unwrap().contains("Bearer known-test-token"));
        }
    }
}
