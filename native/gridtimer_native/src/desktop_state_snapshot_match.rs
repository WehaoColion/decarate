//! Compare an exact journal row without copying its text or protected payload.
use super::*;
use rusqlite::types::ValueRef;

pub(super) fn snapshot_matches_row(
    connection: &Connection,
    expected: &DesktopStateSnapshot,
) -> DesktopStateStoreResult<bool> {
    connection
        .query_row(
            "SELECT id, owner, app_data_json, protected_sync_state, schema_version,
                    revision, item_count, semantic_summary, raw_sha256,
                    canonical_json_sha256, sync_state_sha256,
                    parent_envelope_sha256, envelope_sha256,
                    created_at_epoch_millis, source
             FROM desktop_state_snapshots WHERE id = ?1",
            params![expected.id],
            |row| snapshot_row_matches(row, expected),
        )
        .optional()
        .map(|matches| matches.unwrap_or(false))
        .map_err(DesktopStateStoreError::from)
}

fn invalid_column_type(
    row: &rusqlite::Row<'_>,
    index: usize,
    value: ValueRef<'_>,
) -> rusqlite::Error {
    // get_ref has already established that this column exists.
    match row.as_ref().column_name(index) {
        Ok(name) => rusqlite::Error::InvalidColumnType(index, name.into(), value.data_type()),
        Err(error) => error,
    }
}

fn text_column<'row>(row: &'row rusqlite::Row<'_>, index: usize) -> rusqlite::Result<&'row str> {
    match row.get_ref(index)? {
        ValueRef::Text(bytes) => {
            std::str::from_utf8(bytes).map_err(|error| rusqlite::Error::Utf8Error(index, error))
        }
        value => Err(invalid_column_type(row, index, value)),
    }
}

fn blob_column<'row>(row: &'row rusqlite::Row<'_>, index: usize) -> rusqlite::Result<&'row [u8]> {
    match row.get_ref(index)? {
        ValueRef::Blob(bytes) => Ok(bytes),
        value => Err(invalid_column_type(row, index, value)),
    }
}

fn snapshot_row_matches(
    row: &rusqlite::Row<'_>,
    expected: &DesktopStateSnapshot,
) -> rusqlite::Result<bool> {
    // Match snapshot_from_row's decoding order and errors. Decode every column
    // before comparing: an earlier mismatch must not conceal a later bad type
    // or malformed UTF-8 value. All borrowed bytes stay inside this row callback.
    let id = row.get::<_, i64>(0)?;
    let owner = text_column(row, 1)?;
    let app_data_json = text_column(row, 2)?;
    let protected_sync_state = blob_column(row, 3)?;
    let schema_version = row.get::<_, i64>(4)?;
    let revision = row.get::<_, i64>(5)?;
    let item_count = row.get::<_, i64>(6)?;
    let semantic_summary = text_column(row, 7)?;
    let raw_sha256 = text_column(row, 8)?;
    let canonical_json_sha256 = text_column(row, 9)?;
    let sync_state_sha256 = text_column(row, 10)?;
    let parent_envelope_sha256 = text_column(row, 11)?;
    let envelope_sha256 = text_column(row, 12)?;
    let created_at_epoch_millis = row.get::<_, i64>(13)?;
    let source = text_column(row, 14)?;

    // Exhaustive destructuring makes adding a persisted snapshot field require
    // an explicit decision here as well as in the owned row decoder.
    let DesktopStateSnapshot {
        id: expected_id,
        owner: expected_owner,
        app_data_json: expected_app_data_json,
        protected_sync_state: expected_protected_sync_state,
        schema_version: expected_schema_version,
        revision: expected_revision,
        item_count: expected_item_count,
        semantic_summary: expected_semantic_summary,
        raw_sha256: expected_raw_sha256,
        canonical_json_sha256: expected_canonical_json_sha256,
        sync_state_sha256: expected_sync_state_sha256,
        parent_envelope_sha256: expected_parent_envelope_sha256,
        envelope_sha256: expected_envelope_sha256,
        created_at_epoch_millis: expected_created_at_epoch_millis,
        source: expected_source,
    } = expected;
    Ok(id == *expected_id
        && owner == expected_owner.as_str()
        && app_data_json == expected_app_data_json.as_str()
        && protected_sync_state == expected_protected_sync_state.as_slice()
        && schema_version == *expected_schema_version
        && revision == *expected_revision
        && item_count == *expected_item_count
        && semantic_summary == expected_semantic_summary.as_str()
        && raw_sha256 == expected_raw_sha256.as_str()
        && canonical_json_sha256 == expected_canonical_json_sha256.as_str()
        && sync_state_sha256 == expected_sync_state_sha256.as_str()
        && parent_envelope_sha256 == expected_parent_envelope_sha256.as_str()
        && envelope_sha256 == expected_envelope_sha256.as_str()
        && created_at_epoch_millis == *expected_created_at_epoch_millis
        && source == expected_source.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::types::Value;

    const COLUMNS: [&str; 15] = [
        "id",
        "owner",
        "app_data_json",
        "protected_sync_state",
        "schema_version",
        "revision",
        "item_count",
        "semantic_summary",
        "raw_sha256",
        "canonical_json_sha256",
        "sync_state_sha256",
        "parent_envelope_sha256",
        "envelope_sha256",
        "created_at_epoch_millis",
        "source",
    ];
    const TEXT_COLUMNS: [usize; 9] = [1, 2, 7, 8, 9, 10, 11, 12, 14];

    fn values(snapshot: &DesktopStateSnapshot) -> Vec<Value> {
        vec![
            Value::Integer(snapshot.id),
            Value::Text(snapshot.owner.clone()),
            Value::Text(snapshot.app_data_json.clone()),
            Value::Blob(snapshot.protected_sync_state.clone()),
            Value::Integer(snapshot.schema_version),
            Value::Integer(snapshot.revision),
            Value::Integer(snapshot.item_count),
            Value::Text(snapshot.semantic_summary.clone()),
            Value::Text(snapshot.raw_sha256.clone()),
            Value::Text(snapshot.canonical_json_sha256.clone()),
            Value::Text(snapshot.sync_state_sha256.clone()),
            Value::Text(snapshot.parent_envelope_sha256.clone()),
            Value::Text(snapshot.envelope_sha256.clone()),
            Value::Integer(snapshot.created_at_epoch_millis),
            Value::Text(snapshot.source.clone()),
        ]
    }

    fn row_query(cast_text: Option<usize>) -> String {
        format!(
            "SELECT {}",
            COLUMNS
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let parameter = format!("?{}", index + 1);
                    if cast_text == Some(index) {
                        format!("CAST({parameter} AS TEXT) AS {name}")
                    } else {
                        format!("{parameter} AS {name}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    fn assert_decoder_error_parity(
        connection: &Connection,
        expected: &DesktopStateSnapshot,
        row_values: &[Value],
        cast_text: Option<usize>,
        expected_error_column: usize,
    ) {
        let query = row_query(cast_text);
        let owned = connection
            .query_row(
                &query,
                rusqlite::params_from_iter(row_values),
                snapshot_from_row,
            )
            .unwrap_err();
        let borrowed = connection
            .query_row(&query, rusqlite::params_from_iter(row_values), |row| {
                snapshot_row_matches(row, expected)
            })
            .unwrap_err();
        match &borrowed {
            rusqlite::Error::InvalidColumnType(index, _, _)
            | rusqlite::Error::Utf8Error(index, _) => {
                assert_eq!(*index, expected_error_column);
            }
            other => panic!("unexpected borrowed row error: {other:?}"),
        }
        assert_eq!(format!("{owned:?}"), format!("{borrowed:?}"));
    }

    fn decoder_snapshot() -> DesktopStateSnapshot {
        DesktopStateSnapshot {
            id: 1,
            owner: "owner\0中文".into(),
            app_data_json: "{\"note\":\"😀\"}".into(),
            protected_sync_state: vec![0, 0xff, 1],
            schema_version: 1,
            revision: i64::MAX,
            item_count: 0,
            semantic_summary: "{}".into(),
            raw_sha256: "1".repeat(64),
            canonical_json_sha256: "2".repeat(64),
            sync_state_sha256: "3".repeat(64),
            parent_envelope_sha256: String::new(),
            envelope_sha256: "4".repeat(64),
            created_at_epoch_millis: i64::MIN,
            source: "local_save".into(),
        }
    }

    #[test]
    fn borrowed_snapshot_comparison_rejects_every_changed_field_and_preserves_decoder_errors() {
        let connection = Connection::open_in_memory().unwrap();
        let expected = decoder_snapshot();
        let original = values(&expected);
        let query = row_query(None);
        assert!(connection
            .query_row(&query, rusqlite::params_from_iter(&original), |row| {
                snapshot_row_matches(row, &expected)
            })
            .unwrap());
        for index in 0..COLUMNS.len() {
            let mut changed = original.clone();
            changed[index] = match &changed[index] {
                Value::Integer(value) => Value::Integer(value.wrapping_add(1)),
                Value::Text(value) => Value::Text(format!("{value}-changed")),
                Value::Blob(value) => {
                    let mut value = value.clone();
                    value.push(2);
                    Value::Blob(value)
                }
                _ => unreachable!(),
            };
            assert!(
                !connection
                    .query_row(&query, rusqlite::params_from_iter(&changed), |row| {
                        snapshot_row_matches(row, &expected)
                    })
                    .unwrap(),
                "changed field {} was accepted",
                COLUMNS[index]
            );

            let mut wrong_type = original.clone();
            wrong_type[index] = match &original[index] {
                Value::Integer(_) => Value::Real(1.0),
                Value::Text(value) => Value::Blob(value.as_bytes().to_vec()),
                Value::Blob(_) => Value::Text(String::new()),
                _ => unreachable!(),
            };
            assert_decoder_error_parity(&connection, &expected, &wrong_type, None, index);
            wrong_type[index] = Value::Null;
            assert_decoder_error_parity(&connection, &expected, &wrong_type, None, index);
        }
        for index in TEXT_COLUMNS {
            let mut malformed = original.clone();
            malformed[index] = Value::Blob(vec![0x80]);
            assert_decoder_error_parity(&connection, &expected, &malformed, Some(index), index);
        }
        let mut mismatch_then_error = original.clone();
        mismatch_then_error[0] = Value::Integer(expected.id + 1);
        mismatch_then_error[14] = Value::Blob(vec![0x80]);
        assert_decoder_error_parity(&connection, &expected, &mismatch_then_error, Some(14), 14);
        mismatch_then_error[1] = Value::Null;
        assert_decoder_error_parity(&connection, &expected, &mismatch_then_error, Some(14), 1);

        let mut empty_payload = expected.clone();
        empty_payload.protected_sync_state.clear();
        assert!(connection
            .query_row(
                &query,
                rusqlite::params_from_iter(values(&empty_payload)),
                |row| { snapshot_row_matches(row, &empty_payload) }
            )
            .unwrap());
    }

    fn committed_fixture(label: &str) -> (PathBuf, DesktopStateStore, DesktopCommittedSnapshot) {
        let directory = std::env::temp_dir().join(format!(
            "desktop-snapshot-match-{label}-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        fs::create_dir_all(&directory).unwrap();
        let store = DesktopStateStore::open(directory.join("state.sqlite3")).unwrap();
        let state = app_data::default_app_data_json(100);
        for owner in ["owner", "other-owner"] {
            store
                .record_with_sync_state(owner, &state, b"protected", 100, "local_save")
                .unwrap();
        }
        let running = app_data::start_slot_app_data_json(&state, 1, 175).unwrap();
        let (_, committed) = DesktopStateStore::record_existing_save_session(
            store.database_path(),
            "owner",
            &running,
            200,
            "local_save",
            |parent| Ok(parent.unwrap().protected_sync_state.clone()),
            |_| Ok(()),
            |_| Ok(()),
        )
        .unwrap()
        .unwrap();
        (directory, store, committed)
    }

    #[test]
    fn changed_snapshot_columns_block_evidence_callbacks_and_mirror_publication() {
        for (index, column) in COLUMNS.iter().enumerate() {
            let (directory, store, mut committed) = committed_fixture(column);
            let connection = store.open_connection(false).unwrap();
            assert!(snapshot_matches_row(&connection, &committed.snapshot).unwrap());
            let mut changed = values(&committed.snapshot).remove(index);
            changed = match index {
                // Keep the journal's monotonic sequence unchanged while making
                // the receipt's original row id absent.
                0 => Value::Integer(0),
                1 => Value::Text("other-owner".into()),
                2 | 7 => Value::Text("{}".into()),
                8..=12 => {
                    let replacement = "f".repeat(64);
                    if changed == Value::Text(replacement.clone()) {
                        Value::Text("e".repeat(64))
                    } else {
                        Value::Text(replacement)
                    }
                }
                _ => match changed {
                    Value::Integer(value) => Value::Integer(value + 1),
                    Value::Text(value) => Value::Text(format!("{value}-changed")),
                    Value::Blob(mut value) => {
                        value.push(1);
                        Value::Blob(value)
                    }
                    _ => unreachable!(),
                },
            };
            assert_eq!(
                connection
                    .execute(
                        &format!("UPDATE desktop_state_snapshots SET {column}=?1 WHERE id=?2"),
                        params![changed, committed.snapshot.id],
                    )
                    .unwrap(),
                1
            );
            assert!(
                !snapshot_matches_row(&connection, &committed.snapshot).unwrap(),
                "{column}"
            );
            assert_eq!(
                read_journal_evidence(&connection).unwrap(),
                committed.evidence
            );
            drop(connection);

            let refreshed = std::cell::Cell::new(false);
            assert!(
                store
                    .with_committed_snapshot_evidence(&committed, |_| {
                        refreshed.set(true);
                        Ok(())
                    })
                    .is_err(),
                "changed field {column} authorized a callback"
            );
            assert!(!refreshed.get(), "{column}");
            let writer = store.lock_recovery_mirror_writer().unwrap();
            let target = directory.join("timer_state.json");
            let temporary = directory.join("timer_state.json.tmp-proof");
            let raw = committed.snapshot.app_data_json.clone();
            assert!(
                writer
                    .prepare_committed("owner", &target, &temporary, &raw, None, &mut committed)
                    .is_err(),
                "changed field {column} authorized a mirror"
            );
            assert!(!target.exists());
            assert!(!temporary.exists());
            drop(writer);
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
