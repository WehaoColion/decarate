// v1.0.2.2 Windows - Inspect export capabilities without cloning document history.
// v2.22.52 - Preserve structured knowledge pages through editing and persistence.
// v2.22.47 - Add copy, plain text and PNG sharing alongside complete HTML export.

const NOTE_HTML_MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

fn complete_note_html(
    note: &DesktopNote,
    folder_label: &str,
    mut read_image: impl FnMut(&DesktopNoteAttachment) -> Result<Vec<u8>, String>,
) -> Result<String, String> {
    if note.document.knowledge.is_some()
        || note.document.blocks.iter().any(|b| b.knowledge.is_some())
    {
        return knowledge_complete_html(note, read_image);
    }
    use base64::Engine as _;
    let mut blocks = note_canvas_blocks(note);
    let mut referenced = HashSet::new();
    for block in &blocks {
        let kind = if block_is_plain_text(block) {
            "TEXT".to_string()
        } else {
            block.block_type.to_ascii_uppercase()
        };
        if !matches!(kind.as_str(), "TEXT" | "IMAGE" | "CONTACT" | "CALL") {
            return Err(format!("文档含暂不支持导出的内容块：{kind}"));
        }
        if let Some(id) = &block.attachment_id {
            referenced.insert(id.clone());
        }
        if note.document.rich_text_enabled && kind == "TEXT" {
            referenced.extend(gridtimer_native::desktop_rich_text_attachment_ids(
                &block.text,
            ));
        }
    }
    // Legacy notes can store attached images without explicit image blocks.
    for attachment in &note.attachments {
        if attachment.kind.eq_ignore_ascii_case("IMAGE") && referenced.insert(attachment.id.clone())
        {
            blocks.push(DesktopNoteBlock {
                block_type: "IMAGE".into(),
                attachment_id: Some(attachment.id.clone()),
                caption: attachment.display_name.clone(),
                ..Default::default()
            });
        }
    }
    let blocks = blocks.iter().map(|block| {
        let phones = block.contact_phones.iter().map(|phone| json!({
            "label": phone.get("label").and_then(Value::as_str).unwrap_or(""),
            "number": phone.get("number").and_then(Value::as_str).unwrap_or(""),
        })).collect::<Vec<_>>();
        json!({
            "type": if block_is_plain_text(block) { "TEXT".to_string() } else { block.block_type.to_ascii_uppercase() },
            "text": if block.knowledge.is_some() { knowledge::blocks_markdown(&[serde_json::to_value(block).unwrap_or(Value::Null)]) } else { block.text.clone() }, "attachment_id": block.attachment_id, "caption": block.caption,
            "contact_name": block.contact_name, "contact_organization": block.contact_organization,
            "contact_phones": phones,
            "call_phone_number": block.call_phone_number, "call_contact_name": block.call_contact_name,
            "call_direction": block.call_direction,
            "call_occurred_at_label": block.call_occurred_at_epoch_millis.map(desktop_local_timestamp),
            "call_duration_label": block.call_duration_millis.map(|value| format_duration(value.max(0))),
        })
    }).collect::<Vec<_>>();
    let payload = json!({
        "title": if note.title.trim().is_empty() { "未命名文档" } else { note.title.trim() },
        "meta": folder_label, "accent_seed": note.accent_seed,
        "markdown_enabled": note.document.markdown_enabled || note.document.blocks.iter().any(|b|b.knowledge.is_some()),
        "rich_text_enabled": note.document.rich_text_enabled, "blocks": blocks,
    });
    let mut html = gridtimer_native::render_desktop_note_document_html(&payload.to_string())
        .ok_or("文档无法生成 HTML")?;
    if html.contains("data-gridtimer-security-blocked=\"oversized\"") {
        return Err("富文本超出安全渲染上限，未导出不完整内容".into());
    }
    let mut total = 0_u64;
    for id in gridtimer_native::desktop_rich_text_attachment_ids(&html) {
        if id.is_empty()
            || !id
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_' | b'.'))
        {
            return Err("图片标识不符合安全格式，未读取文件".into());
        }
        let attachment = note
            .attachments
            .iter()
            .find(|item| item.id == id && item.kind.eq_ignore_ascii_case("IMAGE"))
            .ok_or_else(|| format!("图片 {id} 缺少当前文档的附件记录"))?;
        let mime = match attachment.mime_type.as_str() {
            "image/png" => "image/png",
            "image/jpeg" | "image/jpg" => "image/jpeg",
            "image/webp" => "image/webp",
            "image/gif" => "image/gif",
            "image/bmp" | "image/x-ms-bmp" => "image/bmp",
            _ => return Err(format!("图片 {id} 的格式无法安全导出")),
        };
        if attachment.size_bytes <= 0 {
            return Err(format!("图片 {id} 缺少有效大小记录"));
        }
        total = total
            .checked_add(attachment.size_bytes as u64)
            .ok_or("图片总大小超出上限")?;
        if total > NOTE_HTML_MAX_IMAGE_BYTES {
            return Err("文档图片总量超过 64 MB，无法导出为单个 HTML 文件".into());
        }
        let bytes =
            read_image(attachment).map_err(|error| format!("图片 {id} 无法导出：{error}"))?;
        // The production reader verifies these too. Keep the export boundary
        // independently strict so future readers cannot publish unchecked bytes.
        if bytes.len() as i64 != attachment.size_bytes
            || format!("{:x}", Sha256::digest(&bytes)) != attachment.sha256.to_ascii_lowercase()
        {
            return Err(format!("图片 {id} 的大小或摘要与文档记录不符"));
        }
        let uri = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        // Match the complete attribute: attachment ids may be prefixes of others.
        html = html.replace(
            &format!("src=\"note-image://{id}\""),
            &format!("src=\"{uri}\""),
        );
    }
    if html.contains("src=\"note-image://") {
        return Err("仍有图片未嵌入，未导出不完整文档".into());
    }
    Ok(html)
}

impl TimerWindowsClient {
    fn export_selected_note_html(&mut self) {
        let note = match self.note_sharing_snapshot() {
            Ok(note) => note,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let folder = note
            .folder_id
            .as_ref()
            .and_then(|id| {
                self.data
                    .note_folders
                    .iter()
                    .find(|folder| &folder.id == id)
            })
            .map(|folder| folder.name.as_str())
            .unwrap_or("");
        let workspace = self.ai_workspace_identity();
        let identity = workspace_note_media_identity(&workspace);
        let root = workspace_note_media_root(
            &workspace.namespace_root,
            &stable_workspace_note_media_key(&identity),
        );
        let mut store = None;
        let html = complete_note_html(&note, folder, |attachment| {
            use gridtimer_native::desktop_note_media::{
                ExistingBoundNoteMediaProbe, ReadOnlyDesktopNoteMediaStore,
            };
            if store.is_none() {
                store = match ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&root, &identity)
                    .map_err(|error| error.to_string())?
                {
                    ExistingBoundNoteMediaProbe::Present(store) => Some(store),
                    ExistingBoundNoteMediaProbe::Missing => {
                        return Err("本机尚未下载文档图片".into())
                    }
                };
            }
            store
                .as_ref()
                .unwrap()
                .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
                .map_err(|error| error.to_string())
        });
        let html = match html {
            Ok(html) => html,
            Err(error) => {
                self.status = format!("未导出：{error}");
                return;
            }
        };
        let name = format!("note_export_{}.html", now_millis());
        match desktop_transfer_file_dialog(true, "导出完整 HTML 文档", &name, "html", "HTML 文档")
        {
            Ok(Some(path)) => match write_transfer_text_verified(&path, &html) {
                Ok(()) => {
                    self.status = format!("文档已完整导出：{}", path.display());
                    self.transfers.last_note_export = Some(path.clone());
                    if let Err(error) = open_local_file(&path) {
                        self.status =
                            append_status(self.status.clone(), format!("自动打开失败：{error}"));
                    }
                }
                Err(error) => self.status = format!("导出失败：{error}"),
            },
            Ok(None) => {}
            Err(error) => self.status = error.to_string(),
        }
    }

    fn ui_note_export_tools(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            self.ui_speech_transcription(ui);
            let can_edit = !self.workspace_edit_locked()
                && !self.selected_note_is_locked()
                && self.selected_note_version_id.is_empty()
                && !self.note_trash_mode
                && !self.knowledge_trash_mode;
            if ui
                .add_enabled(can_edit, egui::Button::new("富文本编辑"))
                .on_hover_text("Ctrl+Shift+E")
                .clicked()
            {
                self.open_rich_editor();
            }
            if self
                .selected_note_ref()
                .is_some_and(|note| note.document.rich_text_enabled)
                && ui
                    .add_enabled(can_edit, egui::Button::new("切到文本"))
                    .clicked()
            {
                self.convert_selected_rich_note_to_text();
            }
            let can_export = !self.selected_note_is_locked()
                && (!self.selected_note_id.is_empty() || self.note_dirty);
            if ui
                .add_enabled(can_export, egui::Button::new("复制内容"))
                .clicked()
            {
                match self.note_sharing_snapshot() {
                    Ok(note) => {
                        ui.output_mut(|o| o.copied_text = note_share_text(&note));
                        self.status = "已复制".into();
                    }
                    Err(error) => self.status = error,
                }
            }
            if ui
                .add_enabled(can_export, egui::Button::new("导出文字"))
                .clicked()
            {
                self.export_note_text();
            }
            if ui
                .add_enabled(
                    can_export && !self.desktop_ui.parity.image_export.busy(),
                    egui::Button::new("导出长图"),
                )
                .clicked()
            {
                self.start_note_image_export();
            }
            if ui
                .add_enabled(can_export, egui::Button::new("导出 HTML"))
                .clicked()
            {
                self.export_selected_note_html();
            }
            if let Some(path) = self.transfers.last_note_export.clone() {
                if ui.button("打开导出文档").clicked() {
                    if let Err(error) = open_local_file(&path) {
                        self.status = error.to_string();
                    }
                }
            }
        });
        ui.add_space(8.0);
    }
}
