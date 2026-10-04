// v1.0.3.4 Windows - Preserve accounts when old media markers cannot be assigned safely.
// Windows 1.0.2 candidate - Cover authenticated historical attachment recovery.
// Windows 1.0.2 candidate - Verify private reference transport and deferred deletion.
// v2.22.56 - Reject private content before publishing a restored database.
// v2.22.42 - Cover remote-only media availability before document merge.
// v2.22.23 - Sync protocol, restore barrier, and resource guard regression tests.
use super::*;
include!("sync_media_deletion_intent_tests.rs");
include!("sync_private_media_tests.rs");
include!("sync_private_media_bytes_tests.rs");
include!("sync_private_history_tests.rs");
include!("sync_backup_privacy_tests.rs");
include!("sync_legacy_privacy_tests.rs");
include!("sync_preschema_privacy_tests.rs");
use std::collections::VecDeque;

const DESKTOP_MEDIA_SERVER_INSTANCE_ID: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";
const DESKTOP_MEDIA_ACCOUNT_NAMESPACE: &str =
    "2222222222222222222222222222222222222222222222222222222222222222";
const OTHER_DESKTOP_MEDIA_SERVER_INSTANCE_ID: &str =
    "3333333333333333333333333333333333333333333333333333333333333333";
const OTHER_DESKTOP_MEDIA_ACCOUNT_NAMESPACE: &str =
    "4444444444444444444444444444444444444444444444444444444444444444";

fn one_shot_media_client_server(
    status: u16,
    response: SyncClientResult,
) -> (String, mpsc::Receiver<HttpRequest>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, request_receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream).unwrap();
        request_sender.send(request).unwrap();
        write_json_response(&mut stream, status, &response).unwrap();
    });
    (
        format!("http://{}:{}", Ipv4Addr::LOCALHOST, address.port()),
        request_receiver,
        worker,
    )
}

#[test]
fn mobile_sync_response_timeout_covers_slow_public_snapshot_delivery() {
    assert_eq!(Duration::from_secs(6), SYNC_CONNECT_TIMEOUT);
    assert_eq!(Duration::from_secs(20), SYNC_WRITE_TIMEOUT);
    assert_eq!(Duration::from_secs(90), SYNC_RESPONSE_READ_TIMEOUT);
    assert!(SYNC_RESPONSE_READ_TIMEOUT > SYNC_WRITE_TIMEOUT);
}

fn encrypted_note_for_sync(
    ciphertext: &str,
    protection_revision: i64,
    current_revision: i64,
) -> Value {
    json!({
        "id": "protected-note",
        "title": "",
        "content": "",
        "kind": "STICKY",
        "document": {},
        "attachments": [],
        "revisions": [],
        "encryption": {
            "formatVersion": 1,
            "keyId": "key-1",
            "protectionRevision": protection_revision,
            "cipherSuite": "AES_256_GCM",
            "kdf": "ARGON2ID",
            "memoryKiB": 32768,
            "iterations": 3,
            "parallelism": 1,
            "saltBase64": "c2FsdA==",
            "keyNonceBase64": "a2V5LW5vbmNl",
            "wrappedKeyBase64": "d3JhcHBlZC1rZXk=",
            "contentNonceBase64": "Y29udGVudC1ub25jZQ==",
            "ciphertextBase64": ciphertext
        },
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": current_revision,
        "deletedAtEpochMillis": null
    })
}

fn with_note_protection_state(mut note: Value, protection_state_revision: i64) -> Value {
    note["protectionStateRevision"] = json!(protection_state_revision);
    note
}

fn plaintext_note_for_sync(
    title: &str,
    protection_state_revision: i64,
    current_revision: i64,
) -> Value {
    json!({
        "id": "protected-note",
        "title": title,
        "content": format!("{title} body"),
        "kind": "STICKY",
        "document": {"blocks": []},
        "attachments": [],
        "revisions": [],
        "versions": [],
        "latestVersionId": "",
        "encryption": null,
        "protectionStateRevision": protection_state_revision,
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": current_revision,
        "deletedAtEpochMillis": null
    })
}

fn merge_note_app_data_bidirectionally(left_note: &Value, right_note: &Value) -> (Value, Value) {
    let mut left: Value = serde_json::from_str(&app_data::default_app_data_json(1_000)).unwrap();
    left["notes"] = json!([left_note]);
    let mut right: Value = serde_json::from_str(&app_data::default_app_data_json(1_000)).unwrap();
    right["notes"] = json!([right_note]);

    let mut forward = left.clone();
    merge_app_data_values(&mut forward, &right);
    let mut reverse = right;
    merge_app_data_values(&mut reverse, &left);
    (forward, reverse)
}

fn merge_sync_note_app_data_bidirectionally(
    left_note: &Value,
    right_note: &Value,
) -> (Value, Value) {
    let app_data_json = |note: &Value| {
        let mut value: Value =
            serde_json::from_str(&app_data::default_app_data_json(20_000)).unwrap();
        value["notes"] = json!([note]);
        value.to_string()
    };
    let left = app_data_json(left_note);
    let right = app_data_json(right_note);
    let forward = merge_sync_app_data_json(&left, &right, 20_000).unwrap();
    let reverse = merge_sync_app_data_json(&right, &left, 20_000).unwrap();
    (
        serde_json::from_str(&forward).unwrap(),
        serde_json::from_str(&reverse).unwrap(),
    )
}

#[test]
fn sync_payload_status_uses_the_actual_serialized_envelope() {
    let mut result = SyncClientResult {
        ok: true,
        app_data_json: Some(app_data::default_app_data_json(1)),
        current_committed: true,
        ..SyncClientResult::default()
    };
    let encoded = prepare_sync_payload(&mut result).unwrap();
    assert_eq!(encoded.len() as i64, result.sync_payload_usage_bytes);
    assert_eq!(
        MAX_RESPONSE_BODY_BYTES as i64,
        result.sync_payload_limit_bytes
    );
    assert!(result.sync_payload_warning.is_empty());

    let rejected = app_data_quota_exceeded_result(100, 200, 150);
    assert!(!rejected.ok);
    assert!(!rejected.current_committed);
    assert!(!rejected.retryable);
    assert_eq!("app_data_quota_exceeded", rejected.mode);
}

struct ScriptedReader {
    responses: VecDeque<io::Result<Vec<u8>>>,
}

impl ScriptedReader {
    fn new(responses: Vec<io::Result<Vec<u8>>>) -> Self {
        Self {
            responses: responses.into(),
        }
    }
}

impl Read for ScriptedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(next) = self.responses.pop_front() else {
            return Ok(0);
        };
        match next {
            Ok(bytes) => {
                let len = bytes.len().min(buffer.len());
                buffer[..len].copy_from_slice(&bytes[..len]);
                Ok(len)
            }
            Err(error) => Err(error),
        }
    }
}

struct TestSqliteStore {
    store: SqliteServerStore,
    directory: PathBuf,
}

#[test]
fn privacy_journal_recovery_entrypoint_publishes_only_redacted_content() {
    let plain = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        r#"{"id":"recovery-private-note","content":"RESTORE_PRIVATE_MARKER_78623"}"#,
        100,
    )
    .unwrap();
    let fixture = account_test_store("recovery-test-token", plain.clone());
    let database = fixture.store.database_path().to_path_buf();
    let backup = fixture.directory.join("before-private-delete.sqlite3");
    let report = fixture.store.create_verified_backup(&backup, 150).unwrap();
    let deleted =
        app_data::delete_note_permanently_app_data_json(&plain, "recovery-private-note", 200)
            .unwrap();
    fixture
        .store
        .compare_and_swap_account("user-1", 0, &deleted, 200)
        .unwrap();
    fixture.store.finish_note_privacy_cleanup().unwrap();
    fs::remove_file(&database).unwrap();

    restore_verified_backup_to_missing_database(
        &backup,
        &database,
        150,
        &report.sha256,
        report.size_bytes,
        300,
    )
    .unwrap();

    // Check the published file before open() can perform startup repairs.
    assert!(!fs::read(&database)
        .unwrap()
        .windows(b"RESTORE_PRIVATE_MARKER_78623".len())
        .any(|window| window == b"RESTORE_PRIVATE_MARKER_78623"));
    let recovered = SqliteServerStore::open(&database, None).unwrap();
    assert_eq!(
        report.server_instance_id,
        recovered.server_instance_id().unwrap()
    );
    assert!(!recovered
        .read_account("user-1")
        .unwrap()
        .app_data_json
        .contains("RESTORE_PRIVATE_MARKER_78623"));
    assert!(recovered
        .list_snapshot_history("user-1", 100)
        .unwrap()
        .iter()
        .all(|snapshot| !snapshot
            .app_data_json
            .contains("RESTORE_PRIVATE_MARKER_78623")));
    assert_eq!(
        report.sha256,
        SqliteServerStore::verify_existing_backup(&backup, 150)
            .unwrap()
            .sha256
    );
    recovered.validate_integrity().unwrap();
}

impl Drop for TestSqliteStore {
    fn drop(&mut self) {
        if self
            .directory
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("gridtimer_sync_core_test_"))
        {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

fn sqlite_test_store(legacy: ServerStore) -> TestSqliteStore {
    let directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    let store = SqliteServerStore::open(directory.join("server_store.sqlite3"), None)
        .expect("test SQLite store should open");
    let now = now_millis();
    for user in legacy.users {
        let user_id = user.id.clone();
        let tokens = user.tokens;
        store
            .create_user(NewStoredUser {
                id: user.id,
                email: user.email,
                password_salt: user.password_salt,
                password_hash: user.password_hash,
                password_scheme: "legacy_sha256".to_string(),
                created_at_epoch_millis: user.created_at_epoch_millis,
                updated_at_epoch_millis: user.updated_at_epoch_millis,
                app_data_json: user.app_data_json,
                account_revision: 0,
            })
            .expect("test user should be created");
        for token in tokens {
            let metadata = store
                .issue_token(
                    &user_id,
                    &token.token,
                    &token.device_name,
                    now,
                    now.saturating_add(TOKEN_TTL_MILLIS),
                )
                .expect("test token should be created");
            let receipt = store
                .ensure_restore_receipt(&user_id, metadata.id)
                .expect("test restore receipt should be created");
            store
                .acknowledge_restore_generation(
                    &user_id,
                    metadata.id,
                    receipt.current_generation,
                    &receipt.receipt,
                )
                .expect("test token restore baseline should be acknowledged");
        }
    }
    TestSqliteStore { store, directory }
}

fn account_test_store(token: &str, app_data_json: String) -> TestSqliteStore {
    sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-1".to_string(),
            email: "test@example.com".to_string(),
            password_salt: "legacy-salt".to_string(),
            password_hash: password_hash("legacy-salt", "secret-password"),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1,
            app_data_json,
            tokens: vec![ServerToken {
                token: token.to_string(),
                device_name: "test-device".to_string(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        }],
    })
}

fn note_app_data(note_id: &str, updated_at_epoch_millis: i64) -> String {
    let mut value: Value =
        serde_json::from_str(&app_data::default_app_data_json(updated_at_epoch_millis)).unwrap();
    value["notes"] = json!([{
        "id": note_id,
        "title": note_id,
        "content": note_id,
        "createdAtEpochMillis": updated_at_epoch_millis,
        "updatedAtEpochMillis": updated_at_epoch_millis,
        "deletedAtEpochMillis": null
    }]);
    value.to_string()
}

#[allow(clippy::too_many_arguments)]
fn workspace_sync_request(
    raw_token: &str,
    request_id: &str,
    app_data_json: String,
    client_updated_at_epoch_millis: i64,
    acknowledged_generation: i64,
    restore_receipt: &str,
    server_instance_id: &str,
    account_namespace: &str,
    workspace_id: &str,
    workspace_proof: &str,
    force_upload: bool,
    force_download: bool,
) -> HttpRequest {
    HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {raw_token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: request_id.to_string(),
            app_data_json,
            client_updated_at_epoch_millis,
            acknowledged_generation,
            restore_receipt: restore_receipt.to_string(),
            server_instance_id: server_instance_id.to_string(),
            account_namespace: account_namespace.to_string(),
            workspace_id: workspace_id.to_string(),
            workspace_proof: workspace_proof.to_string(),
            allow_workspace_identity_rebind: false,
            previous_server_instance_id: String::new(),
            previous_account_namespace: String::new(),
            device_name: "workspace-test".to_string(),
            force_upload,
            force_download,
        })
        .unwrap(),
        ..HttpRequest::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn workspace_identity_rebind_request(
    raw_token: &str,
    request_id: &str,
    app_data_json: String,
    current_server_instance_id: &str,
    current_account_namespace: &str,
    previous_server_instance_id: &str,
    previous_account_namespace: &str,
    workspace_id: &str,
) -> HttpRequest {
    HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {raw_token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: request_id.to_string(),
            app_data_json,
            client_updated_at_epoch_millis: 100,
            acknowledged_generation: 0,
            restore_receipt: String::new(),
            server_instance_id: current_server_instance_id.to_string(),
            account_namespace: current_account_namespace.to_string(),
            workspace_id: workspace_id.to_string(),
            workspace_proof: String::new(),
            allow_workspace_identity_rebind: true,
            previous_server_instance_id: previous_server_instance_id.to_string(),
            previous_account_namespace: previous_account_namespace.to_string(),
            device_name: "workspace-rebind-test".to_string(),
            force_upload: false,
            force_download: false,
        })
        .unwrap(),
        ..HttpRequest::default()
    }
}

fn read_request_from_bytes(bytes: Vec<u8>) -> io::Result<HttpRequest> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    let sender = thread::spawn(move || -> io::Result<()> {
        let mut stream = TcpStream::connect(address)?;
        stream.write_all(&bytes)?;
        stream.shutdown(Shutdown::Write)
    });
    let (mut stream, _) = listener.accept()?;
    let result = read_http_request(&mut stream);
    sender
        .join()
        .map_err(|_| io::Error::other("request sender panicked"))??;
    result
}

#[test]
fn revision_scans_nested_app_data_timestamps() {
    let raw = r#"{"slots":[{"updatedAt":10}],"notes":[{"updatedAtEpochMillis":25}]}"#;
    assert_eq!(25, app_data_revision_millis(raw, 1));
}

#[test]
fn sync_response_returns_account_history_for_empty_phone_request() {
    let token = "phone-token".to_string();
    let mut server_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000))
        .expect("default app data should decode");
    server_value["sessions"] = json!([
        {
            "id": "s1",
            "slotId": 1,
            "slotTitle": "finished task",
            "startedAtEpochMillis": 500,
            "endedAtEpochMillis": 1000,
            "durationMillis": 500
        }
    ]);
    let server_app_data =
        serde_json::to_string(&server_value).expect("server app data should encode");
    let store = sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-1".to_string(),
            email: "phone@example.com".to_string(),
            password_salt: "salt".to_string(),
            password_hash: "hash".to_string(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1_000,
            app_data_json: server_app_data,
            tokens: vec![ServerToken {
                token: token.clone(),
                device_name: "desktop".to_string(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        }],
    });
    let client_app_data = app_data::default_app_data_json(9_999);
    let request_body = serde_json::to_string(&SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: client_app_data,
        client_updated_at_epoch_millis: 9_999,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        device_name: "phone".to_string(),
        force_upload: false,
        force_download: false,
        ..SyncSnapshotRequest::default()
    })
    .expect("request should encode");
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: request_body,
        ..HttpRequest::default()
    };

    let (status, result) = handle_sync_mode(&request, &store.store, false);

    assert_eq!(200, status);
    assert!(result.ok);
    assert!(matches!(result.mode.as_str(), "merged" | "unchanged"));
    assert!(result
        .app_data_json
        .as_deref()
        .unwrap_or_default()
        .contains("\"sessions\""));
    assert!(result
        .app_data_json
        .as_deref()
        .unwrap_or_default()
        .contains("\"id\":\"s1\""));
}

#[test]
fn sync_default_downloads_account_history_when_phone_has_newer_local_history() {
    let token = "phone-token".to_string();
    let mut server_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000))
        .expect("default server app data should decode");
    server_value["sessions"] = json!([
        {
            "id": "cloud-session",
            "slotId": 1,
            "slotTitle": "cloud history",
            "startedAtEpochMillis": 500,
            "endedAtEpochMillis": 1_000,
            "durationMillis": 500
        }
    ]);
    let server_app_data =
        serde_json::to_string(&server_value).expect("server app data should encode");
    let store = sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-1".to_string(),
            email: "phone@example.com".to_string(),
            password_salt: "salt".to_string(),
            password_hash: "hash".to_string(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1_000,
            app_data_json: server_app_data,
            tokens: vec![ServerToken {
                token: token.clone(),
                device_name: "desktop".to_string(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        }],
    });
    let mut client_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(50_000))
        .expect("default client app data should decode");
    client_value["sessions"] = json!([
        {
            "id": "phone-session",
            "slotId": 2,
            "slotTitle": "phone history",
            "startedAtEpochMillis": 55_000,
            "endedAtEpochMillis": 60_000,
            "durationMillis": 5_000
        }
    ]);
    let client_app_data =
        serde_json::to_string(&client_value).expect("client app data should encode");
    let request_body = serde_json::to_string(&SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: client_app_data,
        client_updated_at_epoch_millis: 60_000,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        device_name: "phone".to_string(),
        force_upload: false,
        force_download: false,
        ..SyncSnapshotRequest::default()
    })
    .expect("request should encode");
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: request_body,
        ..HttpRequest::default()
    };

    let (status, result) = handle_sync_mode(&request, &store.store, false);
    let persisted_account_json = store
        .store
        .read_account("user-1")
        .expect("account should exist")
        .app_data_json
        .clone();

    assert_eq!(200, status);
    assert!(result.ok);
    assert_eq!("merged", result.mode);
    let returned_json = result.app_data_json.as_deref().unwrap_or_default();
    assert!(returned_json.contains("\"id\":\"cloud-session\""));
    assert!(returned_json.contains("\"id\":\"phone-session\""));
    assert!(persisted_account_json.contains("\"id\":\"cloud-session\""));
    assert!(persisted_account_json.contains("\"id\":\"phone-session\""));
}

#[test]
fn upload_local_http_endpoint_merges_and_persists_both_snapshots() {
    let token = "force-upload-merge-token";
    let store = account_test_store(token, note_app_data("server-note", 100));
    let identity = store.store.server_account_identity("user-1").unwrap();
    let workspace_id = "45".repeat(32);
    let workspace_proof = store
        .store
        .workspace_capability_proof("user-1", &workspace_id, 0)
        .unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server_store = Arc::new(store.store.clone());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_connection(
            stream,
            server_store,
            Arc::new(Mutex::new(ServerRuntimeInfo::default())),
            Arc::new(Mutex::new(LoginRateLimiter::default())),
        )
        .unwrap();
    });

    let encoded = upload_local_app_data_with_workspace_capability_json(
        &format!("http://{address}"),
        token,
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        &workspace_proof,
        &note_app_data("local-note", 200),
        200,
        0,
        "",
        "force upload phone",
    );
    server.join().unwrap();
    let result: SyncClientResult = serde_json::from_str(&encoded).unwrap();
    let persisted = store.store.read_account("user-1").unwrap();
    let returned = result.app_data_json.as_deref().unwrap_or_default();

    assert!(result.ok);
    assert_eq!("merged_force_upload", result.mode);
    assert!(returned.contains("server-note"));
    assert!(returned.contains("local-note"));
    assert!(persisted.app_data_json.contains("server-note"));
    assert!(persisted.app_data_json.contains("local-note"));
}

#[test]
fn sync_force_download_keeps_account_history_when_phone_has_newer_local_history() {
    let token = "phone-token".to_string();
    let mut server_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000))
        .expect("default server app data should decode");
    server_value["sessions"] = json!([
        {
            "id": "cloud-session",
            "slotId": 1,
            "slotTitle": "cloud history",
            "startedAtEpochMillis": 500,
            "endedAtEpochMillis": 1_000,
            "durationMillis": 500
        }
    ]);
    let server_app_data =
        serde_json::to_string(&server_value).expect("server app data should encode");
    let store = sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-1".to_string(),
            email: "phone@example.com".to_string(),
            password_salt: "salt".to_string(),
            password_hash: "hash".to_string(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1_000,
            app_data_json: server_app_data,
            tokens: vec![ServerToken {
                token: token.clone(),
                device_name: "desktop".to_string(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        }],
    });
    let mut client_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(50_000))
        .expect("default client app data should decode");
    client_value["sessions"] = json!([
        {
            "id": "phone-session",
            "slotId": 2,
            "slotTitle": "phone history",
            "startedAtEpochMillis": 55_000,
            "endedAtEpochMillis": 60_000,
            "durationMillis": 5_000
        }
    ]);
    let client_app_data =
        serde_json::to_string(&client_value).expect("client app data should encode");
    let request_body = serde_json::to_string(&SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: client_app_data,
        client_updated_at_epoch_millis: 60_000,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        device_name: "phone".to_string(),
        force_upload: false,
        force_download: true,
        ..SyncSnapshotRequest::default()
    })
    .expect("request should encode");
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: request_body,
        ..HttpRequest::default()
    };

    let (status, result) = handle_sync_mode(&request, &store.store, false);
    let persisted_account_json = store
        .store
        .read_account("user-1")
        .expect("account should exist")
        .app_data_json
        .clone();

    assert_eq!(200, status);
    assert!(result.ok);
    assert_eq!("downloaded_account", result.mode);
    let returned_json = result.app_data_json.as_deref().unwrap_or_default();
    assert!(returned_json.contains("\"id\":\"cloud-session\""));
    assert!(!returned_json.contains("\"id\":\"phone-session\""));
    assert!(persisted_account_json.contains("\"id\":\"cloud-session\""));
    assert!(!persisted_account_json.contains("\"id\":\"phone-session\""));
}

#[test]
fn sync_default_preserves_newer_phone_history_when_account_snapshot_is_old() {
    let token = "phone-token".to_string();
    let mut server_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000))
        .expect("default server app data should decode");
    server_value["sessions"] = json!([
        {
            "id": "may-session",
            "slotId": 1,
            "slotTitle": "old account history",
            "startedAtEpochMillis": 900,
            "endedAtEpochMillis": 1_000,
            "durationMillis": 100
        }
    ]);
    let server_app_data =
        serde_json::to_string(&server_value).expect("server app data should encode");
    let store = sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-1".to_string(),
            email: "phone@example.com".to_string(),
            password_salt: "salt".to_string(),
            password_hash: "hash".to_string(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1_000,
            app_data_json: server_app_data,
            tokens: vec![ServerToken {
                token: token.clone(),
                device_name: "desktop".to_string(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        }],
    });
    let mut client_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(50_000))
        .expect("default client app data should decode");
    client_value["sessions"] = json!([
        {
            "id": "june-local-session",
            "slotId": 2,
            "slotTitle": "new local work",
            "startedAtEpochMillis": 59_000,
            "endedAtEpochMillis": 60_000,
            "durationMillis": 1_000
        }
    ]);
    let client_app_data =
        serde_json::to_string(&client_value).expect("client app data should encode");
    let request_body = serde_json::to_string(&SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: client_app_data,
        client_updated_at_epoch_millis: 60_000,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        device_name: "phone".to_string(),
        force_upload: false,
        force_download: false,
        ..SyncSnapshotRequest::default()
    })
    .expect("request should encode");
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: request_body,
        ..HttpRequest::default()
    };

    let (status, result) = handle_sync_mode(&request, &store.store, false);
    let persisted_account_json = store
        .store
        .read_account("user-1")
        .expect("account should exist")
        .app_data_json
        .clone();

    assert_eq!(200, status);
    assert!(result.ok);
    assert_eq!("merged", result.mode);
    let returned_json = result.app_data_json.as_deref().unwrap_or_default();
    assert!(returned_json.contains("\"id\":\"may-session\""));
    assert!(returned_json.contains("\"id\":\"june-local-session\""));
    assert!(persisted_account_json.contains("\"id\":\"may-session\""));
    assert!(persisted_account_json.contains("\"id\":\"june-local-session\""));
}

#[test]
fn sync_merges_account_and_local_notes_without_losing_either() {
    let token = "phone-token".to_string();
    let mut server_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000))
        .expect("default server app data should decode");
    server_value["notes"] = json!([
        {
            "id": "cloud-note",
            "title": "account note",
            "content": "kept in account",
            "createdAtEpochMillis": 900,
            "updatedAtEpochMillis": 1_000,
            "deletedAtEpochMillis": null
        }
    ]);
    let server_app_data =
        serde_json::to_string(&server_value).expect("server app data should encode");
    let store = sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-1".to_string(),
            email: "phone@example.com".to_string(),
            password_salt: "salt".to_string(),
            password_hash: "hash".to_string(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 1_000,
            app_data_json: server_app_data,
            tokens: vec![ServerToken {
                token: token.clone(),
                device_name: "desktop".to_string(),
                created_at_epoch_millis: 1,
                last_seen_at_epoch_millis: 1,
            }],
        }],
    });
    let mut client_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(50_000))
        .expect("default client app data should decode");
    client_value["notes"] = json!([
        {
            "id": "phone-note",
            "title": "local note",
            "content": "kept on phone",
            "createdAtEpochMillis": 49_000,
            "updatedAtEpochMillis": 50_000,
            "deletedAtEpochMillis": null
        }
    ]);
    let client_app_data =
        serde_json::to_string(&client_value).expect("client app data should encode");
    let request_body = serde_json::to_string(&SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: client_app_data,
        client_updated_at_epoch_millis: 50_000,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        device_name: "phone".to_string(),
        force_upload: false,
        force_download: false,
        ..SyncSnapshotRequest::default()
    })
    .expect("request should encode");
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: request_body,
        ..HttpRequest::default()
    };

    let (status, result) = handle_sync_mode(&request, &store.store, false);
    let persisted_account_json = store
        .store
        .read_account("user-1")
        .expect("account should exist")
        .app_data_json
        .clone();

    assert_eq!(200, status);
    assert!(result.ok);
    assert_eq!("merged", result.mode);
    let returned_json = result.app_data_json.as_deref().unwrap_or_default();
    assert!(returned_json.contains("\"id\":\"cloud-note\""));
    assert!(returned_json.contains("\"id\":\"phone-note\""));
    assert!(persisted_account_json.contains("\"id\":\"cloud-note\""));
    assert!(persisted_account_json.contains("\"id\":\"phone-note\""));
}

#[test]
fn note_merge_does_not_use_child_revision_time_for_current_document() {
    let mut account =
        serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000)).unwrap();
    account["notes"] = json!([{
        "id": "shared-note",
        "title": "new current",
        "content": "new current body",
        "attachments": [],
        "revisions": [],
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 300,
        "deletedAtEpochMillis": null
    }]);
    let mut local = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000)).unwrap();
    local["notes"] = json!([{
        "id": "shared-note",
        "title": "old current",
        "content": "old current body",
        "attachments": [],
        "revisions": [{
            "id": "new-history",
            "title": "history captured later",
            "content": "historical body",
            "capturedAtEpochMillis": 900
        }],
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 200,
        "deletedAtEpochMillis": null
    }]);

    let merged = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 1_000)
        .expect("notes should merge");
    let merged: Value = serde_json::from_str(&merged).unwrap();
    let note = &merged["notes"][0];

    assert_eq!("new current", note["title"]);
    assert_eq!("new current body", note["content"]);
    assert_eq!("new-history", note["revisions"][0]["id"]);
}

#[test]
fn note_merge_unions_child_collections_and_resolves_each_child_by_own_time() {
    let mut account =
        serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000)).unwrap();
    account["notes"] = json!([{
        "id": "shared-note",
        "title": "account current",
        "content": "account body",
        "attachments": [
            {
                "id": "account-attachment",
                "fileName": "account.jpg",
                "displayName": "account attachment",
                "createdAtEpochMillis": 100
            },
            {
                "id": "shared-attachment",
                "fileName": "old.jpg",
                "displayName": "older child",
                "createdAtEpochMillis": 100
            }
        ],
        "revisions": [
            {
                "id": "account-revision",
                "title": "account history",
                "capturedAtEpochMillis": 100
            },
            {
                "id": "shared-revision",
                "title": "older child",
                "capturedAtEpochMillis": 100
            }
        ],
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 400,
        "deletedAtEpochMillis": null
    }]);
    let mut local = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000)).unwrap();
    local["notes"] = json!([{
        "id": "shared-note",
        "title": "local older current",
        "content": "local older body",
        "attachments": [
            {
                "id": "local-attachment",
                "fileName": "local.jpg",
                "displayName": "local attachment",
                "createdAtEpochMillis": 300
            },
            {
                "id": "shared-attachment",
                "fileName": "new.jpg",
                "displayName": "newer child",
                "createdAtEpochMillis": 300
            }
        ],
        "revisions": [
            {
                "id": "local-revision",
                "title": "local history",
                "capturedAtEpochMillis": 300
            },
            {
                "id": "shared-revision",
                "title": "newer child",
                "capturedAtEpochMillis": 300
            }
        ],
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 200,
        "deletedAtEpochMillis": null
    }]);

    let merged = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 1_000)
        .expect("notes should merge");
    let merged: Value = serde_json::from_str(&merged).unwrap();
    let note = &merged["notes"][0];
    let attachment_ids = note["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    let revision_ids = note["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value["id"].as_str())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!("account current", note["title"]);
    assert_eq!(3, attachment_ids.len());
    assert!(attachment_ids.contains("account-attachment"));
    assert!(attachment_ids.contains("local-attachment"));
    assert_eq!(4, revision_ids.len());
    assert!(revision_ids.contains("account-revision"));
    assert!(revision_ids.contains("local-revision"));
    let shared_attachment = note["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == "shared-attachment")
        .unwrap();
    let shared_revision = note["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == "shared-revision")
        .unwrap();
    assert_eq!("newer child", shared_attachment["displayName"]);
    assert_eq!("newer child", shared_revision["title"]);
}

#[test]
fn note_merge_preserves_disjoint_product_versions_and_resolves_sequence_collisions() {
    fn branch(latest_id: &str, latest_body: &str, latest_created_at: i64) -> Value {
        let mut value: Value =
            serde_json::from_str(&app_data::default_app_data_json(1_000)).unwrap();
        value["notes"] = json!([{
            "id": "versioned-note",
            "title": latest_body,
            "content": latest_body,
            "kind": "STICKY",
            "document": {"blocks": [{"id": latest_id, "type": "TEXT", "text": latest_body}]},
            "attachments": [],
            "revisions": [],
            "versions": [
                {
                    "id": "version-1",
                    "noteId": "versioned-note",
                    "sequence": 1,
                    "title": "base",
                    "content": "base",
                    "kind": "STICKY",
                    "document": {"blocks": [{"id": "base", "type": "TEXT", "text": "base"}]},
                    "attachments": [],
                    "createdAtEpochMillis": 100,
                    "updatedAtEpochMillis": 100,
                    "isLatest": false,
                    "deletedAtEpochMillis": null
                },
                {
                    "id": latest_id,
                    "noteId": "versioned-note",
                    "sequence": 2,
                    "title": latest_body,
                    "content": latest_body,
                    "kind": "STICKY",
                    "document": {"blocks": [{"id": latest_id, "type": "TEXT", "text": latest_body}]},
                    "attachments": [],
                    "createdAtEpochMillis": latest_created_at,
                    "updatedAtEpochMillis": latest_created_at,
                    "isLatest": true,
                    "deletedAtEpochMillis": null
                }
            ],
            "latestVersionId": latest_id,
            "createdAtEpochMillis": 100,
            "updatedAtEpochMillis": latest_created_at,
            "deletedAtEpochMillis": null
        }]);
        value
    }

    let account = branch("version-account", "account branch", 200);
    let local = branch("version-local", "local branch", 210);
    let forward = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 1_000)
        .expect("version branches should merge");
    let reverse = merge_sync_app_data_json(&local.to_string(), &account.to_string(), 1_000)
        .expect("reverse version branches should merge identically");
    let forward: Value = serde_json::from_str(&forward).unwrap();
    let reverse: Value = serde_json::from_str(&reverse).unwrap();
    let note = &forward["notes"][0];
    let versions = note["versions"].as_array().unwrap();
    let ids = versions
        .iter()
        .filter_map(|version| version["id"].as_str())
        .collect::<HashSet<_>>();
    let sequences = versions
        .iter()
        .filter_map(|version| version["sequence"].as_i64())
        .collect::<HashSet<_>>();

    assert_eq!(3, versions.len());
    assert_eq!(3, ids.len());
    assert!(ids.contains("version-1"));
    assert!(ids.contains("version-account"));
    assert!(ids.contains("version-local"));
    assert_eq!(HashSet::from([1, 2, 3]), sequences);
    assert_eq!(
        1,
        versions
            .iter()
            .filter(|version| version["isLatest"] == true)
            .count()
    );
    assert_eq!("version-local", note["latestVersionId"]);
    assert_eq!("local branch", note["content"]);
    assert_eq!(
        forward["notes"][0]["versions"],
        reverse["notes"][0]["versions"]
    );
}

#[test]
fn note_merge_materializes_losing_concurrent_body_and_attachment_as_revision() {
    fn branch(title: &str, body: &str, kind: &str, attachment_id: &str, hash_char: char) -> Value {
        let mut value: Value =
            serde_json::from_str(&app_data::default_app_data_json(1_000)).unwrap();
        value["notes"] = json!([{
            "id": "shared-note",
            "title": title,
            "content": body,
            "kind": kind,
            "document": {
                "blocks": [{"id": format!("block-{attachment_id}"), "type": "TEXT", "text": body}]
            },
            "attachments": [{
                "id": attachment_id,
                "fileName": format!("{attachment_id}.png"),
                "displayName": attachment_id,
                "mimeType": "image/png",
                "sizeBytes": 10,
                "sha256": hash_char.to_string().repeat(64),
                "createdAtEpochMillis": 400,
                "updatedAtEpochMillis": 500
            }],
            "revisions": [],
            "createdAtEpochMillis": 100,
            "updatedAtEpochMillis": 500,
            "deletedAtEpochMillis": null
        }]);
        value
    }

    let account = branch(
        "Account edit",
        "account body",
        "STICKY",
        "account-image",
        'a',
    );
    let local = branch("Local edit", "local body", "DOCUMENT", "local-image", 'b');
    let merged = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 100)
        .expect("concurrent note bodies should merge");
    let merged: Value = serde_json::from_str(&merged).unwrap();
    let note = &merged["notes"][0];
    let current_title = note["title"].as_str().unwrap();
    let losing_title = if current_title == "Account edit" {
        "Local edit"
    } else {
        "Account edit"
    };
    let losing_body = if losing_title == "Account edit" {
        "account body"
    } else {
        "local body"
    };
    let losing_attachment = if losing_title == "Account edit" {
        "account-image"
    } else {
        "local-image"
    };
    let losing_kind = if losing_title == "Account edit" {
        "STICKY"
    } else {
        "DOCUMENT"
    };
    let revision = note["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|revision| revision["title"] == losing_title)
        .expect("losing current must be recoverable from history");

    assert_eq!(losing_body, revision["content"]);
    assert_eq!(losing_kind, revision["kind"]);
    assert_eq!(500, revision["capturedAtEpochMillis"]);
    assert!(revision["id"]
        .as_str()
        .is_some_and(|id| id.starts_with("merge-revision-") && id.len() == 79));
    assert!(revision["attachmentIds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == losing_attachment));
    let restored_attachment = revision["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|attachment| attachment["id"] == losing_attachment)
        .expect("losing attachment metadata must be in the revision");
    assert_eq!(10, restored_attachment["sizeBytes"]);

    let reverse = merge_sync_app_data_json(&local.to_string(), &account.to_string(), 100)
        .expect("reverse merge should preserve the same losing revision");
    let reverse: Value = serde_json::from_str(&reverse).unwrap();
    let reverse_revision_ids = reverse["notes"][0]["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|revision| revision["id"].as_str())
        .collect::<HashSet<_>>();
    assert!(reverse_revision_ids.contains(revision["id"].as_str().unwrap()));
}

#[test]
fn note_merge_materializes_kind_only_conflict_as_revision() {
    let mut account: Value = serde_json::from_str(&app_data::default_app_data_json(1_000)).unwrap();
    account["notes"] = json!([{
        "id": "kind-conflict",
        "title": "same title",
        "content": "same body",
        "kind": "STICKY",
        "document": {"blocks": []},
        "attachments": [],
        "revisions": [],
        "createdAtEpochMillis": 100,
        "updatedAtEpochMillis": 500,
        "deletedAtEpochMillis": null
    }]);
    let mut local = account.clone();
    local["notes"][0]["kind"] = json!("DOCUMENT");

    let merged = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 100)
        .expect("kind-only conflict should stay recoverable");
    let merged: Value = serde_json::from_str(&merged).unwrap();
    let current_kind = merged["notes"][0]["kind"].as_str().unwrap();
    let losing_kind = if current_kind == "STICKY" {
        "DOCUMENT"
    } else {
        "STICKY"
    };
    assert!(merged["notes"][0]["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|revision| revision["kind"] == losing_kind
            && revision["title"] == "same title"
            && revision["content"] == "same body"));
}

#[test]
fn note_protection_state_revision_uses_legacy_envelope_fallback() {
    let ciphertext = encrypted_note_for_sync("legacy-sealed-v5", 5, 100);
    let plaintext = plaintext_note_for_sync("legacy plaintext", 0, 900);
    let explicit = with_note_protection_state(encrypted_note_for_sync("sealed-v11", 11, 200), 11);
    assert_eq!(5, note_protection_state_revision(&ciphertext));
    assert_eq!(0, note_protection_state_revision(&plaintext));
    assert_eq!(11, note_protection_state_revision(&explicit));
}

#[test]
fn note_protection_state_allows_authenticated_disable_bidirectionally() {
    let ciphertext = with_note_protection_state(encrypted_note_for_sync("sealed-v5", 5, 900), 5);
    let plaintext = plaintext_note_for_sync("authenticated plaintext", 6, 100);
    let (forward, reverse) = merge_note_app_data_bidirectionally(&ciphertext, &plaintext);
    assert_eq!(forward, reverse);
    assert_eq!(plaintext, forward["notes"][0]);
}

#[test]
fn note_protection_state_allows_reencryption_bidirectionally() {
    let plaintext = plaintext_note_for_sync("authenticated plaintext", 6, 900);
    let reencrypted = with_note_protection_state(encrypted_note_for_sync("sealed-v7", 7, 100), 7);
    let (forward, reverse) = merge_note_app_data_bidirectionally(&plaintext, &reencrypted);
    assert_eq!(forward, reverse);
    assert_eq!(reencrypted, forward["notes"][0]);
}

#[test]
fn note_protection_transitions_survive_sanitized_sync_merge_bidirectionally() {
    let old_ciphertext =
        with_note_protection_state(encrypted_note_for_sync("sealed-v5", 5, 9_999), 5);
    let disabled_plaintext = plaintext_note_for_sync("authenticated plaintext", 6, 100);
    let (disabled_forward, disabled_reverse) =
        merge_sync_note_app_data_bidirectionally(&old_ciphertext, &disabled_plaintext);
    assert_eq!(disabled_forward, disabled_reverse);
    assert_eq!(6, disabled_forward["notes"][0]["protectionStateRevision"]);
    assert!(disabled_forward["notes"][0]["encryption"].is_null());
    assert_eq!(
        "authenticated plaintext",
        disabled_forward["notes"][0]["title"]
    );

    let reencrypted = with_note_protection_state(encrypted_note_for_sync("sealed-v7", 7, 200), 7);
    let (reencrypted_forward, reencrypted_reverse) =
        merge_sync_note_app_data_bidirectionally(&disabled_forward["notes"][0], &reencrypted);
    assert_eq!(reencrypted_forward, reencrypted_reverse);
    assert_eq!(
        7,
        reencrypted_forward["notes"][0]["protectionStateRevision"]
    );
    assert_eq!(
        "sealed-v7",
        reencrypted_forward["notes"][0]["encryption"]["ciphertextBase64"]
    );

    let (replay_forward, replay_reverse) =
        merge_sync_note_app_data_bidirectionally(&reencrypted_forward["notes"][0], &old_ciphertext);
    assert_eq!(replay_forward, replay_reverse);
    assert_eq!(7, replay_forward["notes"][0]["protectionStateRevision"]);

    let stale_plaintext = plaintext_note_for_sync("stale plaintext", 6, 10_000);
    let (plain_replay_forward, plain_replay_reverse) =
        merge_sync_note_app_data_bidirectionally(&replay_forward["notes"][0], &stale_plaintext);
    assert_eq!(plain_replay_forward, plain_replay_reverse);
    assert_eq!(
        "sealed-v7",
        plain_replay_forward["notes"][0]["encryption"]["ciphertextBase64"]
    );
}

#[test]
fn note_protection_state_rejects_stale_ciphertext_replay_bidirectionally() {
    let old_ciphertext =
        with_note_protection_state(encrypted_note_for_sync("sealed-v5", 5, 9_999), 5);
    let disabled_plaintext = plaintext_note_for_sync("authenticated plaintext", 6, 100);
    let stale_plaintext = plaintext_note_for_sync("pre-encryption stale", 0, 10_000);
    let (forward, reverse) =
        merge_note_app_data_bidirectionally(&disabled_plaintext, &old_ciphertext);
    assert_eq!(forward, reverse);
    assert_eq!(disabled_plaintext, forward["notes"][0]);

    let (plain_forward, plain_reverse) =
        merge_note_app_data_bidirectionally(&disabled_plaintext, &stale_plaintext);
    assert_eq!(plain_forward, plain_reverse);
    assert_eq!(disabled_plaintext, plain_forward["notes"][0]);
}

#[test]
fn note_protection_state_rejects_replays_after_reencryption_bidirectionally() {
    let reencrypted = with_note_protection_state(encrypted_note_for_sync("sealed-v7", 7, 200), 7);
    let old_ciphertext =
        with_note_protection_state(encrypted_note_for_sync("sealed-v5", 5, 9_999), 5);
    let stale_plaintext = plaintext_note_for_sync("stale plaintext", 6, 10_000);

    let (cipher_forward, cipher_reverse) =
        merge_note_app_data_bidirectionally(&reencrypted, &old_ciphertext);
    assert_eq!(cipher_forward, cipher_reverse);
    assert_eq!(reencrypted, cipher_forward["notes"][0]);

    let (plain_forward, plain_reverse) =
        merge_note_app_data_bidirectionally(&reencrypted, &stale_plaintext);
    assert_eq!(plain_forward, plain_reverse);
    assert_eq!(reencrypted, plain_forward["notes"][0]);
}

#[test]
fn encrypted_note_merge_prefers_protection_state_and_never_resurrects_plaintext() {
    let protected = encrypted_note_for_sync("sealed-v5", 5, 100);
    let plaintext = json!({
        "id": "protected-note",
        "title": "stale plaintext title",
        "content": "stale plaintext body",
        "kind": "STICKY",
        "document": {"blocks": [{"id": "leak", "type": "TEXT", "text": "leak"}]},
        "attachments": [{"id": "leak-attachment", "updatedAtEpochMillis": 999}],
        "revisions": [{"id": "leak-revision", "title": "leak", "updatedAtEpochMillis": 999}],
        "protectionStateRevision": 5,
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 999,
        "deletedAtEpochMillis": null
    });

    let forward = merge_note_value(&protected, &plaintext);
    let reverse = merge_note_value(&plaintext, &protected);

    assert_eq!(protected, forward);
    assert_eq!(forward, reverse);
    assert_eq!("sealed-v5", forward["encryption"]["ciphertextBase64"]);
    assert_eq!(json!([]), forward["attachments"]);
    assert_eq!(json!([]), forward["revisions"]);
    assert_ne!("stale plaintext title", forward["title"]);
}

#[test]
fn encrypted_note_merge_is_atomic_with_lww_and_deterministic_tie_breaking() {
    let lower_protection = encrypted_note_for_sync("sealed-v3", 3, 900);
    let higher_protection = encrypted_note_for_sync("sealed-v4", 4, 100);
    assert_eq!(
        higher_protection,
        merge_note_value(&lower_protection, &higher_protection)
    );

    let older_current = encrypted_note_for_sync("sealed-old", 8, 500);
    let newer_current = encrypted_note_for_sync("sealed-new", 8, 600);
    assert_eq!(
        newer_current,
        merge_note_value(&older_current, &newer_current)
    );

    let tie_left = encrypted_note_for_sync("sealed-a", 9, 700);
    let tie_right = encrypted_note_for_sync("sealed-b", 9, 700);
    let forward = merge_note_value(&tie_left, &tie_right);
    let reverse = merge_note_value(&tie_right, &tie_left);
    assert_eq!(forward, reverse);
    assert!(forward == tie_left || forward == tie_right);
    assert_eq!(json!([]), forward["attachments"]);
    assert_eq!(json!([]), forward["revisions"]);
}

#[test]
fn encrypted_note_counts_as_meaningful_app_data_without_plaintext_preview() {
    let root = json!({
        "slots": [],
        "sessions": [],
        "archivedTasks": [],
        "notes": [encrypted_note_for_sync("sealed", 1, 100)],
        "financeProfile": {}
    });
    assert!(has_meaningful_app_data(&root));
}

#[test]
fn note_merge_uses_later_deletion_as_the_parent_state() {
    let mut account =
        serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000)).unwrap();
    account["notes"] = json!([{
        "id": "shared-note",
        "title": "newer live edit",
        "content": "live",
        "attachments": [],
        "revisions": [],
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 500,
        "deletedAtEpochMillis": null
    }]);
    let mut local = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000)).unwrap();
    local["notes"] = json!([{
        "id": "shared-note",
        "title": "deleted branch",
        "content": "deleted",
        "attachments": [],
        "revisions": [],
        "createdAtEpochMillis": 10,
        "updatedAtEpochMillis": 200,
        "deletedAtEpochMillis": 700
    }]);

    let merged = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 1_000)
        .expect("notes should merge");
    let merged: Value = serde_json::from_str(&merged).unwrap();
    let note = &merged["notes"][0];

    assert_eq!("deleted branch", note["title"]);
    assert_eq!(Some(700), note["deletedAtEpochMillis"].as_i64());
}

#[test]
fn finance_merge_keeps_disjoint_day_and_month_history_from_both_branches() {
    let mut account = json!({
        "financeProfile": {
            "activeIncomeMonthly": 100,
            "dailyLedgers": {"2026-07-01": {"note": "account day"}},
            "monthlySnapshots": {"2026-06": {"note": "account month"}}
        },
        "financeProfileUpdatedAtEpochMillis": 200,
        "financeDayLedgerRevisions": {"2026-07-01": 210},
        "financeMonthSnapshotRevisions": {"2026-06": 220},
        "tombstones": []
    });
    let local = json!({
        "financeProfile": {
            "activeIncomeMonthly": 300,
            "dailyLedgers": {"2026-07-02": {"note": "local day"}},
            "monthlySnapshots": {"2026-07": {"note": "local month"}}
        },
        "financeProfileUpdatedAtEpochMillis": 300,
        "financeDayLedgerRevisions": {"2026-07-02": 310},
        "financeMonthSnapshotRevisions": {"2026-07": 320},
        "tombstones": []
    });

    merge_app_data_values(&mut account, &local);

    assert_eq!(300, account["financeProfile"]["activeIncomeMonthly"]);
    assert_eq!(
        "account day",
        account["financeProfile"]["dailyLedgers"]["2026-07-01"]["note"]
    );
    assert_eq!(
        "local day",
        account["financeProfile"]["dailyLedgers"]["2026-07-02"]["note"]
    );
    assert_eq!(
        "account month",
        account["financeProfile"]["monthlySnapshots"]["2026-06"]["note"]
    );
    assert_eq!(
        "local month",
        account["financeProfile"]["monthlySnapshots"]["2026-07"]["note"]
    );
}

#[test]
fn finance_merge_uses_per_key_revision_and_finance_tombstone() {
    let mut account = json!({
        "financeProfile": {
            "dailyLedgers": {
                "2026-07-01": {"note": "newer account key"},
                "2026-07-02": {"note": "must be deleted"}
            },
            "monthlySnapshots": {}
        },
        "financeProfileUpdatedAtEpochMillis": 500,
        "financeDayLedgerRevisions": {"2026-07-01": 700, "2026-07-02": 600},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    let local = json!({
        "financeProfile": {
            "dailyLedgers": {"2026-07-01": {"note": "older local key"}},
            "monthlySnapshots": {}
        },
        "financeProfileUpdatedAtEpochMillis": 800,
        "financeDayLedgerRevisions": {"2026-07-01": 650},
        "financeMonthSnapshotRevisions": {},
        "tombstones": [{
            "entityType": "financeDayLedger",
            "entityId": "2026-07-02",
            "deletedAtEpochMillis": 900
        }]
    });

    merge_app_data_values(&mut account, &local);

    assert_eq!(
        "newer account key",
        account["financeProfile"]["dailyLedgers"]["2026-07-01"]["note"]
    );
    assert!(account["financeProfile"]["dailyLedgers"]
        .get("2026-07-02")
        .is_none());
    assert!(account["syncConflictHistory"]
        .as_array()
        .unwrap()
        .iter()
        .any(|conflict| conflict["entityType"] == "financeDayLedger"
            && conflict["entityId"] == "2026-07-02"
            && conflict["payload"]["note"] == "must be deleted"
            && conflict["capturedAtEpochMillis"] == 900));
}

#[test]
fn finance_same_container_merges_disjoint_entries_and_records_note_conflicts() {
    let mut account = json!({
        "financeProfile": {
            "dailyLedgers": {
                "2026-07-01": {
                    "incomes": [{"id": "income-account", "name": "account income", "kind": "ACTIVE", "amount": 10, "note": "a"}],
                    "expenses": [{"id": "expense-shared", "name": "shared", "bucket": "FOOD", "amount": 3, "note": "same"}],
                    "note": "account day"
                }
            },
            "monthlySnapshots": {
                "2026-07": {
                    "assets": [{"id": "asset-account", "name": "account asset", "kind": "PRODUCTIVE_ASSET", "amount": 100}],
                    "liabilities": [],
                    "note": "account month"
                }
            }
        },
        "financeProfileUpdatedAtEpochMillis": 100,
        "financeDayLedgerRevisions": {"2026-07-01": 200},
        "financeMonthSnapshotRevisions": {"2026-07": 210},
        "tombstones": [],
        "syncConflictHistory": []
    });
    let local = json!({
        "financeProfile": {
            "dailyLedgers": {
                "2026-07-01": {
                    "incomes": [{"id": "income-local", "name": "local income", "kind": "ASSET", "amount": 20, "note": "b"}],
                    "expenses": [{"id": "expense-shared", "name": "shared", "bucket": "FOOD", "amount": 3, "note": "same"}],
                    "note": "local day"
                }
            },
            "monthlySnapshots": {
                "2026-07": {
                    "assets": [{"id": "asset-local", "name": "local asset", "kind": "CASH_RESERVE", "amount": 200}],
                    "liabilities": [{"id": "debt-local", "name": "local debt", "kind": "LIABILITY_BALANCE", "amount": 50}],
                    "note": "local month"
                }
            }
        },
        "financeProfileUpdatedAtEpochMillis": 110,
        "financeDayLedgerRevisions": {"2026-07-01": 300},
        "financeMonthSnapshotRevisions": {"2026-07": 310},
        "tombstones": [],
        "syncConflictHistory": []
    });

    merge_app_data_values(&mut account, &local);
    let ledger = &account["financeProfile"]["dailyLedgers"]["2026-07-01"];
    assert_eq!("local day", ledger["note"]);
    assert_eq!(2, ledger["incomes"].as_array().unwrap().len());
    assert_eq!("local income", ledger["incomes"][0]["name"]);
    assert_eq!(1, ledger["expenses"].as_array().unwrap().len());
    let month = &account["financeProfile"]["monthlySnapshots"]["2026-07"];
    assert_eq!(2, month["assets"].as_array().unwrap().len());
    assert_eq!("local asset", month["assets"][0]["name"]);
    assert_eq!(1, month["liabilities"].as_array().unwrap().len());
    let conflicts = account["syncConflictHistory"].as_array().unwrap();
    assert!(conflicts.iter().any(|conflict| {
        conflict["entityType"] == "financeDayLedger"
            && conflict["entityId"] == "2026-07-01"
            && conflict["losingRevisionEpochMillis"] == 200
            && conflict["payload"]["note"] == "account day"
            && conflict["payload"]["incomes"][0]["name"] == "account income"
    }));
    assert!(conflicts.iter().any(|conflict| {
        conflict["entityType"] == "financeMonthSnapshot"
            && conflict["entityId"] == "2026-07"
            && conflict["losingRevisionEpochMillis"] == 210
            && conflict["payload"]["note"] == "account month"
            && conflict["payload"]["assets"][0]["name"] == "account asset"
    }));

    let mut reverse = local.clone();
    merge_app_data_values(
        &mut reverse,
        &json!({
            "financeProfile": {
                "dailyLedgers": {
                    "2026-07-01": {
                        "incomes": [{"id": "income-account", "name": "account income", "kind": "ACTIVE", "amount": 10, "note": "a"}],
                        "expenses": [{"id": "expense-shared", "name": "shared", "bucket": "FOOD", "amount": 3, "note": "same"}],
                        "note": "account day"
                    }
                },
                "monthlySnapshots": {
                    "2026-07": {
                        "assets": [{"id": "asset-account", "name": "account asset", "kind": "PRODUCTIVE_ASSET", "amount": 100}],
                        "liabilities": [],
                        "note": "account month"
                    }
                }
            },
            "financeProfileUpdatedAtEpochMillis": 100,
            "financeDayLedgerRevisions": {"2026-07-01": 200},
            "financeMonthSnapshotRevisions": {"2026-07": 210},
            "tombstones": [],
            "syncConflictHistory": []
        }),
    );
    assert_eq!(account["financeProfile"], reverse["financeProfile"]);
    assert_eq!(
        account["syncConflictHistory"],
        reverse["syncConflictHistory"]
    );
}

#[test]
fn finance_same_entry_id_uses_newer_fields_and_preserves_conflict_snapshot() {
    let mut account = json!({
        "financeProfile": {
            "dailyLedgers": {
                "2026-07-03": {
                    "incomes": [{"id": "salary", "name": "salary", "kind": "ACTIVE", "amount": 100, "note": "old"}],
                    "expenses": [],
                    "note": ""
                }
            },
            "monthlySnapshots": {}
        },
        "financeProfileUpdatedAtEpochMillis": 100,
        "financeDayLedgerRevisions": {"2026-07-03": 100},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    let local = json!({
        "financeProfile": {
            "dailyLedgers": {
                "2026-07-03": {
                    "incomes": [{"id": "salary", "name": "salary", "kind": "ACTIVE", "amount": 120, "note": "corrected"}],
                    "expenses": [],
                    "note": ""
                }
            },
            "monthlySnapshots": {}
        },
        "financeProfileUpdatedAtEpochMillis": 200,
        "financeDayLedgerRevisions": {"2026-07-03": 200},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });

    merge_app_data_values(&mut account, &local);
    let visible = account["financeProfile"]["dailyLedgers"]["2026-07-03"]["incomes"]
        .as_array()
        .unwrap();
    assert_eq!(1, visible.len());
    assert_eq!(120, visible[0]["amount"]);
    assert!(account["syncConflictHistory"]
        .as_array()
        .unwrap()
        .iter()
        .any(|conflict| conflict["entityType"] == "financeDayLedger"
            && conflict["entityId"] == "2026-07-03"
            && conflict["payload"]["incomes"][0]["amount"] == 100));
}

#[test]
fn finance_same_entry_id_uses_row_revision_even_when_container_revision_is_older() {
    let mut account = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-03": {
            "incomes": [{
                "id": "salary",
                "updatedAtEpochMillis": 100,
                "deletedAtEpochMillis": 0,
                "name": "salary",
                "kind": "ACTIVE",
                "amount": 100,
                "note": "container winner"
            }],
            "expenses": [],
            "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 300,
        "financeDayLedgerRevisions": {"2026-07-03": 300},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    let local = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-03": {
            "incomes": [{
                "id": "salary",
                "updatedAtEpochMillis": 250,
                "deletedAtEpochMillis": 0,
                "name": "salary",
                "kind": "ACTIVE",
                "amount": 125,
                "note": "newer row"
            }],
            "expenses": [],
            "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 200,
        "financeDayLedgerRevisions": {"2026-07-03": 200},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });

    merge_app_data_values(&mut account, &local);

    let row = &account["financeProfile"]["dailyLedgers"]["2026-07-03"]["incomes"][0];
    assert_eq!(125, row["amount"]);
    assert_eq!(250, row["updatedAtEpochMillis"]);
}

#[test]
fn finance_row_tombstone_survives_a_later_merge_from_an_old_device() {
    let live_row = json!({
        "id": "salary",
        "updatedAtEpochMillis": 100,
        "deletedAtEpochMillis": 0,
        "name": "salary",
        "kind": "ACTIVE",
        "amount": 100,
        "note": ""
    });
    let deleted_row = json!({
        "id": "salary",
        "updatedAtEpochMillis": 100,
        "deletedAtEpochMillis": 200,
        "name": "salary",
        "kind": "ACTIVE",
        "amount": 100,
        "note": ""
    });
    let mut account = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-06": {
            "incomes": [live_row.clone()], "expenses": [], "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 500,
        "financeDayLedgerRevisions": {"2026-07-06": 500},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    let deleted_branch = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-06": {
            "incomes": [deleted_row], "expenses": [], "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 400,
        "financeDayLedgerRevisions": {"2026-07-06": 400},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });

    merge_app_data_values(&mut account, &deleted_branch);
    assert_eq!(
        200,
        account["financeProfile"]["dailyLedgers"]["2026-07-06"]["incomes"][0]
            ["deletedAtEpochMillis"]
    );

    let stale_device = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-06": {
            "incomes": [live_row], "expenses": [], "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 600,
        "financeDayLedgerRevisions": {"2026-07-06": 600},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    merge_app_data_values(&mut account, &stale_device);

    let rows = account["financeProfile"]["dailyLedgers"]["2026-07-06"]["incomes"]
        .as_array()
        .unwrap();
    assert_eq!(1, rows.len());
    assert_eq!(200, rows[0]["deletedAtEpochMillis"]);
}

#[test]
fn finance_equal_row_revision_prefers_deletion_in_both_directions() {
    let active = json!({
        "id": "same",
        "updatedAtEpochMillis": 200,
        "deletedAtEpochMillis": 0,
        "amount": 10
    });
    let deleted = json!({
        "id": "same",
        "updatedAtEpochMillis": 100,
        "deletedAtEpochMillis": 200,
        "amount": 10
    });

    assert_eq!(
        200,
        merge_finance_entry_values(&active, &deleted)["deletedAtEpochMillis"]
    );
    assert_eq!(
        200,
        merge_finance_entry_values(&deleted, &active)["deletedAtEpochMillis"]
    );
}

#[test]
fn finance_merge_normalizes_missing_and_null_row_revisions_to_zero() {
    let primary = json!([{
        "id": "legacy-null",
        "updatedAtEpochMillis": null,
        "deletedAtEpochMillis": null,
        "name": "legacy",
        "amount": 10
    }]);
    let secondary = json!([{
        "id": "legacy-null",
        "name": "legacy",
        "amount": 10
    }]);

    let merged =
        merge_finance_entry_multisets(primary.as_array().unwrap(), secondary.as_array().unwrap());

    assert_eq!(1, merged.len());
    assert_eq!(Some(0), merged[0]["updatedAtEpochMillis"].as_i64());
    assert_eq!(Some(0), merged[0]["deletedAtEpochMillis"].as_i64());
}

#[test]
fn finance_legacy_multiset_union_keeps_unique_rows_without_double_counting_baseline() {
    let mut account = json!({
        "financeProfile": {
            "dailyLedgers": {"2026-07-04": {
                "incomes": [
                    {"name": "shared", "kind": "ACTIVE", "amount": 10, "note": ""},
                    {"name": "account only", "kind": "ACTIVE", "amount": 20, "note": ""}
                ],
                "expenses": [],
                "note": ""
            }},
            "monthlySnapshots": {}
        },
        "financeProfileUpdatedAtEpochMillis": 100,
        "financeDayLedgerRevisions": {"2026-07-04": 100},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    let local = json!({
        "financeProfile": {
            "dailyLedgers": {"2026-07-04": {
                "incomes": [
                    {"name": "shared", "kind": "ACTIVE", "amount": 10, "note": ""},
                    {"name": "local only", "kind": "ACTIVE", "amount": 30, "note": ""}
                ],
                "expenses": [],
                "note": ""
            }},
            "monthlySnapshots": {}
        },
        "financeProfileUpdatedAtEpochMillis": 200,
        "financeDayLedgerRevisions": {"2026-07-04": 200},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });

    merge_app_data_values(&mut account, &local);
    let rows = account["financeProfile"]["dailyLedgers"]["2026-07-04"]["incomes"]
        .as_array()
        .unwrap();
    let mut amounts = rows
        .iter()
        .filter_map(|row| row["amount"].as_i64())
        .collect::<Vec<_>>();
    amounts.sort_unstable();
    assert_eq!(vec![10, 20, 30], amounts);
    assert_eq!(1, rows.iter().filter(|row| row["name"] == "shared").count());
}

#[test]
fn finance_concurrent_income_and_expense_on_same_day_are_both_visible() {
    let mut account = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-05": {
            "incomes": [{"id": "income-a", "name": "salary", "kind": "ACTIVE", "amount": 100, "note": ""}],
            "expenses": [],
            "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 100,
        "financeDayLedgerRevisions": {"2026-07-05": 100},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });
    let local = json!({
        "financeProfile": {"dailyLedgers": {"2026-07-05": {
            "incomes": [],
            "expenses": [{"id": "expense-b", "name": "meal", "bucket": "FOOD", "amount": 20, "note": ""}],
            "note": ""
        }}, "monthlySnapshots": {}},
        "financeProfileUpdatedAtEpochMillis": 200,
        "financeDayLedgerRevisions": {"2026-07-05": 200},
        "financeMonthSnapshotRevisions": {},
        "tombstones": []
    });

    merge_app_data_values(&mut account, &local);
    let ledger = &account["financeProfile"]["dailyLedgers"]["2026-07-05"];
    assert_eq!("income-a", ledger["incomes"][0]["id"]);
    assert_eq!("expense-b", ledger["expenses"][0]["id"]);
    assert!(account["syncConflictHistory"]
        .as_array()
        .map(Vec::is_empty)
        .unwrap_or(true));
}

#[test]
fn merge_flattens_corrupted_nested_slot_order() {
    let mut account_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(1_000))
        .expect("default account app data should decode");
    account_value["slotOrder"] = json!([
        [11, 4, 5, 6, 2, 8, 1, 14, 9, 12, 3, 10, 7, 13],
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
    ]);
    let mut local_value = serde_json::from_str::<Value>(&app_data::default_app_data_json(2_000))
        .expect("default local app data should decode");
    local_value["slotOrder"] = json!([2, 1, 3]);

    let merged_json =
        merge_sync_app_data_json(&account_value.to_string(), &local_value.to_string(), 3_000)
            .expect("corrupted nested slot order should be repairable during merge");
    let merged_value: Value =
        serde_json::from_str(&merged_json).expect("merged app data should decode");
    let merged_order = merged_value["slotOrder"]
        .as_array()
        .expect("slot order should remain an array");

    assert!(merged_order.iter().all(|value| value.as_i64().is_some()));
    assert_eq!(Some(2), merged_order[0].as_i64());
    assert_eq!(Some(1), merged_order[1].as_i64());
    assert_eq!(Some(3), merged_order[2].as_i64());
    assert_eq!(Some(&json!(11)), merged_order.get(3));
    assert!(app_data::sanitize_app_data_json(&merged_json, 3_000).is_some());
    let reverse_json =
        merge_sync_app_data_json(&local_value.to_string(), &account_value.to_string(), 3_000)
            .expect("equal-revision slot order merge must converge");
    let reverse: Value = serde_json::from_str(&reverse_json).unwrap();
    assert_eq!(merged_value["slotOrder"], reverse["slotOrder"]);
}

#[test]
fn timer_slot_merge_respects_newer_blank_but_not_zero_or_equal_blank() {
    let meaningful = json!({
        "id": 1,
        "title": "active work",
        "note": "kept",
        "categoryId": 2,
        "accumulatedMillis": 100,
        "runningSinceEpochMillis": null,
        "activeRunId": null,
        "updatedAt": 100
    });
    let newer_blank = json!({
        "id": 1,
        "title": "",
        "note": "",
        "categoryId": null,
        "accumulatedMillis": 0,
        "runningSinceEpochMillis": null,
        "updatedAt": 200
    });
    let mut equal_blank = newer_blank.clone();
    equal_blank["updatedAt"] = json!(100);
    let mut zero_blank = newer_blank.clone();
    zero_blank["updatedAt"] = json!(0);

    let newer = choose_timer_slot_value(&meaningful, &newer_blank);
    assert_eq!("", newer["title"]);
    assert_eq!("", newer["note"]);
    assert!(newer["categoryId"].is_null());
    assert_eq!(0, newer["accumulatedMillis"]);
    assert_eq!(200, newer["updatedAt"]);

    let equal = choose_timer_slot_value(&meaningful, &equal_blank);
    assert_eq!("active work", equal["title"]);
    assert_eq!("kept", equal["note"]);
    assert_eq!(2, equal["categoryId"]);
    assert_eq!(100, equal["accumulatedMillis"]);
    assert_eq!(100, equal["updatedAt"]);

    let zero = choose_timer_slot_value(&meaningful, &zero_blank);
    assert_eq!("active work", zero["title"]);
    assert_eq!("kept", zero["note"]);
    assert_eq!(2, zero["categoryId"]);
    assert_eq!(100, zero["accumulatedMillis"]);
    assert_eq!(100, zero["updatedAt"]);
    assert_eq!(200, max_revision_in_value(&newer_blank).unwrap());
}

#[test]
fn timer_slot_merge_combines_independently_revised_fields_and_newer_clear() {
    let account = json!({
        "id": 1,
        "title": "account title",
        "categoryId": "work",
        "note": "old note",
        "accumulatedMillis": 100,
        "runningSinceEpochMillis": null,
        "microBreakPhase": "FOCUS",
        "microBreakCycleIndex": 0,
        "microBreakPhaseProgressMillis": 0,
        "updatedAt": 500,
        "titleUpdatedAtEpochMillis": 500,
        "categoryUpdatedAtEpochMillis": 300,
        "noteUpdatedAtEpochMillis": 100,
        "accumulatedUpdatedAtEpochMillis": 300,
        "runningUpdatedAtEpochMillis": 300,
        "microBreakUpdatedAtEpochMillis": 300
    });
    let local = json!({
        "id": 1,
        "title": "stale title",
        "categoryId": null,
        "note": "local note",
        "accumulatedMillis": 200,
        "runningSinceEpochMillis": 700,
        "activeRunId": "run-local",
        "microBreakPhase": "BREAK",
        "microBreakCycleIndex": 2,
        "microBreakPhaseProgressMillis": 50,
        "updatedAt": 800,
        "titleUpdatedAtEpochMillis": 200,
        "categoryUpdatedAtEpochMillis": 800,
        "noteUpdatedAtEpochMillis": 700,
        "accumulatedUpdatedAtEpochMillis": 600,
        "runningUpdatedAtEpochMillis": 650,
        "microBreakUpdatedAtEpochMillis": 750
    });

    let merged = choose_timer_slot_value(&account, &local);
    assert_eq!("account title", merged["title"]);
    assert!(merged["categoryId"].is_null());
    assert_eq!("local note", merged["note"]);
    assert_eq!(200, merged["accumulatedMillis"]);
    assert_eq!(700, merged["runningSinceEpochMillis"]);
    assert_eq!("run-local", merged["activeRunId"]);
    assert_eq!("BREAK", merged["microBreakPhase"]);
    assert_eq!(2, merged["microBreakCycleIndex"]);
    assert_eq!(800, merged["updatedAt"]);
    assert_eq!(500, merged["titleUpdatedAtEpochMillis"]);
    assert_eq!(800, merged["categoryUpdatedAtEpochMillis"]);

    let reverse = choose_timer_slot_value(&local, &account);
    assert_eq!(merged, reverse);
}

#[test]
fn category_merge_uses_revision_instead_of_longer_stale_name() {
    let stale = json!({
        "id": "custom",
        "name": "very long stale category name",
        "accentSeed": "red",
        "updatedAtEpochMillis": 100
    });
    let current = json!({
        "id": "custom",
        "name": "new",
        "accentSeed": "blue",
        "updatedAtEpochMillis": 200
    });
    assert_eq!(current, choose_category_value(&stale, &current));
    assert_eq!(current, choose_category_value(&current, &stale));
}

#[test]
fn session_and_archive_same_id_losers_are_deduplicated_in_conflict_history() {
    let mut account: Value = serde_json::from_str(&app_data::default_app_data_json(1)).unwrap();
    account["sessions"] = json!([{
        "id": "session-shared",
        "slotId": 1,
        "slotTitle": "account session",
        "startedAtEpochMillis": 100,
        "endedAtEpochMillis": 500,
        "durationMillis": 400,
        "updatedAtEpochMillis": 500
    }]);
    account["archivedTasks"] = json!([{
        "id": "archive-shared",
        "originalSlotId": 1,
        "title": "account archive",
        "note": "recover me",
        "accumulatedMillis": 100,
        "archivedAtEpochMillis": 500
    }]);
    let mut local = account.clone();
    local["sessions"][0]["slotTitle"] = json!("local session");
    local["sessions"][0]["endedAtEpochMillis"] = json!(600);
    local["sessions"][0]["durationMillis"] = json!(500);
    local["sessions"][0]["updatedAtEpochMillis"] = json!(600);
    local["archivedTasks"][0]["title"] = json!("local archive");
    local["archivedTasks"][0]["archivedAtEpochMillis"] = json!(600);

    let first = merge_sync_app_data_json(&account.to_string(), &local.to_string(), 100)
        .expect("same-id histories should merge");
    let first_value: Value = serde_json::from_str(&first).unwrap();
    let conflicts = first_value["syncConflictHistory"].as_array().unwrap();
    assert!(conflicts.iter().any(|conflict| {
        conflict["entityType"] == "session"
            && conflict["entityId"] == "session-shared"
            && conflict["payload"]["slotTitle"] == "account session"
    }));
    assert!(conflicts.iter().any(|conflict| {
        conflict["entityType"] == "archivedTask"
            && conflict["entityId"] == "archive-shared"
            && conflict["payload"]["title"] == "account archive"
    }));

    let replay = merge_sync_app_data_json(&first, &local.to_string(), 100).unwrap();
    let replay: Value = serde_json::from_str(&replay).unwrap();
    assert_eq!(
        conflicts.len(),
        replay["syncConflictHistory"].as_array().unwrap().len(),
        "first={} replay={}",
        first_value,
        replay
    );
}

#[test]
fn session_merge_uses_updated_at_without_corrupting_business_interval() {
    let account = json!({
        "id": "run-1-focus-0",
        "slotId": 1,
        "slotTitle": "account correction",
        "startedAtEpochMillis": 100,
        "endedAtEpochMillis": 300,
        "durationMillis": 200,
        "updatedAtEpochMillis": 900
    });
    let local = json!({
        "id": "run-1-focus-0",
        "slotId": 1,
        "slotTitle": "stale long interval",
        "startedAtEpochMillis": 100,
        "endedAtEpochMillis": 9_999,
        "durationMillis": 9_899,
        "updatedAtEpochMillis": 800
    });
    assert_eq!(account, choose_session_value(&account, &local));

    let mut account_root: Value = json!({
        "sessions": [account.clone()],
        "syncConflictHistory": []
    });
    let local_root: Value = json!({"sessions": [local.clone()]});
    merge_app_data_values(&mut account_root, &local_root);
    assert_eq!(300, account_root["sessions"][0]["endedAtEpochMillis"]);
    assert_eq!(200, account_root["sessions"][0]["durationMillis"]);
    let conflict = account_root["syncConflictHistory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["entityType"] == "session")
        .unwrap();
    assert_eq!(800, conflict["losingRevisionEpochMillis"]);
    assert_eq!(9_999, conflict["payload"]["endedAtEpochMillis"]);
}

#[test]
fn password_hash_uses_salt() {
    assert_ne!(password_hash("a", "secret"), password_hash("b", "secret"));
    assert_eq!(password_hash("a", "secret"), password_hash("a", "secret"));
}

#[test]
fn base_url_accepts_http_and_https() {
    let local = parse_base_url("http://127.0.0.1:8917").expect("http URL should parse");
    assert_eq!(SyncUrlScheme::Http, local.scheme);
    assert_eq!(8917, local.port);

    let public = parse_base_url("https://sync.example.com/api").expect("https URL should parse");
    assert_eq!(SyncUrlScheme::Https, public.scheme);
    assert_eq!(443, public.port);
    assert_eq!("https://sync.example.com/api", format_base_url(&public));
}

#[test]
fn response_reader_finishes_once_content_length_is_satisfied() {
    let body = "{\"ok\":true,\"message\":\"Logged in.\",\"mode\":\"ok\"}";
    let response = format!(
        concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/json\r\n",
            "Content-Length: {}\r\n",
            "Connection: close\r\n",
            "\r\n",
            "{}"
        ),
        body.len(),
        body
    )
    .into_bytes();
    let split_index = 58;
    let mut reader = ScriptedReader::new(vec![
        Ok(response[..split_index].to_vec()),
        Ok(response[split_index..].to_vec()),
        Err(io::Error::new(io::ErrorKind::TimedOut, "late eof timeout")),
    ]);

    let payload = read_http_response(&mut reader).expect("response should be readable");
    assert_eq!(payload, response);

    let parsed = parse_client_response(&payload);
    assert!(parsed.ok);
    assert_eq!("Logged in.", parsed.message);
}

#[test]
fn lan_discovery_candidates_stay_in_same_subnet() {
    let candidates = sibling_ipv4_candidates(Ipv4Addr::new(192, 168, 1, 23));

    assert_eq!(253, candidates.len());
    assert!(candidates.contains(&Ipv4Addr::new(192, 168, 1, 134)));
    assert!(!candidates.contains(&Ipv4Addr::new(192, 168, 1, 23)));
    assert!(!candidates.contains(&Ipv4Addr::new(192, 168, 2, 134)));
}

#[test]
fn public_hosts_are_not_scanned() {
    assert!(!is_discoverable_lan_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(is_discoverable_lan_ipv4(Ipv4Addr::new(192, 168, 1, 23)));
}

#[test]
fn resolved_server_url_serializes_as_camel_case() {
    let encoded = encode_result(&SyncClientResult {
        ok: true,
        message: "ok".to_string(),
        resolved_server_url: "http://192.168.1.134:8917".to_string(),
        public_server_url: "http://8.8.8.8:8917".to_string(),
        ..SyncClientResult::default()
    });

    assert!(encoded.contains("\"resolvedServerUrl\":\"http://192.168.1.134:8917\""));
    assert!(encoded.contains("\"publicServerUrl\":\"http://8.8.8.8:8917\""));
}

#[test]
fn public_sync_ipv4_rejects_private_and_cgnat_ranges() {
    assert!(is_public_sync_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(!is_public_sync_ipv4(Ipv4Addr::new(192, 168, 1, 134)));
    assert!(!is_public_sync_ipv4(Ipv4Addr::new(100, 64, 1, 2)));
}

#[test]
fn public_url_rejects_local_authority_and_request_line_injection() {
    assert_eq!(
        Some("https://sync.example.com".to_string()),
        normalized_public_server_url("https://sync.example.com/")
    );
    for rejected in [
        "https://localhost",
        "https://machine.local",
        "https://127.0.0.1",
        "https://192.168.1.2",
        "https://[::1]",
        "https://[fe80::1]",
        "https://user@sync.example.com",
        "https://sync.example.com/path",
        "https://sync.example.com?query=1",
        "https://sync.example.com#fragment",
        "https://sync.example.com\r\nInjected: yes",
    ] {
        assert_eq!(None, normalized_public_server_url(rejected), "{rejected}");
    }
    assert!(parse_base_url("https://user@sync.example.com").is_err());
    assert!(parse_base_url("https://sync.example.com/path?query=1").is_err());
}

#[test]
fn request_parser_rejects_transfer_encoding_duplicate_length_and_short_body() {
    let transfer_encoding = b"POST /v1/login HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
    assert_eq!(
        io::ErrorKind::InvalidData,
        parse_content_length(transfer_encoding, false)
            .expect_err("Transfer-Encoding must be rejected")
            .kind()
    );
    let duplicate_length =
        b"POST /v1/login HTTP/1.1\r\nContent-Length: 3\r\nContent-Length: 3\r\n\r\n";
    assert_eq!(
        io::ErrorKind::InvalidData,
        parse_content_length(duplicate_length, false)
            .expect_err("duplicate Content-Length must be rejected")
            .kind()
    );
    let short_body = b"POST /v1/login HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort".to_vec();
    assert_eq!(
        io::ErrorKind::UnexpectedEof,
        read_request_from_bytes(short_body)
            .expect_err("short body must fail")
            .kind()
    );
}

#[test]
fn request_parser_caps_header_bytes() {
    let oversized = format!(
        "GET /health HTTP/1.1\r\nX-Fill: {}",
        "a".repeat(MAX_HEADER_BYTES + 1)
    )
    .into_bytes();
    assert_eq!(
        io::ErrorKind::InvalidData,
        read_request_from_bytes(oversized)
            .expect_err("oversized headers must fail")
            .kind()
    );
}

#[test]
fn auth_rate_limiter_combines_register_and_login_budget() {
    let mut limiter = LoginRateLimiter::default();
    let address = IpAddr::V4(Ipv4Addr::LOCALHOST);
    for attempt in 0..LOGIN_RATE_MAX_ATTEMPTS {
        assert!(limiter.allow(address, attempt as i64));
    }
    assert!(!limiter.allow(address, LOGIN_RATE_MAX_ATTEMPTS as i64));
    assert!(limiter.allow(address, LOGIN_RATE_WINDOW_MILLIS + 1));
}

#[test]
fn missing_live_database_is_restored_from_verified_startup_backup_before_open() {
    let test_store = account_test_store("missing-live-token", app_data::default_app_data_json(100));
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let expected_identity = test_store.store.server_instance_id().unwrap();
    let expected_account = test_store.store.read_account("user-1").unwrap();
    let workspace_id = "67".repeat(32);
    let expected_workspace_proof = test_store
        .store
        .workspace_capability_proof("user-1", &workspace_id, 0)
        .unwrap();
    let source_backup =
        ensure_startup_backup(&test_store.store, &sqlite_path, 1_000, true).unwrap();
    fs::remove_file(&sqlite_path).unwrap();
    let _ = fs::remove_file(sync_sqlite_sidecar_path(&sqlite_path, "-wal"));
    let _ = fs::remove_file(sync_sqlite_sidecar_path(&sqlite_path, "-shm"));

    let isolated_runtime_directory = test_store.directory.join("isolated-runtime-recovery");
    let (recovered, recovered_path, post_recovery_backup) =
        initialize_sqlite_store_with_runtime_recovery_directory(
            &sqlite_path,
            Some(&isolated_runtime_directory),
        )
        .expect("verified backup should restore live DB");
    assert_eq!(sqlite_path, recovered_path);
    assert_eq!(expected_identity, recovered.server_instance_id().unwrap());
    assert_eq!(expected_account, recovered.read_account("user-1").unwrap());
    assert!(recovered
        .verify_workspace_capability("user-1", &workspace_id, 0, &expected_workspace_proof,)
        .unwrap());
    assert!(matches!(
        recovered
            .authenticate_token("missing-live-token", now_millis())
            .unwrap(),
        TokenAuthentication::Active(token) if token.user_id == "user-1"
    ));
    assert!(source_backup.exists());
    assert!(post_recovery_backup.exists());
    recovered.validate_integrity().unwrap();
}

#[test]
fn mismatched_v2_startup_state_binding_cannot_fall_back_to_its_backup() {
    let test_store = account_test_store(
        "wrong-startup-binding",
        app_data::default_app_data_json(100),
    );
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let startup_backup =
        ensure_startup_backup(&test_store.store, &sqlite_path, 1_000, true).unwrap();
    let state_path = startup_backup_state_path(&sqlite_path);
    let mut state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    let actual = state["targetStoreFingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    state["targetStoreFingerprint"] = json!(if actual.starts_with('f') {
        "e".repeat(64)
    } else {
        "f".repeat(64)
    });
    fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    fs::remove_file(&sqlite_path).unwrap();
    let _ = fs::remove_file(sync_sqlite_sidecar_path(&sqlite_path, "-wal"));
    let _ = fs::remove_file(sync_sqlite_sidecar_path(&sqlite_path, "-shm"));

    let runtime_directory = test_store.directory.join("isolated-runtime-recovery");
    let error = recover_missing_sqlite_store_if_needed(&sqlite_path, &runtime_directory, 2_000)
        .expect_err("a mismatched target binding must fail closed");
    assert_eq!(io::ErrorKind::InvalidData, error.kind());
    assert!(!sqlite_path.exists());
    assert!(startup_backup.exists());
}

#[test]
fn deleted_live_directory_is_restored_from_independent_runtime_backup() {
    let test_store = account_test_store(
        "runtime-restore-token",
        app_data::default_app_data_json(100),
    );
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let expected_identity = test_store.store.server_instance_id().unwrap();
    let expected_account = test_store.store.read_account("user-1").unwrap();
    let workspace_id = "89".repeat(32);
    let expected_workspace_proof = test_store
        .store
        .workspace_capability_proof("user-1", &workspace_id, 0)
        .unwrap();
    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    let runtime_backup =
        perform_runtime_backup(&test_store.store, &recovery_directory, 2_000).unwrap();
    fs::remove_dir_all(&test_store.directory).unwrap();
    assert!(!sqlite_path.exists());
    assert!(!sqlite_path.parent().unwrap().exists());

    let restored_source =
        recover_missing_sqlite_store_if_needed(&sqlite_path, &recovery_directory, 3_000)
            .unwrap()
            .expect("independent runtime backup should be selected");
    assert_eq!(runtime_backup.destination, restored_source);
    let (recovered, _, _) = initialize_sqlite_store_with_runtime_recovery_directory(
        &sqlite_path,
        Some(&recovery_directory),
    )
    .unwrap();
    assert_eq!(expected_identity, recovered.server_instance_id().unwrap());
    assert_eq!(expected_account, recovered.read_account("user-1").unwrap());
    assert!(recovered
        .verify_workspace_capability("user-1", &workspace_id, 0, &expected_workspace_proof,)
        .unwrap());
    assert!(matches!(
        recovered
            .authenticate_token("runtime-restore-token", now_millis())
            .unwrap(),
        TokenAuthentication::Active(token) if token.user_id == "user-1"
    ));
    drop(recovered);
    fs::remove_dir_all(&recovery_directory).unwrap();
}

#[test]
fn shared_runtime_directory_recovers_only_the_bound_target_store() {
    let store_a = account_test_store("bound-runtime-a", note_app_data("store-a", 100));
    let store_b = account_test_store("bound-runtime-b", note_app_data("store-b", 200));
    let sqlite_path_a = store_a.store.database_path().to_path_buf();
    let identity_a = store_a.store.server_instance_id().unwrap();
    let identity_b = store_b.store.server_instance_id().unwrap();
    assert_ne!(identity_a, identity_b);
    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    let backup_a = perform_runtime_backup(&store_a.store, &recovery_directory, 1_000).unwrap();
    let backup_b = perform_runtime_backup(&store_b.store, &recovery_directory, 2_000).unwrap();
    assert!(runtime_backup_manifest_path(&backup_a.destination).is_file());
    assert!(runtime_backup_manifest_path(&backup_b.destination).is_file());

    let unbound_target = std::env::temp_dir()
        .join(format!("gridtimer_sync_core_test_{}", random_token(12)))
        .join("server_store.sqlite3");
    let wrong_target_error =
        recover_missing_sqlite_store_if_needed(&unbound_target, &recovery_directory, 3_000)
            .expect_err("a different target path must not borrow another store's backup");
    assert_eq!(io::ErrorKind::InvalidData, wrong_target_error.kind());
    assert!(!unbound_target.exists());

    fs::remove_dir_all(&store_a.directory).unwrap();
    let restored_source =
        recover_missing_sqlite_store_if_needed(&sqlite_path_a, &recovery_directory, 3_001)
            .unwrap()
            .expect("the backup bound to the requested path should be selected");
    assert_eq!(backup_a.destination, restored_source);
    assert_ne!(backup_b.destination, restored_source);
    let recovered = SqliteServerStore::open(&sqlite_path_a, None).unwrap();
    assert_eq!(identity_a, recovered.server_instance_id().unwrap());
    assert!(recovered
        .read_account("user-1")
        .unwrap()
        .app_data_json
        .contains("store-a"));
    drop(recovered);
    fs::remove_dir_all(&recovery_directory).unwrap();
    let _ = fs::remove_dir_all(unbound_target.parent().unwrap());
}

#[test]
fn unbound_legacy_runtime_backups_with_multiple_identities_fail_closed() {
    let store_a = account_test_store("legacy-runtime-a", note_app_data("legacy-a", 100));
    let store_b = account_test_store("legacy-runtime-b", note_app_data("legacy-b", 200));
    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    fs::create_dir_all(&recovery_directory).unwrap();
    for (store, timestamp) in [(&store_a.store, 1_000), (&store_b.store, 2_000)] {
        let identity = store.server_instance_id().unwrap();
        let destination =
            unique_runtime_backup_path(&recovery_directory, &identity, timestamp).unwrap();
        store
            .create_verified_backup(&destination, timestamp)
            .expect("legacy backup should verify");
        assert!(!runtime_backup_manifest_path(&destination).exists());
    }
    let target = std::env::temp_dir()
        .join(format!("gridtimer_sync_core_test_{}", random_token(12)))
        .join("server_store.sqlite3");
    let error = recover_missing_sqlite_store_if_needed(&target, &recovery_directory, 3_000)
        .expect_err("multiple unbound identities must be treated as ambiguous");
    assert_eq!(io::ErrorKind::InvalidData, error.kind());
    assert!(error
        .to_string()
        .contains("refusing to create an empty database"));
    assert!(!target.exists());
    fs::remove_dir_all(&recovery_directory).unwrap();
    let _ = fs::remove_dir_all(target.parent().unwrap());
}

#[test]
fn one_unbound_legacy_runtime_identity_without_anchor_fails_closed() {
    let source = account_test_store("single-legacy-runtime", note_app_data("single-legacy", 100));
    let identity = source.store.server_instance_id().unwrap();
    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    fs::create_dir_all(&recovery_directory).unwrap();
    let destination = unique_runtime_backup_path(&recovery_directory, &identity, 1_000).unwrap();
    source
        .store
        .create_verified_backup(&destination, 1_000)
        .unwrap();
    let target = std::env::temp_dir()
        .join(format!("gridtimer_sync_core_test_{}", random_token(12)))
        .join("server_store.sqlite3");
    let error = recover_missing_sqlite_store_if_needed(&target, &recovery_directory, 2_000)
        .expect_err("an unbound runtime copy cannot identify an unrelated missing target");
    assert_eq!(io::ErrorKind::InvalidData, error.kind());
    assert!(!target.exists());
    assert!(destination.exists());
    fs::remove_dir_all(&recovery_directory).unwrap();
    let _ = fs::remove_dir_all(target.parent().unwrap());
}

#[test]
fn legacy_runtime_backup_uses_identity_derived_from_v1_startup_state() {
    let source = account_test_store(
        "anchored-legacy-runtime",
        note_app_data("anchored-legacy", 100),
    );
    let sqlite_path = source.store.database_path().to_path_buf();
    let identity = source.store.server_instance_id().unwrap();
    ensure_startup_backup(&source.store, &sqlite_path, 1_000, true).unwrap();

    let state_path = startup_backup_state_path(&sqlite_path);
    let mut legacy_state: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    legacy_state.as_object_mut().unwrap().remove("stateVersion");
    legacy_state
        .as_object_mut()
        .unwrap()
        .remove("serverInstanceId");
    legacy_state
        .as_object_mut()
        .unwrap()
        .remove("targetStoreFingerprint");
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&legacy_state).unwrap(),
    )
    .unwrap();

    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    fs::create_dir_all(&recovery_directory).unwrap();
    let legacy_runtime = unique_runtime_backup_path(&recovery_directory, &identity, 2_000).unwrap();
    source
        .store
        .create_verified_backup(&legacy_runtime, 2_000)
        .unwrap();
    assert!(!runtime_backup_manifest_path(&legacy_runtime).exists());

    fs::remove_file(&sqlite_path).unwrap();
    let _ = fs::remove_file(sync_sqlite_sidecar_path(&sqlite_path, "-wal"));
    let _ = fs::remove_file(sync_sqlite_sidecar_path(&sqlite_path, "-shm"));
    let selected = recover_missing_sqlite_store_if_needed(&sqlite_path, &recovery_directory, 3_000)
        .unwrap()
        .expect("the verified local state should anchor the legacy runtime identity");
    assert_eq!(legacy_runtime, selected);
    let recovered = SqliteServerStore::open(&sqlite_path, None).unwrap();
    assert_eq!(identity, recovered.server_instance_id().unwrap());
    drop(recovered);
    fs::remove_dir_all(&recovery_directory).unwrap();
}

#[test]
fn corrupt_runtime_backup_evidence_prevents_empty_first_run() {
    let root = std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    let sqlite_path = root.join("deleted-live").join("server_store.sqlite3");
    let recovery_directory = root.join("independent-recovery");
    fs::create_dir_all(&recovery_directory).unwrap();
    let fake_identity = "a".repeat(64);
    let corrupt =
        recovery_directory.join(format!("sync_server_{fake_identity}_runtime_1000.sqlite3"));
    fs::write(&corrupt, b"corrupt runtime backup").unwrap();

    let error = recover_missing_sqlite_store_if_needed(&sqlite_path, &recovery_directory, 2_000)
        .unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, error.kind());
    assert!(error
        .to_string()
        .contains("refusing to create an empty database"));
    assert!(!sqlite_path.exists());
    assert!(corrupt.exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn runtime_backup_filename_identity_must_match_verified_database_identity() {
    let test_store = account_test_store(
        "runtime-identity-token",
        app_data::default_app_data_json(100),
    );
    let recovery_directory = test_store.directory.join("identity-recovery");
    let report = perform_runtime_backup(&test_store.store, &recovery_directory, 1_000).unwrap();
    let actual_identity = test_store.store.server_instance_id().unwrap();
    let spoofed_identity = if actual_identity.starts_with('a') {
        "b".repeat(64)
    } else {
        "a".repeat(64)
    };
    let spoofed_path = recovery_directory.join(format!(
        "sync_server_{spoofed_identity}_runtime_1000.sqlite3"
    ));
    fs::rename(&report.destination, &spoofed_path).unwrap();

    let (evidence, candidate) =
        latest_verified_runtime_backup(&recovery_directory, test_store.store.database_path(), None)
            .unwrap();
    assert!(evidence);
    assert!(candidate.is_none());
    assert!(spoofed_path.exists());
}

#[test]
fn unreadable_runtime_backup_location_never_falls_through_to_empty_first_run() {
    let root = std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    fs::create_dir_all(&root).unwrap();
    let sqlite_path = root.join("deleted-live").join("server_store.sqlite3");
    let invalid_recovery_location = root.join("configured-recovery-location");
    fs::write(&invalid_recovery_location, b"this path must be a directory").unwrap();

    assert!(recover_missing_sqlite_store_if_needed(
        &sqlite_path,
        &invalid_recovery_location,
        2_000,
    )
    .is_err());
    assert!(!sqlite_path.exists());
    assert!(invalid_recovery_location.is_file());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_live_database_with_only_corrupt_evidence_fails_closed() {
    let directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    fs::create_dir_all(&directory).unwrap();
    let sqlite_path = directory.join("server_store.sqlite3");
    let corrupt_backup = directory.join("server_store_startup_1000.sqlite3");
    fs::write(&corrupt_backup, b"not a sqlite backup").unwrap();
    fs::write(startup_backup_state_path(&sqlite_path), b"{broken state").unwrap();

    let isolated_runtime_directory = directory.join("isolated-runtime-recovery");
    let error = initialize_sqlite_store_with_runtime_recovery_directory(
        &sqlite_path,
        Some(&isolated_runtime_directory),
    )
    .unwrap_err();
    assert_eq!(io::ErrorKind::InvalidData, error.kind());
    assert!(error
        .to_string()
        .contains("refusing to create an empty database"));
    assert!(!sqlite_path.exists());
    assert!(corrupt_backup.exists());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn occupied_endpoint_fails_before_a_second_server_touches_the_store() {
    let endpoint_guard = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let bind_addr = endpoint_guard.local_addr().unwrap().to_string();
    let directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    let sqlite_path = directory.join("server_store.sqlite3");
    assert!(!directory.exists());

    let error = run_sync_server(&bind_addr, &sqlite_path)
        .expect_err("a second server must fail before initializing shared storage");

    assert_eq!(io::ErrorKind::AddrInUse, error.kind());
    assert!(
        !directory.exists(),
        "port ownership must be established before database creation or migration"
    );
    drop(endpoint_guard);
}

#[test]
fn initial_runtime_backup_retries_transient_sqlite_disk_io_without_interval_gap() {
    let mut attempts = 0_usize;
    let mut waits = Vec::new();

    let result = retry_transient_runtime_backup(
        "initial runtime recovery backup",
        4,
        || {
            attempts += 1;
            if attempts < 3 {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "SQLite error: disk I/O error",
                ))
            } else {
                Ok("verified")
            }
        },
        |delay| waits.push(delay),
    )
    .expect("a transient SQLite I/O failure should be retried immediately");

    assert_eq!("verified", result);
    assert_eq!(3, attempts);
    assert_eq!(
        vec![Duration::from_millis(100), Duration::from_millis(200)],
        waits
    );
    assert!(
        waits.iter().copied().sum::<Duration>()
            < Duration::from_millis(RUNTIME_BACKUP_MIN_INTERVAL_MILLIS as u64),
        "startup recovery must not wait for the periodic six-hour cycle"
    );
}

#[test]
fn interrupted_runtime_backup_marker_cleans_only_its_owned_temp_files() {
    let directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    fs::create_dir_all(&directory).unwrap();
    let server_instance_id = "a".repeat(64);
    let target_store_fingerprint = "b".repeat(64);
    let destination = directory.join(format!(
        "sync_server_{server_instance_id}_runtime_1000.sqlite3"
    ));
    let prefix = runtime_backup_temp_file_prefix(&destination, 1_000);
    let pending = PendingRuntimeBackup {
        state_version: RUNTIME_BACKUP_PENDING_VERSION,
        target_store_fingerprint: target_store_fingerprint.clone(),
        destination_file_name: destination
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        temp_file_prefix: prefix.clone(),
        created_at_epoch_millis: 1_000,
    };
    write_pending_runtime_backup(&directory, &pending).unwrap();
    let owned = [
        format!("{prefix}0.sqlite3"),
        format!("{prefix}0.sqlite3-wal"),
        format!("{prefix}0.sqlite3-shm"),
        format!("{prefix}0.sqlite3-journal"),
    ];
    for name in &owned {
        fs::write(directory.join(name), b"interrupted").unwrap();
    }
    let near_match = directory.join(format!("{prefix}unsafe.sqlite3"));
    fs::write(&near_match, b"preserve").unwrap();

    assert_eq!(
        owned.len(),
        cleanup_pending_runtime_backup(&directory, &target_store_fingerprint).unwrap()
    );
    for name in &owned {
        assert!(!directory.join(name).exists());
    }
    assert!(near_match.exists());
    assert!(
        !pending_runtime_backup_path(&directory, &target_store_fingerprint)
            .unwrap()
            .exists()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn connection_capacity_response_carries_verified_server_identity() {
    let busy = busy_server_result();
    assert!(!busy.ok);
    assert_eq!("busy", busy.mode);
    assert_eq!(SYNC_SERVER_BUILD_ID, busy.server_build_id);
    assert!(!busy.server_process_name.is_empty());
}

#[test]
fn truly_first_run_can_create_a_new_database_and_verified_backup() {
    let directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    fs::create_dir_all(&directory).unwrap();
    let sqlite_path = directory.join("server_store.sqlite3");

    let isolated_runtime_directory = directory.join("isolated-runtime-recovery");
    let (store, opened_path, backup) = initialize_sqlite_store_with_runtime_recovery_directory(
        &sqlite_path,
        Some(&isolated_runtime_directory),
    )
    .expect("first run should create a store");
    assert_eq!(sqlite_path, opened_path);
    assert!(sqlite_path.exists());
    assert!(backup.exists());
    assert_eq!(0, store.stats().unwrap().users);
    store.validate_integrity().unwrap();
    drop(store);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn startup_backup_pruning_matches_only_owned_files_and_uses_both_limits() {
    let test_store = sqlite_test_store(ServerStore::default());
    let now = 20_000_000_000_000_i64;
    for offset in 0..99_i64 {
        fs::write(
            test_store
                .directory
                .join(format!("server_store_startup_{}.sqlite3", now - offset)),
            b"backup",
        )
        .expect("recent backup should be written");
    }
    let cutoff = now - STARTUP_BACKUP_RETAIN_MILLIS;
    let retained_by_count = test_store
        .directory
        .join(format!("server_store_startup_{}.sqlite3", cutoff - 1));
    let deleted_one = test_store
        .directory
        .join(format!("server_store_startup_{}.sqlite3", cutoff - 2));
    let deleted_two = test_store
        .directory
        .join(format!("server_store_startup_{}_1.sqlite3", cutoff - 3));
    for path in [&retained_by_count, &deleted_one, &deleted_two] {
        fs::write(path, b"old backup").expect("old backup should be written");
    }
    let protected_legacy = test_store.directory.join(format!(
        "server_store_pre_sqlite_migration_{}.json",
        cutoff - 4
    ));
    let protected_near_match = test_store.directory.join(format!(
        "server_store_startup_{}_unsafe.sqlite3",
        cutoff - 5
    ));
    fs::write(&protected_legacy, b"legacy").expect("legacy backup should be written");
    fs::write(&protected_near_match, b"near match").expect("near match should be written");

    assert_eq!(
        2,
        prune_old_startup_backups(test_store.store.database_path(), now, &retained_by_count,)
            .expect("backup pruning should succeed")
    );
    assert!(retained_by_count.exists());
    assert!(!deleted_one.exists());
    assert!(!deleted_two.exists());
    assert!(protected_legacy.exists());
    assert!(protected_near_match.exists());
    assert_eq!(
        Some(cutoff - 3),
        startup_backup_timestamp(deleted_two.file_name().unwrap())
    );
    assert_eq!(
        None,
        startup_backup_timestamp(protected_near_match.file_name().unwrap())
    );
}

#[test]
fn unchanged_startups_reuse_one_verified_backup() {
    let test_store = account_test_store("backup-token", app_data::default_app_data_json(100));
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let first = ensure_startup_backup(&test_store.store, &sqlite_path, 1_000, true)
        .expect("first startup backup should be created");
    let second = ensure_startup_backup(&test_store.store, &sqlite_path, 2_000, false)
        .expect("unchanged startup should reuse the verified backup");

    assert_eq!(first, second);
    let backups = fs::read_dir(&test_store.directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| startup_backup_timestamp(&entry.file_name()).is_some())
        .count();
    assert_eq!(1, backups);
    SqliteServerStore::verify_existing_backup(&first, 1_000)
        .expect("reused startup backup should remain valid");
}

#[test]
fn missing_backup_state_throttles_once_but_cannot_pin_a_stale_backup_forever() {
    let test_store = account_test_store("state-token", app_data::default_app_data_json(100));
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let first = ensure_startup_backup(&test_store.store, &sqlite_path, 1_000, true).unwrap();
    let current = test_store.store.read_account("user-1").unwrap();
    test_store
        .store
        .compare_and_swap_account(
            "user-1",
            current.revision,
            "{\"changedAfterBackup\":true}",
            1_500,
        )
        .unwrap();
    fs::remove_file(startup_backup_state_path(&sqlite_path)).unwrap();

    let throttled = ensure_startup_backup(&test_store.store, &sqlite_path, 2_000, false)
        .expect("recent verified backup may throttle this startup");
    assert_eq!(first, throttled);
    assert!(!startup_backup_state_path(&sqlite_path).exists());

    let refreshed = ensure_startup_backup(
        &test_store.store,
        &sqlite_path,
        1_000 + STARTUP_BACKUP_MIN_INTERVAL_MILLIS + 1,
        false,
    )
    .expect("expired throttle must create a current backup");
    assert_ne!(first, refreshed);
    assert!(startup_backup_state_path(&sqlite_path).exists());
}

#[test]
fn startup_backup_byte_budget_keeps_latest_verified_copy() {
    let test_store = account_test_store("budget-token", app_data::default_app_data_json(100));
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let mut backups = Vec::new();
    for (index, timestamp) in [1_000_i64, 2_000, 3_000].into_iter().enumerate() {
        if index > 0 {
            let current = test_store.store.read_account("user-1").unwrap();
            test_store
                .store
                .compare_and_swap_account(
                    "user-1",
                    current.revision,
                    &format!("{{\"backupGeneration\":{index}}}"),
                    timestamp,
                )
                .unwrap();
        }
        let path = unique_startup_backup_path(&sqlite_path, timestamp).unwrap();
        backups.push(
            test_store
                .store
                .create_verified_backup(&path, timestamp)
                .unwrap(),
        );
    }
    let latest = backups.last().unwrap().clone();

    let deleted = prune_startup_backups_with_limits(
        &sqlite_path,
        4_000,
        100,
        i64::MAX,
        latest.size_bytes,
        Some(&latest.destination),
    )
    .unwrap();

    assert_eq!(2, deleted);
    assert!(latest.destination.exists());
    assert!(!backups[0].destination.exists());
    assert!(!backups[1].destination.exists());
    let verified = SqliteServerStore::verify_existing_backup(&latest.destination, 3_000)
        .expect("latest retained backup should verify");
    assert_eq!(latest.sha256, verified.sha256);
}

#[test]
fn startup_backup_budget_never_sacrifices_older_verified_copy_for_newer_corruption() {
    let test_store = account_test_store("corrupt-token", app_data::default_app_data_json(100));
    let sqlite_path = test_store.store.database_path().to_path_buf();
    let valid_path = unique_startup_backup_path(&sqlite_path, 1_000).unwrap();
    let valid = test_store
        .store
        .create_verified_backup(&valid_path, 1_000)
        .unwrap();
    let corrupt_path = test_store
        .directory
        .join("server_store_startup_2000.sqlite3");
    fs::write(&corrupt_path, vec![0xA5; valid.size_bytes as usize]).unwrap();
    let (verified_latest, _) = latest_verified_startup_backup(&sqlite_path)
        .unwrap()
        .expect("older valid backup should be found past newer corruption");
    assert_eq!(valid.destination, verified_latest.destination);

    prune_startup_backups_with_limits(
        &sqlite_path,
        3_000,
        100,
        i64::MAX,
        valid.size_bytes,
        Some(&verified_latest.destination),
    )
    .unwrap();

    assert!(valid.destination.exists());
    assert!(!corrupt_path.exists());
    SqliteServerStore::verify_existing_backup(&valid.destination, 1_000).unwrap();
}

#[test]
fn runtime_backup_uses_separate_directory_and_refuses_corrupt_primary_without_pruning() {
    let test_store = account_test_store("runtime-backup-token", app_data::default_app_data_json(1));
    let recovery_directory = test_store.directory.join("recovery-volume");
    let first = perform_runtime_backup(&test_store.store, &recovery_directory, 1_000).unwrap();
    assert_eq!(
        Some(recovery_directory.as_path()),
        first.destination.parent()
    );
    SqliteServerStore::verify_existing_backup(&first.destination, 1_000).unwrap();
    let server_instance_id = test_store.store.server_instance_id().unwrap();
    assert_eq!(
        Some(1_000),
        runtime_backup_timestamp(first.destination.file_name().unwrap(), &server_instance_id)
    );

    let account = test_store.store.read_account("user-1").unwrap();
    test_store
        .store
        .compare_and_swap_account(
            "user-1",
            account.revision,
            &app_data::default_app_data_json(2),
            2,
        )
        .unwrap();
    let second = perform_runtime_backup(&test_store.store, &recovery_directory, 2_000).unwrap();
    assert_ne!(first.sha256, second.sha256);
    let verified_count_before_corruption = fs::read_dir(&recovery_directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| runtime_backup_timestamp(&entry.file_name(), &server_instance_id).is_some())
        .count();

    let connection = rusqlite::Connection::open(test_store.store.database_path()).unwrap();
    connection
        .execute(
            "UPDATE account_snapshots SET updated_at_epoch_millis = 999999 \
                 WHERE user_id = 'user-1'",
            [],
        )
        .unwrap();
    drop(connection);
    assert!(perform_runtime_backup(&test_store.store, &recovery_directory, 3_000).is_err());
    let backup_count_after_failure = fs::read_dir(&recovery_directory)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| runtime_backup_timestamp(&entry.file_name(), &server_instance_id).is_some())
        .count();
    assert_eq!(verified_count_before_corruption, backup_count_after_failure);
    assert!(first.destination.exists());
    assert!(second.destination.exists());
}

#[test]
fn runtime_rotation_isolated_by_target_path_for_cloned_server_identity() {
    let original = account_test_store("runtime-clone-original", app_data::default_app_data_json(1));
    let clone_directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    fs::create_dir_all(&clone_directory).unwrap();
    let clone_path = clone_directory.join("server_store.sqlite3");
    original
        .store
        .create_verified_backup(&clone_path, 500)
        .unwrap();
    let clone = TestSqliteStore {
        store: SqliteServerStore::open(&clone_path, None).unwrap(),
        directory: clone_directory,
    };
    let server_instance_id = original.store.server_instance_id().unwrap();
    assert_eq!(
        server_instance_id,
        clone.store.server_instance_id().unwrap()
    );
    let original_fingerprint =
        runtime_backup_target_fingerprint(original.store.database_path()).unwrap();
    let clone_fingerprint = runtime_backup_target_fingerprint(clone.store.database_path()).unwrap();
    assert_ne!(original_fingerprint, clone_fingerprint);

    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    let original_old = perform_runtime_backup(&original.store, &recovery_directory, 1_000).unwrap();
    let clone_old = perform_runtime_backup(&clone.store, &recovery_directory, 1_100).unwrap();
    let original_new = perform_runtime_backup(&original.store, &recovery_directory, 2_000).unwrap();
    let clone_new = perform_runtime_backup(&clone.store, &recovery_directory, 2_100).unwrap();

    let deleted_original = prune_runtime_backups_with_limits(
        &recovery_directory,
        &server_instance_id,
        &original_fingerprint,
        10_000,
        &original_new.destination,
        1,
        0,
        u64::MAX,
    )
    .unwrap();
    assert_eq!(1, deleted_original);
    assert!(!original_old.destination.exists());
    assert!(!runtime_backup_manifest_path(&original_old.destination).exists());
    assert!(original_new.destination.exists());
    assert!(clone_old.destination.exists());
    assert!(clone_new.destination.exists());

    let deleted_clone = prune_runtime_backups_with_limits(
        &recovery_directory,
        &server_instance_id,
        &clone_fingerprint,
        10_000,
        &clone_new.destination,
        1,
        0,
        u64::MAX,
    )
    .unwrap();
    assert_eq!(1, deleted_clone);
    assert!(!clone_old.destination.exists());
    assert!(!runtime_backup_manifest_path(&clone_old.destination).exists());
    assert!(clone_new.destination.exists());
    assert!(original_new.destination.exists());
    fs::remove_dir_all(&recovery_directory).unwrap();
}

#[test]
fn runtime_rotation_byte_budget_ignores_clone_path_and_unbound_legacy_copy() {
    let original = account_test_store(
        "runtime-budget-original",
        app_data::default_app_data_json(1),
    );
    let clone_directory =
        std::env::temp_dir().join(format!("gridtimer_sync_core_test_{}", random_token(12)));
    fs::create_dir_all(&clone_directory).unwrap();
    let clone_path = clone_directory.join("server_store.sqlite3");
    original
        .store
        .create_verified_backup(&clone_path, 500)
        .unwrap();
    let clone = TestSqliteStore {
        store: SqliteServerStore::open(&clone_path, None).unwrap(),
        directory: clone_directory,
    };
    let server_instance_id = original.store.server_instance_id().unwrap();
    assert_eq!(
        server_instance_id,
        clone.store.server_instance_id().unwrap()
    );
    let original_fingerprint =
        runtime_backup_target_fingerprint(original.store.database_path()).unwrap();

    let recovery_directory = std::env::temp_dir().join(format!(
        "gridtimer_runtime_recovery_test_{}",
        random_token(12)
    ));
    fs::create_dir_all(&recovery_directory).unwrap();
    let unbound_legacy =
        unique_runtime_backup_path(&recovery_directory, &server_instance_id, 900).unwrap();
    original
        .store
        .create_verified_backup(&unbound_legacy, 900)
        .unwrap();
    assert!(!runtime_backup_manifest_path(&unbound_legacy).exists());
    let clone_backup = perform_runtime_backup(&clone.store, &recovery_directory, 1_000).unwrap();
    let original_backup =
        perform_runtime_backup(&original.store, &recovery_directory, 2_000).unwrap();

    let deleted = prune_runtime_backups_with_limits(
        &recovery_directory,
        &server_instance_id,
        &original_fingerprint,
        10_000,
        &original_backup.destination,
        100,
        i64::MAX,
        original_backup.size_bytes,
    )
    .unwrap();
    assert_eq!(0, deleted);
    assert!(original_backup.destination.exists());
    assert!(clone_backup.destination.exists());
    assert!(runtime_backup_manifest_path(&clone_backup.destination).exists());
    assert!(unbound_legacy.exists());
    assert!(!runtime_backup_manifest_path(&unbound_legacy).exists());
    fs::remove_dir_all(&recovery_directory).unwrap();
}

#[test]
fn ambiguous_legacy_scoped_media_is_deferred_without_changing_the_account() {
    let mut account: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    account["notes"] = json!([{
        "id": "page-b", "title": "retained", "content": "retained",
        "createdAtEpochMillis": 100, "updatedAtEpochMillis": 100,
        "attachments": [{"id": "shared-image", "mimeType": "image/png",
            "sha256": "a".repeat(64), "sizeBytes": 4, "updatedAtEpochMillis": 100}]
    }]);
    let mut legacy: Value = serde_json::from_str(&app_data::default_app_data_json(300)).unwrap();
    legacy["tombstones"] = json!([{
        "entityType": "noteMedia", "entityId": "shared-image",
        "deletedAtEpochMillis": 300
    }]);
    let cause =
        merge_migration_app_data_json(&account.to_string(), 100, &legacy.to_string(), 300, 400)
            .expect_err("unscoped old deletion must stay ambiguous");
    assert!(deferred_legacy_media_intent(&cause));
    assert!(!deferred_legacy_media_intent(&StoreError::Integrity(
        "unrelated".into()
    )));

    let test_store = sqlite_test_store(ServerStore {
        users: vec![ServerUser {
            id: "user-a".into(),
            email: "a@example.test".into(),
            password_salt: "salt".into(),
            password_hash: "hash".into(),
            created_at_epoch_millis: 1,
            updated_at_epoch_millis: 100,
            app_data_json: account.to_string(),
            tokens: Vec::new(),
        }],
    });
    let client_root = test_store.directory.join("client");
    fs::create_dir_all(&client_root).unwrap();
    fs::write(
        client_root.join("sync_account.json"),
        r#"{"userId":"user-a","token":""}"#,
    )
    .unwrap();
    let scoped = windows_scoped_state_path(&client_root, "user-a");
    fs::create_dir_all(scoped.parent().unwrap()).unwrap();
    let original = legacy.to_string();
    fs::write(&scoped, &original).unwrap();
    let before = test_store
        .store
        .read_account("user-a")
        .unwrap()
        .app_data_json;
    assert!(
        !import_windows_client_snapshot_from_root(&test_store.store, &client_root, 400).unwrap()
    );
    assert_eq!(
        before,
        test_store
            .store
            .read_account("user-a")
            .unwrap()
            .app_data_json
    );
    assert_eq!(original, fs::read_to_string(scoped).unwrap());
}

#[test]
fn migration_timestamps_preserve_new_scalars_and_restore_old_entities() {
    let mut current: Value = serde_json::from_str(&app_data::default_app_data_json(200))
        .expect("current snapshot should decode");
    current["slotOrder"] = json!([2, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
    current["financeProfile"]["activeIncomeMonthly"] = json!(0);
    current["themeMode"] = json!("DARK");
    current["sessions"] = json!([]);
    let mut old: Value = serde_json::from_str(&app_data::default_app_data_json(100))
        .expect("old snapshot should decode");
    old["slotOrder"] = json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
    old["financeProfile"]["activeIncomeMonthly"] = json!(99_000);
    old["themeMode"] = json!("LIGHT");
    old["sessions"] = json!([{
        "id": "old-session",
        "slotId": 1,
        "slotTitle": "restored history",
        "startedAtEpochMillis": 10,
        "endedAtEpochMillis": 20,
        "durationMillis": 10
    }]);
    for value in [&mut current, &mut old] {
        let root = value.as_object_mut().unwrap();
        for revision_name in [
            "slotOrderUpdatedAtEpochMillis",
            "notePreferencesUpdatedAtEpochMillis",
            "financeProfileUpdatedAtEpochMillis",
            "themeModeUpdatedAtEpochMillis",
        ] {
            root.remove(revision_name);
        }
    }

    let current_first = merge_migration_app_data_json("", 0, &current.to_string(), 200, 300)
        .expect("current snapshot should prepare");
    let merged = merge_migration_app_data_json(&current_first, 200, &old.to_string(), 100, 300)
        .expect("historical snapshot should merge");
    let merged: Value = serde_json::from_str(&merged).expect("merged snapshot should decode");
    assert_eq!(2, merged["slotOrder"][0]);
    assert_eq!(1, merged["slotOrder"][1]);
    assert_eq!(0, merged["financeProfile"]["activeIncomeMonthly"]);
    assert_eq!("DARK", merged["themeMode"]);
    assert_eq!(200, merged["slotOrderUpdatedAtEpochMillis"]);
    assert!(merged["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "old-session"));
}

#[test]
fn theme_merge_keeps_mode_and_oled_marker_as_one_revisioned_value() {
    let mut oled: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    oled["themeMode"] = json!("DARK");
    oled["oledThemeEnabled"] = json!(true);
    oled["themeModeUpdatedAtEpochMillis"] = json!(100);

    let mut light: Value = serde_json::from_str(&app_data::default_app_data_json(200)).unwrap();
    light["themeMode"] = json!("LIGHT");
    light["oledThemeEnabled"] = json!(false);
    light["themeModeUpdatedAtEpochMillis"] = json!(200);

    let cleared = merge_sync_app_data_json(&oled.to_string(), &light.to_string(), 300)
        .expect("newer non-OLED preference should merge");
    let cleared: Value = serde_json::from_str(&cleared).unwrap();
    assert_eq!("LIGHT", cleared["themeMode"]);
    assert_eq!(false, cleared["oledThemeEnabled"]);

    oled["themeModeUpdatedAtEpochMillis"] = json!(400);
    let restored = merge_sync_app_data_json(&light.to_string(), &oled.to_string(), 500)
        .expect("newer OLED preference should merge");
    let restored: Value = serde_json::from_str(&restored).unwrap();
    assert_eq!("DARK", restored["themeMode"]);
    assert_eq!(true, restored["oledThemeEnabled"]);
}

#[test]
fn equal_revision_theme_merge_is_commutative_and_keeps_a_valid_pair() {
    let mut dark: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    dark["themeMode"] = json!("DARK");
    dark["oledThemeEnabled"] = json!(false);
    dark["themeModeUpdatedAtEpochMillis"] = json!(100);

    let mut oled = dark.clone();
    oled["oledThemeEnabled"] = json!(true);

    let forward = merge_sync_app_data_json(&dark.to_string(), &oled.to_string(), 200)
        .expect("forward theme merge");
    let reverse = merge_sync_app_data_json(&oled.to_string(), &dark.to_string(), 200)
        .expect("reverse theme merge");
    assert_eq!(forward, reverse);

    let merged: Value = serde_json::from_str(&forward).unwrap();
    assert_eq!("DARK", merged["themeMode"]);
    assert_eq!(true, merged["oledThemeEnabled"]);
    assert_eq!(100, merged["themeModeUpdatedAtEpochMillis"]);
}

#[test]
fn global_windows_snapshot_stays_bound_when_active_account_switches() {
    let mut account_a_seed: Value =
        serde_json::from_str(&app_data::default_app_data_json(1)).unwrap();
    account_a_seed["sessions"] = json!([
        {
            "id": "common-session-1", "slotId": 1, "slotTitle": "common",
            "startedAtEpochMillis": 1, "endedAtEpochMillis": 2, "durationMillis": 1
        },
        {
            "id": "common-session-2", "slotId": 1, "slotTitle": "common",
            "startedAtEpochMillis": 2, "endedAtEpochMillis": 3, "durationMillis": 1
        },
        {
            "id": "common-session-3", "slotId": 1, "slotTitle": "common",
            "startedAtEpochMillis": 3, "endedAtEpochMillis": 4, "durationMillis": 1
        }
    ]);
    let test_store = sqlite_test_store(ServerStore {
        users: vec![
            ServerUser {
                id: "user-a".to_string(),
                email: "a@example.test".to_string(),
                password_salt: "salt-a".to_string(),
                password_hash: "hash-a".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: account_a_seed.to_string(),
                tokens: Vec::new(),
            },
            ServerUser {
                id: "user-b".to_string(),
                email: "b@example.test".to_string(),
                password_salt: "salt-b".to_string(),
                password_hash: "hash-b".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: app_data::default_app_data_json(1),
                tokens: Vec::new(),
            },
        ],
    });
    let client_root = test_store.directory.join("client");
    fs::create_dir_all(&client_root).unwrap();
    fs::write(
        client_root.join("sync_account.json"),
        r#"{"userId":"user-a","token":""}"#,
    )
    .unwrap();
    let mut global: Value = serde_json::from_str(&app_data::default_app_data_json(10_000)).unwrap();
    global["sessions"] = json!([
        {
            "id": "common-session-1", "slotId": 1, "slotTitle": "common",
            "startedAtEpochMillis": 1, "endedAtEpochMillis": 2, "durationMillis": 1
        },
        {
            "id": "common-session-2", "slotId": 1, "slotTitle": "common",
            "startedAtEpochMillis": 2, "endedAtEpochMillis": 3, "durationMillis": 1
        },
        {
            "id": "common-session-3", "slotId": 1, "slotTitle": "common",
            "startedAtEpochMillis": 3, "endedAtEpochMillis": 4, "durationMillis": 1
        },
        {
            "id": "a-global-session",
            "slotId": 1,
            "slotTitle": "A only",
            "startedAtEpochMillis": 100,
            "endedAtEpochMillis": 200,
            "durationMillis": 100
        }
    ]);
    global["notes"] = json!([{
        "id": "a-global-note",
        "title": "A only",
        "content": "must never reach B",
        "createdAtEpochMillis": 100,
        "updatedAtEpochMillis": 200,
        "deletedAtEpochMillis": null
    }]);
    fs::write(client_root.join("timer_state.json"), global.to_string()).unwrap();
    let mut scoped_a: Value =
        serde_json::from_str(&app_data::default_app_data_json(10_000)).unwrap();
    scoped_a["sessions"] = json!([{
        "id": "a-scoped-session",
        "slotId": 1,
        "slotTitle": "A scoped",
        "startedAtEpochMillis": 50,
        "endedAtEpochMillis": 75,
        "durationMillis": 25
    }]);
    let scoped_a_path = windows_scoped_state_path(&client_root, "user-a");
    fs::create_dir_all(scoped_a_path.parent().unwrap()).unwrap();
    fs::write(&scoped_a_path, scoped_a.to_string()).unwrap();

    import_windows_client_snapshot_from_root(&test_store.store, &client_root, 10_000).unwrap();
    let account_a: Value = serde_json::from_str(
        &test_store
            .store
            .read_account("user-a")
            .unwrap()
            .app_data_json,
    )
    .unwrap();
    assert!(account_a["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-global-session"));
    assert!(account_a["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-scoped-session"));
    assert!(account_a["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-global-note"));

    global["sessions"].as_array_mut().unwrap().push(json!({
        "id": "a-later-global-session",
        "slotId": 1,
        "slotTitle": "still A",
        "startedAtEpochMillis": 300,
        "endedAtEpochMillis": 400,
        "durationMillis": 100
    }));
    fs::write(client_root.join("timer_state.json"), global.to_string()).unwrap();
    fs::write(
        client_root.join("sync_account.json"),
        r#"{"userId":"user-b","token":""}"#,
    )
    .unwrap();
    import_windows_client_snapshot_from_root(&test_store.store, &client_root, 11_000).unwrap();
    let account_b: Value = serde_json::from_str(
        &test_store
            .store
            .read_account("user-b")
            .unwrap()
            .app_data_json,
    )
    .unwrap();
    assert!(!account_b["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-global-session" || value["id"] == "a-later-global-session"));
    assert!(!account_b["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-global-note"));

    let mut scoped_b: Value =
        serde_json::from_str(&app_data::default_app_data_json(12_000)).unwrap();
    scoped_b["sessions"] = json!([{
        "id": "b-scoped-session",
        "slotId": 1,
        "slotTitle": "B only",
        "startedAtEpochMillis": 500,
        "endedAtEpochMillis": 600,
        "durationMillis": 100
    }]);
    let scoped_b_path = windows_scoped_state_path(&client_root, "user-b");
    fs::create_dir_all(scoped_b_path.parent().unwrap()).unwrap();
    fs::write(&scoped_b_path, scoped_b.to_string()).unwrap();
    import_windows_client_snapshot_from_root(&test_store.store, &client_root, 12_000).unwrap();
    let account_b: Value = serde_json::from_str(
        &test_store
            .store
            .read_account("user-b")
            .unwrap()
            .app_data_json,
    )
    .unwrap();
    assert!(account_b["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "b-scoped-session"));
    assert!(!account_b["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-global-session"));
}

#[test]
fn global_snapshot_owner_comes_from_content_not_first_active_login() {
    fn history_snapshot(ids: &[&str], now: i64) -> String {
        let mut value: Value = serde_json::from_str(&app_data::default_app_data_json(now)).unwrap();
        value["sessions"] = Value::Array(
            ids.iter()
                .enumerate()
                .map(|(index, id)| {
                    json!({
                        "id": id,
                        "slotId": 1,
                        "slotTitle": "history",
                        "startedAtEpochMillis": 10 + index as i64,
                        "endedAtEpochMillis": 20 + index as i64,
                        "durationMillis": 10
                    })
                })
                .collect(),
        );
        value.to_string()
    }

    let test_store = sqlite_test_store(ServerStore {
        users: vec![
            ServerUser {
                id: "user-a".to_string(),
                email: "a-owner@example.test".to_string(),
                password_salt: "salt-a".to_string(),
                password_hash: "hash-a".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: history_snapshot(&["common-1", "common-2", "common-3"], 1_000),
                tokens: Vec::new(),
            },
            ServerUser {
                id: "user-b".to_string(),
                email: "b-active@example.test".to_string(),
                password_salt: "salt-b".to_string(),
                password_hash: "hash-b".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: app_data::default_app_data_json(1_000),
                tokens: Vec::new(),
            },
        ],
    });
    let client_root = test_store.directory.join("active-b-client");
    fs::create_dir_all(&client_root).unwrap();
    fs::write(
        client_root.join("sync_account.json"),
        r#"{"userId":"user-b","token":""}"#,
    )
    .unwrap();
    fs::write(
        client_root.join("timer_state.json"),
        history_snapshot(
            &["common-1", "common-2", "common-3", "a-unique-restored"],
            1_000,
        ),
    )
    .unwrap();

    assert!(
        import_windows_client_snapshot_from_root(&test_store.store, &client_root, 1_000).unwrap()
    );
    let account_a: Value = serde_json::from_str(
        &test_store
            .store
            .read_account("user-a")
            .unwrap()
            .app_data_json,
    )
    .unwrap();
    let account_b: Value = serde_json::from_str(
        &test_store
            .store
            .read_account("user-b")
            .unwrap()
            .app_data_json,
    )
    .unwrap();
    assert!(account_a["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-unique-restored"));
    assert!(!account_b["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value["id"] == "a-unique-restored"));

    let unmatched_store = sqlite_test_store(ServerStore {
        users: vec![
            ServerUser {
                id: "user-a".to_string(),
                email: "a-empty@example.test".to_string(),
                password_salt: "salt-a".to_string(),
                password_hash: "hash-a".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: app_data::default_app_data_json(1_000),
                tokens: Vec::new(),
            },
            ServerUser {
                id: "user-b".to_string(),
                email: "b-empty@example.test".to_string(),
                password_salt: "salt-b".to_string(),
                password_hash: "hash-b".to_string(),
                created_at_epoch_millis: 1,
                updated_at_epoch_millis: 1,
                app_data_json: app_data::default_app_data_json(1_000),
                tokens: Vec::new(),
            },
        ],
    });
    let unmatched_root = unmatched_store.directory.join("unmatched-client");
    fs::create_dir_all(&unmatched_root).unwrap();
    fs::write(
        unmatched_root.join("sync_account.json"),
        r#"{"userId":"user-b","token":""}"#,
    )
    .unwrap();
    fs::write(
        unmatched_root.join("timer_state.json"),
        history_snapshot(&["unmatched-1", "unmatched-2", "unmatched-3"], 1_000),
    )
    .unwrap();
    assert!(!import_windows_client_snapshot_from_root(
        &unmatched_store.store,
        &unmatched_root,
        1_000,
    )
    .unwrap());
    let unmatched_b: Value = serde_json::from_str(
        &unmatched_store
            .store
            .read_account("user-b")
            .unwrap()
            .app_data_json,
    )
    .unwrap();
    assert!(unmatched_b["sessions"].as_array().unwrap().is_empty());
}

#[test]
fn legacy_login_upgrades_password_to_argon2id_and_logout_revokes_token() {
    let store = account_test_store("existing-token", String::new());
    let login_body = serde_json::to_string(&LoginRequest {
        request_id: random_token(18),
        email: "test@example.com".to_string(),
        password: "secret-password".to_string(),
        device_name: "phone".to_string(),
    })
    .unwrap();
    let (status, login) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);
    assert!(login.ok);
    let upgraded = store
        .store
        .find_user_by_id("user-1")
        .unwrap()
        .expect("user should exist");
    assert_eq!("argon2id_phc", upgraded.password_scheme);
    assert!(upgraded.password_salt.is_empty());
    assert!(verify_stored_password(&upgraded, "secret-password").unwrap());
    let logout_request = HttpRequest {
        headers: vec![(
            "Authorization".to_string(),
            format!("Bearer {}", login.token),
        )],
        ..HttpRequest::default()
    };
    assert_eq!(200, handle_logout(&logout_request, &store.store).0);
    assert_eq!(
        TokenAuthentication::Revoked,
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap()
    );
}

#[test]
fn pending_login_token_can_prove_discovery_without_activation() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let login_body = serde_json::to_string(&LoginRequest {
        request_id: random_token(18),
        email: "test@example.com".to_string(),
        password: "secret-password".to_string(),
        device_name: "phone".to_string(),
    })
    .unwrap();
    let (login_status, login) = handle_login(&login_body, &store.store);
    assert_eq!(200, login_status);

    let proof_now = now_millis();
    let public_server_url = "https://sync.example.com";
    let nonce = "discovery-proof-nonce";
    let proof_body = json!({
        "userId": login.user_id,
        "tokenId": login.token_id,
        "nonce": nonce,
    })
    .to_string();
    let (proof_status, proof) =
        handle_discovery_proof_at(&proof_body, &store.store, public_server_url, proof_now);
    assert_eq!(200, proof_status);
    assert!(proof.ok);
    assert_eq!("discovery_proof", proof.mode);
    assert_eq!(public_server_url, proof.public_server_url);
    assert_eq!(nonce, proof.nonce);
    assert!(!proof.proof.is_empty());
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, proof_now)
            .unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));

    let (legacy_recovery_status, _) = handle_discovery_proof_at(
        &proof_body,
        &store.store,
        public_server_url,
        proof_now.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS + 1),
    );
    assert_eq!(200, legacy_recovery_status);
    assert!(matches!(
        store
            .store
            .authenticate_token(
                &login.token,
                proof_now.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS + 1)
            )
            .unwrap(),
        TokenAuthentication::Expired
    ));

    assert!(store.store.revoke_token(&login.token, proof_now).unwrap());
    assert_eq!(
        401,
        handle_discovery_proof_at(&proof_body, &store.store, public_server_url, proof_now,).0
    );
}

#[test]
fn recently_active_expired_phone_requires_proof_and_recovers_while_old_sessions_stay_expired() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let now = now_millis();
    let expiry = now.saturating_sub(10 * 60 * 60 * 1_000);
    let created = expiry.saturating_sub(TOKEN_TTL_MILLIS);
    let public_url = "https://sync.example.com";
    let nonce = "recent-active-recovery-nonce";
    let recent_raw_token = "recent-active-phone-token";
    let recent = store
        .store
        .issue_token("user-1", recent_raw_token, "phone", created, expiry)
        .unwrap();
    let last_seen = expiry.saturating_sub(7 * 60 * 60 * 1_000);
    assert!(matches!(
        store
            .store
            .authenticate_token(recent_raw_token, last_seen)
            .unwrap(),
        TokenAuthentication::Active(_)
    ));

    let invalid_body = json!({
        "userId": "user-1",
        "tokenId": recent.token_id,
        "nonce": nonce,
        "clientProof": "0".repeat(64),
    })
    .to_string();
    assert_eq!(
        401,
        handle_discovery_proof_at(&invalid_body, &store.store, public_url, now).0
    );
    assert_eq!(
        TokenAuthentication::Expired,
        store
            .store
            .authenticate_token(recent_raw_token, now)
            .unwrap()
    );

    let proof_body = |raw_token: &str, token_id: &str| {
        let key = Sha256::digest(raw_token.as_bytes());
        let message = format!(
            "gridtimer.discovery.pending-recovery.v1\nuser-1\n{token_id}\n{nonce}\n{public_url}"
        );
        json!({
            "userId": "user-1",
            "tokenId": token_id,
            "nonce": nonce,
            "clientProof": hmac_sha256_hex(&key, message.as_bytes()),
        })
        .to_string()
    };
    let (status, proof) = handle_discovery_proof_at(
        &proof_body(recent_raw_token, &recent.token_id),
        &store.store,
        public_url,
        now,
    );
    assert_eq!(200, status);
    assert!(proof.ok);
    assert!(matches!(
        store
            .store
            .authenticate_token(recent_raw_token, now)
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
    let recovered_expiry = store
        .store
        .list_token_metadata("user-1")
        .unwrap()
        .into_iter()
        .find(|token| token.token_id == recent.token_id)
        .unwrap()
        .expires_at_epoch_millis;
    assert_eq!(now.saturating_add(TOKEN_TTL_MILLIS), recovered_expiry);

    let stale_raw_token = "stale-active-phone-token";
    let stale = store
        .store
        .issue_token("user-1", stale_raw_token, "phone", created, expiry)
        .unwrap();
    assert_eq!(
        401,
        handle_discovery_proof_at(
            &proof_body(stale_raw_token, &stale.token_id),
            &store.store,
            public_url,
            now,
        )
        .0
    );
    assert_eq!(
        TokenAuthentication::Expired,
        store
            .store
            .authenticate_token(stale_raw_token, now)
            .unwrap()
    );

    let revoked_raw_token = "revoked-active-phone-token";
    let revoked = store
        .store
        .issue_token("user-1", revoked_raw_token, "phone", created, expiry)
        .unwrap();
    assert!(store
        .store
        .revoke_token(revoked_raw_token, last_seen)
        .unwrap());
    assert_eq!(
        401,
        handle_discovery_proof_at(
            &proof_body(revoked_raw_token, &revoked.token_id),
            &store.store,
            public_url,
            now,
        )
        .0
    );
}

#[test]
fn recently_used_desktop_login_recovers_media_and_download_after_offline_expiry() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let now = now_millis();
    let expiry = now.saturating_sub(54 * 60 * 60 * 1_000);
    let created = expiry.saturating_sub(TOKEN_TTL_MILLIS);
    let last_seen = expiry.saturating_sub(60 * 60 * 1_000);
    let media_token = "recent-desktop-media-token";
    let media = store
        .store
        .issue_token("user-1", media_token, "desktop", created, expiry)
        .unwrap();
    let media_receipt = store
        .store
        .ensure_restore_receipt("user-1", media.id)
        .unwrap();
    store
        .store
        .acknowledge_restore_generation(
            "user-1",
            media.id,
            media_receipt.current_generation,
            &media_receipt.receipt,
        )
        .unwrap();
    assert!(matches!(
        store
            .store
            .authenticate_token(media_token, last_seen)
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
    let media_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {media_token}"))],
        body: json!({
            "acknowledgedGeneration": media_receipt.current_generation,
            "restoreReceipt": media_receipt.receipt,
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (media_status, media_result) = handle_media_manifest(&media_request, &store.store);
    assert_eq!(200, media_status, "{}", media_result.message);
    assert!(media_result.ok);
    assert_eq!("media_manifest", media_result.mode);
    let renewed_expiry = store
        .store
        .list_token_metadata("user-1")
        .unwrap()
        .into_iter()
        .find(|token| token.token_id == media.token_id)
        .unwrap()
        .expires_at_epoch_millis;
    assert!(renewed_expiry >= now.saturating_add(TOKEN_TTL_MILLIS));
    assert!(renewed_expiry <= now_millis().saturating_add(TOKEN_TTL_MILLIS));

    let sync_token = "recent-desktop-download-token";
    let sync = store
        .store
        .issue_token("user-1", sync_token, "desktop", created, expiry)
        .unwrap();
    let sync_receipt = store
        .store
        .ensure_restore_receipt("user-1", sync.id)
        .unwrap();
    store
        .store
        .acknowledge_restore_generation(
            "user-1",
            sync.id,
            sync_receipt.current_generation,
            &sync_receipt.receipt,
        )
        .unwrap();
    assert!(matches!(
        store
            .store
            .authenticate_token(sync_token, last_seen)
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
    let download_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {sync_token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: random_token(18),
            app_data_json: app_data::default_app_data_json(1),
            client_updated_at_epoch_millis: 1,
            acknowledged_generation: sync_receipt.current_generation,
            restore_receipt: sync_receipt.receipt,
            device_name: "desktop".to_string(),
            force_download: true,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (sync_status, sync_result) = handle_sync(&download_request, &store.store);
    assert_eq!(200, sync_status, "{}", sync_result.message);
    assert!(sync_result.ok);
    assert!(matches!(
        store
            .store
            .authenticate_token(sync_token, now_millis())
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
}

#[test]
fn desktop_expiry_recovery_rejects_stale_revoked_and_old_sessions() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let now = now_millis();
    let expiry = now.saturating_sub(54 * 60 * 60 * 1_000);
    let created = expiry.saturating_sub(TOKEN_TTL_MILLIS);
    let stale = "stale-desktop-token";
    store
        .store
        .issue_token("user-1", stale, "desktop", created, expiry)
        .unwrap();
    assert_eq!(
        401,
        authenticate_token_request(&store.store, stale, now)
            .unwrap_err()
            .0
    );

    let revoked = "revoked-desktop-token";
    store
        .store
        .issue_token("user-1", revoked, "desktop", created, expiry)
        .unwrap();
    assert!(store
        .store
        .revoke_token(revoked, expiry.saturating_sub(60 * 60 * 1_000))
        .unwrap());
    assert_eq!(
        401,
        authenticate_token_request(&store.store, revoked, now)
            .unwrap_err()
            .0
    );

    let old_expiry = now.saturating_sub(DESKTOP_ACTIVE_TOKEN_RECOVERY_GRACE_MILLIS + 1);
    let old_created = old_expiry.saturating_sub(TOKEN_TTL_MILLIS);
    let old = "old-desktop-token";
    store
        .store
        .issue_token("user-1", old, "desktop", old_created, old_expiry)
        .unwrap();
    assert!(matches!(
        store
            .store
            .authenticate_token(old, old_expiry.saturating_sub(60 * 60 * 1_000))
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
    assert_eq!(
        401,
        authenticate_token_request(&store.store, old, now)
            .unwrap_err()
            .0
    );
}

#[test]
fn active_login_renews_before_expiry_without_renewing_revoked_tokens() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let now = now_millis();
    let raw_token = "sliding-active-phone-token";
    let old_expiry = now.saturating_add(24 * 60 * 60 * 1_000);
    let token = store
        .store
        .issue_token(
            "user-1",
            raw_token,
            "phone",
            now.saturating_sub(TOKEN_TTL_MILLIS - 24 * 60 * 60 * 1_000),
            old_expiry,
        )
        .unwrap();
    let authenticated = authenticate_token_request(&store.store, raw_token, now).unwrap();
    assert_eq!(token.token_id, authenticated.token_identifier);
    assert_eq!(
        now.saturating_add(TOKEN_TTL_MILLIS),
        authenticated.expires_at_epoch_millis
    );
    assert_eq!(
        authenticated.expires_at_epoch_millis,
        store
            .store
            .list_token_metadata("user-1")
            .unwrap()
            .into_iter()
            .find(|item| item.token_id == token.token_id)
            .unwrap()
            .expires_at_epoch_millis
    );
    assert!(store.store.revoke_token(raw_token, now).unwrap());
    assert_eq!(
        401,
        authenticate_token_request(&store.store, raw_token, now + 1)
            .unwrap_err()
            .0
    );
}

#[test]
fn cleanup_tombstone_requires_token_proof_and_recovers_only_once_before_sync_activation() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let public_server_url = "https://sync.example.com";
    let nonce = "pending-recovery-proof-nonce";
    let now = now_millis();
    let created_at = now.saturating_sub(TOKEN_ACTIVATION_LEASE_MILLIS + 1_000);
    let original_expiry = created_at.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS);
    let raw_token = "abandoned-first-sync-token";
    let metadata = store
        .store
        .issue_pending_token("user-1", raw_token, "phone", created_at, original_expiry)
        .unwrap();
    assert_eq!(1, store.store.cleanup_expired_pending_tokens(now).unwrap());

    let invalid_body = json!({
        "userId": "user-1",
        "tokenId": metadata.token_id,
        "nonce": nonce,
        "clientProof": "0".repeat(64),
    })
    .to_string();
    assert_eq!(
        401,
        handle_discovery_proof_at(&invalid_body, &store.store, public_server_url, now).0
    );
    assert_eq!(
        TokenAuthentication::Revoked,
        store.store.authenticate_token(raw_token, now).unwrap()
    );

    let token_key = Sha256::digest(raw_token.as_bytes());
    let recovery_message = format!(
        "gridtimer.discovery.pending-recovery.v1\nuser-1\n{}\n{nonce}\n{public_server_url}",
        metadata.token_id
    );
    let client_proof = hmac_sha256_hex(&token_key, recovery_message.as_bytes());
    let valid_body = json!({
        "userId": "user-1",
        "tokenId": metadata.token_id,
        "nonce": nonce,
        "clientProof": client_proof,
    })
    .to_string();
    let (status, recovered) =
        handle_discovery_proof_at(&valid_body, &store.store, public_server_url, now);
    assert_eq!(200, status);
    assert!(recovered.message.contains("safely renewing"));
    assert!(matches!(
        store.store.authenticate_token(raw_token, now).unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));
    let recovered_expiry = store
        .store
        .list_token_metadata("user-1")
        .unwrap()
        .into_iter()
        .find(|token| token.token_id == metadata.token_id)
        .unwrap()
        .expires_at_epoch_millis;
    assert_eq!(
        now.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS),
        recovered_expiry
    );

    assert_eq!(
        200,
        handle_discovery_proof_at(
            &valid_body,
            &store.store,
            public_server_url,
            now.saturating_add(1),
        )
        .0
    );
    assert_eq!(
        recovered_expiry,
        store
            .store
            .list_token_metadata("user-1")
            .unwrap()
            .into_iter()
            .find(|token| token.token_id == metadata.token_id)
            .unwrap()
            .expires_at_epoch_millis
    );

    let identity = store.store.server_account_identity("user-1").unwrap();
    let initial = workspace_sync_request(
        raw_token,
        &random_token(18),
        app_data::default_app_data_json(now),
        now,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    let (initial_status, initial_result) = handle_sync(&initial, &store.store);
    assert_eq!(200, initial_status);
    assert!(initial_result.restore_required);
    let committed = workspace_sync_request(
        raw_token,
        &random_token(18),
        initial_result.app_data_json.clone().unwrap(),
        initial_result.server_updated_at_epoch_millis,
        initial_result.current_generation,
        &initial_result.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_eq!(200, handle_sync(&committed, &store.store).0);
    assert!(matches!(
        store
            .store
            .authenticate_token(raw_token, now_millis())
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
}

#[test]
fn legacy_client_recovers_cleanup_tombstone_only_during_normal_sync_commit() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let public_server_url = "https://sync.example.com";
    let nonce = "legacy-pending-recovery-proof-nonce";
    let now = now_millis();
    let created_at = now.saturating_sub(TOKEN_ACTIVATION_LEASE_MILLIS + 1_000);
    let original_expiry = created_at.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS);
    let raw_token = "legacy-abandoned-first-sync-token";
    let metadata = store
        .store
        .issue_pending_token("user-1", raw_token, "phone", created_at, original_expiry)
        .unwrap();
    assert_eq!(1, store.store.cleanup_expired_pending_tokens(now).unwrap());

    let legacy_proof_body = json!({
        "userId": "user-1",
        "tokenId": metadata.token_id,
        "nonce": nonce,
    })
    .to_string();
    let (proof_status, proof) =
        handle_discovery_proof_at(&legacy_proof_body, &store.store, public_server_url, now);
    assert_eq!(200, proof_status);
    let key = Sha256::digest(raw_token.as_bytes());
    assert_eq!(
        hmac_sha256_hex(&key, format!("{nonce}\n{public_server_url}").as_bytes()),
        proof.proof
    );
    assert_eq!(
        TokenAuthentication::Revoked,
        store.store.authenticate_token(raw_token, now).unwrap()
    );
    assert_eq!(
        original_expiry,
        store
            .store
            .list_token_metadata("user-1")
            .unwrap()
            .into_iter()
            .find(|token| token.token_id == metadata.token_id)
            .unwrap()
            .expires_at_epoch_millis
    );

    let identity = store.store.server_account_identity("user-1").unwrap();
    let forced = workspace_sync_request(
        raw_token,
        &random_token(18),
        app_data::default_app_data_json(now),
        now,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        true,
        false,
    );
    assert_eq!(401, handle_sync(&forced, &store.store).0);
    assert_eq!(
        TokenAuthentication::Revoked,
        store.store.authenticate_token(raw_token, now).unwrap()
    );

    let initial = workspace_sync_request(
        raw_token,
        &random_token(18),
        app_data::default_app_data_json(now),
        now,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    let (initial_status, initial_result) = handle_sync(&initial, &store.store);
    assert_eq!(200, initial_status);
    assert!(initial_result.restore_required);
    assert_eq!(
        TokenAuthentication::Revoked,
        store.store.authenticate_token(raw_token, now).unwrap()
    );

    let committed = workspace_sync_request(
        raw_token,
        &random_token(18),
        initial_result.app_data_json.clone().unwrap(),
        initial_result.server_updated_at_epoch_millis,
        initial_result.current_generation,
        &initial_result.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_eq!(200, handle_sync(&committed, &store.store).0);
    let activated_expiry = match store
        .store
        .authenticate_token(raw_token, now_millis())
        .unwrap()
    {
        TokenAuthentication::Active(token) => token.expires_at_epoch_millis,
        state => panic!("expected recovered token to be active, got {state:?}"),
    };
    assert!(activated_expiry > original_expiry);
    assert_eq!(200, handle_sync(&committed, &store.store).0);
    assert!(matches!(
        store
            .store
            .authenticate_token(raw_token, now_millis())
            .unwrap(),
        TokenAuthentication::Active(AuthenticatedToken {
            expires_at_epoch_millis,
            ..
        }) if expires_at_epoch_millis == activated_expiry
    ));

    let explicitly_logged_out = "legacy-explicit-logout-token";
    let explicit_created = now.saturating_sub(TOKEN_ACTIVATION_LEASE_MILLIS + 2_000);
    let explicit_expiry = explicit_created.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS);
    store
        .store
        .issue_pending_token(
            "user-1",
            explicitly_logged_out,
            "phone",
            explicit_created,
            explicit_expiry,
        )
        .unwrap();
    store.store.cleanup_expired_pending_tokens(now).unwrap();
    assert_eq!(
        Some("user-1".to_string()),
        store
            .store
            .revoke_token_for_logout(explicitly_logged_out, now)
            .unwrap()
    );
    let explicit_request = workspace_sync_request(
        explicitly_logged_out,
        &random_token(18),
        app_data::default_app_data_json(now),
        now,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_eq!(401, handle_sync(&explicit_request, &store.store).0);

    let outside_window = "legacy-outside-recovery-window-token";
    let outside_created = now.saturating_sub(
        TOKEN_ACTIVATION_LEASE_MILLIS + PENDING_TOKEN_RECOVERY_WINDOW_MILLIS + 2_000,
    );
    let outside_expiry = outside_created.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS);
    store
        .store
        .issue_pending_token(
            "user-1",
            outside_window,
            "phone",
            outside_created,
            outside_expiry,
        )
        .unwrap();
    store.store.cleanup_expired_pending_tokens(now).unwrap();
    let outside_request = workspace_sync_request(
        outside_window,
        &random_token(18),
        app_data::default_app_data_json(now),
        now,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_eq!(401, handle_sync(&outside_request, &store.store).0);
}

#[test]
fn pending_recovery_rejects_explicit_revocation_and_expired_recovery_window() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let public_server_url = "https://sync.example.com";
    let nonce = "pending-recovery-rejection-nonce";
    let created_at = 1_000;
    let original_expiry = created_at + TOKEN_ACTIVATION_LEASE_MILLIS;

    for (raw_token, explicit_logout, proof_now) in [
        ("explicitly-revoked-pending", true, original_expiry + 1),
        (
            "recovery-window-expired",
            false,
            original_expiry + PENDING_TOKEN_RECOVERY_WINDOW_MILLIS + 1,
        ),
    ] {
        let metadata = store
            .store
            .issue_pending_token("user-1", raw_token, "phone", created_at, original_expiry)
            .unwrap();
        assert_eq!(
            1,
            store
                .store
                .cleanup_expired_pending_tokens(original_expiry)
                .unwrap()
        );
        if explicit_logout {
            assert_eq!(
                Some("user-1".to_string()),
                store
                    .store
                    .revoke_token_for_logout(raw_token, original_expiry + 1)
                    .unwrap()
            );
        }
        let key = Sha256::digest(raw_token.as_bytes());
        let message = format!(
            "gridtimer.discovery.pending-recovery.v1\nuser-1\n{}\n{nonce}\n{public_server_url}",
            metadata.token_id
        );
        let proof_body = json!({
            "userId": "user-1",
            "tokenId": metadata.token_id,
            "nonce": nonce,
            "clientProof": hmac_sha256_hex(&key, message.as_bytes()),
        })
        .to_string();
        let (status, result) =
            handle_discovery_proof_at(&proof_body, &store.store, public_server_url, proof_now);
        assert_eq!(401, status);
        if explicit_logout {
            assert_ne!("pending_reauthentication_required", result.mode);
        } else {
            assert_eq!("pending_reauthentication_required", result.mode);
        }
    }
}

#[test]
fn fresh_login_token_is_pending_until_durable_followup_sync() {
    let store = account_test_store("existing-token", app_data::default_app_data_json(1));
    let login_body = serde_json::to_string(&LoginRequest {
        request_id: random_token(18),
        email: "test@example.com".to_string(),
        password: "secret-password".to_string(),
        device_name: "phone".to_string(),
    })
    .unwrap();
    let (status, login) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));

    let media_request = HttpRequest {
        headers: vec![(
            "Authorization".to_string(),
            format!("Bearer {}", login.token),
        )],
        ..HttpRequest::default()
    };
    let (media_status, media_result) = handle_media_manifest(&media_request, &store.store);
    assert_eq!(401, media_status);
    assert!(media_result.message.contains("activation"));

    let identity = store.store.server_account_identity("user-1").unwrap();
    for (force_upload, force_download, force_upload_endpoint) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let forced = workspace_sync_request(
            &login.token,
            &random_token(18),
            app_data::default_app_data_json(10),
            10,
            0,
            "",
            &identity.server_instance_id,
            &identity.account_namespace,
            &"a".repeat(64),
            "",
            force_upload,
            force_download,
        );
        let (forced_status, forced_result) =
            handle_sync_mode(&forced, &store.store, force_upload_endpoint);
        assert_eq!(409, forced_status);
        assert_eq!("token_activation_required", forced_result.mode);
        assert!(matches!(
            store
                .store
                .authenticate_token(&login.token, now_millis())
                .unwrap(),
            TokenAuthentication::PendingActivation(_)
        ));
    }

    let normal = workspace_sync_request(
        &login.token,
        &random_token(18),
        app_data::default_app_data_json(10),
        10,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    let (normal_status, normal_result) = handle_sync(&normal, &store.store);
    assert_eq!(200, normal_status);
    assert!(normal_result.restore_required);
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));

    let future_generation = workspace_sync_request(
        &login.token,
        &random_token(18),
        normal_result.app_data_json.clone().unwrap(),
        normal_result.server_updated_at_epoch_millis,
        normal_result.current_generation + 1,
        &normal_result.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_ne!(200, handle_sync(&future_generation, &store.store).0);
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));

    let mut future_schema: Value =
        serde_json::from_str(normal_result.app_data_json.as_deref().unwrap()).unwrap();
    future_schema["schemaVersion"] = json!(app_data::APP_DATA_SCHEMA_VERSION + 1);
    let invalid_app_data = workspace_sync_request(
        &login.token,
        &random_token(18),
        future_schema.to_string(),
        normal_result.server_updated_at_epoch_millis,
        normal_result.current_generation,
        &normal_result.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_eq!(426, handle_sync(&invalid_app_data, &store.store).0);
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));

    let collision_request_id = random_token(18);
    let existing_token_commit = workspace_sync_request(
        "existing-token",
        &collision_request_id,
        app_data::default_app_data_json(20),
        20,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_eq!(200, handle_sync(&existing_token_commit, &store.store).0);
    let rejected_before_commit = workspace_sync_request(
        &login.token,
        &collision_request_id,
        normal_result.app_data_json.clone().unwrap(),
        normal_result.server_updated_at_epoch_millis,
        normal_result.current_generation,
        &normal_result.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    assert_ne!(200, handle_sync(&rejected_before_commit, &store.store).0);
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap(),
        TokenAuthentication::PendingActivation(_)
    ));

    let committed = workspace_sync_request(
        &login.token,
        &random_token(18),
        normal_result.app_data_json.clone().unwrap(),
        normal_result.server_updated_at_epoch_millis,
        normal_result.current_generation,
        &normal_result.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &"a".repeat(64),
        "",
        false,
        false,
    );
    let (committed_status, committed_result) = handle_sync(&committed, &store.store);
    assert_eq!(200, committed_status);
    assert!(committed_result.ok);
    assert!(!committed_result.restore_required);
    assert!(matches!(
        store
            .store
            .authenticate_token(&login.token, now_millis())
            .unwrap(),
        TokenAuthentication::Active(_)
    ));
    assert_eq!(200, handle_media_manifest(&media_request, &store.store).0);

    assert_eq!(200, handle_logout(&media_request, &store.store).0);
    assert_eq!(200, handle_logout(&media_request, &store.store).0);
}

#[test]
fn authenticated_response_identity_is_stable_and_always_uses_token_owner() {
    let token = "response-namespace-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let request = HttpRequest {
        method: "POST".to_string(),
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        ..HttpRequest::default()
    };
    let expected = store.store.server_account_identity("user-1").unwrap();
    let (status, result) = bind_response_account_identity(
        &request,
        "/v1/media/upload",
        &store.store,
        (
            409,
            SyncClientResult {
                user_id: "request-body-user".to_string(),
                mode: "media_conflict".to_string(),
                ..SyncClientResult::default()
            },
        ),
    );
    assert_eq!(409, status);
    assert_eq!("user-1", result.user_id);
    assert_eq!(
        hex_bytes(&Sha256::digest(token.as_bytes()))[..32],
        result.token_id
    );
    assert_eq!(expected.server_instance_id, result.server_instance_id);
    assert_eq!(expected.account_namespace, result.account_namespace);

    let (quota_status, quota_result) = bind_response_account_identity(
        &request,
        "/v1/media/upload",
        &store.store,
        (507, snapshot_history_quota_exceeded_result(10, 20, 15)),
    );
    assert_eq!(507, quota_status);
    assert_eq!("user-1", quota_result.user_id);
    assert_eq!(
        hex_bytes(&Sha256::digest(token.as_bytes()))[..32],
        quota_result.token_id
    );
    assert_eq!(expected.server_instance_id, quota_result.server_instance_id);
    assert_eq!(expected.account_namespace, quota_result.account_namespace);
    assert!(!quota_result.current_committed);

    let (failure_status, failure_result) = bind_response_account_identity(
        &request,
        "/v1/sync",
        &store.store,
        store_failure(
            "fixture storage failure",
            StoreError::Integrity("fixture".into()),
        ),
    );
    assert_eq!(500, failure_status);
    assert!(!failure_result.ok);
    assert!(!failure_result.current_committed);
    assert_eq!("user-1", failure_result.user_id);
    assert_eq!(
        expected.server_instance_id,
        failure_result.server_instance_id
    );
    assert_eq!(expected.account_namespace, failure_result.account_namespace);
    assert_eq!("Sync store is unavailable.", failure_result.message);

    let (metadata_status, metadata_result) = bind_response_account_identity(
        &request,
        "/v1/sync",
        &store.store,
        store_failure(
            "fixture metadata failure",
            StoreError::Integrity("new or changed attachment metadata is incomplete".into()),
        ),
    );
    assert_eq!(422, metadata_status);
    assert_eq!("media_metadata_incomplete", metadata_result.mode);
    assert!(!metadata_result.current_committed);
    assert_eq!(
        expected.account_namespace,
        metadata_result.account_namespace
    );

    let (_, anonymous_failure) = bind_response_account_identity(
        &HttpRequest {
            method: "POST".into(),
            ..HttpRequest::default()
        },
        "/v1/sync",
        &store.store,
        (500, error_result("Sync store is unavailable.")),
    );
    assert!(anonymous_failure.user_id.is_empty());
    assert!(anonymous_failure.account_namespace.is_empty());

    let pending_token = "pending-response-namespace-token";
    let pending = store
        .store
        .issue_pending_token(
            "user-1",
            pending_token,
            "new phone",
            now_millis(),
            now_millis().saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS),
        )
        .unwrap();
    let pending_request = HttpRequest {
        method: "POST".to_string(),
        headers: vec![(
            "Authorization".to_string(),
            format!("Bearer {pending_token}"),
        )],
        ..HttpRequest::default()
    };
    let (pending_status, pending_result) = bind_response_account_identity(
        &pending_request,
        "/v1/sync",
        &store.store,
        (
            200,
            SyncClientResult {
                ok: true,
                user_id: "user-1".to_string(),
                restore_required: true,
                mode: "baseline_required".to_string(),
                ..SyncClientResult::default()
            },
        ),
    );
    assert_eq!(200, pending_status);
    assert_eq!("user-1", pending_result.user_id);
    assert_eq!(pending.token_id, pending_result.token_id);
    assert_eq!(
        expected.server_instance_id,
        pending_result.server_instance_id
    );
    assert_eq!(expected.account_namespace, pending_result.account_namespace);

    let (_, login_result) = bind_response_account_identity(
        &HttpRequest::default(),
        "/v1/login",
        &store.store,
        (
            200,
            SyncClientResult {
                ok: true,
                user_id: "user-1".to_string(),
                mode: "logged_in".to_string(),
                ..SyncClientResult::default()
            },
        ),
    );
    assert_eq!(expected.server_instance_id, login_result.server_instance_id);
    assert_eq!(expected.account_namespace, login_result.account_namespace);
}

#[test]
fn registration_token_accepts_guest_seed_but_later_login_requires_restore_receipt() {
    let store = sqlite_test_store(ServerStore::default());
    let register_body = serde_json::to_string(&RegisterRequest {
        request_id: random_token(18),
        email: "guest-seed@example.test".to_string(),
        password: "secret-password".to_string(),
        device_name: "first phone".to_string(),
    })
    .unwrap();
    let (status, registration) = handle_register(&register_body, &store.store);
    assert_eq!(200, status);
    assert!(registration.ok);

    let mut guest_seed: Value =
        serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    guest_seed["notes"] = json!([{
        "id": "guest-note",
        "title": "created before registration",
        "content": "must become the first account snapshot",
        "createdAtEpochMillis": 100,
        "updatedAtEpochMillis": 100,
        "deletedAtEpochMillis": null
    }]);
    let seed_request = HttpRequest {
        headers: vec![(
            "Authorization".to_string(),
            format!("Bearer {}", registration.token),
        )],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "registration-guest-seed".to_string(),
            app_data_json: guest_seed.to_string(),
            client_updated_at_epoch_millis: 100,
            acknowledged_generation: 0,
            restore_receipt: String::new(),
            device_name: "first phone".to_string(),
            force_upload: false,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, seeded) = handle_sync(&seed_request, &store.store);
    assert_eq!(200, status);
    assert!(seeded.ok);
    assert!(!seeded.restore_required);
    let seeded_account = store.store.read_account(&registration.user_id).unwrap();
    assert!(seeded_account.app_data_json.contains("guest-note"));

    let login_body = serde_json::to_string(&LoginRequest {
        request_id: random_token(18),
        email: "guest-seed@example.test".to_string(),
        password: "secret-password".to_string(),
        device_name: "second phone".to_string(),
    })
    .unwrap();
    let (status, login) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);
    let mut stale: Value = serde_json::from_str(&app_data::default_app_data_json(9_999)).unwrap();
    stale["notes"] = json!([{
        "id": "stale-note",
        "title": "must not overwrite guest seed",
        "content": "stale",
        "createdAtEpochMillis": 9_999,
        "updatedAtEpochMillis": 9_999,
        "deletedAtEpochMillis": null
    }]);
    let first_login_sync = HttpRequest {
        headers: vec![(
            "Authorization".to_string(),
            format!("Bearer {}", login.token),
        )],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "login-must-download-baseline".to_string(),
            app_data_json: stale.to_string(),
            client_updated_at_epoch_millis: 9_999,
            acknowledged_generation: 0,
            restore_receipt: String::new(),
            device_name: "second phone".to_string(),
            force_upload: false,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, baseline) = handle_sync(&first_login_sync, &store.store);
    assert_eq!(200, status);
    assert!(baseline.restore_required);
    assert_eq!(64, baseline.restore_receipt.len());
    assert_eq!(
        Some(seeded_account.app_data_json.clone()),
        baseline.app_data_json
    );
    assert_eq!(
        seeded_account,
        store.store.read_account(&registration.user_id).unwrap()
    );

    let acknowledge = HttpRequest {
        headers: first_login_sync.headers,
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "login-baseline-durable".to_string(),
            app_data_json: seeded_account.app_data_json.clone(),
            client_updated_at_epoch_millis: seeded_account.updated_at_epoch_millis,
            acknowledged_generation: baseline.current_generation,
            restore_receipt: baseline.restore_receipt,
            device_name: "second phone".to_string(),
            force_upload: false,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, acknowledged) = handle_sync(&acknowledge, &store.store);
    assert_eq!(200, status);
    assert!(!acknowledged.restore_required);
    assert!(acknowledged.restore_receipt.is_empty());
    assert_eq!(
        seeded_account,
        store.store.read_account(&registration.user_id).unwrap()
    );
}

#[test]
fn generation_zero_workspace_baseline_merges_then_proof_crosses_login_tokens() {
    let server_note = note_app_data("server-note", 100);
    let local_note = note_app_data("local-note", 200);
    let store = account_test_store("established-token", server_note.clone());
    let identity = store.store.server_account_identity("user-1").unwrap();
    let workspace_id = "01".repeat(32);
    let login_body = serde_json::to_string(&LoginRequest {
        request_id: "workspace-login-one".to_string(),
        email: "test@example.com".to_string(),
        password: "secret-password".to_string(),
        device_name: "workspace phone".to_string(),
    })
    .unwrap();
    let (status, login) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);

    let baseline_request = workspace_sync_request(
        &login.token,
        "workspace-baseline",
        local_note.clone(),
        200,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        "",
        false,
        false,
    );
    let (status, baseline) = handle_sync(&baseline_request, &store.store);
    assert_eq!(200, status);
    assert!(baseline.ok);
    assert!(baseline.restore_required);
    assert!(baseline.baseline_merge_required);
    assert_eq!("baseline_required", baseline.mode);
    assert!(baseline.workspace_proof.is_empty());
    assert_eq!(workspace_id, baseline.workspace_id);
    assert_eq!(Some(server_note.clone()), baseline.app_data_json);
    assert_eq!(64, baseline.restore_receipt.len());
    assert_eq!(
        server_note,
        store.store.read_account("user-1").unwrap().app_data_json
    );

    let merged =
        merge_sync_app_data_json(baseline.app_data_json.as_deref().unwrap(), &local_note, 300)
            .unwrap();
    let acknowledge_request = workspace_sync_request(
        &login.token,
        "workspace-baseline-ack",
        merged,
        300,
        baseline.current_generation,
        &baseline.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        "",
        false,
        false,
    );
    let (status, acknowledged) = handle_sync(&acknowledge_request, &store.store);
    assert_eq!(200, status);
    assert!(acknowledged.ok);
    assert!(!acknowledged.restore_required);
    assert!(!acknowledged.baseline_merge_required);
    assert_eq!(workspace_id, acknowledged.workspace_id);
    assert_eq!(64, acknowledged.workspace_proof.len());
    assert!(store
        .store
        .verify_workspace_capability("user-1", &workspace_id, 0, &acknowledged.workspace_proof,)
        .unwrap());
    let persisted = store.store.read_account("user-1").unwrap();
    assert!(persisted.app_data_json.contains("server-note"));
    assert!(persisted.app_data_json.contains("local-note"));

    let (status, second_login) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);
    assert_ne!(login.token, second_login.token);
    let cross_token_request = workspace_sync_request(
        &second_login.token,
        "workspace-cross-token",
        note_app_data("offline-after-logout", 400),
        400,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        &acknowledged.workspace_proof,
        false,
        false,
    );
    let (status, cross_token) = handle_sync(&cross_token_request, &store.store);
    assert_eq!(200, status);
    assert!(cross_token.ok);
    assert!(!cross_token.restore_required);
    assert_eq!(acknowledged.workspace_proof, cross_token.workspace_proof);
    let persisted = store.store.read_account("user-1").unwrap();
    assert!(persisted.app_data_json.contains("server-note"));
    assert!(persisted.app_data_json.contains("local-note"));
    assert!(persisted.app_data_json.contains("offline-after-logout"));
}

#[test]
fn restore_generation_invalidates_old_workspace_proof_until_exact_restore_ack() {
    let store = account_test_store("established-token", note_app_data("initial", 100));
    let identity = store.store.server_account_identity("user-1").unwrap();
    let workspace_id = "23".repeat(32);
    let old_proof = store
        .store
        .workspace_capability_proof("user-1", &workspace_id, 0)
        .unwrap();
    store
        .store
        .compare_and_swap_account("user-1", 0, &note_app_data("restore-target", 200), 200)
        .unwrap();
    store
        .store
        .compare_and_swap_account("user-1", 1, &note_app_data("discarded", 300), 300)
        .unwrap();
    let restored = store
        .store
        .restore_account_snapshot("user-1", 1, 2, 400)
        .unwrap();
    assert!(restored.app_data_json.contains("restore-target"));
    assert!(matches!(
        store
            .store
            .verify_workspace_capability("user-1", &workspace_id, 0, &old_proof),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation: 1,
            ..
        })
    ));

    let login_body = serde_json::to_string(&LoginRequest {
        request_id: "post-restore-login".to_string(),
        email: "test@example.com".to_string(),
        password: "secret-password".to_string(),
        device_name: "restored workspace".to_string(),
    })
    .unwrap();
    let (status, login) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);
    let stale_request = workspace_sync_request(
        &login.token,
        "old-proof-after-restore",
        note_app_data("offline-before-restore", 500),
        500,
        0,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        &old_proof,
        false,
        false,
    );
    let before = store.store.read_account("user-1").unwrap();
    let (status, exact_restore) = handle_sync(&stale_request, &store.store);
    assert_eq!(200, status);
    assert!(exact_restore.restore_required);
    assert!(!exact_restore.baseline_merge_required);
    assert_eq!("restore_required", exact_restore.mode);
    assert_eq!(1, exact_restore.current_generation);
    assert!(exact_restore.workspace_proof.is_empty());
    assert_eq!(
        Some(before.app_data_json.clone()),
        exact_restore.app_data_json
    );
    assert_eq!(before, store.store.read_account("user-1").unwrap());

    let acknowledge = workspace_sync_request(
        &login.token,
        "exact-restore-ack",
        exact_restore.app_data_json.clone().unwrap(),
        exact_restore.server_updated_at_epoch_millis,
        1,
        &exact_restore.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        "",
        false,
        false,
    );
    let (status, reissued) = handle_sync(&acknowledge, &store.store);
    assert_eq!(200, status);
    assert!(reissued.ok);
    assert!(!reissued.restore_required);
    assert_eq!(64, reissued.workspace_proof.len());
    assert_ne!(old_proof, reissued.workspace_proof);
    assert!(store
        .store
        .verify_workspace_capability("user-1", &workspace_id, 1, &reissued.workspace_proof,)
        .unwrap());

    let (status, relogin) = handle_login(&login_body, &store.store);
    assert_eq!(200, status);
    let post_restore_offline = workspace_sync_request(
        &relogin.token,
        "post-restore-cross-token",
        note_app_data("offline-after-restore", 600),
        600,
        1,
        "",
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        &reissued.workspace_proof,
        false,
        false,
    );
    let (status, merged) = handle_sync(&post_restore_offline, &store.store);
    assert_eq!(200, status);
    assert!(merged.ok);
    assert!(!merged.restore_required);
    assert!(store
        .store
        .read_account("user-1")
        .unwrap()
        .app_data_json
        .contains("offline-after-restore"));
}

#[test]
fn workspace_proof_from_another_account_is_rejected_without_writing() {
    let store = account_test_store("account-a-token", note_app_data("account-a", 100));
    store
        .store
        .create_user(NewStoredUser {
            id: "user-2".to_string(),
            email: "second@example.test".to_string(),
            password_salt: "salt-2".to_string(),
            password_hash: "hash-2".to_string(),
            password_scheme: "legacy_sha256".to_string(),
            created_at_epoch_millis: 10,
            updated_at_epoch_millis: 10,
            app_data_json: note_app_data("account-b", 100),
            account_revision: 0,
        })
        .unwrap();
    let now = now_millis();
    let token_b = store
        .store
        .issue_token(
            "user-2",
            "account-b-token",
            "phone-b",
            now,
            now.saturating_add(TOKEN_TTL_MILLIS),
        )
        .unwrap();
    let identity_b = store.store.server_account_identity("user-2").unwrap();
    let workspace_id = "45".repeat(32);
    let proof_a = store
        .store
        .workspace_capability_proof("user-1", &workspace_id, 0)
        .unwrap();
    let before = store.store.read_account("user-2").unwrap();
    let request = workspace_sync_request(
        "account-b-token",
        "wrong-account-proof",
        note_app_data("must-not-write", 999),
        999,
        0,
        "",
        &identity_b.server_instance_id,
        &identity_b.account_namespace,
        &workspace_id,
        &proof_a,
        false,
        false,
    );
    let (status, rejected) = handle_sync(&request, &store.store);
    assert_eq!(409, status);
    assert!(!rejected.ok);
    assert_eq!("workspace_proof_invalid", rejected.mode);
    assert_eq!("user-2", rejected.user_id);
    assert_eq!(token_b.token_id, rejected.token_id);
    assert_eq!(identity_b.server_instance_id, rejected.server_instance_id);
    assert_eq!(identity_b.account_namespace, rejected.account_namespace);
    assert!(rejected.token.is_empty());
    assert!(rejected.workspace_id.is_empty());
    assert!(rejected.workspace_proof.is_empty());
    assert!(rejected.app_data_json.is_none());
    assert!(!rejected.current_committed);
    assert_eq!(before, store.store.read_account("user-2").unwrap());
    let barrier = store
        .store
        .restore_barrier_state("user-2", token_b.id)
        .unwrap();
    assert!(!barrier.token_restore_acknowledged);
}

#[test]
fn generation_zero_identity_rebind_uses_baseline_then_receipt_before_merging() {
    let store = account_test_store("seed-token", note_app_data("server-note", 100));
    let now = now_millis();
    let raw_token = "migrated-active-token";
    let token = store
        .store
        .issue_token(
            "user-1",
            raw_token,
            "migrated phone",
            now,
            now.saturating_add(TOKEN_TTL_MILLIS),
        )
        .unwrap();
    let identity = store.store.server_account_identity("user-1").unwrap();
    let previous_server_instance_id = "ab".repeat(32);
    let previous_account_namespace =
        account_namespace_identifier(&previous_server_instance_id, "user-1");
    let workspace_id = "cd".repeat(32);
    let local_json = note_app_data("local-note", 200);
    let before = store.store.read_account("user-1").unwrap();

    let baseline_request = workspace_identity_rebind_request(
        raw_token,
        "identity-rebind-baseline",
        local_json.clone(),
        &identity.server_instance_id,
        &identity.account_namespace,
        &previous_server_instance_id,
        &previous_account_namespace,
        &workspace_id,
    );
    let (status, baseline) = handle_sync(&baseline_request, &store.store);
    assert_eq!(200, status);
    assert!(baseline.ok);
    assert!(baseline.current_committed);
    assert!(baseline.restore_required);
    assert!(baseline.baseline_merge_required);
    assert!(baseline.workspace_identity_rebound);
    assert_eq!("baseline_required", baseline.mode);
    assert_eq!(0, baseline.current_generation);
    assert_eq!(workspace_id, baseline.workspace_id);
    assert!(baseline.workspace_proof.is_empty());
    assert!(!baseline.restore_receipt.is_empty());
    assert_eq!(before, store.store.read_account("user-1").unwrap());
    assert!(
        !store
            .store
            .restore_barrier_state("user-1", token.id)
            .unwrap()
            .token_restore_acknowledged
    );

    let merged =
        merge_sync_app_data_json(baseline.app_data_json.as_deref().unwrap(), &local_json, now)
            .unwrap();
    let acknowledge_request = workspace_sync_request(
        raw_token,
        "identity-rebind-acknowledge",
        merged,
        200,
        0,
        &baseline.restore_receipt,
        &identity.server_instance_id,
        &identity.account_namespace,
        &workspace_id,
        "",
        false,
        false,
    );
    let (status, acknowledged) = handle_sync(&acknowledge_request, &store.store);
    assert_eq!(200, status);
    assert!(acknowledged.ok);
    assert!(!acknowledged.restore_required);
    assert!(!acknowledged.workspace_identity_rebound);
    assert_eq!(workspace_id, acknowledged.workspace_id);
    assert!(store
        .store
        .verify_workspace_capability("user-1", &workspace_id, 0, &acknowledged.workspace_proof,)
        .unwrap());
    let account = store.store.read_account("user-1").unwrap();
    assert!(account.app_data_json.contains("server-note"));
    assert!(account.app_data_json.contains("local-note"));
    assert!(
        store
            .store
            .restore_barrier_state("user-1", token.id)
            .unwrap()
            .token_restore_acknowledged
    );
}

#[test]
fn identity_rebind_rejects_unsafe_variants_without_changing_account_snapshot() {
    for variant in [
        "force-upload",
        "force-download",
        "nonzero-ack",
        "nonempty-proof",
        "nonempty-receipt",
        "wrong-previous-namespace",
        "same-server",
    ] {
        let store = account_test_store("seed-token", note_app_data("server-note", 100));
        let now = now_millis();
        let raw_token = format!("rebind-token-{variant}");
        store
            .store
            .issue_token(
                "user-1",
                &raw_token,
                variant,
                now,
                now.saturating_add(TOKEN_TTL_MILLIS),
            )
            .unwrap();
        let identity = store.store.server_account_identity("user-1").unwrap();
        let previous_server_instance_id = "ef".repeat(32);
        let previous_account_namespace =
            account_namespace_identifier(&previous_server_instance_id, "user-1");
        let mut request = workspace_identity_rebind_request(
            &raw_token,
            &format!("invalid-rebind-{variant}"),
            note_app_data("must-not-write", 999),
            &identity.server_instance_id,
            &identity.account_namespace,
            &previous_server_instance_id,
            &previous_account_namespace,
            &"12".repeat(32),
        );
        let mut snapshot: SyncSnapshotRequest = serde_json::from_str(&request.body).unwrap();
        match variant {
            "force-upload" => snapshot.force_upload = true,
            "force-download" => snapshot.force_download = true,
            "nonzero-ack" => snapshot.acknowledged_generation = 1,
            "nonempty-proof" => snapshot.workspace_proof = "34".repeat(32),
            "nonempty-receipt" => snapshot.restore_receipt = "old-receipt".to_string(),
            "wrong-previous-namespace" => snapshot.previous_account_namespace = "56".repeat(32),
            "same-server" => {
                snapshot.previous_server_instance_id = identity.server_instance_id.clone();
                snapshot.previous_account_namespace = identity.account_namespace.clone();
            }
            _ => unreachable!(),
        }
        request.body = serde_json::to_string(&snapshot).unwrap();
        let before = store.store.read_account("user-1").unwrap();
        let (status, rejected) = handle_sync(&request, &store.store);
        assert_eq!(409, status, "variant={variant}");
        assert!(!rejected.ok, "variant={variant}");
        assert_eq!(
            "workspace_identity_rebind_invalid", rejected.mode,
            "variant={variant}"
        );
        assert!(!rejected.current_committed, "variant={variant}");
        assert_eq!(before, store.store.read_account("user-1").unwrap());
    }

    let store = account_test_store("already-acknowledged", note_app_data("server-note", 100));
    let identity = store.store.server_account_identity("user-1").unwrap();
    let previous_server_instance_id = "78".repeat(32);
    let previous_account_namespace =
        account_namespace_identifier(&previous_server_instance_id, "user-1");
    let before = store.store.read_account("user-1").unwrap();
    let request = workspace_identity_rebind_request(
        "already-acknowledged",
        "rebind-already-acknowledged",
        note_app_data("must-not-write", 999),
        &identity.server_instance_id,
        &identity.account_namespace,
        &previous_server_instance_id,
        &previous_account_namespace,
        &"9a".repeat(32),
    );
    let (status, rejected) = handle_sync(&request, &store.store);
    assert_eq!(409, status);
    assert_eq!("workspace_identity_rebind_invalid", rejected.mode);
    assert_eq!(before, store.store.read_account("user-1").unwrap());
}

#[test]
fn schema_11_is_upgraded_to_15_and_schema_16_is_rejected() {
    assert_eq!(15, app_data::APP_DATA_SCHEMA_VERSION);
    let schema_11 = json!({
        "schemaVersion": 11,
        "sessions": [{
            "id": "schema-11-session",
            "slotId": 1,
            "slotTitle": "legacy session",
            "startedAtEpochMillis": 100,
            "endedAtEpochMillis": 200,
            "durationMillis": 100
        }]
    });
    assert_eq!(
        AppDataSchemaStatus::Supported,
        inspect_app_data_schema(&schema_11.to_string())
    );
    let upgraded = sanitize_sync_app_data(&schema_11.to_string(), 1_000).unwrap();
    let upgraded: Value = serde_json::from_str(&upgraded).unwrap();
    assert_eq!(15, upgraded["schemaVersion"]);
    assert_eq!("schema-11-session", upgraded["sessions"][0]["id"]);

    let schema_16 = json!({
        "schemaVersion": 16,
        "futureField": {"mustNotBeDropped": true}
    })
    .to_string();
    assert_eq!(
        AppDataSchemaStatus::Future("16".to_string()),
        inspect_app_data_schema(&schema_16)
    );
    assert!(sanitize_sync_app_data(&schema_16, 1_000).is_none());
}

#[test]
fn current_schema_unknown_fields_fail_closed_without_rewriting_account() {
    let mut unknown_top: Value = serde_json::from_str(&app_data::default_app_data_json(1)).unwrap();
    unknown_top["unknownTopLevelVNext"] = json!({"opaque": [1, 2, 3]});
    assert!(sanitize_sync_app_data(&unknown_top.to_string(), 1_000).is_none());

    let mut unknown_nested: Value =
        serde_json::from_str(&app_data::default_app_data_json(1)).unwrap();
    unknown_nested["financeProfile"]["unknownNestedVNext"] =
        json!({"mustNotBeSilentlyDropped": true});
    assert!(sanitize_sync_app_data(&unknown_nested.to_string(), 1_000).is_none());

    let token = "same-schema-unknown-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let before = store.store.read_account("user-1").unwrap();
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "same-schema-unknown".to_string(),
            app_data_json: unknown_nested.to_string(),
            client_updated_at_epoch_millis: 2,
            acknowledged_generation: 0,
            restore_receipt: String::new(),
            device_name: "future-shaped-client".to_string(),
            force_upload: false,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, response) = handle_sync(&request, &store.store);
    assert_eq!(400, status);
    assert!(!response.ok);
    assert_eq!(before, store.store.read_account("user-1").unwrap());
}

#[test]
fn future_schema_requests_never_rewrite_existing_account_state() {
    let token = "future-schema-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let before = store.store.read_account("user-1").unwrap();
    let future_documents = [
        json!({
            "schemaVersion": app_data::APP_DATA_SCHEMA_VERSION + 1,
            "futureTopLevel": {"mustSurvive": true}
        }),
        json!({
            "schemaVersion": app_data::APP_DATA_SCHEMA_VERSION + 1,
            "notes": [{
                "id": "future-note",
                "futureNestedDocument": {"blocksV2": [1, 2, 3]}
            }]
        }),
        json!({
            "schemaVersion": app_data::APP_DATA_SCHEMA_VERSION + 1,
            "tombstones": [{
                "entityType": "futureEntity",
                "entityId": "future-id",
                "deletedAtEpochMillis": 9_999,
                "futureDeletionVector": {"device": 7}
            }]
        }),
    ];

    for (index, future) in future_documents.into_iter().enumerate() {
        let request = HttpRequest {
            headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
            body: serde_json::to_string(&SyncSnapshotRequest {
                request_id: format!("future-schema-{index}"),
                app_data_json: future.to_string(),
                client_updated_at_epoch_millis: 9_999,
                acknowledged_generation: 0,
                restore_receipt: String::new(),
                device_name: "future-client".to_string(),
                force_upload: false,
                force_download: false,
                ..SyncSnapshotRequest::default()
            })
            .unwrap(),
            ..HttpRequest::default()
        };
        let (status, response) = handle_sync(&request, &store.store);
        assert_eq!(426, status);
        assert!(!response.ok);
        assert_eq!("upgrade_required", response.mode);
        assert!(response.message.contains("Upgrade the sync server"));
        assert_eq!(before, store.store.read_account("user-1").unwrap());
    }
}

#[test]
fn stored_future_schema_blocks_merge_and_download_without_rewrite() {
    let token = "stored-future-schema-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let future = json!({
        "schemaVersion": app_data::APP_DATA_SCHEMA_VERSION + 1,
        "futureTopLevel": {"mustRemainExact": [1, 2, 3]},
        "tombstones": [{
            "entityType": "futureEntity",
            "entityId": "kept",
            "deletedAtEpochMillis": 99_999,
            "futureClock": "opaque"
        }]
    })
    .to_string();
    let connection = rusqlite::Connection::open(store.store.database_path()).unwrap();
    let content_sha256 = hex_bytes(&Sha256::digest(future.as_bytes()));
    let (revision, updated_at_epoch_millis, restore_generation) = connection
        .query_row(
            "SELECT revision, updated_at_epoch_millis, restore_generation \
                 FROM account_snapshots WHERE user_id = 'user-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    let envelope_sha256 = crate::server_store::account_snapshot_envelope_sha256(
        "user-1",
        &future,
        revision,
        updated_at_epoch_millis,
        restore_generation,
    );
    connection
        .execute(
            "UPDATE account_snapshots \
                 SET app_data_json = ?1, content_sha256 = ?2, envelope_sha256 = ?3 \
                 WHERE user_id = 'user-1'",
            rusqlite::params![future, content_sha256, envelope_sha256],
        )
        .unwrap();
    drop(connection);
    let before = store.store.read_account("user-1").unwrap();
    let supported = app_data::default_app_data_json(2);

    for force_download in [false, true] {
        let request = HttpRequest {
            headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
            body: serde_json::to_string(&SyncSnapshotRequest {
                request_id: format!("stored-future-{force_download}"),
                app_data_json: supported.clone(),
                client_updated_at_epoch_millis: 2,
                acknowledged_generation: 0,
                restore_receipt: String::new(),
                device_name: "older-server-client".to_string(),
                force_upload: false,
                force_download,
                ..SyncSnapshotRequest::default()
            })
            .unwrap(),
            ..HttpRequest::default()
        };
        let (status, response) = handle_sync(&request, &store.store);
        assert_eq!(426, status);
        assert_eq!("upgrade_required", response.mode);
        assert_eq!(before, store.store.read_account("user-1").unwrap());
    }
}

#[test]
fn sync_request_id_replay_does_not_increment_revision_twice() {
    let token = "dedup-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let mut client: Value = serde_json::from_str(&app_data::default_app_data_json(2)).unwrap();
    client["notes"] = json!([{
        "id": "dedup-note",
        "title": "one write",
        "content": "one write",
        "createdAtEpochMillis": 2,
        "updatedAtEpochMillis": 2,
        "deletedAtEpochMillis": null
    }]);
    let body = serde_json::to_string(&SyncSnapshotRequest {
        request_id: "same-request-id".to_string(),
        app_data_json: client.to_string(),
        client_updated_at_epoch_millis: 2,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        device_name: "phone".to_string(),
        force_upload: false,
        force_download: false,
        ..SyncSnapshotRequest::default()
    })
    .unwrap();
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body,
        ..HttpRequest::default()
    };
    let first = handle_sync(&request, &store.store);
    let first_revision = store.store.read_account("user-1").unwrap().revision;
    let second = handle_sync(&request, &store.store);
    let second_revision = store.store.read_account("user-1").unwrap().revision;
    assert_eq!(200, first.0);
    assert_eq!(200, second.0);
    assert_eq!(encode_result(&first.1), encode_result(&second.1));
    assert_eq!(first_revision, second_revision);
}

#[test]
fn restored_account_forces_exact_download_before_any_stale_upload_merge() {
    let token = "restore-barrier-token";
    let restored_json = app_data::default_app_data_json(1);
    let store = account_test_store(token, restored_json.clone());
    let authenticated = match store.store.authenticate_token(token, now_millis()).unwrap() {
        TokenAuthentication::Active(value) => value,
        other => panic!("expected active token, got {other:?}"),
    };
    let mut newer: Value = serde_json::from_str(&app_data::default_app_data_json(100)).unwrap();
    newer["notes"] = json!([{
        "id": "post-backup-note",
        "title": "must disappear on restore",
        "content": "newer branch",
        "createdAtEpochMillis": 100,
        "updatedAtEpochMillis": 100,
        "deletedAtEpochMillis": null
    }]);
    store
        .store
        .compare_and_swap_account("user-1", 0, &newer.to_string(), 100)
        .unwrap();
    store
        .store
        .restore_account_snapshot("user-1", 0, 1, 200)
        .unwrap();
    let exact_restored = store.store.read_account("user-1").unwrap();
    assert_eq!(restored_json, exact_restored.app_data_json);

    let stale_media = b"stale-media-after-restore";
    let media_upload_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: json!({
            "requestId": "stale-media-upload-after-restore",
            "acknowledgedGeneration": 0,
            "attachmentId": "stale-media",
            "sha256": hex_bytes(&Sha256::digest(stale_media)),
            "mimeType": "application/octet-stream",
            "sizeBytes": stale_media.len(),
            "updatedAtEpochMillis": 300,
            "contentBase64": BASE64_STANDARD.encode(stale_media)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, media_upload) = handle_media_upload(&media_upload_request, &store.store);
    assert_eq!(409, status);
    assert!(media_upload.restore_required);
    assert_eq!(1, media_upload.current_generation);

    let media_delete_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: json!({
            "requestId": "stale-media-delete-after-restore",
            "acknowledgedGeneration": 0,
            "attachmentId": "missing-media",
            "deletedAtEpochMillis": 301
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, media_delete) = handle_media_delete(&media_delete_request, &store.store);
    assert_eq!(409, status);
    assert!(media_delete.restore_required);
    assert_eq!(1, media_delete.current_generation);
    assert!(store
        .store
        .list_media_metadata("user-1", true)
        .unwrap()
        .is_empty());

    let mut stale: Value = serde_json::from_str(&app_data::default_app_data_json(9_999)).unwrap();
    stale["notes"] = json!([{
        "id": "stale-offline-note",
        "title": "must not overwrite restore",
        "content": "stale device",
        "createdAtEpochMillis": 9_999,
        "updatedAtEpochMillis": 9_999,
        "deletedAtEpochMillis": null
    }]);
    let stale_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "stale-after-restore".to_string(),
            app_data_json: stale.to_string(),
            client_updated_at_epoch_millis: 9_999,
            acknowledged_generation: 0,
            restore_receipt: String::new(),
            device_name: "offline-phone".to_string(),
            force_upload: true,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, forced) = handle_sync(&stale_request, &store.store);
    assert_eq!(200, status);
    assert!(forced.ok);
    assert!(forced.restore_required);
    assert_eq!(1, forced.current_generation);
    assert_eq!("restore_required", forced.mode);
    assert_eq!(
        Some(exact_restored.app_data_json.clone()),
        forced.app_data_json
    );
    assert_eq!(exact_restored, store.store.read_account("user-1").unwrap());
    assert_eq!(
        0,
        store
            .store
            .restore_barrier_state("user-1", authenticated.token_id)
            .unwrap()
            .token_last_seen_generation
    );

    let acknowledged_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "acknowledge-restore".to_string(),
            app_data_json: exact_restored.app_data_json.clone(),
            client_updated_at_epoch_millis: 200,
            acknowledged_generation: 1,
            restore_receipt: forced.restore_receipt.clone(),
            device_name: "offline-phone".to_string(),
            force_upload: false,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, acknowledged) = handle_sync(&acknowledged_request, &store.store);
    assert_eq!(200, status);
    assert!(acknowledged.ok);
    assert!(!acknowledged.restore_required);
    assert_eq!(1, acknowledged.current_generation);
    assert_eq!(
        1,
        store
            .store
            .restore_barrier_state("user-1", authenticated.token_id)
            .unwrap()
            .token_last_seen_generation
    );

    let (status, forced_again) = handle_sync(&stale_request, &store.store);
    assert_eq!(200, status);
    assert!(forced_again.restore_required);
    assert_eq!(exact_restored, store.store.read_account("user-1").unwrap());
}

#[test]
fn client_generation_ahead_returns_rollback_for_sync_and_media_without_writes() {
    let token = "generation-rollback-http-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let authenticated = match store.store.authenticate_token(token, now_millis()).unwrap() {
        TokenAuthentication::Active(value) => value,
        other => panic!("expected active token, got {other:?}"),
    };
    for expected_revision in 0..3 {
        store
            .store
            .restore_account_snapshot("user-1", 0, expected_revision, 100 + expected_revision)
            .unwrap();
    }
    let account_before = store.store.read_account("user-1").unwrap();
    let barrier_before = store
        .store
        .restore_barrier_state("user-1", authenticated.token_id)
        .unwrap();
    assert_eq!(3, barrier_before.current_generation);

    let sync_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "ahead-sync".to_string(),
            app_data_json: app_data::default_app_data_json(9_999),
            client_updated_at_epoch_millis: 9_999,
            acknowledged_generation: 5,
            restore_receipt: "must-not-be-consumed".to_string(),
            device_name: "rolled-back phone".to_string(),
            force_upload: true,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, sync_response) = handle_sync(&sync_request, &store.store);
    assert_eq!(409, status);
    assert_eq!("server_generation_rollback", sync_response.mode);
    assert!(!sync_response.restore_required);
    assert!(sync_response.app_data_json.is_none());
    assert!(sync_response.restore_receipt.is_empty());

    let media = b"must-not-be-written";
    let upload = HttpRequest {
        headers: sync_request.headers.clone(),
        body: json!({
            "requestId": "ahead-media-upload",
            "acknowledgedGeneration": 5,
            "restoreReceipt": "must-not-be-consumed",
            "attachmentId": "ahead-media",
            "sha256": hex_bytes(&Sha256::digest(media)),
            "mimeType": "application/octet-stream",
            "sizeBytes": media.len(),
            "updatedAtEpochMillis": 10_000,
            "contentBase64": BASE64_STANDARD.encode(media)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, upload_response) = handle_media_upload(&upload, &store.store);
    assert_eq!(409, status);
    assert_eq!("server_generation_rollback", upload_response.mode);
    assert!(!upload_response.restore_required);
    assert!(upload_response.restore_receipt.is_empty());

    let deletion = HttpRequest {
        headers: sync_request.headers,
        body: json!({
            "requestId": "ahead-media-delete",
            "acknowledgedGeneration": 5,
            "restoreReceipt": "must-not-be-consumed",
            "attachmentId": "ahead-media",
            "deletedAtEpochMillis": 10_001
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, delete_response) = handle_media_delete(&deletion, &store.store);
    assert_eq!(409, status);
    assert_eq!("server_generation_rollback", delete_response.mode);
    assert!(!delete_response.restore_required);
    assert!(delete_response.restore_receipt.is_empty());

    assert_eq!(account_before, store.store.read_account("user-1").unwrap());
    assert!(store
        .store
        .list_media_metadata("user-1", true)
        .unwrap()
        .is_empty());
    assert_eq!(
        barrier_before,
        store
            .store
            .restore_barrier_state("user-1", authenticated.token_id)
            .unwrap()
    );
}

#[test]
fn json_and_media_upload_accept_same_large_future_logical_revision() {
    let token = "future-logical-revision-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let future_revision = now_millis().saturating_add(31 * 24 * 60 * 60 * 1_000);
    let content = b"future-clock-media";
    let sha256 = hex_bytes(&Sha256::digest(content));
    let mut app: Value =
        serde_json::from_str(&app_data::default_app_data_json(future_revision)).unwrap();
    app["notes"] = json!([{
        "id": "future-note",
        "title": "future clock",
        "content": "JSON and BLOB must share one logical revision policy",
        "attachments": [{
            "id": "future-media",
            "sha256": sha256.clone(),
            "mimeType": "application/octet-stream",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": future_revision
        }],
        "revisions": [],
        "createdAtEpochMillis": future_revision,
        "updatedAtEpochMillis": future_revision,
        "deletedAtEpochMillis": null
    }]);
    let sync_request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: serde_json::to_string(&SyncSnapshotRequest {
            request_id: "future-json".to_string(),
            app_data_json: app.to_string(),
            client_updated_at_epoch_millis: future_revision,
            acknowledged_generation: 0,
            restore_receipt: String::new(),
            device_name: "future phone".to_string(),
            force_upload: true,
            force_download: false,
            ..SyncSnapshotRequest::default()
        })
        .unwrap(),
        ..HttpRequest::default()
    };
    let (status, sync_response) = handle_sync(&sync_request, &store.store);
    assert_eq!(200, status);
    assert!(sync_response.ok);

    let upload = HttpRequest {
        headers: sync_request.headers,
        body: json!({
            "requestId": "future-media",
            "acknowledgedGeneration": 0,
            "attachmentId": "future-media",
            "sha256": sha256,
            "mimeType": "application/octet-stream",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": future_revision,
            "contentBase64": BASE64_STANDARD.encode(content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, upload_response) = handle_media_upload(&upload, &store.store);
    assert_eq!(200, status);
    assert!(upload_response.ok);
    let stored = store
        .store
        .read_media("user-1", "future-media")
        .unwrap()
        .unwrap();
    assert_eq!(future_revision, stored.metadata.updated_at_epoch_millis);
    assert_eq!(content.as_slice(), stored.content.as_slice());
}

#[test]
fn concurrent_sync_requests_retry_cas_and_keep_both_histories() {
    let token = "concurrent-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for index in 0..2 {
        let store = store.store.clone();
        let barrier = Arc::clone(&barrier);
        let token = token.to_string();
        workers.push(thread::spawn(move || {
            let mut data: Value =
                serde_json::from_str(&app_data::default_app_data_json(10 + index)).unwrap();
            data["sessions"] = json!([{
                "id": format!("concurrent-{index}"),
                "slotId": index + 1,
                "slotTitle": format!("worker {index}"),
                "startedAtEpochMillis": 10 + index,
                "endedAtEpochMillis": 20 + index,
                "durationMillis": 10
            }]);
            let request = HttpRequest {
                headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
                body: serde_json::to_string(&SyncSnapshotRequest {
                    request_id: format!("concurrent-request-{index}"),
                    app_data_json: data.to_string(),
                    client_updated_at_epoch_millis: 20 + index,
                    acknowledged_generation: 0,
                    restore_receipt: String::new(),
                    device_name: format!("worker-{index}"),
                    force_upload: false,
                    force_download: false,
                    ..SyncSnapshotRequest::default()
                })
                .unwrap(),
                ..HttpRequest::default()
            };
            barrier.wait();
            handle_sync(&request, &store)
        }));
    }
    for worker in workers {
        assert_eq!(200, worker.join().unwrap().0);
    }
    let account = store.store.read_account("user-1").unwrap();
    assert!(account.app_data_json.contains("concurrent-0"));
    assert!(account.app_data_json.contains("concurrent-1"));
}

#[test]
fn desktop_media_preflight_accepts_matching_server_bytes_without_a_local_copy() {
    use crate::desktop_note_media::DesktopNoteMediaStore;
    use crate::desktop_note_media_sync::{
        sync_desktop_note_media_with_direction, DesktopNoteMediaSyncDirection,
    };
    let root = std::env::temp_dir().join(format!("gridtimer_remote_media_{}", random_token(12)));
    let store = DesktopNoteMediaStore::new(&root).unwrap();
    let sha256 = "a".repeat(64);
    let attachment_id = "history-image";
    let user_id = "media-preflight-user";
    let token = "media-preflight-token";
    // The current note removed the image, but a recovery point still needs it.
    // Its matching bytes exist on the authenticated account server only.
    let data = json!({
        "notes": [{"attachments": [], "revisions": [{"attachments": [{
            "id": attachment_id, "sha256": sha256, "mimeType": "image/png",
            "sizeBytes": 12, "updatedAtEpochMillis": 100
        }]}]}],
        "tombstones": [{"entityType": "noteAttachment", "entityId": attachment_id,
            "deletedAtEpochMillis": 200}]
    });
    let (url, captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_manifest".to_string(),
            media_items: vec![MediaManifestItem {
                attachment_id: attachment_id.to_string(),
                sha256: sha256.clone(),
                mime_type: "image/png".to_string(),
                size_bytes: 12,
                updated_at_epoch_millis: 100,
                deleted_at_epoch_millis: 0,
            }],
            ..SyncClientResult::default()
        },
    );
    let summary = sync_desktop_note_media_with_direction(
        &store,
        &data.to_string(),
        &url,
        token,
        user_id,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
        DesktopNoteMediaSyncDirection::UploadOnly,
    );
    let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!("/v1/media/manifest", request.path);
    assert_eq!(0, summary.failures, "{:?}", summary.failure_messages);
    assert_eq!(
        0, summary.conflicts,
        "Remote availability must not block document merge"
    );
    assert_eq!(
        Some(&sha256),
        summary.resolved_sha256_by_attachment_id.get(attachment_id)
    );
    assert_eq!(0, summary.uploaded);
    assert_eq!(0, summary.downloaded);
    assert!(summary.remote_deleted_at_by_attachment_id.is_empty());
    assert!(store.manifest_entry_for(attachment_id).unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_media_upload_and_delete_serialize_restore_barrier_requests() {
    let user_id = "desktop-media-user";
    let token = "desktop-media-token";
    let content = b"desktop attachment";
    let sha256 = hex_bytes(&Sha256::digest(content));
    let upload_item = MediaManifestItem {
        attachment_id: "desktop-attachment".to_string(),
        sha256: sha256.clone(),
        mime_type: "text/plain".to_string(),
        size_bytes: content.len() as i64,
        updated_at_epoch_millis: 123,
        deleted_at_epoch_millis: 0,
    };
    let (server_url, captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            message: "Media uploaded.".to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_upload".to_string(),
            media_item: Some(upload_item),
            ..SyncClientResult::default()
        },
    );
    let result = desktop_media_upload(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        7,
        " restore-receipt ",
        " desktop-attachment ",
        &sha256.to_ascii_uppercase(),
        " text/plain ",
        content.len() as i64,
        123,
        content,
        true,
    );
    assert!(result.ok, "{}", result.message);
    let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!("POST", request.method);
    assert_eq!("/v1/media/upload", request.path);
    assert!(request.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == &format!("Bearer {token}")
    }));
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(7, body["acknowledgedGeneration"]);
    assert_eq!("restore-receipt", body["restoreReceipt"]);
    assert_eq!("desktop-attachment", body["attachmentId"]);
    assert_eq!(sha256, body["sha256"]);
    assert_eq!("text/plain", body["mimeType"]);
    assert_eq!(content.len() as i64, body["sizeBytes"]);
    assert_eq!(123, body["updatedAtEpochMillis"]);
    assert_eq!(true, body["restoreDeleted"]);
    assert_eq!(BASE64_STANDARD.encode(content), body["contentBase64"]);
    assert!(valid_request_id(
        body["requestId"].as_str().unwrap_or_default()
    ));

    let delete_item = MediaManifestItem {
        attachment_id: "desktop-attachment".to_string(),
        deleted_at_epoch_millis: 456,
        ..MediaManifestItem::default()
    };
    let (server_url, captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            message: "Media deletion recorded.".to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_delete".to_string(),
            media_item: Some(delete_item),
            ..SyncClientResult::default()
        },
    );
    let result = desktop_media_delete(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        8,
        "delete-receipt",
        "desktop-attachment",
        456,
    );
    assert!(result.ok, "{}", result.message);
    let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!("/v1/media/delete", request.path);
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(8, body["acknowledgedGeneration"]);
    assert_eq!("delete-receipt", body["restoreReceipt"]);
    assert_eq!("desktop-attachment", body["attachmentId"]);
    assert_eq!(456, body["deletedAtEpochMillis"]);
    assert!(valid_request_id(
        body["requestId"].as_str().unwrap_or_default()
    ));

    let (server_url, _captured, worker) = one_shot_media_client_server(
        409,
        SyncClientResult {
            ok: false,
            message: "Media was deleted by a newer revision.".to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_deleted".to_string(),
            media_item: Some(MediaManifestItem {
                attachment_id: "other-attachment".to_string(),
                deleted_at_epoch_millis: 500,
                ..MediaManifestItem::default()
            }),
            ..SyncClientResult::default()
        },
    );
    let conflict = desktop_media_upload(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        8,
        "",
        "desktop-attachment",
        &sha256,
        "text/plain",
        content.len() as i64,
        500,
        content,
        false,
    );
    worker.join().unwrap();
    assert!(!conflict.ok);
    assert_eq!(
        "Media upload response attachmentId mismatch.",
        conflict.message
    );
}

#[test]
fn desktop_media_manifest_and_download_parse_bound_responses() {
    let user_id = "desktop-media-user";
    let token = "desktop-media-token";
    let content = b"downloaded attachment";
    let sha256 = hex_bytes(&Sha256::digest(content));
    let active_item = MediaManifestItem {
        attachment_id: "download-attachment".to_string(),
        sha256: sha256.clone(),
        mime_type: "application/octet-stream".to_string(),
        size_bytes: content.len() as i64,
        updated_at_epoch_millis: 900,
        deleted_at_epoch_millis: 0,
    };
    let tombstone = MediaManifestItem {
        attachment_id: "deleted-attachment".to_string(),
        deleted_at_epoch_millis: 901,
        ..MediaManifestItem::default()
    };
    let (server_url, captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            message: "Media manifest loaded.".to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_manifest".to_string(),
            media_items: vec![active_item.clone(), tombstone],
            legacy_media_references: vec![LegacyMediaReference {
                attachment_id: "legacy-gap".to_string(),
                mime_type: "image/png".to_string(),
                size_bytes: 125,
            }],
            ..SyncClientResult::default()
        },
    );
    let manifest = desktop_media_manifest(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        -1,
        " manifest-receipt ",
    );
    assert!(manifest.ok, "{}", manifest.message);
    assert_eq!(2, manifest.media_items.len());
    assert_eq!(1, manifest.legacy_media_references.len());
    let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!("/v1/media/manifest", request.path);
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(0, body["acknowledgedGeneration"]);
    assert_eq!("manifest-receipt", body["restoreReceipt"]);

    let (server_url, captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            message: "Media downloaded.".to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_download".to_string(),
            media_item: Some(active_item.clone()),
            media_content_base64: BASE64_STANDARD.encode(content),
            ..SyncClientResult::default()
        },
    );
    let download = desktop_media_download(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        9,
        "download-receipt",
        "download-attachment",
    );
    assert!(download.ok, "{}", download.message);
    assert_eq!(
        content.to_vec(),
        BASE64_STANDARD
            .decode(download.media_content_base64)
            .unwrap()
    );
    let request = captured.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!("/v1/media/download", request.path);
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(9, body["acknowledgedGeneration"]);
    assert_eq!("download-receipt", body["restoreReceipt"]);
    assert_eq!("download-attachment", body["attachmentId"]);

    let (server_url, _captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: OTHER_DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_manifest".to_string(),
            ..SyncClientResult::default()
        },
    );
    let cross_server = desktop_media_manifest(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        9,
        "",
    );
    worker.join().unwrap();
    assert!(!cross_server.ok);
    assert_eq!(
        "Media response server instance does not match the authenticated server.",
        cross_server.message
    );

    let (server_url, _captured, worker) = one_shot_media_client_server(
        409,
        SyncClientResult {
            ok: false,
            message: "Media was deleted by a newer revision.".to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: OTHER_DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            mode: "media_deleted".to_string(),
            media_item: Some(active_item),
            ..SyncClientResult::default()
        },
    );
    let cross_namespace = desktop_media_download(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        9,
        "",
        "download-attachment",
    );
    worker.join().unwrap();
    assert!(!cross_namespace.ok);
    assert_eq!(
        "Media response account namespace does not match the authenticated account.",
        cross_namespace.message
    );

    let (server_url, _captured, worker) = one_shot_media_client_server(
        401,
        SyncClientResult {
            ok: false,
            message: "Missing sync token.".to_string(),
            ..SyncClientResult::default()
        },
    );
    let unauthenticated = desktop_media_manifest(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        9,
        "",
    );
    worker.join().unwrap();
    assert!(!unauthenticated.ok);
    assert_eq!("Missing sync token.", unauthenticated.message);
}

#[test]
fn legacy_media_manifest_rejects_wrong_identity_duplicates_and_invalid_metadata() {
    let token = "legacy-manifest-token";
    let user = "legacy-manifest-user";
    let valid = SyncClientResult {
        ok: true,
        user_id: user.to_string(),
        token_id: token_identifier(token),
        server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
        account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
        mode: "media_manifest".to_string(),
        legacy_media_references: vec![LegacyMediaReference {
            attachment_id: "old-image".to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 123,
        }],
        ..Default::default()
    };
    let validate = |result| {
        validate_desktop_media_manifest_response(
            result,
            user,
            token,
            DESKTOP_MEDIA_SERVER_INSTANCE_ID,
            DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        )
    };
    assert!(validate(valid.clone()).ok);
    for field in [
        "user",
        "token",
        "server",
        "namespace",
        "duplicate",
        "id",
        "size",
        "mime",
    ] {
        let mut invalid = valid.clone();
        match field {
            "user" => invalid.user_id = "other".to_string(),
            "token" => invalid.token_id = token_identifier("other"),
            "server" => {
                invalid.server_instance_id = OTHER_DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string()
            }
            "namespace" => invalid.account_namespace = "c".repeat(64),
            "duplicate" => invalid
                .legacy_media_references
                .push(invalid.legacy_media_references[0].clone()),
            "id" => invalid.legacy_media_references[0].attachment_id = "../bad".to_string(),
            "size" => invalid.legacy_media_references[0].size_bytes = -1,
            "mime" => invalid.legacy_media_references[0].mime_type = "bad mime".to_string(),
            _ => unreachable!(),
        }
        assert!(!validate(invalid).ok, "{field}");
    }
}

#[test]
fn desktop_media_rejects_size_and_sha256_mismatches() {
    let content = b"verified bytes";
    let sha256 = hex_bytes(&Sha256::digest(content));
    let size_mismatch = desktop_media_upload(
        "http://127.0.0.1:1",
        "desktop-user",
        "desktop-token",
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
        "attachment",
        &sha256,
        "application/octet-stream",
        content.len() as i64 + 1,
        100,
        content,
        false,
    );
    assert!(!size_mismatch.ok);
    assert_eq!(
        "attachment size does not match its content",
        size_mismatch.message
    );
    let hash_mismatch = desktop_media_upload(
        "http://127.0.0.1:1",
        "desktop-user",
        "desktop-token",
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
        "attachment",
        &"0".repeat(64),
        "application/octet-stream",
        content.len() as i64,
        100,
        content,
        false,
    );
    assert!(!hash_mismatch.ok);
    assert_eq!(
        "attachment SHA-256 does not match its content",
        hash_mismatch.message
    );
}

#[test]
fn desktop_media_download_rejects_corrupt_size_and_sha256() {
    let user_id = "desktop-media-user";
    let token = "desktop-media-token";
    let expected = b"expected";
    let corrupt = b"corrupt!";
    let expected_sha256 = hex_bytes(&Sha256::digest(expected));
    let base_item = MediaManifestItem {
        attachment_id: "corrupt-attachment".to_string(),
        sha256: expected_sha256,
        mime_type: "application/octet-stream".to_string(),
        size_bytes: corrupt.len() as i64,
        updated_at_epoch_millis: 100,
        deleted_at_epoch_millis: 0,
    };
    let (server_url, _captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            media_item: Some(base_item.clone()),
            media_content_base64: BASE64_STANDARD.encode(corrupt),
            ..SyncClientResult::default()
        },
    );
    let result = desktop_media_download(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
        "corrupt-attachment",
    );
    worker.join().unwrap();
    assert!(!result.ok);
    assert_eq!("Media download SHA-256 mismatch.", result.message);

    let size_item = MediaManifestItem {
        size_bytes: corrupt.len() as i64 + 1,
        sha256: hex_bytes(&Sha256::digest(corrupt)),
        ..base_item
    };
    let (server_url, _captured, worker) = one_shot_media_client_server(
        200,
        SyncClientResult {
            ok: true,
            user_id: user_id.to_string(),
            token_id: token_identifier(token),
            server_instance_id: DESKTOP_MEDIA_SERVER_INSTANCE_ID.to_string(),
            account_namespace: DESKTOP_MEDIA_ACCOUNT_NAMESPACE.to_string(),
            media_item: Some(size_item),
            media_content_base64: BASE64_STANDARD.encode(corrupt),
            ..SyncClientResult::default()
        },
    );
    let result = desktop_media_download(
        &server_url,
        user_id,
        token,
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
        "corrupt-attachment",
    );
    worker.join().unwrap();
    assert!(!result.ok);
    assert_eq!("Media download size mismatch.", result.message);
}

#[test]
fn desktop_media_rejects_dangerous_server_urls() {
    let invalid_server_identity = desktop_media_manifest(
        "http://127.0.0.1:1",
        "desktop-user",
        "desktop-token",
        "",
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
    );
    assert!(!invalid_server_identity.ok);
    assert_eq!(
        "Media serverInstanceId is invalid.",
        invalid_server_identity.message
    );
    let invalid_account_namespace = desktop_media_manifest(
        "http://127.0.0.1:1",
        "desktop-user",
        "desktop-token",
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        "not-a-namespace",
        0,
        "",
    );
    assert!(!invalid_account_namespace.ok);
    assert_eq!(
        "Media accountNamespace is invalid.",
        invalid_account_namespace.message
    );

    let insecure = desktop_media_manifest(
        "http://sync.example.com",
        "desktop-user",
        "desktop-token",
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
    );
    assert!(!insecure.ok);
    assert!(insecure.message.contains("require HTTPS"));

    let injected = desktop_media_manifest(
        "https://sync.example.com\r\nInjected: yes",
        "desktop-user",
        "desktop-token",
        DESKTOP_MEDIA_SERVER_INSTANCE_ID,
        DESKTOP_MEDIA_ACCOUNT_NAMESPACE,
        0,
        "",
    );
    assert!(!injected.ok);
    assert!(injected.message.contains("unsupported characters"));
}

#[test]
fn media_upload_download_and_attachment_id_conflict_are_enforced() {
    let token = "media-token";
    let store = account_test_store(token, String::new());
    let content = b"note attachment";
    let sha256 = hex_bytes(&Sha256::digest(content));
    let request = HttpRequest {
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        body: json!({
            "requestId": "media-request-1",
            "attachmentId": "attachment-1",
            "sha256": sha256.clone(),
            "mimeType": "text/plain",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": 100,
            "contentBase64": BASE64_STANDARD.encode(content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    assert_eq!(200, handle_media_upload(&request, &store.store).0);
    let download = HttpRequest {
        headers: request.headers.clone(),
        body: json!({"attachmentId": "attachment-1"}).to_string(),
        ..HttpRequest::default()
    };
    let (status, downloaded) = handle_media_download(&download, &store.store);
    assert_eq!(200, status);
    assert_eq!(
        content.as_slice(),
        BASE64_STANDARD
            .decode(downloaded.media_content_base64)
            .unwrap()
            .as_slice()
    );
    let deletion = HttpRequest {
        headers: request.headers.clone(),
        body: json!({
            "requestId": "media-delete-1",
            "attachmentId": "attachment-1",
            "deletedAtEpochMillis": 200
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, deleted) = handle_media_delete(&deletion, &store.store);
    assert_eq!(200, status);
    assert_eq!("media_delete", deleted.mode);
    assert_eq!(
        200,
        deleted.media_item.as_ref().unwrap().deleted_at_epoch_millis
    );
    assert_eq!(0, store.store.media_usage_bytes("user-1", false).unwrap());
    let (status, stale_upload) = handle_media_upload(&request, &store.store);
    assert_eq!(409, status);
    assert_eq!("media_deleted", stale_upload.mode);
    assert_eq!(
        200,
        stale_upload
            .media_item
            .as_ref()
            .unwrap()
            .deleted_at_epoch_millis
    );
    let fast_clock_upload = HttpRequest {
        headers: request.headers.clone(),
        body: json!({
            "requestId": "media-fast-clock-stale",
            "attachmentId": "attachment-1",
            "sha256": sha256.clone(),
            "mimeType": "text/plain",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": 9_999_999_999_i64,
            "contentBase64": BASE64_STANDARD.encode(content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (status, fast_clock_stale) = handle_media_upload(&fast_clock_upload, &store.store);
    assert_eq!(409, status);
    assert_eq!("media_deleted", fast_clock_stale.mode);
    assert_eq!(
        200,
        fast_clock_stale
            .media_item
            .as_ref()
            .unwrap()
            .deleted_at_epoch_millis
    );
    let manifest_request = HttpRequest {
        headers: request.headers.clone(),
        ..HttpRequest::default()
    };
    let (status, manifest) = handle_media_manifest(&manifest_request, &store.store);
    assert_eq!(200, status);
    assert_eq!(1, manifest.media_items.len());
    assert_eq!(200, manifest.media_items[0].deleted_at_epoch_millis);

    let restore = HttpRequest {
        headers: request.headers.clone(),
        body: json!({
            "requestId": "media-restore-1",
            "attachmentId": "attachment-1",
            "sha256": sha256,
            "mimeType": "text/plain",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": 201,
            "restoreDeleted": true,
            "contentBase64": BASE64_STANDARD.encode(content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    assert_eq!(200, handle_media_upload(&restore, &store.store).0);
    assert_eq!(
        content.len() as i64,
        store.store.media_usage_bytes("user-1", false).unwrap()
    );
    let conflict_content = b"different";
    let conflict = HttpRequest {
        headers: request.headers,
        body: json!({
            "requestId": "media-request-2",
            "attachmentId": "attachment-1",
            "sha256": hex_bytes(&Sha256::digest(conflict_content)),
            "mimeType": "text/plain",
            "sizeBytes": conflict_content.len(),
            "updatedAtEpochMillis": 202,
            "contentBase64": BASE64_STANDARD.encode(conflict_content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    assert_eq!(409, handle_media_upload(&conflict, &store.store).0);
}

#[test]
fn media_manifest_and_download_fail_closed_across_restore_generations() {
    let token = "media-restore-barrier-token";
    let store = account_test_store(token, note_app_data("initial", 100));
    store
        .store
        .compare_and_swap_account("user-1", 0, &note_app_data("restore-target", 200), 200)
        .unwrap();
    store
        .store
        .compare_and_swap_account("user-1", 1, &note_app_data("discarded", 300), 300)
        .unwrap();
    let headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
    let content = b"restore barrier media";
    let upload = HttpRequest {
        headers: headers.clone(),
        body: json!({
            "requestId": "media-restore-barrier-upload",
            "acknowledgedGeneration": 0,
            "attachmentId": "restore-barrier-attachment",
            "sha256": hex_bytes(&Sha256::digest(content)),
            "mimeType": "application/octet-stream",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": 400,
            "contentBase64": BASE64_STANDARD.encode(content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    assert_eq!(200, handle_media_upload(&upload, &store.store).0);

    let manifest_generation_zero = HttpRequest {
        headers: headers.clone(),
        body: json!({"acknowledgedGeneration": 0}).to_string(),
        ..HttpRequest::default()
    };
    let (manifest_status, manifest) =
        handle_media_manifest(&manifest_generation_zero, &store.store);
    assert_eq!(200, manifest_status);
    assert_eq!(0, manifest.current_generation);
    assert_eq!(1, manifest.media_items.len());

    store
        .store
        .restore_account_snapshot("user-1", 1, 2, 500)
        .unwrap();
    let (stale_manifest_status, stale_manifest) =
        handle_media_manifest(&manifest_generation_zero, &store.store);
    assert_eq!(409, stale_manifest_status);
    assert_eq!("restore_required", stale_manifest.mode);
    assert_eq!(1, stale_manifest.current_generation);
    assert!(!stale_manifest.restore_receipt.is_empty());

    let stale_download = HttpRequest {
        headers: headers.clone(),
        body: json!({
            "acknowledgedGeneration": 0,
            "attachmentId": "restore-barrier-attachment"
        })
        .to_string(),
        ..HttpRequest::default()
    };
    let (download_status, blocked_download) = handle_media_download(&stale_download, &store.store);
    assert_eq!(409, download_status);
    assert_eq!("restore_required", blocked_download.mode);
    assert_eq!(1, blocked_download.current_generation);
    assert!(blocked_download.media_content_base64.is_empty());

    let ahead_manifest = HttpRequest {
        headers,
        body: json!({"acknowledgedGeneration": 2}).to_string(),
        ..HttpRequest::default()
    };
    let (ahead_status, ahead) = handle_media_manifest(&ahead_manifest, &store.store);
    assert_eq!(409, ahead_status);
    assert_eq!("server_generation_rollback", ahead.mode);
    assert_eq!(1, ahead.current_generation);
}

#[test]
fn verified_runtime_backup_precedes_deleted_media_gc_and_gc_is_idempotent() {
    let token = "runtime-media-gc-token";
    let store = account_test_store(token, app_data::default_app_data_json(1));
    let headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
    let attachment_id = "runtime-gc-attachment";
    let content = b"retained until verified backup";
    let upload = HttpRequest {
        headers: headers.clone(),
        body: json!({
            "requestId": "runtime-gc-upload",
            "attachmentId": attachment_id,
            "sha256": hex_bytes(&Sha256::digest(content)),
            "mimeType": "application/octet-stream",
            "sizeBytes": content.len(),
            "updatedAtEpochMillis": 100,
            "contentBase64": BASE64_STANDARD.encode(content)
        })
        .to_string(),
        ..HttpRequest::default()
    };
    assert_eq!(200, handle_media_upload(&upload, &store.store).0);
    let deletion = HttpRequest {
        headers,
        body: json!({
            "requestId": "runtime-gc-delete",
            "attachmentId": attachment_id,
            "deletedAtEpochMillis": 200
        })
        .to_string(),
        ..HttpRequest::default()
    };
    assert_eq!(200, handle_media_delete(&deletion, &store.store).0);
    let live_database = rusqlite::Connection::open(store.store.database_path()).unwrap();
    let retained_bytes: i64 = live_database
        .query_row(
            "SELECT length(content) FROM note_media WHERE user_id = ?1 AND attachment_id = ?2",
            rusqlite::params!["user-1", attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(content.len() as i64, retained_bytes);

    let recovery_directory = store.directory.join("runtime-media-gc-backups");
    let report = perform_runtime_backup(&store.store, &recovery_directory, 1_000).unwrap();
    let backup = rusqlite::Connection::open(&report.destination).unwrap();
    let backed_up_bytes: i64 = backup
        .query_row(
            "SELECT length(content) FROM note_media WHERE user_id = ?1 AND attachment_id = ?2",
            rusqlite::params!["user-1", attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(content.len() as i64, backed_up_bytes);
    drop(backup);

    assert_eq!(
        1,
        run_post_backup_maintenance(&store.store, i64::MAX / 4).unwrap()
    );
    let remaining_media_rows: i64 = live_database
        .query_row(
            "SELECT COUNT(*) FROM note_media WHERE user_id = ?1 AND attachment_id = ?2",
            rusqlite::params!["user-1", attachment_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(0, remaining_media_rows);
    let tombstone_revision: i64 = live_database
            .query_row(
                "SELECT deleted_revision_epoch_millis FROM note_media_tombstones WHERE user_id = ?1 AND attachment_id = ?2",
                rusqlite::params!["user-1", attachment_id],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(200, tombstone_revision);
    let manifest = store.store.list_media_metadata("user-1", true).unwrap();
    assert_eq!(1, manifest.len());
    assert_eq!(Some(200), manifest[0].deleted_at_epoch_millis);
    assert_eq!(
        0,
        run_post_backup_maintenance(&store.store, i64::MAX / 4).unwrap()
    );
}

#[test]
fn https_post_transport_does_not_follow_redirects() {
    let redirect_target = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    redirect_target.set_nonblocking(true).unwrap();
    let redirect_target_address = redirect_target.local_addr().unwrap();
    let redirected_posts = Arc::new(AtomicUsize::new(0));
    let redirected_posts_worker = Arc::clone(&redirected_posts);
    let target_worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match redirect_target.accept() {
                Ok((mut stream, _)) => {
                    redirected_posts_worker.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    let _ = read_http_request(&mut stream);
                    let body = "{\"ok\":true,\"message\":\"redirected\"}";
                    let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                    let _ = stream.flush();
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    let source = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let source_address = source.local_addr().unwrap();
    let source_worker = thread::spawn(move || {
        let (mut stream, _) = source.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        read_http_request(&mut stream).unwrap();
        let body = "{\"ok\":false,\"message\":\"redirect refused\"}";
        write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://{redirect_target_address}/capture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        stream.flush().unwrap();
    });
    let base = ParsedBaseUrl {
        scheme: SyncUrlScheme::Http,
        host: Ipv4Addr::LOCALHOST.to_string(),
        port: source_address.port(),
        base_path: String::new(),
    };
    let result = send_json_to_ureq_base(
        &base,
        "/v1/login",
        Some("secret-bearer"),
        "{\"password\":\"secret-password\"}",
    )
    .expect("302 response should be returned without redirecting");
    source_worker.join().unwrap();
    target_worker.join().unwrap();
    assert!(!result.ok);
    assert_eq!(0, redirected_posts.load(Ordering::SeqCst));
}

#[test]
fn discovery_hmac_matches_standard_sha256_vector() {
    assert_eq!(
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",
        hmac_sha256_hex(b"key", b"The quick brown fox jumps over the lazy dog")
    );
}

#[test]
fn global_media_and_registration_capacity_errors_have_stable_507_responses() {
    for error in [
        StoreError::MediaServerRetainedQuotaExceeded {
            usage_bytes: 10,
            projected_bytes: 11,
            limit_bytes: 10,
        },
        StoreError::MediaServerIdentityQuotaExceeded {
            usage_items: 10,
            projected_items: 11,
            limit_items: 10,
        },
        StoreError::DiskReserveExceeded {
            available_bytes: 9,
            required_bytes: 10,
        },
        StoreError::Io(std::io::Error::from_raw_os_error(112)),
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            Some("database or disk is full".to_string()),
        )),
    ] {
        let (status, result) = store_failure("capacity test", error);
        assert_eq!(507, status);
        assert!(!result.ok);
        assert_eq!("media_server_capacity_exceeded", result.mode);
        assert!(!result.retryable);
        assert!(!result.current_committed);
        assert!(!result.message.contains("account quota"));
    }

    let (status, result) = store_failure(
        "non-capacity I/O test",
        StoreError::Io(std::io::Error::from_raw_os_error(5)),
    );
    assert_eq!(500, status);
    assert_eq!("", result.mode);

    let (status, result) = store_failure(
        "account identity test",
        StoreError::MediaAccountIdentityQuotaExceeded {
            usage_items: 10,
            projected_items: 11,
            limit_items: 10,
        },
    );
    assert_eq!(507, status);
    assert_eq!("media_account_quota_exceeded", result.mode);
    assert!(!result.ok);
    assert!(!result.retryable);
    assert!(!result.current_committed);

    let (status, result) = store_failure(
        "registration test",
        StoreError::RegisteredAccountQuotaExceeded {
            current_accounts: 16,
            limit_accounts: 16,
        },
    );
    assert_eq!(507, status);
    assert_eq!("registration_capacity_exceeded", result.mode);
    assert!(!result.ok);
    assert!(!result.retryable);
    assert!(!result.current_committed);
}
