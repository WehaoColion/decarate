// v2.22.52 - Guard data validity, hierarchy cycles and date boundaries.
use super::*;

#[test]
fn knowledge_hierarchy_rejects_cycles_missing_parents_and_locked_targets() {
    let mut records = vec![
        PageRecord {
            id: "a".into(),
            ..Default::default()
        },
        PageRecord {
            id: "b".into(),
            meta: KnowledgePage {
                parent_id: Some("a".into()),
                ..Default::default()
            },
            ..Default::default()
        },
        PageRecord {
            id: "c".into(),
            meta: KnowledgePage {
                parent_id: Some("b".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    assert!(!can_move_page(&records, "a", Some("c")));
    assert!(!can_move_page(&records, "a", Some("missing")));
    assert!(can_move_page(&records, "c", Some("a")));
    assert_eq!(ancestors(&records, "c"), ["a", "b"]);
    records[0].meta.locked = true;
    assert!(!can_move_page(&records, "c", Some("a")));
    assert!(can_move_page(&records, "c", None));
}

#[test]
fn knowledge_database_rejects_forged_ids_and_bad_view_state() {
    let mut database = KnowledgeDatabase::task_database();
    assert!(database.validate().is_ok());
    database.fields.push(database.fields[0].clone());
    assert!(database.validate().is_err());
    database.fields.pop();
    database.default_view_id = "missing".into();
    assert!(database.validate().is_err());
    database.default_view_id = database.views[0].id.clone();
    database.views[0]
        .column_widths
        .insert("status".into(), f32::NAN);
    assert!(database.validate().is_err());
}

#[test]
fn knowledge_dates_and_numeric_values_do_not_silently_normalize_invalid_data() {
    assert_eq!(date_day("1970-01-01"), Some(0));
    for date in [
        "1900-02-29",
        "2026-02-30",
        "2026-9-1",
        "2026-01-32",
        "0000-01-01",
    ] {
        assert_eq!(date_day(date), None);
    }
    for date in [
        "0001-01-01",
        "2000-02-29",
        "2024-02-29",
        "2026-09-15",
        "9999-12-31",
    ] {
        assert_eq!(day_date(date_day(date).unwrap()), date);
    }
    assert!(!CellValue::Number(f64::INFINITY).valid());
    assert!(!CellValue::Date(DateRange {
        start: "2026-09-15".into(),
        end: "2026-09-14".into()
    })
    .valid());
    assert!(!CellValue::Number(0.0).empty());
    assert!(!CellValue::Checkbox(false).empty());
}

#[test]
fn knowledge_unknown_versions_and_malformed_tables_are_rejected() {
    let mut page = KnowledgePage::default();
    page.version = KNOWLEDGE_FORMAT_VERSION + 1;
    assert!(page.validate().is_err());
    let mut block = KnowledgeBlock {
        kind: BlockKind::Table,
        table: vec![vec!["a".into()], vec!["b".into(), "c".into()]],
        ..Default::default()
    };
    assert!(block.validate().is_err());
    block.table[0].push("d".into());
    assert!(block.validate().is_ok());
    assert!(!safe_web_url("javascript:alert(1)"));
    assert!(!safe_web_url("file:///C:/secret"));
    assert!(!safe_web_url("https://name:secret@example.com"));
    assert!(safe_web_url("https://example.com/page?q=1"));
}
