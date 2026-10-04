// v0.0.1 - Replay attachment deletion on a backup copy and check offline recovery boundaries.
// This tool reads a local export only. It never connects to Android or writes app storage.
use gridtimer_native::{app_data, product_identity};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn decoded(raw: &str) -> Value {
    serde_json::from_str(raw).expect("valid JSON")
}

fn preserved(original: &Value, changed: &Value, qa_id: &str) {
    for (key, value) in original.as_object().unwrap() {
        if key != "notes" && key != "tombstones" {
            assert_eq!(value, &changed[key], "unrelated domain changed: {key}");
        }
    }
    let notes = changed["notes"].as_array().unwrap();
    for note in original["notes"].as_array().unwrap() {
        if note["id"] != qa_id {
            let after = notes.iter().find(|item| item["id"] == note["id"]);
            assert_eq!(Some(note), after, "an unrelated note changed");
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 3 {
        return Err("usage: tenfold_offline_acceptance <local-backup-json> <result-json>".into());
    }
    let raw = fs::read_to_string(&args[1])?;
    let input_hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
    let started = Instant::now();
    let normalized =
        app_data::sanitize_app_data_json(&raw, now).ok_or("backup cannot be decoded")?;
    let original = decoded(&normalized);
    let raw_value = decoded(&raw);
    let qa = original["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["title"] == "QA20260912K")
        .ok_or("named synthetic QA note is missing")?;
    let qa_id = qa["id"].as_str().unwrap();
    let ids: Vec<_> = qa["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        !ids.is_empty(),
        "backup must still contain the crash fixture's attachment"
    );
    let mut cases = Vec::new();
    let restored = app_data::restore_note_app_data_json(&normalized, qa_id, now + 1).unwrap();
    preserved(&original, &decoded(&restored), qa_id);
    cases.push("restore named QA note without altering other domains");
    let mut current = restored;
    for (i, id) in ids.iter().enumerate() {
        current =
            app_data::delete_note_attachment_app_data_json(&current, qa_id, id, now + 2 + i as i64)
                .ok_or("attachment deletion was rejected")?;
        preserved(&original, &decoded(&current), qa_id);
    }
    let after = decoded(&current);
    let note_after = after["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == qa_id)
        .unwrap();
    assert!(note_after["attachments"].as_array().unwrap().is_empty());
    cases.push("delete original crash-fixture image including its last attachment");
    let reopened = app_data::sanitize_app_data_json(&current, now + 10).unwrap();
    assert_eq!(
        after,
        decoded(&reopened),
        "serialization/reload changed the deletion result"
    );
    cases.push("round-trip the modified snapshot through the production loader");
    let deleted =
        app_data::delete_note_permanently_app_data_json(&normalized, qa_id, now + 20).unwrap();
    let deleted = decoded(&deleted);
    assert!(deleted["notes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["id"] != qa_id));
    preserved(&original, &deleted, qa_id);
    cases.push("permanently delete media QA on the local copy with other notes intact");
    let absent = app_data::delete_note_attachment_app_data_json(
        &normalized,
        qa_id,
        "missing-qa-attachment",
        now + 21,
    )
    .unwrap();
    assert_eq!(original, decoded(&absent));
    cases.push("deleting a missing attachment is a no-op");

    let profile = original["financeProfile"].to_string();
    for day in [
        "2026-02-30",
        "2026-02-29",
        "2026-04-31",
        "2026-13-01",
        "2026-00-01",
    ] {
        assert!(
            gridtimer_native::finance_day_ledger_or_default_json(&profile, day).is_none(),
            "accepted invalid calendar date"
        );
    }
    assert!(gridtimer_native::finance_day_ledger_or_default_json(&profile, "2028-02-29").is_some());
    cases.push("reject impossible dates and accept a leap-year date");
    let finance_backup = gridtimer_native::encode_finance_backup_json(
        &profile,
        product_identity::ANDROID_APP_VERSION,
        now,
    )
    .unwrap();
    let finance_restored =
        gridtimer_native::decode_finance_backup_profile_json(&finance_backup).unwrap();
    assert_eq!(decoded(&profile), decoded(&finance_restored));
    cases.push("round-trip every existing finance record through the backup decoder");
    let mut future = decoded(&finance_backup);
    future["schemaVersion"] = json!(999);
    assert!(gridtimer_native::decode_finance_backup_profile_json(&future.to_string()).is_none());
    assert!(gridtimer_native::decode_finance_backup_profile_json("{broken}").is_none());
    cases.push("reject future and malformed finance backup formats");

    let export = gridtimer_native::render_desktop_note_document_html(&json!({
        "title":"Offline checklist", "meta":"", "accent_seed":"amber", "markdown_enabled":true,
        "rich_text_enabled":false, "blocks":[{"type":"TEXT", "text":"- [x] 777\n- [ ] 777\n  - [x] 中文😀\n\n<script>window.test=1</script>"}]
    }).to_string()).unwrap();
    assert!(export.contains("☑ 777") && export.contains("☐ 777") && export.contains("☑ 中文😀"));
    assert!(!export.contains("<script") && !export.contains("window.test=1"));
    cases.push("render opposite checklist states through the shared Android HTML renderer");
    let original_note_count = original["notes"].as_array().unwrap().len() - 1;
    let result = json!({
        "version":"0.0.1", "target":product_identity::ANDROID_APP_VERSION,
        "input_sha256":input_hash, "input_unchanged":fs::read_to_string(&args[1])? == raw,
        "normalization_changes_input_representation":raw_value != original,
        "passed":true, "checks":cases, "check_count":cases.len(),
        "preserved_other_notes":original_note_count,
        "preserved_sessions":original["sessions"].as_array().unwrap().len(),
        "preserved_archives":original["archivedTasks"].as_array().unwrap().len(),
        "preserved_categories":original["categories"].as_array().unwrap().len(),
        "elapsed_ms":started.elapsed().as_millis(),
        "scope":"Host execution of shared production Rust functions on a local copy; no Android device, process, filesystem, JNI runtime, or UI validation."
    });
    fs::write(&args[2], serde_json::to_vec_pretty(&result)?)?;
    println!(
        "{} offline checks passed; {} unrelated notes preserved; input unchanged",
        cases.len(),
        original_note_count
    );
    Ok(())
}
