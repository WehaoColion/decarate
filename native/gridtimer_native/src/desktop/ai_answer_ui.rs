// v1.1.0.7 Windows - Explicit provider requests, immutable send previews and offline formula reading.
#[path = "ai_query_boundary.rs"]
mod desktop_ai_query_boundary;
use desktop_ai_query_boundary::{QueryBinding, QueryBoundary, QueryConsent, QueryMode};

struct DesktopAiPreview {
    binding: QueryBinding,
    consent: QueryConsent,
    mode: QueryMode,
    question: String,
    sources: Vec<(String, String, String, String)>,
    notes_version: u64,
    recipient: String,
    model: String,
}

struct DesktopAiProbeResult {
    workspace: String,
    configuration: String,
    result: ai_client::AiConnectionTestResult,
}

#[derive(Default)]
struct DesktopAiSession {
    last_workspace: Option<String>,
    mode: QueryMode,
    boundary: QueryBoundary,
    preview: Option<DesktopAiPreview>,
    message: String,
    probe_rx: Option<mpsc::Receiver<DesktopAiProbeResult>>,
    probe_binding: Option<(String, String)>,
    probe_message: String,
    probe_ok: bool,
}

fn desktop_ai_configuration_binding(session: &DesktopSyncSession) -> String {
    let value = json!([
        session.ai_base_url.trim().trim_end_matches('/'),
        session.ai_model.trim(),
        session.ai_api_key.trim()
    ]);
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn desktop_ai_configured_recipient(session: &DesktopSyncSession) -> Result<String, String> {
    if session.ai_api_key.trim().is_empty() || session.ai_api_key.chars().any(char::is_control) {
        return Err("请填写有效的 API Key".into());
    }
    if session.ai_model.trim().is_empty() || session.ai_model.chars().any(char::is_control) {
        return Err("请填写模型名称".into());
    }
    let base = session.ai_base_url.trim().trim_end_matches('/');
    if base.is_empty() || base.ends_with("/responses") || base.ends_with("/chat/completions") {
        return Err("请填写接口根地址，不包含 /responses 或 /chat/completions".into());
    }
    ai_client::legal_recipient_host(base)
}

impl TimerWindowsClient {
    fn desktop_ai_current_binding(
        &self,
        mode: QueryMode,
        question: &str,
        sources: &[(String, String, String, String)],
    ) -> QueryBinding {
        QueryBinding {
            workspace: self.background_job_workspace_fingerprint(),
            configuration: desktop_ai_configuration_binding(&self.sync),
            payload: format!(
                "{:x}",
                Sha256::digest(
                    json!([mode.wire(), question, sources])
                        .to_string()
                        .as_bytes()
                )
            ),
        }
    }

    fn invalidate_desktop_ai_configuration(&mut self) {
        self.desktop_ai.preview = None;
        self.desktop_ai.probe_binding = None;
        self.desktop_ai.probe_message.clear();
        self.desktop_ai.probe_ok = false;
        if self.knowledge_ai_pending {
            self.cancel_desktop_ai_query();
        }
    }

    fn cancel_desktop_ai_query(&mut self) {
        self.desktop_ai.preview = None;
        self.desktop_ai.boundary.cancel();
        self.task_supervisor
            .cancel_kinds(&[RuntimeTaskKind::KnowledgeAiRequest]);
        self.desktop_ai.message = if self.knowledge_ai_pending {
            "已取消后续处理，正在等待本次在途请求结束；已发送的请求仍可能计费".into()
        } else {
            "已取消发送预览".into()
        };
    }

    fn observe_desktop_ai_scope(&mut self) {
        let workspace = self.background_job_workspace_fingerprint();
        let configuration = desktop_ai_configuration_binding(&self.sync);
        let previous_workspace = self.desktop_ai.last_workspace.replace(workspace.clone());
        if previous_workspace
            .as_deref()
            .is_some_and(|previous| previous != workspace.as_str())
        {
            self.cancel_desktop_ai_query();
            self.knowledge_ai_question_draft.clear();
            self.knowledge_ai_answer = None;
            self.desktop_ai.probe_binding = None;
            self.desktop_ai.probe_ok = false;
            self.desktop_ai.probe_message.clear();
            if self.rich_editor.read_only && self.rich_editor.workspace != workspace {
                self.rich_editor = DesktopRichEditor::default();
            }
        }
        if self.workspace_edit_locked() {
            self.desktop_ai.preview = None;
            self.desktop_ai.probe_ok = false;
            self.desktop_ai.probe_binding = None;
            if self.desktop_ai.boundary.busy() {
                self.cancel_desktop_ai_query();
            }
        }
        if !self
            .desktop_ai
            .boundary
            .active_scope_matches(&workspace, &configuration)
        {
            self.cancel_desktop_ai_query();
        }
        if self
            .knowledge_ai_answer
            .as_ref()
            .is_some_and(|answer| answer.workspace_binding != workspace)
        {
            self.knowledge_ai_answer = None;
            self.knowledge_ai_question_draft.clear();
        }
        if self.desktop_ai.preview.as_ref().is_some_and(|preview| {
            preview.binding.workspace != workspace
                || preview.binding.configuration != configuration
                || (preview.mode == QueryMode::Knowledge
                    && preview.notes_version != self.notes_cache_version())
        }) {
            self.desktop_ai.preview = None;
            self.desktop_ai.message = "资料或配置已经变化，请重新预览后发送".into();
        }
        if self
            .desktop_ai
            .probe_binding
            .as_ref()
            .is_some_and(|binding| binding != &(workspace.clone(), configuration.clone()))
        {
            self.desktop_ai.probe_binding = None;
            self.desktop_ai.probe_ok = false;
            self.desktop_ai.probe_message.clear();
        }
    }

    fn prepare_desktop_ai_preview(&mut self) -> Result<(), String> {
        self.desktop_ai.preview = None;
        if self.workspace_edit_locked() {
            return Err("账户或资料正在切换，完成后再发送 AI 请求".into());
        }
        if !self.background_work_is_allowed()
            || self.desktop_ai.boundary.busy()
            || self.knowledge_ai_pending
        {
            return Err("上一条请求尚未结束，请稍后再试".into());
        }
        let recipient = desktop_ai_configured_recipient(&self.sync)?;
        if !self.workspace_persistence_ready {
            return Err("本地资料尚未通过核验，未调用模型".into());
        }
        let question = self.knowledge_ai_question_draft.trim().to_owned();
        if question.is_empty()
            || !question
                .chars()
                .any(|ch| !ch.is_whitespace() && !ch.is_control())
        {
            return Err("先输入一个问题".into());
        }
        if question.chars().count() > 1000 {
            return Err("问题最多 1000 个字符，请缩短后发送".into());
        }
        self.flush_note_draft().map_err(|error| error.to_string())?;
        let mode = self.desktop_ai.mode;
        let mut sources = Vec::new();
        if mode == QueryMode::Knowledge {
            let selected = self.data.note_preferences.selected_folder_id.as_deref();
            let folders = self
                .data
                .note_folders
                .iter()
                .map(|folder| (folder.id.as_str(), folder.name.as_str()))
                .collect::<BTreeMap<_, _>>();
            let candidates = self
                .data
                .notes
                .iter()
                .filter(|note| {
                    desktop_note_kind(note) == DesktopNoteKind::Document
                        && note.deleted_at_epoch_millis.is_none()
                        && note.encryption.is_none()
                        && (self.knowledge_ai_scope_all
                            || (selected.is_some() && note.folder_id.as_deref() == selected))
                })
                .map(|note| {
                    (
                        note.id.clone(),
                        note.title.clone(),
                        desktop_note_document_text(&note.content, &note.document),
                        note.folder_id
                            .as_deref()
                            .and_then(|id| folders.get(id))
                            .copied()
                            .unwrap_or("未入库")
                            .to_owned(),
                    )
                })
                .filter(|note| !note.2.trim().is_empty())
                .collect::<Vec<_>>();
            let titles = candidates
                .iter()
                .map(|source| source.1.clone())
                .collect::<Vec<_>>();
            let bodies = candidates
                .iter()
                .map(|source| source.2.clone())
                .collect::<Vec<_>>();
            let folder_names = candidates
                .iter()
                .map(|source| source.3.clone())
                .collect::<Vec<_>>();
            let ranked = gridtimer_native::rank_desktop_knowledge_sources(
                &question,
                &titles,
                &bodies,
                &folder_names,
            );
            let mut remaining = 4500usize;
            for index in ranked.into_iter().take(5) {
                let Some((id, title, body, folder)) = candidates.get(index) else {
                    continue;
                };
                let excerpt = body.chars().take(remaining.min(900)).collect::<String>();
                remaining = remaining.saturating_sub(excerpt.chars().count());
                if !excerpt.trim().is_empty() {
                    sources.push((
                        id.clone(),
                        title.chars().take(1000).collect(),
                        excerpt,
                        folder.chars().take(1000).collect(),
                    ));
                }
                if remaining == 0 {
                    break;
                }
            }
            if sources.is_empty() {
                return Err("所选范围没有可读知识页；可切换“直接问 AI”".into());
            }
        }
        let binding = self.desktop_ai_current_binding(mode, &question, &sources);
        self.desktop_ai.preview = Some(DesktopAiPreview {
            consent: QueryConsent::preview(binding.clone()),
            binding,
            mode,
            question,
            sources,
            notes_version: self.notes_cache_version(),
            recipient,
            model: self.sync.ai_model.trim().to_owned(),
        });
        self.desktop_ai.message = "请核对本次发送范围，再确认发送".into();
        Ok(())
    }

    fn send_desktop_ai_query(&mut self) {
        let Some(mut preview) = self.desktop_ai.preview.take() else {
            return;
        };
        let binding = self.desktop_ai_current_binding(
            self.desktop_ai.mode,
            self.knowledge_ai_question_draft.trim(),
            &preview.sources,
        );
        let ready = self.background_work_is_allowed()
            && !self.workspace_edit_locked()
            && self.workspace_persistence_ready
            && !self.knowledge_ai_pending
            && !self.note_dirty
            && desktop_ai_configured_recipient(&self.sync).is_ok()
            && (preview.mode == QueryMode::Direct
                || preview.notes_version == self.notes_cache_version());
        let Some(request_id) =
            self.desktop_ai
                .boundary
                .begin(&mut preview.consent, &binding, ready)
        else {
            self.desktop_ai.message = "发送条件已经变化，请重新预览资料".into();
            return;
        };
        let origin_workspace = self.ai_workspace_identity();
        let api_key = self.sync.ai_api_key.clone();
        let base_url = self.sync.ai_base_url.clone();
        let model = self.sync.ai_model.clone();
        let question = preview.question;
        let mode = preview.mode;
        let source_ids = preview
            .sources
            .iter()
            .map(|source| source.0.clone())
            .collect::<Vec<_>>();
        let source_titles = preview
            .sources
            .iter()
            .map(|source| source.1.clone())
            .collect::<Vec<_>>();
        let source_excerpts = preview
            .sources
            .iter()
            .map(|source| source.2.clone())
            .collect::<Vec<_>>();
        let source_folders = preview
            .sources
            .iter()
            .map(|source| source.3.clone())
            .collect::<Vec<_>>();
        let recipient_host = preview.recipient;
        let (tx, rx) = mpsc::channel();
        self.knowledge_ai_result_rx = Some(rx);
        self.knowledge_ai_pending = true;
        self.knowledge_ai_answer = None;
        self.desktop_ai.message = "正在向所示服务发送问题并等待 AI 回答…".into();
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::KnowledgeAiRequest,
            TaskDurability::Ephemeral,
            move |cancellation| {
                if cancellation.is_cancelled() {
                    return;
                }
                let result = ai_client::complete_android_query(
                    &api_key,
                    &base_url,
                    &model,
                    mode.wire(),
                    &question,
                    &source_titles,
                    &source_folders,
                    &source_excerpts,
                );
                if !cancellation.is_cancelled() {
                    let _ = tx.send(KnowledgeAiTaskResult {
                        request_id,
                        origin_workspace,
                        recipient_host,
                        model,
                        question,
                        source_ids,
                        source_titles,
                        source_folders,
                        result,
                    });
                }
            },
        ) {
            self.desktop_ai.boundary.finish(request_id, "", "");
            self.knowledge_ai_pending = false;
            self.knowledge_ai_result_rx = None;
            self.desktop_ai.message = format!("AI 请求无法启动：{error}");
        }
    }

    fn open_desktop_ai_answer_reader(&mut self) {
        if self.rich_editor.active || self.flush_document_operation_drafts().is_err() {
            return;
        }
        let Some(answer) = self.knowledge_ai_answer.as_ref() else {
            return;
        };
        let mut page = gridtimer_native::android_answer_render::render_android_ai_answer_html(
            &answer.content,
            palette().is_dark,
        );
        let nonce = page
            .split("<script nonce=\"")
            .nth(1)
            .and_then(|tail| tail.split('"').next())
            .unwrap_or("")
            .to_owned();
        if nonce.is_empty() {
            self.desktop_ai.message = "排版初始化失败，回答原文已保留".into();
            return;
        }
        let background = if palette().is_dark {
            "#16191d"
        } else {
            "#fffdf9"
        };
        let toolbar = format!("<style>body{{background:{background};font-family:'Segoe UI','Microsoft YaHei',sans-serif}}main{{max-width:880px;margin:24px auto;padding:0 28px 60px}}header{{position:sticky;top:0;background:{background};padding:12px 24px;display:flex;align-items:center;justify-content:space-between;border-bottom:1px solid #8798a855}}button{{font:inherit;color:inherit;background:transparent;border:1px solid #8798a888;border-radius:6px;padding:6px 16px;cursor:pointer}}</style><header><span>AI 回答 · {} · {}</span><button id=reader-close>返回问题</button></header>", escape_html_text(&answer.model), escape_html_text(&answer.recipient_host));
        page = page.replace("<body>", &format!("<body>{toolbar}"));
        page = page.replace("</body>", &format!(r#"<script nonce="{nonce}">(()=>{{const send=value=>window.ipc?.postMessage(JSON.stringify(value));document.getElementById('reader-close').addEventListener('click',()=>send({{kind:'reader-close'}}));document.addEventListener('keydown',event=>{{if(event.key==='Escape')send({{kind:'reader-close'}})}});if(document.documentElement.dataset.mathReady==='true')send({{kind:'ready'}});}})();</script></body>"#));
        self.rich_editor = DesktopRichEditor {
            active: true,
            read_only: true,
            started: Some(Instant::now()),
            note_id: self.selected_note_id.clone(),
            workspace: self.background_job_workspace_fingerprint(),
            page: Some(page),
            ..Default::default()
        };
    }

    fn launch_desktop_ai_probe(&mut self) {
        if self.desktop_ai.probe_rx.is_some()
            || !self.background_work_is_allowed()
            || self.workspace_edit_locked()
        {
            return;
        }
        if let Err(error) = desktop_ai_configured_recipient(&self.sync) {
            self.desktop_ai.probe_message = error;
            return;
        }
        if self.flush_sync_draft().is_err() {
            self.desktop_ai.probe_message = self.status.clone();
            return;
        }
        let workspace = self.background_job_workspace_fingerprint();
        let configuration = desktop_ai_configuration_binding(&self.sync);
        let api_key = self.sync.ai_api_key.clone();
        let base_url = self.sync.ai_base_url.clone();
        let model = self.sync.ai_model.clone();
        let (tx, rx) = mpsc::channel();
        self.desktop_ai.probe_rx = Some(rx);
        self.desktop_ai.probe_ok = false;
        self.desktop_ai.probe_message = "正在发送一次合成资料连接测试…".into();
        if let Err(error) = self.task_supervisor.spawn(
            RuntimeTaskKind::HealthCheck,
            TaskDurability::Ephemeral,
            move |cancellation| {
                if cancellation.is_cancelled() {
                    return;
                }
                let result = ai_client::test_connection(&api_key, &base_url, &model);
                if !cancellation.is_cancelled() {
                    let _ = tx.send(DesktopAiProbeResult {
                        workspace,
                        configuration,
                        result,
                    });
                }
            },
        ) {
            self.desktop_ai.probe_rx = None;
            self.desktop_ai.probe_message = format!("连接测试无法启动：{error}");
        }
    }

    fn poll_desktop_ai_probe(&mut self) {
        let Some(rx) = self.desktop_ai.probe_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(task)
                if !self.workspace_edit_locked()
                    && task.workspace == self.background_job_workspace_fingerprint()
                    && task.configuration == desktop_ai_configuration_binding(&self.sync) =>
            {
                self.desktop_ai.probe_ok = task.result.ok;
                self.desktop_ai.probe_binding = Some((task.workspace, task.configuration));
                self.desktop_ai.probe_message = task.result.message;
            }
            Ok(_) => {
                self.desktop_ai.probe_message = "配置或账户已经变化，本次测试结果已丢弃".into();
            }
            Err(mpsc::TryRecvError::Empty) => self.desktop_ai.probe_rx = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.desktop_ai.probe_message = "连接测试已中断，可重新测试".into()
            }
        }
    }

    fn open_desktop_ai_entry(&mut self, mode: QueryMode) {
        self.switch_tab(AppTab::Knowledge);
        if self.tab != AppTab::Knowledge {
            return;
        }
        self.desktop_ui.navigation.editor_visible = false;
        self.knowledge_trash_mode = false;
        self.knowledge_view = KnowledgeCollectionView::Recent;
        self.knowledge_ai_panel_open = true;
        self.desktop_ai.mode = mode;
        self.desktop_ai.preview = None;
    }

    fn ui_desktop_ai_preview(&mut self, ctx: &egui::Context) {
        if self.desktop_ai.preview.is_none() {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("确认发送给 AI").id(egui::Id::new("desktop_ai_send_preview")).collapsible(false).resizable(true).default_width(640.0).show(ctx, |ui| {
            let preview = self.desktop_ai.preview.as_ref().unwrap();
            ui.label(format!("接收方：{} · 模型：{}", preview.recipient, preview.model));
            ui.label(if preview.mode == QueryMode::Direct { "范围：仅你的问题，不发送知识页".into() } else { format!("范围：你的问题 + {} 个知识页节选（最多 5 页，每页最多 900 字；加密及已删除内容不纳入）", preview.sources.len()) });
            let bytes = preview.question.len() + preview.sources.iter().map(|source| source.1.len() + source.2.len() + source.3.len()).sum::<usize>();
            ui.label(format!("资料正文：{bytes} 字节 · 1 次请求 · 会消耗服务商额度"));
            ui.label("资料将发送到以上接收方；请求设置 store: false，服务商仍按自身规则处理与保存资料。");
            ui.horizontal(|ui| {
                confirm = ui.add_enabled(self.workspace_persistence_ready && self.background_work_is_allowed() && !self.workspace_edit_locked() && !self.knowledge_ai_pending, egui::Button::new("确认发送问题")).clicked();
                cancel = ui.button("返回修改").clicked();
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                ui.label(egui::RichText::new("问题").strong()); ui.label(&preview.question);
                for (index, (_, title, excerpt, folder)) in preview.sources.iter().enumerate() {
                    egui::CollapsingHeader::new(format!("[{}] {title} · {folder}", index + 1)).show(ui, |ui| { ui.label(excerpt); });
                }
            });
        });
        if cancel {
            self.desktop_ai.preview = None;
        }
        if confirm {
            self.send_desktop_ai_query();
        }
    }
}
