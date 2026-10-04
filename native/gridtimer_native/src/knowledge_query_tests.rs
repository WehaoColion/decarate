// v2.22.52 - Business-state checks for computed and filtered records.
use super::*;

fn fixture() -> Vec<PageRecord> {
    let mut db = KnowledgeDatabase::task_database();
    db.fields.extend([
        DatabaseField {
            id: "cost".into(),
            name: "成本".into(),
            kind: FieldKind::Number,
            ..Default::default()
        },
        DatabaseField {
            id: "double".into(),
            name: "翻倍".into(),
            kind: FieldKind::Formula,
            formula: "[成本] * 2".into(),
            ..Default::default()
        },
        DatabaseField {
            id: "loop".into(),
            name: "循环".into(),
            kind: FieldKind::Formula,
            formula: "[循环] + 1".into(),
            ..Default::default()
        },
        DatabaseField {
            id: "related".into(),
            name: "关联".into(),
            kind: FieldKind::Relation,
            ..Default::default()
        },
        DatabaseField {
            id: "total".into(),
            name: "合计".into(),
            kind: FieldKind::Rollup,
            rollup_relation_field: "related".into(),
            rollup_target_field: "cost".into(),
            aggregate: AggregateKind::Sum,
            ..Default::default()
        },
    ]);
    let mut records = vec![PageRecord {
        id: "db".into(),
        meta: KnowledgePage {
            database: Some(db),
            ..Default::default()
        },
        ..Default::default()
    }];
    for (id, n, state) in [
        ("a", 10.0, "进行中"),
        ("b", 2.0, "未开始"),
        ("c", 7.0, "已完成"),
    ] {
        records.push(PageRecord {
            id: id.into(),
            title: format!("任务 {id}"),
            meta: KnowledgePage {
                parent_id: Some("db".into()),
                properties: BTreeMap::from([
                    ("cost".into(), CellValue::Number(n)),
                    ("status".into(), CellValue::Select(state.into())),
                ]),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    records
}

#[test]
fn nested_filters_numeric_sort_and_computed_field_resolve_real_records() {
    let data = fixture();
    let mut engine = QueryEngine::new(&data, 0);
    let view = DatabaseView {
        filter: Some(Filter::All {
            filters: vec![
                Filter::Rule {
                    field: "cost".into(),
                    operator: FilterOperator::Greater,
                    value: CellValue::Number(1.0),
                },
                Filter::Any {
                    filters: vec![
                        Filter::Rule {
                            field: "status".into(),
                            operator: FilterOperator::Equals,
                            value: CellValue::Select("进行中".into()),
                        },
                        Filter::Rule {
                            field: "status".into(),
                            operator: FilterOperator::Equals,
                            value: CellValue::Select("未开始".into()),
                        },
                    ],
                },
            ],
        }),
        sorts: vec![SortRule {
            field: "double".into(),
            descending: false,
        }],
        ..Default::default()
    };
    assert_eq!(engine.rows("db", &view), vec!["b", "a"]);
    assert_eq!(
        engine.value("a", "double").unwrap(),
        CellValue::Number(20.0)
    );
}

#[test]
fn relation_rollup_excludes_encrypted_and_deleted_records_and_detects_cycles() {
    let mut data = fixture();
    data[1].meta.properties.insert(
        "related".into(),
        CellValue::Relation(vec!["b".into(), "c".into()]),
    );
    assert_eq!(
        QueryEngine::new(&data, 0).value("a", "total").unwrap(),
        CellValue::Number(9.0)
    );
    data[3].encrypted = true;
    let mut engine = QueryEngine::new(&data, 0);
    assert_eq!(engine.value("a", "total").unwrap(), CellValue::Number(2.0));
    assert!(engine.value("a", "loop").unwrap_err().contains("循环"));
    assert!(engine.value("c", "title").is_err());
    assert!(!engine
        .rows("db", &DatabaseView::default())
        .contains(&"c".into()));
}

#[test]
fn formulas_are_lazy_bounded_and_reject_invalid_arithmetic() {
    let eval = |text: &str| evaluate_formula(text, |_| Err("无字段".into()), 0);
    assert_eq!(
        eval("if(false, 1 / 0, 2 + 3 * 4)").unwrap(),
        CellValue::Number(14.0)
    );
    assert_eq!(
        eval("concat('中文', round(10 / 3, 2))").unwrap().text(),
        "中文3.33"
    );
    assert_eq!(
        eval("dateBetween('2024-03-01', '2024-02-28')").unwrap(),
        CellValue::Number(2.0)
    );
    for input in [
        "1/0",
        "1e309",
        "shell('write')",
        "2 +",
        "dateAdd('2024-01-01', 1.5)",
        "1e308*1e308",
    ] {
        assert!(eval(input).is_err(), "{input}");
    }
    assert!(eval(&format!("{}1{}", "(".repeat(1000), ")".repeat(1000))).is_err());
    assert_eq!(eval("false && (1/0)").unwrap(), CellValue::Checkbox(false));
}

#[test]
fn property_validation_protects_required_types_and_choices() {
    let field = DatabaseField {
        id: "x".into(),
        name: "数量".into(),
        kind: FieldKind::Number,
        required: true,
        ..Default::default()
    };
    assert!(parse_cell(&field, "").is_err());
    assert!(parse_cell(&field, "NaN").is_err());
    assert_eq!(parse_cell(&field, "0").unwrap(), CellValue::Number(0.0));
    let field = DatabaseField {
        kind: FieldKind::Select,
        options: vec!["甲".into()],
        ..field
    };
    assert!(parse_cell(&field, "乙").is_err());
    assert!(validate_cell(&field, &CellValue::Text("甲".into())).is_err());
    assert_eq!(
        aggregate(
            &[
                CellValue::Number(0.0),
                CellValue::Number(4.0),
                CellValue::Empty
            ],
            AggregateKind::Average
        ),
        CellValue::Number(2.0)
    );
    assert_eq!(aggregate(&[], AggregateKind::Minimum), CellValue::Empty);
}
