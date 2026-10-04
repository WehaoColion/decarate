// v2.22.52 - Import round trips and identity boundaries.
use super::*;
#[test]
fn csv_roundtrip_keeps_quotes_newlines_zero_and_rejects_partial_rows() {
    let rows = vec![
        vec!["标题".into(), "数量".into(), "备注".into()],
        vec!["甲,乙".into(), "0".into(), "换行\n\"引号\"".into()],
    ];
    assert_eq!(parse_csv(&write_csv(&rows)).unwrap(), rows);
    for bad in [
        "甲,乙\n1",
        "甲,甲\n1,2",
        "甲,乙\n\"未结束,2",
        "甲,乙\n\"a\"x,b",
    ] {
        assert!(parse_csv(bad).is_err(), "{bad}");
    }
    let mut n = 0;
    let pages = csv_to_pages(&write_csv(&rows), "导入", |p| {
        n += 1;
        format!("{p}-{n}")
    })
    .unwrap();
    let state = crate::app_data::sanitize_app_data_json("{}", 1000).unwrap();
    assert!(crate::app_data::upsert_knowledge_pages_app_data_json(
        &state,
        &serde_json::to_string(&pages).unwrap(),
        1001
    )
    .is_some());
}
#[test]
fn markdown_preserves_todos_code_and_table_cells() {
    let mut n = 0;
    let blocks=markdown_blocks("## 章节\n\n- [x] 已办\n\n```rust\nlet n = 1;\n```\n\n| 名称 | 数字 |\n| --- | --- |\n| a&#124;b | 0 |",||{n+=1;format!("b{n}")});
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[1]["knowledge"]["checked"], true);
    assert_eq!(blocks[3]["knowledge"]["table"][1][0], "a|b");
    let encoded = blocks_markdown(&blocks);
    let reparsed = markdown_blocks(&encoded, || "next".into());
    for (a, b) in blocks.iter().zip(&reparsed) {
        assert_eq!(a["text"], b["text"]);
        assert_eq!(a["knowledge"], b["knowledge"]);
    }
}
#[test]
fn copied_subtree_remaps_parent_and_cross_page_block_references() {
    let pages = vec![
        json!({"id":"root","document":{"knowledge":KnowledgePage::default(),"blocks":[{"id":"a","type":"TEXT","text":"root","knowledge":KnowledgeBlock::default()}]}}),
        json!({"id":"child","document":{"knowledge":KnowledgePage{parent_id:Some("root".into()),..Default::default()},"blocks":[{"id":"b","type":"TEXT","text":"ref","knowledge":KnowledgeBlock{kind:BlockKind::BlockLink,target_page_id:"root".into(),target_block_id:"a".into(),..Default::default()}}]}}),
    ];
    let mut n = 0;
    let copied = remap_pages(&pages, |p| {
        n += 1;
        format!("{p}-{n}")
    })
    .unwrap();
    assert_ne!(copied[0]["id"], pages[0]["id"]);
    assert_eq!(
        copied[1]["document"]["knowledge"]["parentId"],
        copied[0]["id"]
    );
    assert_eq!(
        copied[1]["document"]["blocks"][0]["knowledge"]["targetBlockId"],
        copied[0]["document"]["blocks"][0]["id"]
    );
    assert!(remap_pages(
        &[json!({"id":"encrypted","encryption":{"ciphertext":"x"}})],
        |p| p.into()
    )
    .is_err());
}

#[test]
fn knowledge_csv_preserves_decimal_types_and_identifier_zeros() {
    let mut count = 0;
    let pages = csv_to_pages(
        "标题,小数,编号\n甲,0.1,001\n乙,-0.25,002\n丙,0,003",
        "数据",
        |prefix| {
            count += 1;
            format!("{prefix}-{count}")
        },
    )
    .unwrap();
    let database: KnowledgePage =
        serde_json::from_value(pages[0]["document"]["knowledge"].clone()).unwrap();
    let fields = &database.database.unwrap().fields;
    assert_eq!(fields[0].kind, FieldKind::Number);
    assert_eq!(fields[1].kind, FieldKind::Text);
    assert_eq!(
        pages[1]["document"]["knowledge"]["properties"][&fields[0].id]["value"],
        json!(0.1)
    );
    assert_eq!(
        pages[1]["document"]["knowledge"]["properties"][&fields[1].id]["value"],
        "001"
    );
}
