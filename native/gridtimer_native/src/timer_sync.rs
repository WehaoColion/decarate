// v2.22.49 - Keep live timer controls and reminder phases on their own device.
//! Only call the receive projection after the caller has checked the account
//! and workspace identity. Pass the latest durable local state at commit time,
//! rather than the older snapshot captured when the request was sent.
use crate::app_data;
use serde_json::{json, Value};
use std::collections::HashMap;

fn runtime_defaults() -> [(&'static str, Value); 7] {
    [
        ("runningSinceEpochMillis", Value::Null),
        ("activeRunId", json!("")),
        ("runningUpdatedAtEpochMillis", json!(0)),
        ("microBreakPhase", json!("FOCUS")),
        ("microBreakCycleIndex", json!(0)),
        ("microBreakPhaseProgressMillis", json!(0)),
        ("microBreakUpdatedAtEpochMillis", json!(0)),
    ]
}

/// Treat an incoming snapshot as saved work, not a command to run this device.
/// Do not advance timers here: transport must not create sessions or ring bells.
pub fn shared_snapshot_json(raw: &str, now: i64) -> Option<String> {
    let sanitized = app_data::sanitize_app_data_json(raw, now)?;
    let mut value: Value = serde_json::from_str(&sanitized).ok()?;
    for slot in value.get_mut("slots")?.as_array_mut()? {
        for (field, default) in runtime_defaults() {
            slot.as_object_mut()?.insert(field.to_owned(), default);
        }
    }
    serde_json::to_string(&value).ok()
}

/// Keep this device's start/stop decision and its reminder countdown together.
/// Completed records, saved totals, titles, notes, and other shared fields keep
/// the incoming merge/download result. New slots and first logins stay idle.
pub fn localize_snapshot_json(incoming: &str, local: &str, now: i64) -> Option<String> {
    let incoming = shared_snapshot_json(incoming, now)?;
    let local = app_data::sanitize_app_data_json(local, now)?;
    let mut next: Value = serde_json::from_str(&incoming).ok()?;
    let previous: Value = serde_json::from_str(&local).ok()?;
    let previous_slots: HashMap<i64, &Value> = previous
        .get("slots")?
        .as_array()?
        .iter()
        .filter_map(|slot| slot.get("id")?.as_i64().map(|id| (id, slot)))
        .collect();
    for slot in next.get_mut("slots")?.as_array_mut()? {
        let own = slot
            .get("id")
            .and_then(Value::as_i64)
            .and_then(|id| previous_slots.get(&id).copied());
        for (field, default) in runtime_defaults() {
            let value = own
                .and_then(|old| old.get(field))
                .cloned()
                .unwrap_or(default);
            slot.as_object_mut()?.insert(field.to_owned(), value);
        }
    }
    serde_json::to_string(&next).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_core;

    fn snapshot(running: bool, revision: i64, run: &str) -> String {
        json!({"schemaVersion":15,"slots":[{
            "id":1,"title":"学习","note":"共享备注","accumulatedMillis":5000,
            "runningSinceEpochMillis":if running { json!(1000) } else { Value::Null },
            "activeRunId":run,"runningUpdatedAtEpochMillis":revision,
            "microBreakPhase":"FOCUS","microBreakCycleIndex":2,
            "microBreakPhaseProgressMillis":3000,"microBreakUpdatedAtEpochMillis":revision,
            "updatedAt":revision
        }]})
        .to_string()
    }

    #[test]
    fn device_timer_sync_never_starts_or_stops_the_receiving_device() {
        for local_running in [false, true] {
            for remote_running in [false, true] {
                let local = snapshot(local_running, 100, "local-run");
                let mut remote: Value =
                    serde_json::from_str(&snapshot(remote_running, 9000, "remote-run")).unwrap();
                remote["slots"][0]["title"] = json!("新名称");
                remote["slots"][0]["accumulatedMillis"] = json!(9000);
                remote["slots"][0]["microBreakPhase"] = json!("BREAK");
                remote["slots"][0]["microBreakCycleIndex"] = json!(8);
                let merged =
                    sync_core::merge_sync_app_data_json(&remote.to_string(), &local, 10000)
                        .unwrap();
                let applied = localize_snapshot_json(&merged, &local, 10000).unwrap();
                let result: Value = serde_json::from_str(&applied).unwrap();
                let local_value: Value =
                    serde_json::from_str(&app_data::sanitize_app_data_json(&local, 10000).unwrap())
                        .unwrap();
                for (field, _) in runtime_defaults() {
                    assert_eq!(
                        result["slots"][0][field], local_value["slots"][0][field],
                        "{field}"
                    );
                }
                assert_eq!(result["slots"][0]["title"], "新名称");
                assert_eq!(result["slots"][0]["accumulatedMillis"], 9000);
                assert_eq!(
                    result["slots"][0]["runningSinceEpochMillis"].is_number(),
                    local_running
                );
                assert_eq!(
                    applied,
                    localize_snapshot_json(&applied, &local, 10000).unwrap()
                );
            }
        }
    }

    #[test]
    fn device_timer_sync_incoming_snapshot_and_first_login_have_no_active_timer_or_reminder() {
        let running = snapshot(true, 9000, "private-run");
        let shared: Value =
            serde_json::from_str(&shared_snapshot_json(&running, 10000).unwrap()).unwrap();
        for (field, default) in runtime_defaults() {
            assert_eq!(shared["slots"][0][field], default, "{field}");
        }
        assert_eq!(shared["slots"][0]["accumulatedMillis"], 5000);
        assert_eq!(shared["slots"][0]["note"], "共享备注");
        let first: Value =
            serde_json::from_str(&localize_snapshot_json(&running, "{}", 10000).unwrap()).unwrap();
        assert!(first["slots"][0]["runningSinceEpochMillis"].is_null());
        assert_eq!(first["slots"][0]["activeRunId"], "");
        assert_eq!(first["slots"][0]["microBreakCycleIndex"], 0);
    }

    #[test]
    fn device_timer_sync_uses_latest_local_pause_and_keeps_both_devices_records() {
        let seed = snapshot(false, 100, "");
        let phone = app_data::start_slot_app_data_json(&seed, 1, 1000).unwrap();
        let computer = app_data::start_slot_app_data_json(&seed, 1, 1000).unwrap();
        let phone_finished = app_data::pause_slots_app_data_json(&phone, &[1], 2000).unwrap();
        let computer_finished = app_data::pause_slots_app_data_json(&computer, &[1], 3000).unwrap();
        let sent = shared_snapshot_json(&phone_finished, 10000).unwrap();
        let merged = sync_core::merge_sync_app_data_json(&sent, &computer_finished, 10000).unwrap();
        let applied: Value = serde_json::from_str(
            &localize_snapshot_json(&merged, &computer_finished, 10000).unwrap(),
        )
        .unwrap();
        let sessions = applied["sessions"].as_array().unwrap();
        assert_eq!(
            sessions.len(),
            2,
            "Simultaneous starts on different devices need distinct records"
        );
        assert_ne!(sessions[0]["id"], sessions[1]["id"]);
        assert!(applied["slots"][0]["runningSinceEpochMillis"].is_null());
        assert_eq!(
            sessions
                .iter()
                .map(|s| s["durationMillis"].as_i64().unwrap())
                .sum::<i64>(),
            3000
        );
        let while_request_in_flight =
            localize_snapshot_json(&phone, &computer_finished, 10000).unwrap();
        let latest: Value = serde_json::from_str(&while_request_in_flight).unwrap();
        assert!(latest["slots"][0]["runningSinceEpochMillis"].is_null());
    }
}
