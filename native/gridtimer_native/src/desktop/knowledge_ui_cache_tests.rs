// Windows - Guard navigation/privacy and immutable database cache invalidation boundaries.
#[test]
fn knowledge_quick_navigation_cache_tracks_query_history_notes_and_workspace() {
    let root = temp_test_dir("quick_navigation_cache_boundaries");
    let mut client = knowledge_test_client(&root);
    for id in ["doc-a", "doc-b"] {
        client
            .data
            .notes
            .iter_mut()
            .find(|note| note.id == id)
            .unwrap()
            .title = format!("match {id}");
    }
    client.data_version += 1;
    client.desktop_ui.experience.palette_query = "match".into();
    let first = client.knowledge_quick_choices();
    assert!(Arc::ptr_eq(&first, &client.knowledge_quick_choices()));
    client.desktop_ui.experience.palette_query = "  MATCH  ".into();
    assert!(Arc::ptr_eq(&first, &client.knowledge_quick_choices()));
    client.desktop_ui.experience.trail.push("doc-b".into());
    let visited = client.knowledge_quick_choices();
    assert!(!Arc::ptr_eq(&first, &visited));
    assert!(visited
        .first()
        .is_some_and(|(choice, _, _)| *choice == KnowledgeQuickChoice::Page("doc-b".into())));

    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .deleted_at_epoch_millis = Some(1);
    client.data_version += 1;
    let deleted = client.knowledge_quick_choices();
    assert!(!deleted
        .iter()
        .any(|(choice, _, _)| *choice == KnowledgeQuickChoice::Page("doc-b".into())));
    assert!(!Arc::ptr_eq(&visited, &deleted));

    // Account/workspace switches invalidate this cache even for equal revision values.
    client.invalidate_knowledge_read_cache();
    assert!(!Arc::ptr_eq(&deleted, &client.knowledge_quick_choices()));
    client.desktop_ui.experience.palette_query = ">".into();
    let commands = client.knowledge_quick_choices();
    assert!(commands
        .iter()
        .all(|(choice, _, _)| !matches!(choice, KnowledgeQuickChoice::Page(_))));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_quick_navigation_never_exposes_encrypted_or_deleted_page_titles() {
    let root = temp_test_dir("quick_navigation_privacy");
    let mut client = knowledge_test_client(&root);
    let parent = client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap();
    parent.title = "private-parent-sentinel".into();
    parent.encryption = Some(DesktopNoteEncryptionEnvelope::default());
    let child = client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-a")
        .unwrap();
    child
        .document
        .knowledge
        .get_or_insert_with(Default::default)
        .parent_id = Some("doc-b".into());
    client.data_version += 1;
    let choices = client.knowledge_quick_choices();
    assert!(!choices.iter().any(
        |(_, title, detail)| title.contains("private-parent-sentinel")
            || detail.contains("private-parent-sentinel")
    ));
    client.desktop_ui.experience.palette_query = "private-parent-sentinel".into();
    assert!(client.knowledge_quick_choices().is_empty());
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .encryption = None;
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .deleted_at_epoch_millis = Some(1);
    client.data_version += 1;
    client.desktop_ui.experience.palette_query.clear();
    assert!(!client
        .knowledge_quick_choices()
        .iter()
        .any(
            |(_, title, detail)| title.contains("private-parent-sentinel")
                || detail.contains("private-parent-sentinel")
        ));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn knowledge_database_frame_cache_preserves_order_query_and_snapshot_boundaries() {
    let root = temp_test_dir("database_frame_cache_boundaries");
    let mut client = knowledge_test_client(&root);
    for id in ["doc-a", "doc-b"] {
        client
            .data
            .notes
            .iter_mut()
            .find(|note| note.id == id)
            .unwrap()
            .title = format!("match-{id}");
    }
    client.data_version += 1;
    let records = client.knowledge_records();
    let lookup = client.knowledge_record_lookup(&records);
    assert!(Arc::ptr_eq(
        &lookup.by_id,
        &client.knowledge_record_lookup(&records).by_id
    ));
    let rows = Arc::new(vec!["doc-b".into(), "missing".into(), "doc-a".into()]);
    assert!(Arc::ptr_eq(
        &rows,
        &client.cached_knowledge_database_rows(&rows, &records, &lookup, "")
    ));
    let first = client.cached_knowledge_database_rows(&rows, &records, &lookup, "MATCH");
    assert_eq!(&**first, &["doc-b", "doc-a"]);
    assert!(Arc::ptr_eq(
        &first,
        &client.cached_knowledge_database_rows(&rows, &records, &lookup, " match ")
    ));
    let reordered = Arc::new(vec!["doc-a".into(), "doc-b".into()]);
    let changed_rows =
        client.cached_knowledge_database_rows(&reordered, &records, &lookup, "match");
    assert_eq!(&**changed_rows, &["doc-a", "doc-b"]);
    assert!(!Arc::ptr_eq(&first, &changed_rows));
    assert!(client
        .cached_knowledge_database_rows(&rows, &records, &lookup, "missing-title")
        .is_empty());

    // An alternate same-sized snapshot cannot reuse or replace the current index/search.
    let alternate = Arc::new(vec![knowledge::PageRecord {
        id: "alternate".into(),
        title: "match alternate".into(),
        ..Default::default()
    }]);
    let alternate_lookup = client.knowledge_record_lookup(&alternate);
    assert!(alternate_lookup.get("alternate").is_some());
    let alternate_rows = Arc::new(vec!["alternate".into()]);
    assert_eq!(
        &**client.cached_knowledge_database_rows(
            &alternate_rows,
            &alternate,
            &alternate_lookup,
            "match"
        ),
        &["alternate"]
    );
    assert!(Arc::ptr_eq(
        &lookup.by_id,
        &client.knowledge_record_lookup(&records).by_id
    ));

    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-a")
        .unwrap()
        .encryption = Some(DesktopNoteEncryptionEnvelope::default());
    client
        .data
        .notes
        .iter_mut()
        .find(|note| note.id == "doc-b")
        .unwrap()
        .deleted_at_epoch_millis = Some(1);
    client.data_version += 1;
    let next_records = client.knowledge_records();
    let next_lookup = client.knowledge_record_lookup(&next_records);
    assert!(!Arc::ptr_eq(&lookup.by_id, &next_lookup.by_id));
    assert!(client
        .cached_knowledge_database_rows(&rows, &next_records, &next_lookup, "match")
        .is_empty());
    client.invalidate_knowledge_read_cache();
    assert!(!Arc::ptr_eq(
        &next_lookup.by_id,
        &client
            .knowledge_record_lookup(&client.knowledge_records())
            .by_id
    ));
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

// Exact pre-optimization quick-search implementation for same-executable comparison.
impl TimerWindowsClient {
    fn legacy_knowledge_quick_choices(&self) -> Vec<(KnowledgeQuickChoice, String, String)> {
        let query = self.desktop_ui.experience.palette_query.trim();
        let commands = [
            (KnowledgeQuickChoice::NewPage, "新建页面", "Ctrl+N"),
            (
                KnowledgeQuickChoice::NewDatabase,
                "新建数据库",
                "表格、看板与日历",
            ),
            (KnowledgeQuickChoice::Home, "返回知识库", "查看全部资料"),
            (KnowledgeQuickChoice::Focus, "切换专注模式", "F8"),
            (
                KnowledgeQuickChoice::Navigator,
                "显示或收起页面树",
                "Ctrl+\\",
            ),
        ];
        if let Some(query) = query.strip_prefix('>') {
            return commands
                .into_iter()
                .filter(|(_, title, detail)| knowledge_quick_rank(title, detail, query).is_some())
                .map(|(choice, title, detail)| (choice, title.into(), detail.into()))
                .collect();
        }
        let mut pages = self
            .data
            .notes
            .iter()
            .filter(|n| {
                desktop_note_kind(n) == DesktopNoteKind::Document
                    && n.deleted_at_epoch_millis.is_none()
            })
            .filter_map(|n| {
                let encrypted = n.encryption.is_some();
                let title = if encrypted {
                    "加密页面"
                } else if n.title.trim().is_empty() {
                    "未命名"
                } else {
                    &n.title
                };
                let parent = n
                    .document
                    .knowledge
                    .as_ref()
                    .and_then(|m| m.parent_id.as_deref())
                    .and_then(|id| {
                        self.data
                            .notes
                            .iter()
                            .find(|p| p.id == id && p.encryption.is_none())
                    })
                    .map_or("知识库", |p| p.title.as_str());
                let detail = if encrypted {
                    "已加密".to_string()
                } else {
                    format!(
                        "{} · {}",
                        parent,
                        if n.document
                            .knowledge
                            .as_ref()
                            .is_some_and(|m| m.database.is_some())
                        {
                            "数据库"
                        } else {
                            "页面"
                        }
                    )
                };
                let rank = knowledge_quick_rank(title, &detail, query)?;
                let recent = self
                    .desktop_ui
                    .experience
                    .trail
                    .iter()
                    .rposition(|id| id == &n.id)
                    .map(|i| i + 1)
                    .unwrap_or(0);
                Some((
                    rank,
                    std::cmp::Reverse(recent),
                    std::cmp::Reverse(n.updated_at_epoch_millis),
                    KnowledgeQuickChoice::Page(n.id.clone()),
                    title.to_string(),
                    detail,
                ))
            })
            .collect::<Vec<_>>();
        pages.sort_by(|a, b| (&a.0, &a.1, &a.2, &a.4).cmp(&(&b.0, &b.1, &b.2, &b.4)));
        let mut choices = pages
            .into_iter()
            .take(80)
            .map(|p| (p.3, p.4, p.5))
            .collect::<Vec<_>>();
        if query.is_empty() {
            choices.extend(
                commands
                    .into_iter()
                    .take(2)
                    .map(|(c, t, d)| (c, t.into(), d.into())),
            );
        }
        choices
    }
}
