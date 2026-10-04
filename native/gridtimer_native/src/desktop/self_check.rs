// v2.22.39 - Check Chinese notes, rich text and finance through shared mutations and durable reloads.

use gridtimer_native as finance_profile;
use gridtimer_native::app_data;
use gridtimer_native::desktop_background_jobs::{
    BackgroundJobKind, BackgroundJobState, DesktopBackgroundJobStore,
};
use gridtimer_native::desktop_state_store::DesktopStateStore;
use gridtimer_native::product_identity::PRODUCT;
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};

const SELF_CHECK_TIME: i64 = 1_700_000_000_000;
const SELF_CHECK_OWNER: &str = "isolated-self-check";

/// Only creates the explicitly supplied directory. The parent must exist;
/// no default profile, network, audio or GUI is accessed.
pub(crate) fn run_desktop_self_check(target_dir: &Path) -> Result<Value, String> {
    // Exclusive creation also rejects files, existing directories and links.
    fs::create_dir(target_dir)
        .map_err(|error| format!("Self-check requires a new directory: {error}"))?;
    let target_dir = fs::canonicalize(target_dir).map_err(|error| error.to_string())?;
    let started_at = std::time::Instant::now();
    let state_path = target_dir.join("timer_state.json");
    let initial = app_data::update_slot_title_app_data_json(
        &app_data::default_app_data_json(SELF_CHECK_TIME),
        1,
        "Windows 本地自检",
        SELF_CHECK_TIME,
    )
    .ok_or("Shared timer title update failed")?;
    let running = app_data::start_slot_app_data_json(&initial, 1, SELF_CHECK_TIME + 1)
        .ok_or("Shared timer start failed")?;
    let projection = app_data::project_timer_views(&running, SELF_CHECK_TIME + 2_001)
        .map_err(|error| error.to_string())?;
    require(
        projection
            .slots
            .iter()
            .any(|slot| slot.id == 1 && slot.is_running && slot.accumulated_millis == 2_000),
        "Running timer did not advance by exactly two seconds",
    )?;
    let paused = app_data::pause_slots_app_data_json(&running, &[1], SELF_CHECK_TIME + 2_001)
        .ok_or("Shared timer pause failed")?;
    super::atomic_save_text(&state_path, &initial, validate_state)
        .map_err(|error| format!("Initial atomic save failed: {error}"))?;
    super::atomic_save_text(&state_path, &paused, validate_state)
        .map_err(|error| format!("Paused atomic save failed: {error}"))?;
    let reloaded = fs::read_to_string(&state_path).map_err(|error| error.to_string())?;
    require(reloaded == paused, "Saved timer bytes did not round trip")?;
    require(
        fs::read_to_string(target_dir.join("timer_state.json.bak"))
            .map_err(|error| error.to_string())?
            == initial,
        "Atomic replacement did not preserve the previous valid state",
    )?;
    let final_projection = app_data::project_timer_views(&reloaded, SELF_CHECK_TIME + 30_001)
        .map_err(|error| error.to_string())?;
    require(
        final_projection
            .slots
            .iter()
            .any(|slot| slot.id == 1 && !slot.is_running && slot.accumulated_millis == 2_000),
        "Paused timer changed after being reloaded",
    )?;
    let paused_value: Value = serde_json::from_str(&paused).map_err(|error| error.to_string())?;
    let sessions = paused_value["sessions"]
        .as_array()
        .ok_or("Timer history is missing")?;
    require(
        sessions.len() == 1 && sessions[0]["durationMillis"] == 2_000,
        "Pause did not record exactly one two-second history entry",
    )?;

    let journal_path = target_dir.join("desktop_state_history.sqlite3");
    let journal = DesktopStateStore::open(&journal_path).map_err(|error| error.to_string())?;
    journal
        .record(
            SELF_CHECK_OWNER,
            &initial,
            SELF_CHECK_TIME,
            "self_check_initial",
        )
        .map_err(|error| error.to_string())?;
    let valid = journal
        .record_with_sync_state(
            SELF_CHECK_OWNER,
            &paused,
            b"synthetic-protected-state",
            SELF_CHECK_TIME + 2_001,
            "self_check_paused",
        )
        .map_err(|error| error.to_string())?;
    let newer = app_data::update_slot_note_app_data_json(
        &paused,
        1,
        "synthetic-newer-snapshot",
        SELF_CHECK_TIME + 3_001,
    )
    .ok_or("Could not prepare journal recovery fixture")?;
    let damaged = journal
        .record(
            SELF_CHECK_OWNER,
            &newer,
            SELF_CHECK_TIME + 3_001,
            "self_check_corruption_fixture",
        )
        .map_err(|error| error.to_string())?;
    // Fault injection only touches this newly created synthetic database.
    // Whitespace keeps JSON valid while invalidating its stored exact-byte hash.
    rusqlite::Connection::open(&journal_path)
        .map_err(|error| error.to_string())?
        .execute(
            "UPDATE desktop_state_snapshots SET app_data_json = app_data_json || ' ' WHERE id = ?1",
            [damaged.id],
        )
        .map_err(|error| error.to_string())?;
    drop(journal);
    let recovered_journal =
        DesktopStateStore::open(&journal_path).map_err(|error| error.to_string())?;
    let recovered = recovered_journal
        .latest_valid(SELF_CHECK_OWNER, SELF_CHECK_TIME + 4_001)
        .map_err(|error| error.to_string())?
        .ok_or("No valid journal recovery snapshot")?;
    require(
        recovered == valid,
        "Journal recovery did not return the exact previous state pair",
    )?;
    require(
        recovered_journal
            .latest_valid("different-owner", SELF_CHECK_TIME + 4_001)
            .map_err(|error| error.to_string())?
            .is_none(),
        "Journal recovery crossed owner boundaries",
    )?;
    fs::write(&state_path, b"{broken-synthetic-primary").map_err(|error| error.to_string())?;
    super::atomic_save_text(&state_path, &recovered.app_data_json, validate_state)
        .map_err(|error| format!("Journal recovery mirror save failed: {error}"))?;
    require(
        fs::read_to_string(&state_path).map_err(|error| error.to_string())? == paused,
        "Recovered primary did not match the durable snapshot",
    )?;

    let jobs_path = target_dir.join("desktop_background_jobs.sqlite3");
    let jobs = DesktopBackgroundJobStore::open(&jobs_path).map_err(|error| error.to_string())?;
    let upload = jobs
        .stage_job(
            BackgroundJobKind::Upload,
            SELF_CHECK_OWNER,
            1,
            SELF_CHECK_TIME,
        )
        .map_err(|error| error.to_string())?;
    let download = jobs
        .stage_job(
            BackgroundJobKind::Download,
            SELF_CHECK_OWNER,
            2,
            SELF_CHECK_TIME,
        )
        .map_err(|error| error.to_string())?;
    jobs.mark_running(&upload.id, SELF_CHECK_TIME + 1)
        .map_err(|error| error.to_string())?;
    jobs.mark_running(&download.id, SELF_CHECK_TIME + 1)
        .map_err(|error| error.to_string())?;
    jobs.mark_completed(&upload.id, SELF_CHECK_TIME + 2)
        .map_err(|error| error.to_string())?;
    let completed = jobs
        .load(&upload.id)
        .map_err(|error| error.to_string())?
        .ok_or("Completed job vanished")?;
    require(
        completed.kind == BackgroundJobKind::Upload
            && completed.state == BackgroundJobState::Completed,
        "Completion receipt changed the wrong task",
    )?;
    jobs.mark_completed(&upload.id, SELF_CHECK_TIME - 1)
        .map_err(|error| error.to_string())?;
    require(
        jobs.load(&upload.id).map_err(|error| error.to_string())? == Some(completed.clone()),
        "Repeated completion changed durable receipt evidence",
    )?;
    require(
        jobs.mark_completed("nonexistent-self-check-job", SELF_CHECK_TIME)
            .is_err(),
        "A nonexistent task accepted a completion receipt",
    )?;
    drop(jobs);
    let reopened_jobs =
        DesktopBackgroundJobStore::open(&jobs_path).map_err(|error| error.to_string())?;
    let interrupted = reopened_jobs
        .recover_interrupted(SELF_CHECK_TIME + 3)
        .map_err(|error| error.to_string())?;
    require(
        interrupted.len() == 1
            && interrupted[0].id == download.id
            && interrupted[0].kind == BackgroundJobKind::Download
            && interrupted[0].state == BackgroundJobState::AwaitingReconciliation
            && interrupted[0].idempotency_key == download.idempotency_key,
        "Restart recovery did not isolate the unfinished download",
    )?;
    require(
        reopened_jobs
            .load(&upload.id)
            .map_err(|error| error.to_string())?
            == Some(completed),
        "Restart recovery altered an already-completed task",
    )?;

    run_notes_finance_roundtrip(&target_dir, &paused)?;

    let result = json!({
        "ok": true, "version": PRODUCT.app_version, "mode": "isolated-desktop-self-check",
        "dataDirectory": target_dir, "elapsedMillis": started_at.elapsed().as_millis() as u64,
        "checks": [
            {"name":"timer_atomic_save_roundtrip", "ok":true, "durationMillis":2000, "sessionCount":1},
            {"name":"journal_corruption_recovery", "ok":true, "recoveredSnapshotId":valid.id, "corruptSnapshotId":damaged.id, "ownerIsolation":true},
            {"name":"background_job_receipts", "ok":true, "completedUploads":1, "interruptedDownloads":1, "repeatReceiptPreserved":true},
            {"name":"notes_chinese_rich_text_roundtrip", "ok":true, "noteCount":2, "existingNoteUpdated":true, "richTextPreserved":true},
            {"name":"finance_cross_domain_roundtrip", "ok":true, "ledgerDays":2, "incomeTotal":145000, "outflowTotal":35000, "netWorth":220000, "timerAndNotesPreserved":true}
        ]
    });
    let report = serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?;
    let mut report_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_dir.join("self_check_result.json"))
        .map_err(|error| error.to_string())?;
    report_file
        .write_all(&report)
        .and_then(|()| report_file.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(result)
}

fn run_notes_finance_roundtrip(target_dir: &Path, timer_state: &str) -> Result<(), String> {
    const PLAIN_ID: &str = "self-check-chinese-note";
    const RICH_ID: &str = "self-check-rich-note";
    const PLAIN_BODY: &str = "第一行中文\n第二行  English 42";
    const UPDATED_BODY: &str = "保存后的中文修改\n双空格  和数字 123";
    const RICH_PLAIN: &str = "中文重点 & English 42\n第二行";
    const RICH_HTML: &str = "<p>中文<strong>重点</strong> &amp; English 42</p><p>第二行</p>";
    let baseline = parse_state(timer_state)?;
    let plain = json!({
        "id": PLAIN_ID, "title": "中文便签", "kind": "STICKY",
        "content": PLAIN_BODY,
        "document": {"blocks": [{"id":"plain-block", "type":"TEXT", "text":PLAIN_BODY}]},
        "createdAtEpochMillis": SELF_CHECK_TIME + 3_100
    });
    let with_plain = app_data::upsert_note_app_data_json(
        timer_state,
        &plain.to_string(),
        SELF_CHECK_TIME + 3_100,
    )
    .ok_or("Shared Chinese note creation failed")?;
    let rich = json!({
        "id": RICH_ID, "title": "富文本便签", "kind": "DOCUMENT",
        "content": RICH_PLAIN,
        "document": {"richTextEnabled":true, "richTextPlainText":RICH_PLAIN,
            "blocks":[{"id":"rich-block", "type":"TEXT", "text":RICH_HTML}]},
        "createdAtEpochMillis": SELF_CHECK_TIME + 3_200
    });
    let with_notes = app_data::upsert_note_app_data_json(
        &with_plain,
        &rich.to_string(),
        SELF_CHECK_TIME + 3_200,
    )
    .ok_or("Shared rich-text note creation failed")?;
    let state_path = target_dir.join("notes_finance_state.json");
    super::atomic_save_text(&state_path, &with_notes, validate_state)
        .map_err(|error| format!("Note save failed: {error}"))?;
    let saved_notes = fs::read_to_string(&state_path).map_err(|error| error.to_string())?;
    let notes_value = parse_state(&saved_notes)?;
    require(
        find_note(&notes_value, PLAIN_ID)?["content"] == PLAIN_BODY
            && find_note(&notes_value, RICH_ID)?["content"] == RICH_PLAIN
            && find_note(&notes_value, RICH_ID)?["document"]["richTextEnabled"] == true
            && find_note(&notes_value, RICH_ID)?["document"]["blocks"][0]["text"] == RICH_HTML,
        "Chinese text, line breaks or rich-text formatting changed after shared save and reload",
    )?;

    let day_one = json!({
        "incomes":[{"id":"salary", "name":"工资", "kind":"ACTIVE", "amount":120000}],
        "expenses":[{"id":"food", "name":"餐费", "bucket":"FOOD", "amount":30000}],
        "note":"第一天账目"
    });
    let day_two = json!({
        "incomes":[{"id":"bonus", "name":"奖金", "kind":"ACTIVE", "amount":25000}],
        "expenses":[{"id":"books", "name":"书籍", "bucket":"LEARNING", "amount":5000}],
        "note":"第二天账目"
    });
    let profile = finance_profile::upsert_finance_day_ledger_json(
        &notes_value["financeProfile"].to_string(),
        "2026-04-08",
        &day_one.to_string(),
    )
    .ok_or("Shared first finance ledger update failed")?;
    let profile = finance_profile::upsert_finance_day_ledger_json(
        &profile,
        "2026-04-09",
        &day_two.to_string(),
    )
    .ok_or("Shared second finance ledger update failed")?;
    let month = json!({
        "assets":[{"id":"cash", "name":"存款", "kind":"CASH_RESERVE", "amount":300000}],
        "liabilities":[{"id":"card", "name":"待还款", "kind":"LIABILITY_BALANCE", "amount":80000}]
    });
    let profile = finance_profile::upsert_finance_month_snapshot_json(
        &profile,
        "2026-04",
        &month.to_string(),
    )
    .ok_or("Shared finance month update failed")?;
    let financed = app_data::update_finance_profile_app_data_json(
        &saved_notes,
        &profile,
        SELF_CHECK_TIME + 3_300,
    )
    .ok_or("Shared app-data finance update failed")?;
    super::atomic_save_text(&state_path, &financed, validate_state)
        .map_err(|error| format!("Finance save failed: {error}"))?;
    let finance_reload = fs::read_to_string(&state_path).map_err(|error| error.to_string())?;
    let financed_value = parse_state(&finance_reload)?;
    require(
        financed_value["notes"] == notes_value["notes"]
            && financed_value["slots"] == baseline["slots"]
            && financed_value["sessions"] == baseline["sessions"],
        "Finance mutation or persistence lost notes, timers or timer history",
    )?;
    let snapshot = finance_profile::build_detailed_finance_snapshot_values(
        &financed_value["financeProfile"].to_string(),
        2026,
        4,
    )
    .ok_or("Reloaded finance snapshot could not be calculated")?;
    require(
        snapshot.total_income == 145000
            && snapshot.total_outflow == 35000
            && snapshot.net_cashflow == 110000
            && snapshot.net_worth == 220000,
        "Reloaded finance rows did not produce the expected cashflow and net worth",
    )?;
    require(
        financed_value["financeProfile"]["dailyLedgers"]["2026-04-08"]["incomes"][0]["name"]
            == "工资"
            && financed_value["financeProfile"]["dailyLedgers"]["2026-04-09"]["note"]
                == "第二天账目",
        "Finance upsert lost the earlier period or Chinese labels",
    )?;

    // Editing a reloaded note must update its existing identity and preserve
    // finance, the other note and the timer domain on the next durable save.
    let mut edited = find_note(&financed_value, PLAIN_ID)?.clone();
    edited["content"] = json!(UPDATED_BODY);
    edited["document"] =
        json!({"blocks":[{"id":"plain-block", "type":"TEXT", "text":UPDATED_BODY}]});
    let edited_state = app_data::upsert_note_app_data_json(
        &finance_reload,
        &edited.to_string(),
        SELF_CHECK_TIME + 3_400,
    )
    .ok_or("Shared existing-note edit failed")?;
    let edited_value = parse_state(&edited_state)?;
    require(
        edited_value["notes"]
            .as_array()
            .is_some_and(|notes| notes.len() == 2)
            && find_note(&edited_value, PLAIN_ID)?["content"] == UPDATED_BODY
            && find_note(&edited_value, RICH_ID)? == find_note(&financed_value, RICH_ID)?
            && edited_value["financeProfile"] == financed_value["financeProfile"]
            && edited_value["slots"] == baseline["slots"]
            && edited_value["sessions"] == baseline["sessions"],
        "Editing an existing Chinese note duplicated it or changed an unrelated domain",
    )?;
    let final_state = app_data::update_slot_title_app_data_json(
        &edited_state,
        1,
        "计时与便签财务同时保留",
        SELF_CHECK_TIME + 3_500,
    )
    .ok_or("Shared final timer update failed")?;
    super::atomic_save_text(&state_path, &final_state, validate_state)
        .map_err(|error| format!("Cross-domain save failed: {error}"))?;
    let final_reloaded = fs::read_to_string(&state_path).map_err(|error| error.to_string())?;
    let final_value = parse_state(&final_reloaded)?;
    require(
        final_value["notes"] == edited_value["notes"]
            && final_value["financeProfile"] == financed_value["financeProfile"]
            && final_value["sessions"] == baseline["sessions"],
        "Final timer save lost reloaded notes, finance or timer history",
    )?;
    let journal_path = target_dir.join("notes_finance_history.sqlite3");
    let journal = DesktopStateStore::open(&journal_path).map_err(|error| error.to_string())?;
    journal
        .record(
            SELF_CHECK_OWNER,
            &final_reloaded,
            SELF_CHECK_TIME + 3_500,
            "self_check_all_domains",
        )
        .map_err(|error| error.to_string())?;
    drop(journal);
    let reopened = DesktopStateStore::open(&journal_path).map_err(|error| error.to_string())?;
    let durable = reopened
        .latest_valid(SELF_CHECK_OWNER, SELF_CHECK_TIME + 4_001)
        .map_err(|error| error.to_string())?
        .ok_or("Cross-domain journal snapshot vanished")?;
    require(
        durable.app_data_json == final_reloaded,
        "Cross-domain journal reopen changed the saved application state",
    )
}

fn parse_state(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|error| error.to_string())
}

fn find_note<'a>(state: &'a Value, id: &str) -> Result<&'a Value, String> {
    state["notes"]
        .as_array()
        .and_then(|notes| notes.iter().find(|note| note["id"] == id))
        .ok_or_else(|| format!("Expected synthetic note is missing: {id}"))
}

fn validate_state(raw: &str) -> Option<String> {
    app_data::sanitize_app_data_json(raw, SELF_CHECK_TIME + 4_001)
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unused_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "gridtimer-self-check-{:032x}",
            rand::random::<u128>()
        ))
    }

    #[test]
    fn isolated_self_check_exercises_durable_paths_and_writes_a_versioned_report() {
        let target = unused_path();
        let result = run_desktop_self_check(&target).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["version"], PRODUCT.app_version);
        assert_eq!(result["checks"].as_array().unwrap().len(), 5);
        assert!(result["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["ok"] == true));
        let stored: Value =
            serde_json::from_slice(&fs::read(target.join("self_check_result.json")).unwrap())
                .unwrap();
        assert_eq!(stored, result);
        assert!(run_desktop_self_check(&target).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(
                &fs::read(target.join("self_check_result.json")).unwrap()
            )
            .unwrap(),
            result
        );
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn isolated_self_check_refuses_existing_files_and_directories_without_touching_them() {
        let target = unused_path();
        fs::create_dir(&target).unwrap();
        let marker = target.join("existing_user_data.txt");
        fs::write(&marker, b"preserve existing bytes").unwrap();
        assert!(run_desktop_self_check(&target).is_err());
        assert!(run_desktop_self_check(&marker).is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"preserve existing bytes");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn isolated_self_check_does_not_create_missing_parent_directories() {
        let parent = unused_path();
        assert!(run_desktop_self_check(&parent.join("nested")).is_err());
        assert!(!parent.exists());
    }
}
