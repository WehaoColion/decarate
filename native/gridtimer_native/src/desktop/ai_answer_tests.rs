// v1.1.0.7 Windows - Specify request denial, source privacy and immutable preview boundaries.
fn desktop_ai_query_client(root: &Path) -> TimerWindowsClient {
    let mut client = test_client_for_account_scope(
        root,
        "synthetic-ai-owner",
        app_state_with_note("n", "资料", "只作隔离测试", None),
    );
    client.sync.ai_api_key = "synthetic-never-sent".into();
    client.sync.ai_base_url = "https://api.deepseek.com".into();
    client.sync.ai_model = "deepseek-flash".into();
    client.knowledge_ai_question_draft = "解释复利".into();
    client
}

#[test]
fn desktop_ai_direct_preview_does_not_include_workspace_sources() {
    let root = temp_test_dir("desktop_ai_direct_privacy");
    let mut client = desktop_ai_query_client(&root);
    client.desktop_ai.mode = QueryMode::Direct;
    client.launch_knowledge_ai();
    let preview = client.desktop_ai.preview.as_ref().unwrap();
    assert_eq!(preview.mode, QueryMode::Direct);
    assert!(preview.sources.is_empty());
    assert!(!client.knowledge_ai_pending);
    assert!(client.knowledge_ai_result_rx.is_none());
    assert_eq!(client.task_supervisor.active_count(), 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_ai_empty_missing_configuration_and_oversized_questions_never_preview() {
    let root = temp_test_dir("desktop_ai_preflight_denied");
    let mut client = desktop_ai_query_client(&root);
    for (key, question) in [
        ("", "问题".to_owned()),
        ("synthetic-never-sent", " \n ".to_owned()),
        ("synthetic-never-sent", "问".repeat(1001)),
    ] {
        client.sync.ai_api_key = key.into();
        client.knowledge_ai_question_draft = question.clone();
        client.launch_knowledge_ai();
        assert!(client.desktop_ai.preview.is_none());
        assert!(!client.knowledge_ai_pending);
        assert_eq!(client.knowledge_ai_question_draft, question);
        assert_eq!(client.task_supervisor.active_count(), 0);
    }
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_ai_changed_configuration_question_and_workspace_deny_actual_send() {
    let root = temp_test_dir("desktop_ai_actual_send_binding");
    let mut client = desktop_ai_query_client(&root);
    client.launch_knowledge_ai();
    assert!(client.desktop_ai.preview.is_some());
    client.sync.ai_model = "changed-model".into();
    client.send_desktop_ai_query();
    assert!(!client.knowledge_ai_pending);
    assert!(client.knowledge_ai_result_rx.is_none());
    client.launch_knowledge_ai();
    client.knowledge_ai_question_draft = "另一个问题".into();
    client.send_desktop_ai_query();
    assert!(!client.knowledge_ai_pending);
    client.launch_knowledge_ai();
    client.sync.user_id = "other-owner".into();
    client.send_desktop_ai_query();
    assert!(!client.knowledge_ai_pending);
    assert_eq!(client.task_supervisor.active_count(), 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_ai_unwritable_workspace_denies_actual_send_after_preview() {
    let root = temp_test_dir("desktop_ai_ready_denied");
    let mut client = desktop_ai_query_client(&root);
    client.launch_knowledge_ai();
    assert!(client.desktop_ai.preview.is_some());
    client.workspace_persistence_ready = false;
    client.send_desktop_ai_query();
    assert!(!client.knowledge_ai_pending);
    assert!(!client.desktop_ai.boundary.busy());
    assert!(client.knowledge_ai_result_rx.is_none());
    assert_eq!(client.task_supervisor.active_count(), 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_ai_pending_account_migration_denies_actual_send_and_probe() {
    let root = temp_test_dir("desktop_ai_pending_account_boundary");
    let mut client = desktop_ai_query_client(&root);
    client.launch_knowledge_ai();
    assert!(client.desktop_ai.preview.is_some());
    let prior_scope = client.background_job_workspace_fingerprint();
    client.sync_task = Some(SyncTaskState {
        kind: SyncTaskKind::Login,
        phase: SyncTaskPhase::AppData,
        started_at_epoch_millis: now_millis(),
    });
    assert_eq!(client.background_job_workspace_fingerprint(), prior_scope);
    client.send_desktop_ai_query();
    assert!(!client.knowledge_ai_pending);
    assert!(client.knowledge_ai_result_rx.is_none());
    client.launch_desktop_ai_probe();
    assert!(client.desktop_ai.probe_rx.is_none());
    client.launch_knowledge_ai();
    assert!(client.desktop_ai.preview.is_none());
    assert_eq!(client.task_supervisor.active_count(), 0);
    client.sync_task = None;
    client.launch_knowledge_ai();
    assert!(client.desktop_ai.preview.is_some());
    client.sync_task = Some(SyncTaskState {
        kind: SyncTaskKind::Download,
        phase: SyncTaskPhase::AppData,
        started_at_epoch_millis: now_millis(),
    });
    client.observe_desktop_ai_scope();
    assert!(client.desktop_ai.preview.is_none());
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_ai_unsent_question_draft_is_cleared_when_account_changes() {
    let root = temp_test_dir("desktop_ai_unsent_scope_privacy");
    let mut client = desktop_ai_query_client(&root);
    client.observe_desktop_ai_scope();
    assert!(!client.knowledge_ai_question_draft.is_empty());
    assert!(client.desktop_ai.preview.is_none());
    assert!(client.knowledge_ai_answer.is_none());
    let original_data = client.state_json.clone();
    client.sync.user_id = "another-synthetic-owner".into();
    client.observe_desktop_ai_scope();
    assert!(client.knowledge_ai_question_draft.is_empty());
    assert!(client.knowledge_ai_answer.is_none());
    assert!(client.desktop_ai.preview.is_none());
    assert_eq!(client.state_json, original_data);
    assert_eq!(client.task_supervisor.active_count(), 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_ai_knowledge_empty_scope_cannot_become_direct_mode() {
    let root = temp_test_dir("desktop_ai_knowledge_scope_empty");
    let mut client = desktop_ai_query_client(&root);
    client.desktop_ai.mode = QueryMode::Knowledge;
    client.knowledge_ai_scope_all = false;
    client.data.note_preferences.selected_folder_id = Some("empty-synthetic-folder".into());
    client.launch_knowledge_ai();
    assert!(client.desktop_ai.preview.is_none());
    assert_eq!(client.desktop_ai.mode, QueryMode::Knowledge);
    assert!(!client.knowledge_ai_pending);
    assert_eq!(client.task_supervisor.active_count(), 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn desktop_ai_saved_answer_renders_math_and_safe_links_without_changing_saved_source() {
    let root = temp_test_dir("desktop_ai_saved_math_document");
    let mut client = desktop_ai_query_client(&root);
    let content = r#"**复利**：本金 \(P\)，总额

\[A=P\times(1+r)^n\]

[参考](https://example.test/reference?q=one&n=two)

无法解析的公式保留原文：\(\unknowncmd{x}\)。

`\(代码\)`，金额 $100 and $200。

```latex
\[代码公式\]
```

<script>alert(27)</script>
"#;
    client.knowledge_ai_answer = Some(KnowledgeAiAnswer {
        workspace_binding: client.background_job_workspace_fingerprint(),
        recipient_host: "offline-render-test".into(),
        model: "synthetic-format-fixture".into(),
        question: "合成保存验收".into(),
        content: content.into(),
        source_ids: Vec::new(),
        source_titles: Vec::new(),
        source_folders: Vec::new(),
        received_at_epoch_millis: now_millis(),
    });
    client.save_knowledge_answer_as_document();
    let note = client.selected_note().expect("saved document");
    assert_eq!(note.kind, DesktopNoteKind::Document.code());
    assert!(!note.document.rich_text_enabled);
    assert_eq!(note.document.blocks[0].text, content);
    let stored_before_rendering = serde_json::to_string(&note).unwrap();
    let html = knowledge_complete_html(&note, |_| Err("no synthetic attachments".into())).unwrap();
    assert_eq!(html.matches("class=\"math-source ").count(), 3);
    assert!(html.contains("data-display=\"false\""));
    assert!(html.contains("data-display=\"true\""));
    assert!(html.contains("href=\"https://example.test/reference?q=one&amp;n=two\""));
    assert!(html.contains("\\(\\unknowncmd{x}\\)"));
    assert!(html.contains("<code>\\(代码\\)</code>"));
    assert!(html.contains("\\[代码公式\\]"));
    assert!(html.contains("$100 and $200"));
    assert!(!html.contains("<script>alert(27)</script>"));
    assert!(html.contains("&lt;script&gt;alert(27)&lt;/script&gt;"));
    assert_eq!(
        serde_json::to_string(&note).unwrap(),
        stored_before_rendering
    );
    assert!(!client.knowledge_ai_pending);
    assert_eq!(client.task_supervisor.active_count(), 0);
    drop(client);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "Interactive native reading acceptance with an explicit new isolated directory; no provider request"]
fn desktop_ai_native_reader_acceptance() {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    let root = PathBuf::from(
        std::env::var_os("WINDOWS_AI_READER_ACCEPTANCE_ROOT").expect("explicit isolated root"),
    );
    fs::create_dir(&root).expect("acceptance root must be new");
    let mut client = desktop_ai_query_client(&root);
    client.sync.ai_api_key.clear();
    client.sync.token.clear();
    client.sync.server_url.clear();
    client.settings.timer_bell_enabled = false;
    client.settings.close_to_tray = false;
    client.open_desktop_ai_entry(QueryMode::Direct);
    let content = r#"# 合成排版验收样例（未调用模型）

**复利公式**：本金 \(P\)，最终金额为
\[
A = P \times (1+r)^n
\]

分式 \(\frac{1}{3}\)，根式 $\sqrt{2}$，上下标 $x_i^2$。

$$\int_0^1 x^2\,dx = \frac{1}{3}$$

| 项目 | 数值 |
| --- | --- |
| 本金 | 10000 |

- 标题、加粗、列表和表格应正确排版。
- 下列代码应保持原样：

```text
\[不会把代码块识别成公式\]
```

无法解析的公式应保留原文：\(\invalidSyntheticMacro{x}\)。

<script>window.syntheticInjection=true</script>
"#;
    client.knowledge_ai_answer = Some(KnowledgeAiAnswer {
        workspace_binding: client.background_job_workspace_fingerprint(),
        recipient_host: "offline-render-test".into(),
        model: "synthetic-format-fixture".into(),
        question: "合成排版验收".into(),
        content: content.into(),
        source_ids: Vec::new(),
        source_titles: Vec::new(),
        source_folders: Vec::new(),
        received_at_epoch_millis: now_millis(),
    });
    client.open_desktop_ai_answer_reader();
    assert!(client.rich_editor.active);
    assert!(!client.knowledge_ai_pending);
    assert_eq!(client.task_supervisor.active_count(), 0);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 820.0])
            .with_position([30.0, 30.0]),
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_any_thread(true);
        })),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "十倍率 · 离线排版验收（无模型请求）",
        options,
        Box::new(move |cc| {
            install_ui_fonts(&cc.egui_ctx);
            install_ui_style(&cc.egui_ctx);
            Box::new(client)
        }),
    )
    .unwrap();
}
