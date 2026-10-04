// v0.0.2 - Verify legacy upgrades, contradictory hashes and private proof integrity.
// v0.0.1 - Guard private declarations, installed decoders and deletion authority.
use crate::sealed_media_references::SealedMediaReferences;
use crate::{app_data, legacy_note_crypto_fixture as legacy, note_crypto as crypto};
use serde_json::{json, Value};
const PASSWORD: &str = "reference-only-password";
fn value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}
fn refs(note: &str, token: &str) -> SealedMediaReferences {
    serde_json::from_str(&crypto::session_media_references_json(note, token).unwrap()).unwrap()
}
fn note(id: &str, attachment: &str) -> Value {
    let attachments = if attachment.is_empty() {
        json!([])
    } else {
        json!([{"id":attachment,"mimeType":"image/png","sha256":"a".repeat(64),"sizeBytes":1,"updatedAtEpochMillis":100}])
    };
    let data = app_data::upsert_note_app_data_json(&app_data::default_app_data_json(100),
        &json!({"id":id,"title":"Reference fixture","content":"Private test text","attachments":attachments,"updatedAtEpochMillis":100}).to_string(),100).unwrap();
    value(&data)["notes"][0].clone()
}

#[test]
fn sealed_media_reference_includes_current_versions_revisions_and_nested_documents() {
    let mut plain = note("ref-all-content", "current-file");
    plain["document"]["blocks"] = json!([{"attachmentId":"document-file"}]);
    plain["revisions"] = json!([{"id":"r-old","attachments":[{"id":"revision-file"}],"attachmentIds":["revision-id-only"],"document":{"nested":[{"attachmentId":"nested-file"}]}}]);
    let mut old = plain.clone();
    old["id"] = json!("v-old");
    old["attachments"] = json!([{"id":"version-file"}]);
    old["attachmentIds"] = json!(["version-id-only"]);
    old.as_object_mut().unwrap().remove("revisions");
    old.as_object_mut().unwrap().remove("versions");
    let mut current = old.clone();
    current["id"] = json!("v-current");
    plain["versions"] = json!([old, current]);
    plain["latestVersionId"] = json!("v-current");
    let (sealed, token) = crypto::encrypt_note(&plain.to_string(), PASSWORD).unwrap();
    let declaration = refs(&sealed, &token);
    crypto::close_session(&token);
    for id in [
        "current-file",
        "document-file",
        "revision-file",
        "revision-id-only",
        "nested-file",
        "version-file",
        "version-id-only",
    ] {
        assert!(
            declaration.ids().iter().any(|item| item == id),
            "missing {id}"
        );
    }
    let metadata = serde_json::to_string(&declaration).unwrap();
    for private in ["Private test text", "Reference fixture", "mimeType"] {
        assert!(!metadata.contains(private));
    }
    assert!(value(&sealed).get("sealedMediaReferences").is_none());
    assert!(crypto::session_media_references_json(&sealed, &token).is_none());
}

#[test]
fn sealed_media_reference_keeps_preexisting_crypto_compatible_and_resolves_legacy() {
    let plain = note("ref-legacy-compatible", "shared-file");
    let (sealed, token) = crypto::encrypt_note(&plain.to_string(), PASSWORD).unwrap();
    assert!(refs(&sealed, &token).ids().contains(&"shared-file".into()));
    crypto::close_session(&token);
    let (old_unlocked, old_token) = legacy::unlock_note(&sealed, PASSWORD).unwrap();
    assert_eq!(value(&old_unlocked)["content"], plain["content"]);
    legacy::close_session(&old_token);
    let (legacy_sealed, old_token) = legacy::encrypt_note(&plain.to_string(), PASSWORD).unwrap();
    legacy::close_session(&old_token);
    let envelope = value(&legacy_sealed)["encryption"].clone();
    let (unlocked, token) = crypto::unlock_note(&legacy_sealed, PASSWORD).unwrap();
    assert_eq!(value(&unlocked)["encryption"], envelope);
    assert!(refs(&unlocked, &token)
        .ids()
        .contains(&"shared-file".into()));
    let resealed = crypto::seal_note(&unlocked, &token).unwrap();
    crypto::close_session(&token);
    let (_, old_token) = legacy::unlock_note(&resealed, PASSWORD).unwrap();
    legacy::close_session(&old_token);
}

#[test]
fn sealed_media_reference_is_invalid_after_ciphertext_owner_or_password_changes() {
    let plain = note("ref-generations", "original-file");
    let (sealed, token) = crypto::encrypt_note(&plain.to_string(), PASSWORD).unwrap();
    let original = value(&sealed);
    let declaration = refs(&sealed, &token);
    assert!(!declaration.valid_for("another-note", &original["encryption"]));
    assert!(crypto::session_media_references_json(&sealed, "wrong-token").is_none());
    crypto::close_session(&token);
    let mut corrupted = serde_json::to_value(&declaration).unwrap();
    corrupted["attachmentIds"] = json!([]);
    assert!(!serde_json::from_value::<SealedMediaReferences>(corrupted)
        .unwrap()
        .valid_metadata());
    let (unlocked, token) = crypto::unlock_note(&sealed, PASSWORD).unwrap();
    let mut edit = value(&unlocked);
    edit["content"] = json!("New content");
    edit["attachments"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"new-file"}));
    let next = crypto::seal_note(&edit.to_string(), &token).unwrap();
    for field in ["keyId", "protectionRevision"] {
        assert_eq!(
            original["encryption"][field],
            value(&next)["encryption"][field]
        );
    }
    assert!(refs(&next, &token).ids().contains(&"new-file".into()));
    assert!(!declaration.valid_for("ref-generations", &value(&next)["encryption"]));
    assert!(crypto::session_media_references_json(&sealed, &token).is_none());
    crypto::close_session(&token);
    let (unlocked, token) = crypto::unlock_note(&next, PASSWORD).unwrap();
    let rotated = crypto::change_password(&unlocked, &token, "rotated-reference-password").unwrap();
    assert!(refs(&rotated, &token).ids().contains(&"new-file".into()));
    assert!(!declaration.valid_for("ref-generations", &value(&rotated)["encryption"]));
    assert!(crypto::commit_password_change(&rotated, &token));
    assert!(crypto::session_media_references_json(&rotated, &token).is_none());
    let (_, old_token) = legacy::unlock_note(&rotated, "rotated-reference-password").unwrap();
    legacy::close_session(&old_token);
}

#[test]
fn sealed_media_reference_aborted_password_candidate_loses_export_authority() {
    let (sealed, token) =
        crypto::encrypt_note(&note("ref-aborted", "abort-file").to_string(), PASSWORD).unwrap();
    crypto::close_session(&token);
    let (unlocked, token) = crypto::unlock_note(&sealed, PASSWORD).unwrap();
    let original = refs(&sealed, &token);
    let candidate = crypto::change_password(&unlocked, &token, "aborted-new-password").unwrap();
    assert!(refs(&candidate, &token)
        .ids()
        .contains(&"abort-file".into()));
    assert!(crypto::abort_password_change(&candidate, &token));
    assert!(crypto::session_media_references_json(&candidate, &token).is_none());
    assert_eq!(original, refs(&sealed, &token));
    crypto::close_session(&token);
}

#[test]
fn sealed_media_reference_private_policy_survives_serialization_and_rejects_wrong_binding() {
    use crate::desktop_state_store::DesktopPrivacyPolicy;
    let (sealed, token) = crypto::encrypt_note(
        &note("ref-private-policy", "private-file").to_string(),
        PASSWORD,
    )
    .unwrap();
    let declaration = refs(&sealed, &token);
    crypto::close_session(&token);
    let sealed = value(&sealed);
    let policy = DesktopPrivacyPolicy::default()
        .including_sealed_media(&sealed, declaration.clone())
        .unwrap();
    let restored: DesktopPrivacyPolicy =
        serde_json::from_str(&serde_json::to_string(&policy).unwrap()).unwrap();
    assert_eq!(
        Some(declaration.clone()),
        restored.media_references_for(&sealed)
    );
    let mut changed = sealed.clone();
    changed["id"] = json!("another-owner");
    assert!(restored.media_references_for(&changed).is_none());
    assert!(restored
        .including_sealed_media(&changed, declaration)
        .is_err());
    changed = sealed;
    changed["encryption"]["ciphertextBase64"] = json!("changed-ciphertext");
    assert!(restored.media_references_for(&changed).is_none());
}

#[test]
fn sealed_media_reference_distinguishes_empty_from_unrepresentable_or_future() {
    let (sealed, token) =
        crypto::encrypt_note(&note("ref-empty", "").to_string(), PASSWORD).unwrap();
    assert!(refs(&sealed, &token).ids().is_empty());
    let mut future = value(&sealed);
    future["encryption"]["formatVersion"] = json!(2);
    assert!(crypto::session_media_references_json(&future.to_string(), &token).is_none());
    crypto::close_session(&token);
    let mut malformed = note("ref-unknown", "normal-file");
    malformed["attachments"][0]["id"] = json!("unsupported/path");
    let (sealed, token) = crypto::encrypt_note(&malformed.to_string(), PASSWORD).unwrap();
    assert!(crypto::session_media_references_json(&sealed, &token).is_none());
    crypto::close_session(&token);
}

#[test]
fn sealed_media_reference_installed_schema_roundtrip_must_remain_compatible() {
    use crate::legacy_app_data_fixture::{self as legacy_schema, AppDataJsonCompatibility};
    let plain = note("ref-installed-schema", "legacy-readable-file");
    let data = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &plain.to_string(),
        100,
    )
    .unwrap();
    let (sealed, session) = crypto::encrypt_note(&plain.to_string(), PASSWORD).unwrap();
    let declaration = refs(&sealed, &session);
    crypto::close_session(&session);
    let current = app_data::upsert_note_app_data_json(&data, &sealed, 200).unwrap();
    let old_client = legacy_schema::app_data_json_compatibility(&current, 200);
    assert!(
        matches!(old_client, AppDataJsonCompatibility::CurrentKnown),
        "new sealed metadata breaks an installed client's shared-data decoder: {old_client:?}"
    );
    let old_roundtrip = legacy_schema::sanitize_app_data_json(&current, 200).unwrap();
    let returned = app_data::sanitize_app_data_json(&old_roundtrip, 200).unwrap();
    assert!(declaration.valid_for(
        "ref-installed-schema",
        &value(&returned)["notes"][0]["encryption"]
    ));
    assert_eq!(
        value(&current)["notes"][0]["encryption"],
        value(&returned)["notes"][0]["encryption"]
    );
}

#[test]
fn sealed_media_reference_shared_opaque_note_cannot_create_attachment_tombstone() {
    let first = note("first-owner", "shared-file");
    let mut other = first.clone();
    other["id"] = json!("sealed-owner");
    let only = app_data::upsert_note_app_data_json(
        &app_data::default_app_data_json(100),
        &first.to_string(),
        100,
    )
    .unwrap();
    let (sealed, token) = crypto::encrypt_note(&other.to_string(), PASSWORD).unwrap();
    crypto::close_session(&token);
    let data = app_data::upsert_note_app_data_json(&only, &sealed, 200).unwrap();
    let deleted =
        app_data::delete_note_permanently_app_data_json(&data, "first-owner", 300).unwrap();
    assert!(value(&deleted)["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["entityType"] != "noteMedia"));
    let deleted =
        app_data::delete_note_permanently_app_data_json(&only, "first-owner", 300).unwrap();
    assert!(value(&deleted)["tombstones"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["entityType"] == "noteMedia" && item["entityId"] == "shared-file"));
}

#[test]
fn sealed_media_reference_content_upgrade_preserves_hashes_and_rejects_contradictions() {
    use crate::desktop_state_store::DesktopPrivacyPolicy;
    use sha2::{Digest, Sha256};
    let plain = note("ref-content-upgrade", "immutable-file");
    let (sealed, token) = crypto::encrypt_note(&plain.to_string(), PASSWORD).unwrap();
    let strong = refs(&sealed, &token);
    crypto::close_session(&token);
    let sealed = value(&sealed);
    let mut old = serde_json::to_value(&strong).unwrap();
    old["formatVersion"] = json!(1);
    old.as_object_mut().unwrap().remove("contentSha256ById");
    old["bindingSha256"] = json!(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                "sealed-note-media-declaration-v1",
                strong.envelope_sha256(),
                strong.ids()
            ))
            .unwrap()
        )
    ));
    let legacy: SealedMediaReferences = serde_json::from_value(old).unwrap();
    assert!(legacy.valid_metadata());
    assert!(strong.covers(&legacy));
    assert!(!legacy.covers(&strong));
    let policy = DesktopPrivacyPolicy::default()
        .including_sealed_media(&sealed, legacy.clone())
        .unwrap()
        .including_sealed_media(&sealed, strong.clone())
        .unwrap();
    let roundtrip: DesktopPrivacyPolicy =
        serde_json::from_str(&serde_json::to_string(&policy).unwrap()).unwrap();
    assert_eq!(
        roundtrip.media_references_for(&sealed),
        Some(strong.clone())
    );
    assert_eq!(
        roundtrip.including_sealed_media(&sealed, legacy).unwrap(),
        roundtrip
    );
    let conflicting = SealedMediaReferences::from_payload(
        "ref-content-upgrade",
        &sealed["encryption"],
        &json!([{"id":"immutable-file", "sha256":"b".repeat(64)}]),
        &json!({}),
        &json!([]),
        &json!([]),
    )
    .unwrap();
    assert!(conflicting.valid_metadata());
    assert!(!strong.covers(&conflicting));
    assert!(roundtrip
        .including_sealed_media(&sealed, conflicting)
        .is_err());
    let mut tampered = serde_json::to_value(&strong).unwrap();
    tampered["contentSha256ById"]["immutable-file"] = json!("b".repeat(64));
    assert!(!serde_json::from_value::<SealedMediaReferences>(tampered)
        .unwrap()
        .valid_metadata());
}

#[test]
fn sealed_media_reference_conflicting_history_retains_ids_without_authorizing_bytes() {
    let (sealed, token) = crypto::encrypt_note(
        &note("ref-conflicting-history", "same-file").to_string(),
        PASSWORD,
    )
    .unwrap();
    crypto::close_session(&token);
    let sealed = value(&sealed);
    let declaration = SealedMediaReferences::from_payload(
        "ref-conflicting-history",
        &sealed["encryption"],
        &json!([{"id":"same-file", "sha256":"a".repeat(64)}]),
        &json!({}),
        &json!([{"attachments":[{"id":"same-file","sha256":"b".repeat(64)}]}]),
        &json!([]),
    )
    .unwrap();
    assert_eq!(declaration.ids(), &["same-file"]);
    assert!(declaration.content_hashes().is_empty());
    assert!(declaration.valid_metadata());
}
