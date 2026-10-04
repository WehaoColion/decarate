// v2.22.52 - Concurrent parent moves converge without creating an unreachable page cycle.
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Keep the newest move in a cycle, detach the oldest edge, and retain the
/// original page as a named recovery point. No title, block or property changes.
pub fn resolve_parent_cycles(data: &mut Value) {
    let Some(notes) = data["notes"].as_array_mut() else {
        return;
    };
    let index = notes
        .iter()
        .enumerate()
        .filter(|(_, p)| p["encryption"].is_null() && p["deletedAtEpochMillis"].is_null())
        .filter_map(|(i, p)| Some((p["id"].as_str()?.to_string(), i)))
        .collect::<BTreeMap<_, _>>();
    let mut finished = HashSet::new();
    for start in index.keys() {
        let mut path = Vec::<String>::new();
        let mut positions = HashMap::new();
        let mut current = start.clone();
        while let Some(&i) = index.get(&current) {
            if finished.contains(&current) {
                break;
            }
            if let Some(&begin) = positions.get(&current) {
                let cycle = &path[begin..];
                let chosen = cycle
                    .iter()
                    .min_by_key(|id| {
                        (
                            notes[index[*id]]["updatedAtEpochMillis"]
                                .as_i64()
                                .unwrap_or(0),
                            *id,
                        )
                    })
                    .unwrap();
                let page = &mut notes[index[chosen]];
                let revision = page["updatedAtEpochMillis"].as_i64().unwrap_or(0);
                let parent = page["document"]["knowledge"]["parentId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let revision_id = format!("parent-conflict-{}-{}-{}", chosen, parent, revision);
                let mut snapshot = json!({"id":revision_id,"label":"父级冲突前","capturedAtEpochMillis":revision,"updatedAtEpochMillis":revision});
                for key in [
                    "title",
                    "content",
                    "kind",
                    "document",
                    "accentSeed",
                    "pinned",
                    "folderId",
                    "attachments",
                ] {
                    if let Some(v) = page.get(key) {
                        snapshot[key] = v.clone();
                    }
                }
                let revisions = page
                    .as_object_mut()
                    .unwrap()
                    .entry("revisions")
                    .or_insert_with(|| json!([]));
                if let Some(revisions) = revisions.as_array_mut() {
                    if !revisions.iter().any(|r| r["id"] == revision_id) {
                        revisions.push(snapshot);
                    }
                }
                page["document"]["knowledge"]["parentId"] = Value::Null;
                break;
            }
            positions.insert(current.clone(), path.len());
            path.push(current.clone());
            let Some(parent) = notes[i]["document"]["knowledge"]["parentId"].as_str() else {
                break;
            };
            current = parent.into();
        }
        finished.extend(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn knowledge_concurrent_moves_keep_pages_and_recoverable_original_parent() {
        use crate::{app_data, sync_core};
        let base = app_data::sanitize_app_data_json("{}", 1000).unwrap();
        let pages = json!([
            {"id":"a","kind":"DOCUMENT","title":"A","document":{"knowledge":{},"blocks":[{"id":"a-text","type":"TEXT","text":"A body"}]}},
            {"id":"b","kind":"DOCUMENT","title":"B","document":{"knowledge":{},"blocks":[{"id":"b-text","type":"TEXT","text":"B body"}]}}
        ]);
        let base = app_data::upsert_knowledge_pages_app_data_json(&base, &pages.to_string(), 1001)
            .unwrap();
        let mut a: Value = serde_json::from_str(&base).unwrap();
        let mut b = a.clone();
        let note_a = a["notes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["id"] == "a")
            .unwrap();
        note_a["document"]["knowledge"]["parentId"] = json!("b");
        let left = app_data::upsert_note_app_data_json(&base, &note_a.to_string(), 1002).unwrap();
        let note_b = b["notes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["id"] == "b")
            .unwrap();
        note_b["document"]["knowledge"]["parentId"] = json!("a");
        let right = app_data::upsert_note_app_data_json(&base, &note_b.to_string(), 1003).unwrap();
        let merged = sync_core::merge_sync_app_data_json(&left, &right, 1004).unwrap();
        let reverse = sync_core::merge_sync_app_data_json(&right, &left, 1004).unwrap();
        let projection = |raw: &str| {
            let value: Value = serde_json::from_str(raw).unwrap();
            value["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p["id"].as_str().unwrap().to_string(), p.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(projection(&merged), projection(&reverse));
        let pages = projection(&merged);
        assert!(pages["a"]["document"]["knowledge"]["parentId"].is_null());
        assert_eq!(pages["b"]["document"]["knowledge"]["parentId"], "a");
        assert_eq!(pages["a"]["document"]["blocks"][0]["text"], "A body");
        assert!(pages["a"]["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["label"] == "父级冲突前" && r["document"]["knowledge"]["parentId"] == "b"));
        assert_eq!(
            app_data::sanitize_app_data_json(&merged, 1005).unwrap(),
            merged
        );
        let again = sync_core::merge_sync_app_data_json(&merged, &right, 1006).unwrap();
        assert_eq!(projection(&again), projection(&merged));
    }
}

#[cfg(test)]
#[test]
fn knowledge_legacy_sync_cannot_flatten_a_page_but_explicit_restore_can() {
    use crate::{app_data, sync_core};
    let base = app_data::sanitize_app_data_json("{}", 1000).unwrap();
    let page = json!({"id":"structured","kind":"DOCUMENT","title":"完整页面","document":{"knowledge":{},"blocks":[{"id":"table","type":"TEXT","text":"表格","knowledge":{"kind":"table","table":[["列"],["原值"]]}}]}});
    let advanced = app_data::upsert_note_app_data_json(&base, &page.to_string(), 1001).unwrap();
    let mut legacy: Value = serde_json::from_str(&advanced).unwrap();
    let old = legacy["notes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["id"] == "structured")
        .unwrap();
    old["document"].as_object_mut().unwrap().remove("knowledge");
    old["document"]["blocks"][0]
        .as_object_mut()
        .unwrap()
        .remove("knowledge");
    old["document"]["blocks"][0]["text"] = json!("旧端新增的文字");
    old["content"] = json!("旧端新增的文字");
    old["updatedAtEpochMillis"] = json!(1003);
    let merged = sync_core::merge_sync_app_data_json(&advanced, &legacy.to_string(), 1004).unwrap();
    let reverse =
        sync_core::merge_sync_app_data_json(&legacy.to_string(), &advanced, 1004).unwrap();
    assert_eq!(merged, reverse);
    let value: Value = serde_json::from_str(&merged).unwrap();
    let current = value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "structured")
        .unwrap();
    assert_eq!(
        current["document"]["blocks"][0]["knowledge"]["table"][1][0],
        "原值"
    );
    let recovered = current["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["document"]["blocks"][0]["text"] == "旧端新增的文字")
        .unwrap();
    let restored = app_data::restore_note_revision_app_data_json(
        &merged,
        "structured",
        recovered["id"].as_str().unwrap(),
        1005,
    )
    .unwrap();
    let resynced = sync_core::merge_sync_app_data_json(&restored, &merged, 1006).unwrap();
    let value: Value = serde_json::from_str(&resynced).unwrap();
    let current = value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "structured")
        .unwrap();
    assert_eq!(current["document"]["blocks"][0]["text"], "旧端新增的文字");
    assert!(current["document"]["knowledge"].is_object());
}
