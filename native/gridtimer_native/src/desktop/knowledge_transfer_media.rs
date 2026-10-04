// v2.22.52 - Portable workspace images and files are verified before any page commits.

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KnowledgeTransferMedia {
    attachment: gridtimer_native::desktop_note_media::NoteAttachmentMetadata,
    content_base64: String,
}

fn prepare_knowledge_transfer(
    value: &Value,
) -> Result<(Vec<Value>, Vec<KnowledgeTransferMedia>), String> {
    use base64::Engine as _;
    use gridtimer_native::desktop_note_media::DesktopNoteMediaStore;
    if value["format"] != "tenfold-knowledge"
        || value["version"].as_u64() != Some(1)
        || value.as_object().is_none_or(|v| {
            v.keys()
                .any(|k| !matches!(k.as_str(), "format" | "version" | "pages" | "media"))
        })
    {
        return Err("不是受支持的知识工作区文件".into());
    }
    let pages = value["pages"].as_array().ok_or("缺少页面列表")?;
    let mut media: Vec<KnowledgeTransferMedia> = if value["media"].is_null() {
        Vec::new()
    } else {
        serde_json::from_value(value["media"].clone()).map_err(|e| e.to_string())?
    };
    if media.len() > 1000 {
        return Err("工作区附件过多".into());
    }
    let mut seen = HashSet::new();
    let mut bytes_total = 0_usize;
    for item in &media {
        if !seen.insert(item.attachment.id.clone()) || item.content_base64.len() > 24 * 1024 * 1024
        {
            return Err("附件重复或超过容量限制".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&item.content_base64)
            .map_err(|_| "附件编码无效")?;
        bytes_total = bytes_total.saturating_add(bytes.len());
        if bytes_total > 48 * 1024 * 1024 {
            return Err("工作区附件超过 48 MB".into());
        }
        DesktopNoteMediaStore::validate_import_blob(&item.attachment, &bytes)
            .map_err(|e| e.to_string())?;
    }
    let mut pages = knowledge::remap_pages(pages, random_desktop_identifier)?;
    let map = media
        .iter()
        .map(|m| {
            (
                m.attachment.id.clone(),
                random_desktop_identifier("attachment"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let metadata = media
        .iter()
        .map(|m| (m.attachment.id.clone(), &m.attachment))
        .collect::<BTreeMap<_, _>>();
    let mut referenced = HashSet::new();
    for page in &mut pages {
        if let Some(attachments) = page["attachments"].as_array_mut() {
            for attachment in attachments {
                let id = attachment["id"].as_str().ok_or("附件编号无效")?.to_string();
                let expected = metadata
                    .get(&id)
                    .ok_or("导入文件缺少附件内容，请重新导出含附件的工作区")?;
                if attachment["sha256"] != expected.sha256
                    || attachment["sizeBytes"].as_i64() != Some(expected.size_bytes)
                    || attachment["mimeType"] != expected.mime_type
                    || attachment["kind"] != expected.kind
                    || attachment["width"].as_i64().unwrap_or(0) != i64::from(expected.width)
                    || attachment["height"].as_i64().unwrap_or(0) != i64::from(expected.height)
                {
                    return Err("页面与附件信息不一致".into());
                }
                referenced.insert(id.clone());
                attachment["id"] = json!(map[&id]);
                attachment["fileName"] = json!(format!("{}.blob", map[&id]));
            }
        }
        if let Some(blocks) = page["document"]["blocks"].as_array_mut() {
            for block in blocks {
                if let Some(id) = block["attachmentId"].as_str() {
                    block["attachmentId"] = json!(map.get(id).ok_or("内容块缺少附件")?);
                }
                if let Some(text) = block["text"].as_str() {
                    let mut text = text.to_string();
                    for (old, new) in &map {
                        for quote in ['"', '\''] {
                            for prefix in ["src=", "data-note-image="] {
                                let scheme = if prefix == "src=" {
                                    "note-image://"
                                } else {
                                    ""
                                };
                                text = text.replace(
                                    &format!("{prefix}{quote}{scheme}{old}{quote}"),
                                    &format!("{prefix}{quote}{scheme}{new}{quote}"),
                                );
                            }
                        }
                    }
                    block["text"] = json!(text);
                }
            }
        }
    }
    if referenced.len() != media.len() {
        return Err("工作区含有未引用的附件".into());
    }
    drop(metadata);
    for item in &mut media {
        item.attachment.id = map[&item.attachment.id].clone();
        item.attachment.file_name = format!("{}.blob", item.attachment.id);
    }
    Ok((pages, media))
}

impl TimerWindowsClient {
    fn knowledge_transfer_json(&self, pages: Vec<Value>) -> Result<String, String> {
        use base64::Engine as _;
        let store = if pages
            .iter()
            .any(|p| p["attachments"].as_array().is_some_and(|a| !a.is_empty()))
        {
            Some(self.note_media_store()?)
        } else {
            None
        };
        let mut media = BTreeMap::new();
        let mut total = 0_usize;
        for page in &pages {
            for value in page["attachments"].as_array().into_iter().flatten() {
                let attachment: gridtimer_native::desktop_note_media::NoteAttachmentMetadata =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                if let Some(existing) = media.get(&attachment.id) {
                    let existing: &KnowledgeTransferMedia = existing;
                    if existing.attachment.sha256 != attachment.sha256 {
                        return Err("同一附件编号指向不同内容".into());
                    }
                    continue;
                }
                let bytes = store
                    .as_ref()
                    .unwrap()
                    .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
                    .map_err(|e| e.to_string())?;
                total += bytes.len();
                if total > 48 * 1024 * 1024 {
                    return Err("附件超过工作区导出上限，请分子页面导出".into());
                }
                let content_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                media.insert(
                    attachment.id.clone(),
                    KnowledgeTransferMedia {
                        attachment,
                        content_base64,
                    },
                );
            }
        }
        let result = serde_json::to_string(&json!({"format":"tenfold-knowledge","version":1,"pages":pages,"media":media.into_values().collect::<Vec<_>>()})).map_err(|e|e.to_string())?;
        if result.len() > 64 * 1024 * 1024 {
            return Err("工作区导出超过 64 MB，请分子页面导出".into());
        }
        Ok(result)
    }

    fn commit_knowledge_import(
        &mut self,
        pages: &[Value],
        media: &[KnowledgeTransferMedia],
    ) -> bool {
        use base64::Engine as _;
        if self.flush_note_draft().is_err() {
            return false;
        }
        let Some(next) = app_data::upsert_knowledge_pages_app_data_json(
            &self.state_json,
            &serde_json::to_string(pages).unwrap_or_default(),
            now_millis(),
        ) else {
            self.status = "导入内容校验失败，原数据未改变".into();
            return false;
        };
        if media.is_empty() {
            return self.replace_state(Some(next), "资料已导入");
        }
        let store = match self.note_media_store() {
            Ok(s) => s,
            Err(e) => {
                self.status = e;
                return false;
            }
        };
        let mut pending = Vec::new();
        for item in media {
            let result = base64::engine::general_purpose::STANDARD
                .decode(&item.content_base64)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    store
                        .begin_imported_blob(&item.attachment, &bytes)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(p) => pending.push(p),
                Err(e) => {
                    for p in &pending {
                        let _ = store.cancel_import(p);
                    }
                    self.status = format!("附件导入失败：{e}");
                    return false;
                }
            }
        }
        match self.replace_state_outcome(Some(next), "资料已导入") {
            StateReplaceOutcome::NotCommitted => {
                for p in &pending {
                    if let Err(e) = store.cancel_import(p) {
                        self.status = append_status(self.status.clone(), e.to_string());
                    }
                }
                false
            }
            StateReplaceOutcome::CommittedWithWarning => true,
            StateReplaceOutcome::Committed => {
                for p in &pending {
                    if let Err(e) = store.commit_import(p) {
                        self.status = format!("页面已保存，附件将从事务日志恢复：{e}");
                    }
                }
                true
            }
        }
    }
}
