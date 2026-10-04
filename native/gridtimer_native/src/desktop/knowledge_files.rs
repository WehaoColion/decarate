// v2.22.52 - Local file blocks share the image store's account-bound transaction journal.

impl TimerWindowsClient {
    fn import_knowledge_file(&mut self, block_id: &str) {
        if self.knowledge_page_locked() || self.flush_note_draft().is_err() {
            return;
        }
        let Some(mut note) = self.selected_note() else {
            return;
        };
        let Some(block) = note.document.blocks.iter().find(|b| b.id == block_id) else {
            return;
        };
        let kind = match block.knowledge.as_ref().map(|m| m.kind) {
            Some(knowledge::BlockKind::File) => "FILE",
            Some(knowledge::BlockKind::Audio) => "AUDIO",
            Some(knowledge::BlockKind::Video) => "VIDEO",
            _ => return,
        };
        if note.encryption.is_some() {
            self.status = "加密页面暂不支持外部附件".into();
            return;
        }
        let path = match desktop_transfer_file_dialog(false, "添加附件", "", "*", "文件") {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        let store = match self.note_media_store() {
            Ok(s) => s,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let pending = match store.begin_file_import(&path, kind) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("附件导入失败：{e}");
                return;
            }
        };
        let attachment: DesktopNoteAttachment = match pending
            .attachment_json()
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(a) => a,
            None => {
                let _ = store.cancel_import(&pending);
                self.status = "附件信息无效".into();
                return;
            }
        };
        let block = note
            .document
            .blocks
            .iter_mut()
            .find(|b| b.id == block_id)
            .unwrap();
        block.attachment_id = Some(attachment.id.clone());
        block.text = attachment.display_name.clone();
        block.knowledge.as_mut().unwrap().url.clear();
        note.attachments.push(attachment);
        let next = app_data::upsert_note_app_data_json(
            &self.state_json,
            &serde_json::to_string(&note).unwrap_or_default(),
            now_millis(),
        );
        match self.replace_state_outcome(next, "附件已添加") {
            StateReplaceOutcome::NotCommitted => {
                if let Err(e) = store.cancel_import(&pending) {
                    self.status = append_status(self.status.clone(), e.to_string());
                }
                return;
            }
            StateReplaceOutcome::CommittedWithWarning => {
                self.refresh_selected_note_after_local_mutation(&note.id);
                return;
            }
            StateReplaceOutcome::Committed => {}
        }
        if let Err(e) = store.commit_import(&pending) {
            self.status = format!("附件引用已保存，待提交日志将恢复文件：{e}");
        }
        self.refresh_selected_note_after_local_mutation(&note.id);
    }

    fn export_knowledge_attachment(&mut self, attachment_id: &str) {
        let Some(note) = self
            .selected_note()
            .filter(|_| !self.selected_note_is_locked())
        else {
            return;
        };
        let Some(attachment) = note.attachments.iter().find(|a| a.id == attachment_id) else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let store = self.note_media_store()?;
            let bytes = store
                .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
                .map_err(|e| e.to_string())?;
            let extension = Path::new(&attachment.display_name)
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| s.len() <= 16 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
                .unwrap_or("bin");
            let name = format!("attachment_{}.{}", attachment.id, extension);
            if let Some(path) = desktop_transfer_file_dialog(true, "保存附件", &name, "*", "文件")
                .map_err(|e| e.to_string())?
            {
                write_knowledge_bytes_verified(&path, &bytes)?;
                self.status = format!("附件已保存：{}", path.display());
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.status = format!("附件保存失败：{e}");
        }
    }
}

fn write_knowledge_bytes_verified(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("目标目录无效")?;
    let staged = parent.join(format!(".{}.tmp", random_desktop_identifier("attachment")));
    let result = (|| -> Result<(), String> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        if fs::read(&staged).map_err(|e| e.to_string())? != bytes {
            return Err("写入校验失败".into());
        }
        // The native Save dialog already confirms an existing destination. Keep
        // the old file until Windows atomically replaces its directory entry.
        atomic_replace_path(&staged, path).map_err(|e| e.to_string())?;
        if fs::read(path).map_err(|e| e.to_string())? != bytes {
            return Err("最终文件校验失败".into());
        }
        Ok(())
    })();
    if result.is_err() && staged.exists() {
        let _ = fs::remove_file(staged);
    }
    result
}
