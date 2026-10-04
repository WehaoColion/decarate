// v2.22.52 - Structured knowledge workspace.
// v2.22.39 - Preserve rejection feedback, unchanged documents, Android formatting and metadata.

#[derive(Default)]
struct DesktopRichEditor {
    read_only: bool,
    active: bool,
    ready: bool,
    started: Option<Instant>,
    diagnostic_requested: bool,
    note_id: String,
    workspace: String,
    page: Option<String>,
    draft: Option<DesktopNoteDocument>,
    images: BTreeMap<String, (String, Vec<u8>)>,
    receiver: Option<mpsc::Receiver<String>>,
    close_after_save: bool,
    close_application: bool,
    snapshot_requested: bool,
    save_requested: bool,
    applied_bounds: Option<egui::Rect>,
    bounds: Option<egui::Rect>,
    last_status: String,
    message_error: Option<String>,
    #[cfg(target_os = "windows")]
    view: Option<wry::WebView>,
    #[cfg(target_os = "windows")]
    context: Option<wry::WebContext>,
}

#[derive(Deserialize)]
struct RichEditorMessage {
    kind: String,
    #[serde(default)]
    html: String,
    #[serde(default)]
    plain_text: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    request_id: String,
}

fn rich_editor_script_json(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

const RICH_EDITOR_PAGE_URL: &str = "http://note-editor.localhost/";

fn rich_editor_navigation_allowed(url: &str) -> bool {
    url == "about:blank" || url == RICH_EDITOR_PAGE_URL
}

fn build_desktop_rich_editor_page(title: &str, html: &str) -> String {
    let page = include_str!("rich_editor_base.html")
        .replace(
            "img-src note-image: data:",
            "img-src http://note-image.localhost data:",
        )
        .replace(
            "/^note-image:\\/\\/([A-Za-z0-9._-]{1,128})$/i",
            "/^(?:note-image:\\/\\/|http:\\/\\/note-image\\.localhost\\/)([A-Za-z0-9._-]{1,128})$/i",
        )
        .replace("'note-image://'", "'http://note-image.localhost/'");
    let theme = match ACTIVE_DESKTOP_THEME.load(AtomicOrdering::Relaxed) {
        2 => "dark",
        3 => "oled",
        _ => "light",
    };
    let initial = json!({"title":title, "theme":theme, "html":html.replace("note-image://", "http://note-image.localhost/")});
    let bridge = include_str!("rich_editor_bridge.html").replace(
        "__RICH_EDITOR_INITIAL__",
        &rich_editor_script_json(&initial),
    );
    let script_start = bridge
        .find("<script nonce=")
        .expect("trusted editor bridge script");
    let (toolbar, script) = bridge.split_at(script_start);
    page.replace("<body>", &format!("<body>{toolbar}"))
        .replace("</body>", &format!("{script}</body>"))
}

fn rich_editor_document(
    original: &DesktopNoteDocument,
    html: &str,
    plain_text: &str,
    attachment_ids: &HashSet<String>,
) -> Result<DesktopNoteDocument, String> {
    if html.len() > 1_048_576 || plain_text.len() > 1_048_576 {
        return Err("内容超过单篇编辑上限，修改仍保留在编辑器中".into());
    }
    let html = html.replace("http://note-image.localhost/", "note-image://");
    let html = gridtimer_native::sanitize_desktop_rich_text_html(&html);
    if html.contains("data-gridtimer-security-blocked=\"oversized\"") {
        return Err("内容超出安全保存上限".into());
    }
    for id in gridtimer_native::desktop_rich_text_attachment_ids(&html) {
        if !attachment_ids.contains(&id) {
            return Err("图片不属于当前文档，未保存该修改".into());
        }
    }
    let mut document = original.clone();
    let mut text_block = document
        .blocks
        .iter()
        .find(|b| block_is_plain_text(b))
        .cloned()
        .unwrap_or_default();
    if text_block.id.is_empty() {
        text_block.id = new_desktop_note_block_id();
    }
    text_block.block_type = "TEXT".into();
    text_block.text = html;
    document.blocks.retain(|b| !block_is_plain_text(b));
    document.blocks.insert(0, text_block);
    document.rich_text_enabled = true;
    document.markdown_enabled = false;
    document.rich_text_plain_text = plain_text.to_string();
    Ok(document)
}

fn apply_rich_document_to_note_value(value: &mut Value, document: Option<&DesktopNoteDocument>) {
    if let Some(document) = document {
        value["document"] = serde_json::to_value(document).unwrap_or(Value::Null);
        value["content"] = json!(document.rich_text_plain_text);
    }
}

fn rich_editor_initial_html(note: Option<&DesktopNote>, plain_draft: &str) -> String {
    if let Some(note) = note.filter(|note| note.document.rich_text_enabled) {
        return note
            .document
            .blocks
            .iter()
            .filter(|block| block_is_plain_text(block))
            .map(|block| gridtimer_native::sanitize_desktop_rich_text_html(&block.text))
            .collect::<Vec<_>>()
            .join("\n");
    }
    // Canvas previews describe other blocks too; those must not become duplicate prose.
    let source = note
        .filter(|note| !note.document.blocks.is_empty())
        .map(|note| {
            note.document
                .blocks
                .iter()
                .filter(|block| block_is_plain_text(block))
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_else(|| plain_draft.to_string());
    let markdown = note.is_some_and(|note| note.document.markdown_enabled)
        || source.lines().any(looks_like_markdown_note_line);
    gridtimer_native::render_desktop_rich_text_body(&source, markdown)
}

impl TimerWindowsClient {
    fn rich_document_for_save(&self) -> Option<&DesktopNoteDocument> {
        (self.rich_editor.note_id == self.selected_note_id)
            .then_some(self.rich_editor.draft.as_ref())
            .flatten()
    }

    fn open_rich_editor(&mut self) {
        if self.note_blocks_draft.iter().any(|b| b.knowledge.is_some()) {
            self.status = "请在块编辑区修改此页面，阅读视图可查看完整排版".into();
            return;
        }
        if self.workspace_edit_locked()
            || self.knowledge_page_locked()
            || !self.selected_note_version_id.is_empty()
            || self.note_trash_mode
            || self.knowledge_trash_mode
        {
            self.status = "请先返回当前可编辑文档".into();
            return;
        }
        if let Err(error) = self.flush_document_operation_drafts() {
            self.status = format!("请先完成保存：{error}");
            return;
        }
        if self.selected_note_id.is_empty() {
            self.selected_note_id = random_desktop_identifier("note");
        }
        let note = self.selected_note();
        let html = rich_editor_initial_html(note.as_ref(), &self.note_content_draft);
        let mut images = BTreeMap::new();
        if let Some(note) = &note {
            use gridtimer_native::desktop_note_media::{
                ExistingBoundNoteMediaProbe, ReadOnlyDesktopNoteMediaStore,
            };
            let workspace = self.ai_workspace_identity();
            let identity = workspace_note_media_identity(&workspace);
            let root = workspace_note_media_root(
                &workspace.namespace_root,
                &stable_workspace_note_media_key(&identity),
            );
            if let Ok(ExistingBoundNoteMediaProbe::Present(store)) =
                ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&root, &identity)
            {
                let mut total = 0_u64;
                for attachment in &note.attachments {
                    if !attachment.kind.eq_ignore_ascii_case("IMAGE") {
                        continue;
                    }
                    if !matches!(
                        attachment.mime_type.as_str(),
                        "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/bmp"
                    ) {
                        continue;
                    }
                    let size = attachment.size_bytes.max(0) as u64;
                    total = total.saturating_add(size);
                    if total > NOTE_HTML_MAX_IMAGE_BYTES {
                        break;
                    }
                    if let Ok(bytes) =
                        store.read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
                    {
                        images.insert(attachment.id.clone(), (attachment.mime_type.clone(), bytes));
                    }
                }
            }
        }
        self.rich_editor = DesktopRichEditor {
            active: true,
            started: Some(Instant::now()),
            note_id: self.selected_note_id.clone(),
            workspace: self.background_job_workspace_fingerprint(),
            page: Some(build_desktop_rich_editor_page(
                &self.note_title_draft,
                &html,
            )),
            images,
            ..Default::default()
        };
        self.status = "富文本编辑".into();
    }

    fn accept_rich_editor_message(&mut self, message: RichEditorMessage) -> Result<(), String> {
        if !self.rich_editor.active
            || self.selected_note_id != self.rich_editor.note_id
            || self.background_job_workspace_fingerprint() != self.rich_editor.workspace
        {
            return Err("文档工作区已经变化，编辑内容未写入其他账户".into());
        }
        if self.rich_editor.read_only {
            match message.kind.as_str() {
                "reader-close" => {
                    self.rich_editor = DesktopRichEditor::default();
                    return Ok(());
                }
                "reader-open" => {
                    if !self.data.notes.iter().any(|p| {
                        p.id == message.title
                            && p.encryption.is_none()
                            && p.deleted_at_epoch_millis.is_none()
                    }) {
                        return Err("引用的页面不可用".into());
                    }
                    self.rich_editor = DesktopRichEditor::default();
                    self.select_note_by_id(&message.title);
                    if !message.request_id.is_empty() {
                        self.desktop_ui.parity.outline_target = Some(message.request_id);
                    }
                    return Ok(());
                }
                "reader-file" => {
                    self.desktop_ui.knowledge.extract_attachment = Some(message.title);
                    return Ok(());
                }
                "reader-external" => {
                    if knowledge::safe_web_url(&message.title) {
                        self.desktop_ui.knowledge.external_url = Some(message.title);
                        return Ok(());
                    }
                    return Err("链接无效".into());
                }
                "ready" => {}
                _ => return Err("阅读视图不能修改文档".into()),
            }
        }
        if message.kind == "oversized" {
            return Err("内容超过单篇编辑上限，修改仍保留在编辑器中".into());
        }
        if message.kind == "composition-pending" {
            return Err("请先完成输入法中的文字，再保存".into());
        }
        if message.kind == "ready" {
            self.rich_editor.ready = true;
            #[cfg(target_os = "windows")]
            if let Some(view) = &self.rich_editor.view {
                let _ = view.set_visible(true);
                let _ = view.focus();
                if self.rich_editor.read_only {
                    let _ =
                        view.evaluate_script("document.getElementById('reader-close')?.focus()");
                }
                let _ = view.evaluate_script("window.SmartisanRichText.focusEditor()");
            }
            return Ok(());
        }
        if !matches!(message.kind.as_str(), "change" | "commit") {
            return Err("编辑器返回了无效消息".into());
        }
        let note = self.selected_note();
        if self.knowledge_page_locked() {
            return Err("文档已锁定，编辑内容仍保留在窗口中".into());
        }
        let original = self
            .rich_editor
            .draft
            .clone()
            .or_else(|| note.as_ref().map(|n| n.document.clone()))
            .unwrap_or_default();
        let ids = note
            .as_ref()
            .map(|n| {
                n.attachments
                    .iter()
                    .map(|a| a.id.clone())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let document = rich_editor_document(&original, &message.html, &message.plain_text, &ids)?;
        let title = message
            .title
            .replace(['\r', '\n'], " ")
            .chars()
            .take(120)
            .collect::<String>();
        let untouched_new_document = note.is_none()
            && self.rich_editor.draft.is_none()
            && !self.note_dirty
            && title.trim().is_empty()
            && document.rich_text_plain_text.trim().is_empty()
            && document.blocks.iter().all(block_is_plain_text)
            && gridtimer_native::desktop_rich_text_attachment_ids(&message.html).is_empty();
        let changed =
            !untouched_new_document && (original != document || self.note_title_draft != title);
        if changed {
            self.note_title_draft = title;
            self.note_content_draft = document.rich_text_plain_text.clone();
            self.note_blocks_draft = document.blocks.clone();
            self.rich_editor.draft = Some(document);
            self.mark_note_dirty();
        }
        // The browser may already have displayed "unsaved" for this event.
        // Republish the host's result even when the snapshot is unchanged.
        self.rich_editor.message_error = None;
        self.rich_editor.last_status.clear();
        if message.kind == "commit" {
            self.rich_editor.snapshot_requested = false;
            self.rich_editor.save_requested = true;
            if matches!(message.request_id.as_str(), "close" | "shutdown") {
                self.rich_editor.close_after_save = true;
            }
            if message.request_id == "shutdown" {
                self.rich_editor.close_application = true;
            }
        }
        Ok(())
    }

    fn poll_rich_editor_events(&mut self, ctx: &egui::Context) {
        if self.rich_editor.read_only && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.rich_editor = DesktopRichEditor::default();
            return;
        }
        if let Some(id) = self.desktop_ui.knowledge.extract_attachment.take() {
            self.export_knowledge_attachment(&id);
        }
        if let Some(url) = self.desktop_ui.knowledge.external_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        let messages = self
            .rich_editor
            .receiver
            .as_ref()
            .map(|rx| rx.try_iter().take(32).collect::<Vec<_>>())
            .unwrap_or_default();
        for raw in messages {
            let parsed = if raw.len() <= 2_200_000 {
                serde_json::from_str::<RichEditorMessage>(&raw)
                    .map_err(|_| "编辑器返回内容无效".to_string())
            } else {
                Err("编辑器返回内容过大".into())
            };
            let result = parsed.and_then(|message| self.accept_rich_editor_message(message));
            if let Err(error) = result {
                self.status = error.clone();
                self.rich_editor.message_error = Some(error);
                self.rich_editor.last_status.clear();
                self.rich_editor.close_after_save = false;
                self.rich_editor.close_application = false;
                self.rich_editor.snapshot_requested = false;
                self.rich_editor.save_requested = false;
                self.unlock_rich_editor_after_failure();
            }
        }
        if !self.rich_editor.active {
            return;
        }
        if !self.rich_editor.ready
            && self
                .rich_editor
                .started
                .is_some_and(|start| start.elapsed() > Duration::from_secs(15))
        {
            let close_application = self.rich_editor.close_application;
            self.rich_editor = DesktopRichEditor::default();
            self.status = "富文本编辑器未能加载，文档已保留，请重新打开".into();
            if close_application {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        if self.rich_editor.save_requested {
            self.rich_editor.save_requested = false;
            self.submit_draft_snapshot(ctx, true);
        }
        if self.rich_editor.close_application && !self.rich_editor.close_after_save {
            self.request_rich_editor_snapshot("shutdown");
        }
        if self.rich_editor.close_after_save {
            if self.note_save_state == DesktopDocumentSaveState::Failed {
                self.rich_editor.close_after_save = false;
                self.rich_editor.close_application = false;
                self.unlock_rich_editor_after_failure();
            } else if self.note_dirty || self.persistence.pending() {
                self.submit_draft_snapshot(ctx, true);
            } else {
                let close_application = self.rich_editor.close_application;
                self.rich_editor = DesktopRichEditor::default();
                if let Some(note) = self.selected_note() {
                    self.load_note_draft_without_flush(&note);
                }
                if close_application {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
        }
        let status = if let Some(error) = &self.rich_editor.message_error {
            error.clone()
        } else if self.note_save_state == DesktopDocumentSaveState::Failed {
            self.status.clone()
        } else if self.persistence.pending() {
            "保存中".into()
        } else if self.note_dirty {
            "未保存".into()
        } else {
            "已保存".into()
        };
        if self.rich_editor.last_status != status {
            #[cfg(target_os = "windows")]
            if let Some(view) = &self.rich_editor.view {
                let message = rich_editor_script_json(&json!(status));
                let _ = view.evaluate_script(&format!(
                    "window.setHostStatus && window.setHostStatus({message});"
                ));
            }
            self.rich_editor.last_status = status;
        }
    }

    fn unlock_rich_editor_after_failure(&self) {
        #[cfg(target_os = "windows")]
        if let Some(view) = &self.rich_editor.view {
            let _ = view.evaluate_script(
                "window.unlockEditorAfterSaveFailure && window.unlockEditorAfterSaveFailure();",
            );
        }
    }

    fn request_rich_editor_snapshot(&mut self, request: &str) {
        if self.rich_editor.read_only {
            self.rich_editor.close_after_save = request != "save";
            return;
        }
        if !self.rich_editor.ready || self.rich_editor.snapshot_requested {
            return;
        }
        #[cfg(target_os = "windows")]
        if let Some(view) = &self.rich_editor.view {
            let request = rich_editor_script_json(&json!(request));
            if view
                .evaluate_script(&format!("window.requestHostSnapshot({request});"))
                .is_ok()
            {
                self.rich_editor.snapshot_requested = true;
            }
        }
    }

    fn sync_rich_editor_webview(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        #[cfg(target_os = "windows")]
        {
            use wry::WebViewBuilderExtWindows;
            if !self.rich_editor.active {
                return;
            }
            let Some(rect) = self.rich_editor.bounds else {
                return;
            };
            let scale = ctx.pixels_per_point();
            let bounds = wry::Rect {
                position: wry::dpi::PhysicalPosition::new(
                    (rect.min.x * scale).round() as i32,
                    (rect.min.y * scale).round() as i32,
                )
                .into(),
                size: wry::dpi::PhysicalSize::new(
                    (rect.width() * scale).max(1.0).round() as u32,
                    (rect.height() * scale).max(1.0).round() as u32,
                )
                .into(),
            };
            if let Some(view) = &self.rich_editor.view {
                if !self.rich_editor.ready
                    && !self.rich_editor.diagnostic_requested
                    && self
                        .rich_editor
                        .started
                        .is_some_and(|start| start.elapsed() > Duration::from_secs(3))
                {
                    self.rich_editor.diagnostic_requested = true;
                    let _ = view.evaluate_script_with_callback("JSON.stringify({ready:document.readyState,bodyChildren:document.body?.childElementCount??-1,hasEditor:!!document.getElementById('editor'),hasBridge:typeof window.AndroidRichText,hasIpc:typeof window.ipc})", |value| {
                        append_client_runtime_log(&format!("RICH_EDITOR_INIT {value}"));
                    });
                }
                if self.rich_editor.applied_bounds != Some(rect) {
                    let _ = view.set_bounds(bounds);
                    self.rich_editor.applied_bounds = Some(rect);
                }
                return;
            }
            let Some(page) = self.rich_editor.page.take() else {
                return;
            };
            let images = std::mem::take(&mut self.rich_editor.images);
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            let cache = self
                .ai_workspace_identity()
                .namespace_root
                .join("rich_editor_cache");
            let mut context = wry::WebContext::new(Some(cache));
            let built = wry::WebViewBuilder::new_with_web_context(&mut context)
                .with_url(RICH_EDITOR_PAGE_URL)
                .with_custom_protocol("note-editor".into(), move |_, request| {
                    let valid =
                        request.uri().path() == "/" && request.method() == wry::http::Method::GET;
                    wry::http::Response::builder()
                        .status(if valid { 200 } else { 404 })
                        .header("Content-Type", "text/html; charset=utf-8")
                        .header("Cache-Control", "no-store")
                        .header("X-Content-Type-Options", "nosniff")
                        .body(std::borrow::Cow::Owned(if valid {
                            page.as_bytes().to_vec()
                        } else {
                            Vec::new()
                        }))
                        .unwrap()
                })
                .with_visible(false)
                .with_focused(false)
                .with_bounds(bounds)
                .with_incognito(true)
                .with_devtools(false)
                .with_general_autofill_enabled(false)
                .with_hotkeys_zoom(false)
                .with_browser_accelerator_keys(false)
                .with_permission_handler(|_| wry::PermissionResponse::Deny)
                .with_navigation_handler(move |url| {
                    let allowed = rich_editor_navigation_allowed(&url);
                    append_client_runtime_log(&format!(
                        "RICH_EDITOR_NAV allowed={allowed} scheme={} chars={}",
                        url.split(':').next().unwrap_or(""),
                        url.len()
                    ));
                    allowed
                })
                .with_on_page_load_handler(|event, url| {
                    let event = match event {
                        wry::PageLoadEvent::Started => "started",
                        wry::PageLoadEvent::Finished => "finished",
                    };
                    append_client_runtime_log(&format!(
                        "RICH_EDITOR_PAGE event={event} scheme={} chars={}",
                        url.split(':').next().unwrap_or(""),
                        url.len()
                    ));
                })
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false)
                .with_custom_protocol("note-image".into(), move |_, request| {
                    let id = request.uri().path().trim_start_matches('/');
                    let image = images.get(id);
                    let (status, mime, bytes) = match image {
                        Some((mime, bytes)) => (200, mime.as_str(), bytes.clone()),
                        None => (404, "text/plain", Vec::new()),
                    };
                    wry::http::Response::builder()
                        .status(status)
                        .header("Content-Type", mime)
                        .header("X-Content-Type-Options", "nosniff")
                        .body(std::borrow::Cow::Owned(bytes))
                        .unwrap()
                })
                .with_ipc_handler(move |request| {
                    if request.uri().host() != Some("note-editor.localhost")
                        && request.uri().scheme_str() != Some("note-editor")
                    {
                        return;
                    }
                    if request.body().len() <= 2_200_000 {
                        let _ = tx.send(request.body().clone());
                    } else {
                        let _ = tx.send(String::from("{\"kind\":\"oversized\"}"));
                    }
                    repaint.request_repaint();
                })
                .build_as_child(frame);
            match built {
                Ok(view) => {
                    self.rich_editor.started = Some(Instant::now());
                    self.rich_editor.receiver = Some(rx);
                    self.rich_editor.view = Some(view);
                    self.rich_editor.context = Some(context);
                    self.rich_editor.applied_bounds = Some(rect);
                }
                Err(error) => {
                    self.rich_editor = DesktopRichEditor::default();
                    self.status = format!("富文本编辑器无法启动，请检查 WebView2 运行时：{error}");
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (ctx, frame);
    }
}
