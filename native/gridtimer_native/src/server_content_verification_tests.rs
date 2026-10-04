// v0.0.1 - Guard compressed corruption, schema domains, privacy changes and envelope checks.
use super::*;

fn row(raw: &str) -> SnapshotContentRow {
    let content = compress_snapshot_content(raw.as_bytes()).unwrap();
    SnapshotContentRow {
        user_id: "content-proof-test-user".into(),
        revision: 7,
        created_at_epoch_millis: 100,
        sha256: sha256_hex(raw.as_bytes()),
        compression: SNAPSHOT_CONTENT_COMPRESSION.into(),
        uncompressed_size_bytes: raw.len() as i64,
        compressed_size_bytes: content.len() as i64,
        content,
    }
}

fn validations() -> usize {
    VALIDATIONS.with(|count| count.get())
}

#[test]
fn content_proof_reuses_success_but_checks_every_byte_and_size_field() {
    let raw = r#"{"contentProof":"compression-boundary"}"#;
    let initial = row(raw);
    let before = validations();
    verify_compressed_schema(&initial).unwrap();
    assert_eq!(validations(), before + 1);
    verify_compressed_schema(&initial).unwrap();
    assert_eq!(validations(), before + 1);
    for change in 0..5 {
        let mut changed = row(raw);
        match change {
            0 => *changed.content.last_mut().unwrap() ^= 1,
            1 => changed.compressed_size_bytes += 1,
            2 => changed.uncompressed_size_bytes += 1,
            3 => changed.compression = "unsupported".into(),
            4 => changed.sha256.replace_range(
                ..1,
                if &changed.sha256[..1] == "0" {
                    "1"
                } else {
                    "0"
                },
            ),
            _ => unreachable!(),
        }
        let before = validations();
        for _ in 0..2 {
            assert!(
                verify_compressed_schema(&changed).is_err(),
                "accepted change {change}"
            );
        }
        assert_eq!(
            validations(),
            before + 2,
            "a failed validation became a proof"
        );
    }
}

#[test]
fn content_proof_structure_and_empty_privacy_never_authorize_future_schema() {
    let future = serde_json::json!({"schemaVersion": APP_DATA_SCHEMA_VERSION + 1,
        "contentProof": "future-schema-domain"})
    .to_string();
    verify_structure(&future).unwrap();
    verify_private_history(&DesktopPrivacyPolicy::default(), row(&future)).unwrap();
    for _ in 0..2 {
        assert!(verify_compressed_schema(&row(&future)).is_err());
    }
    for invalid in ["[1,2]", "{broken", "{\"value\":1e999}"] {
        assert!(verify_structure(invalid).is_err());
        assert!(verify_compressed_schema(&row(invalid)).is_err());
    }
}

#[test]
fn content_proof_never_replaces_current_account_or_history_hash_checks() {
    let raw = r#"{"contentProof":"current-envelope"}"#;
    let mut snapshot = AccountSnapshot {
        user_id: "proof-owner".into(),
        app_data_json: raw.into(),
        revision: 8,
        updated_at_epoch_millis: 100,
    };
    let content = sha256_hex(raw.as_bytes());
    let envelope = account_snapshot_envelope_sha256(&snapshot.user_id, raw, 8, 100, 2);
    verify_current_account_snapshot(&snapshot, 2, &content, &envelope).unwrap();
    verify_current_account_snapshot(&snapshot, 2, &content, &envelope).unwrap();
    snapshot.user_id = "another-owner".into();
    assert!(verify_current_account_snapshot(&snapshot, 2, &content, &envelope).is_err());
    snapshot.user_id = "proof-owner".into();
    assert!(verify_current_account_snapshot(&snapshot, 3, &content, &envelope).is_err());
    snapshot.revision += 1;
    assert!(verify_current_account_snapshot(&snapshot, 2, &content, &envelope).is_err());
    snapshot.revision -= 1;
    snapshot.app_data_json = raw.replace("current-envelope", "changed-envelope");
    assert!(verify_current_account_snapshot(&snapshot, 2, &content, &envelope).is_err());
    let history = AccountSnapshotHistory {
        user_id: snapshot.user_id,
        revision: 8,
        app_data_json: snapshot.app_data_json,
        created_at_epoch_millis: 100,
        sha256: content,
    };
    assert!(verify_snapshot_history_entry(&history).is_err());
}

fn note_body(id: &str) -> String {
    crate::app_data::upsert_note_app_data_json(
        &crate::app_data::default_app_data_json(100),
        &serde_json::json!({"id":id,"title":"Proof fixture","content":"private test body",
            "updatedAtEpochMillis":100})
        .to_string(),
        100,
    )
    .unwrap()
}

fn deletion_policy(raw: &str, id: &str) -> DesktopPrivacyPolicy {
    let deleted = crate::app_data::delete_note_permanently_app_data_json(raw, id, 200).unwrap();
    DesktopPrivacyPolicy::default()
        .including_snapshot(&deleted)
        .unwrap()
}

#[test]
fn content_proof_private_history_rechecks_policy_body_and_compressed_corruption() {
    let raw = note_body("proof-private-note");
    let other = note_body("proof-other-note");
    let unrelated = deletion_policy(&other, "proof-other-note");
    let changed = deletion_policy(&raw, "proof-private-note");
    verify_compressed_schema(&row(&raw)).unwrap();
    verify_private_history(&unrelated, row(&raw)).unwrap();
    let before = validations();
    verify_private_history(&unrelated, row(&raw)).unwrap();
    assert_eq!(validations(), before);
    for _ in 0..2 {
        assert!(verify_private_history(&changed, row(&raw)).is_err());
        assert!(verify_private_history(&unrelated, row(&other)).is_err());
    }
    let mut corrupt = row(&raw);
    *corrupt.content.last_mut().unwrap() ^= 1;
    assert!(verify_private_history(&unrelated, corrupt).is_err());
    let mut malformed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    malformed["notes"][0]["encryption"] = serde_json::json!({"cipherSuite":"unknown"});
    assert!(verify_private_history(&unrelated, row(&malformed.to_string())).is_err());
}

#[test]
fn content_proof_memory_stays_bounded_after_more_distinct_documents() {
    let mut proofs = Proofs::default();
    for index in 0..MAX_ENTRIES + 50 {
        proofs.remember(Sha256::digest(index.to_le_bytes()).into());
    }
    assert_eq!(proofs.0.len(), MAX_ENTRIES);
    assert!(!proofs
        .0
        .contains(&Sha256::digest(0usize.to_le_bytes()).into()));
}
