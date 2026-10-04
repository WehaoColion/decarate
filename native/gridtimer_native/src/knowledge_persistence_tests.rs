// v2.22.52 - Protect advanced data, hierarchy and atomic imports.
use super::*;
use serde_json::{json, Value};

fn page(id: &str, parent: Option<&str>) -> Value {
    let mut meta = crate::knowledge::KnowledgePage::default();
    meta.parent_id = parent.map(str::to_string);
    meta.database = Some(crate::knowledge::KnowledgeDatabase::task_database());
    json!({"id":id,"kind":"DOCUMENT","title":id,"document":{"knowledge":meta,"blocks":[]}})
}

#[test]
fn knowledge_atomic_import_roundtrip_and_fail_closed() {
    let state = sanitize_app_data_json("{}", 1000).unwrap();
    let a = page("a", None);
    let b = page("b", Some("a"));
    let saved =
        upsert_knowledge_pages_app_data_json(&state, &json!([b, a]).to_string(), 1001).unwrap();
    assert_eq!(sanitize_app_data_json(&saved, 1002).unwrap(), saved);
    let value: Value = serde_json::from_str(&saved).unwrap();
    assert!(value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["document"]["knowledge"]["database"]["views"]
            .as_array()
            .unwrap()
            .len()
            == 7));
    assert!(upsert_knowledge_pages_app_data_json(
        &saved,
        &json!([page("new", None), page("bad", Some("missing"))]).to_string(),
        1003
    )
    .is_none());
    assert!(upsert_knowledge_pages_app_data_json(
        &saved,
        &json!([page("a", Some("b"))]).to_string(),
        1003
    )
    .is_none());
    assert!(upsert_note_app_data_json(
        &saved,
        &json!({"id":"a","kind":"DOCUMENT","title":"stale editor","content":"flat"}).to_string(),
        1003
    )
    .is_none());
}

#[test]
fn knowledge_page_lock_protects_body_but_allows_explicit_unlock() {
    let state = sanitize_app_data_json("{}", 1000).unwrap();
    let mut p = page("locked", None);
    p["document"]["knowledge"]["locked"] = json!(true);
    let saved = upsert_note_app_data_json(&state, &p.to_string(), 1001).unwrap();
    let value: Value = serde_json::from_str(&saved).unwrap();
    let original = value["notes"][0].clone();
    let mut changed = original.clone();
    changed["title"] = json!("overwrite");
    assert!(upsert_note_app_data_json(&saved, &changed.to_string(), 1002).is_none());
    changed = original;
    changed["document"]["knowledge"]["locked"] = json!(false);
    assert!(upsert_note_app_data_json(&saved, &changed.to_string(), 1002).is_some());
}

#[test]
fn knowledge_nested_columns_and_named_history_preserve_content() {
    let state = sanitize_app_data_json("{}", 1000).unwrap();
    let p = json!({"id":"nested","kind":"DOCUMENT","title":"Nested","document":{"knowledge":{},"blocks":[
        {"id":"outer","type":"TEXT","knowledge":{"kind":"columns","columns":2}},
        {"id":"inner","type":"TEXT","knowledge":{"kind":"columns","columns":4,"parentId":"outer","column":1}},
        {"id":"todo","type":"TEXT","text":"Pay attention","knowledge":{"kind":"todo","parentId":"inner","column":3}}
    ]}});
    let saved =
        upsert_note_app_data_json(&state, &p.to_string(), 1001).expect("valid nested columns");
    let named =
        capture_knowledge_version_app_data_json(&saved, "nested", "Before editing", 1002).unwrap();
    let mut value: Value = serde_json::from_str(&named).unwrap();
    value["notes"][0]["document"]["blocks"][2]["text"] = json!("Changed");
    let edited = upsert_note_app_data_json(&named, &value["notes"][0].to_string(), 1003).unwrap();
    let roundtrip = sanitize_app_data_json(&edited, 1004).unwrap();
    let result: Value = serde_json::from_str(&roundtrip).unwrap();
    let revisions = result["notes"][0]["revisions"].as_array().unwrap();
    assert!(revisions
        .iter()
        .any(|r| r["label"] == "Before editing"
            && r["document"]["blocks"][2]["text"] == "Pay attention"));
    assert_eq!(
        result["slots"],
        serde_json::from_str::<Value>(&state).unwrap()["slots"]
    );
    assert!(capture_knowledge_version_app_data_json(&edited, "nested", " ", 1004).is_none());
    let mut invalid = p.clone();
    invalid["document"]["blocks"][1]["knowledge"]["column"] = json!(2);
    assert!(upsert_note_app_data_json(&state, &invalid.to_string(), 1004).is_none());
    invalid = p;
    invalid["document"]["knowledge"]["unknownAdvancedSetting"] = json!(true);
    assert!(
        upsert_knowledge_pages_app_data_json(&state, &json!([invalid]).to_string(), 1004).is_none()
    );
}
