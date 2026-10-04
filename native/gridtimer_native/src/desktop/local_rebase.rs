// v2.22.35 - Rebase a prepared local operation over an acknowledged draft receipt.
use std::collections::BTreeSet;

fn rebase_local_snapshot(base: &str, next: &str, current: &str) -> Option<String> {
    let base: Value = serde_json::from_str(base).ok()?;
    let next: Value = serde_json::from_str(next).ok()?;
    let current: Value = serde_json::from_str(current).ok()?;
    let merged = rebase_local_value(Some(&base), Some(&next), Some(&current), "", 0).ok()??;
    serde_json::to_string(&merged).ok()
}

fn rebase_local_value(
    base: Option<&Value>,
    next: Option<&Value>,
    current: Option<&Value>,
    key: &str,
    depth: usize,
) -> Result<Option<Value>, ()> {
    if depth > 128 {
        return Err(());
    }
    if base == next {
        return Ok(current.cloned());
    }
    if base == current || next == current {
        return Ok(next.cloned());
    }
    if key == "updatedAt" || key == "updatedAtEpochMillis" || key.ends_with("UpdatedAtEpochMillis")
    {
        if let (Some(a), Some(b)) = (
            next.and_then(Value::as_i64),
            current.and_then(Value::as_i64),
        ) {
            return Ok(Some(json!(a.max(b))));
        }
    }
    match (base, next, current) {
        (Some(Value::Object(base)), Some(Value::Object(next)), Some(Value::Object(current))) => {
            let mut merged = serde_json::Map::new();
            let keys = base
                .keys()
                .chain(next.keys())
                .chain(current.keys())
                .collect::<BTreeSet<_>>();
            for key in keys {
                if let Some(value) = rebase_local_value(
                    base.get(key),
                    next.get(key),
                    current.get(key),
                    key,
                    depth + 1,
                )? {
                    merged.insert(key.clone(), value);
                }
            }
            Ok(Some(Value::Object(merged)))
        }
        (Some(Value::Array(base)), Some(Value::Array(next)), Some(Value::Array(current))) => {
            let index = |values: &[Value]| -> Result<BTreeMap<String, Value>, ()> {
                let mut result = BTreeMap::new();
                for value in values {
                    let id = local_rebase_entity_id(value).ok_or(())?;
                    if result.insert(id, value.clone()).is_some() {
                        return Err(());
                    }
                }
                Ok(result)
            };
            let base_index = index(base)?;
            let next_index = index(next)?;
            let current_index = index(current)?;
            let mut seen = BTreeSet::new();
            let mut merged = Vec::new();
            // Preserve visible local ordering, then append records added by the receipt.
            for value in next.iter().chain(current.iter()).chain(base.iter()) {
                let id = local_rebase_entity_id(value).ok_or(())?;
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(value) = rebase_local_value(
                    base_index.get(&id),
                    next_index.get(&id),
                    current_index.get(&id),
                    "",
                    depth + 1,
                )? {
                    merged.push(value);
                }
            }
            Ok(Some(Value::Array(merged)))
        }
        // Two different edits to the same value must be retried, never silently lost.
        _ => Err(()),
    }
}

fn local_rebase_entity_id(value: &Value) -> Option<String> {
    match value.as_object()?.get("id")? {
        Value::String(id) if !id.is_empty() => Some(format!("s:{id}")),
        Value::Number(id) => Some(format!("n:{id}")),
        _ => None,
    }
}

#[cfg(test)]
mod local_rebase_tests {
    use super::*;

    #[test]
    fn conflicting_body_changes_fail_without_choosing_a_winner() {
        let base = json!({"notes":[{"id":"a","content":"before"}]}).to_string();
        let next = json!({"notes":[{"id":"a","content":"local action"}]}).to_string();
        let current = json!({"notes":[{"id":"a","content":"saved draft"}]}).to_string();
        assert!(rebase_local_snapshot(&base, &next, &current).is_none());
    }

    #[test]
    fn concurrent_entity_additions_and_independent_fields_survive() {
        let base =
            json!({"notes":[{"id":"a","content":"before","title":"old","updatedAtEpochMillis":1}]})
                .to_string();
        let next = json!({"notes":[{"id":"a","content":"before","title":"new","updatedAtEpochMillis":2},{"id":"b","content":"added"}]}).to_string();
        let current = json!({"notes":[{"id":"a","content":"saved draft","title":"old","updatedAtEpochMillis":3},{"id":"c","content":"also added"}]}).to_string();
        let saved: Value =
            serde_json::from_str(&rebase_local_snapshot(&base, &next, &current).unwrap()).unwrap();
        assert_eq!(saved["notes"][0]["content"], "saved draft");
        assert_eq!(saved["notes"][0]["title"], "new");
        assert_eq!(saved["notes"][0]["updatedAtEpochMillis"], 3);
        assert_eq!(saved["notes"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn deleting_an_entity_changed_by_the_worker_requires_retry() {
        let base = json!({"notes":[{"id":"a","content":"before"}]}).to_string();
        let current = json!({"notes":[{"id":"a","content":"saved draft"}]}).to_string();
        assert!(rebase_local_snapshot(&base, "{\"notes\":[]}", &current).is_none());
    }
}
