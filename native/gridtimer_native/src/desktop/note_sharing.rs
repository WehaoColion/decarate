// v2.22.47 - Copy and export selected note versions as text or a generated PNG.

#[derive(Default)]
struct NoteImageExport {
    page: Option<String>,
    path: Option<PathBuf>,
    workspace: String,
    started: Option<Instant>,
    receiver: Option<mpsc::Receiver<String>>,
    #[cfg(target_os = "windows")]
    view: Option<wry::WebView>,
    #[cfg(target_os = "windows")]
    context: Option<wry::WebContext>,
}
impl NoteImageExport {
    fn busy(&self) -> bool {
        self.path.is_some()
    }
}

fn note_share_text(note: &DesktopNote) -> String {
    let body = if note.document.rich_text_enabled {
        desktop_note_rich_text_plain_text(note)
    } else {
        desktop_note_document_text(&note.content, &note.document)
    };
    if note.title.trim().is_empty() {
        body
    } else {
        format!("{}\n\n{}", note.title, body)
    }
}

fn note_image_page(html: &str) -> String {
    let data = rich_editor_script_json(&json!(html));
    // Generate the image from document content; this never captures the screen.
    // Only embedded, verified images are loaded. Output has a bounded pixel area.
    format!(
        r##"<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-note-export'; img-src data:; style-src 'unsafe-inline'"></head><body><script nonce="note-export">
    (async()=>{{try{{
    const doc=new DOMParser().parseFromString({data},'text/html');
    doc.querySelectorAll('script,style,nav').forEach(x=>x.remove());
    doc.querySelectorAll('br').forEach(x=>x.replaceWith(doc.createTextNode('\n')));
    const root=doc.querySelector('article')||doc.querySelector('main')||doc.body;
    const c=document.createElement('canvas'), g=c.getContext('2d');
    const ops=[]; let y=64; const width=1080, pad=64, usable=width-2*pad;
    const text=(s,size=26,bold=false)=>{{
      g.font=`${{bold?'600':'400'}} ${{size}}px 'Microsoft YaHei',sans-serif`;
      const font=g.font; const step=Math.ceil(size*1.6);
      for(const line of s.replace(/\r/g,'').split('\n')){{
        let buffer='';
        for(const ch of line){{
          if(g.measureText(buffer+ch).width>usable && buffer){{ops.push({{text:buffer,y,font}});y+=step;buffer='';}}
          buffer+=ch;
        }}
        ops.push({{text:buffer,y,font}});y+=step;
        if(y>20000) throw Error('内容超过长图高度上限，请导出 HTML');
      }}
    }};
    const walk=async(el)=>{{
      if(el.nodeType===3){{if(el.textContent.trim())text(el.textContent);return;}}
      if(el.nodeType!==1)return;
      const tag=el.tagName;
      if(tag==='IMG'){{
        const src=el.getAttribute('src')||'';
        if(!/^data:image\/(png|jpeg|webp|gif|bmp);base64,/i.test(src))throw Error('图片尚未完整嵌入');
        const img=new Image();img.src=src;await img.decode();
        const w=Math.min(usable,img.naturalWidth),h=img.naturalHeight*w/img.naturalWidth;
        if(!Number.isFinite(h)||h<=0||y+h>20000)throw Error('图片超过长图高度上限');
        ops.push({{img,y,w,h}});y+=h+20;return;
      }}
      if(!el.querySelector('img')&&/^(H[1-6]|P|PRE|BLOCKQUOTE|LI|FIGCAPTION)$/.test(tag)){{
        const heading=/^H[1-6]$/.test(tag);
        text((tag==='LI'?'• ':'')+el.textContent,heading?(tag==='H1'?42:32):26,heading);
        y+=heading?18:10;return;
      }}
      for(const child of el.childNodes)await walk(child);
    }};
    await walk(root);c.width=width;c.height=Math.ceil(y+64);
    g.fillStyle='#fffdf7';g.fillRect(0,0,c.width,c.height);g.textBaseline='top';g.fillStyle='#242424';
    for(const op of ops){{if(op.img)g.drawImage(op.img,pad,op.y,op.w,op.h);else{{g.font=op.font;g.fillText(op.text,pad,op.y);}}}}
    window.ipc.postMessage(JSON.stringify({{png:c.toDataURL('image/png').split(',')[1]}}));
    }}catch(e){{window.ipc.postMessage(JSON.stringify({{error:String(e.message||e)}}));}}}})();
    </script></body></html>"##
    )
}

impl TimerWindowsClient {
    fn convert_selected_rich_note_to_text(&mut self) -> bool {
        if self.workspace_edit_locked()
            || self.selected_note_is_locked()
            || !self.selected_note_version_id.is_empty()
            || self.note_trash_mode
            || self.knowledge_trash_mode
            || self.rich_editor.active
        {
            return false;
        }
        let note = match self.note_sharing_snapshot() {
            Ok(n) => n,
            Err(e) => {
                self.status = e;
                return false;
            }
        };
        if !note.document.rich_text_enabled {
            return false;
        }
        let now = now_millis();
        let Some(mut updated) =
            desktop_note_with_recovery_point(&note, &random_desktop_identifier("revision"), now)
        else {
            return false;
        };
        let plain = desktop_note_rich_text_plain_text(&note);
        let mut blocks = vec![new_desktop_text_block(&plain)];
        blocks.extend(
            note.document
                .blocks
                .iter()
                .filter(|b| !block_is_plain_text(b))
                .cloned(),
        );
        // Retain media records, structured blocks, all versions and the full
        // rich recovery point. Encryption uses the existing unlocked session.
        updated.content = note_canvas_storage_text(&blocks);
        updated.document.blocks = blocks;
        updated.document.rich_text_enabled = false;
        updated.document.rich_text_plain_text.clear();
        updated.document.markdown_enabled = false;
        let Ok(mut raw) = serde_json::to_string(&updated) else {
            return false;
        };
        if note.encryption.is_some() {
            let Some(sealed) =
                gridtimer_native::seal_desktop_note_json(&raw, &self.note_crypto_session_token)
            else {
                return false;
            };
            raw = sealed;
        }
        if self.replace_state(
            app_data::upsert_note_app_data_json(&self.state_json, &raw, now),
            "已切换为文本，原格式保存在恢复记录中",
        ) {
            self.load_note_draft_without_flush(&updated);
            true
        } else {
            false
        }
    }

    fn note_sharing_snapshot(&mut self) -> Result<DesktopNote, String> {
        if self.rich_editor.active {
            return Err("请先保存并关闭富文本编辑器".into());
        }
        if self.selected_note_is_locked() {
            return Err("请先解锁文档".into());
        }
        self.flush_document_operation_drafts()
            .map_err(|e| format!("请先完成保存：{e}"))?;
        let mut note = self.selected_note().ok_or("请先保存文档")?;
        if !self.selected_note_version_id.is_empty() {
            let version = self
                .selected_note_version()
                .ok_or("所选版本已不可用，请重新选择")?;
            note.title = version.title;
            note.content = version.content;
            note.document = version.document;
            note.attachments = version.attachments;
            note.folder_id = version.folder_id;
            note.accent_seed = version.accent_seed;
        }
        Ok(note)
    }

    fn export_note_text(&mut self) {
        let note = match self.note_sharing_snapshot() {
            Ok(n) => n,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        match desktop_transfer_file_dialog(
            true,
            "导出便签文字",
            &format!("note_{}.txt", now_millis()),
            "txt",
            "文本文件",
        ) {
            Ok(Some(path)) => match write_transfer_text_verified(&path, &note_share_text(&note)) {
                Ok(()) => {
                    self.status = format!("已导出：{}", path.display());
                    self.transfers.last_note_export = Some(path);
                }
                Err(e) => self.status = format!("导出失败：{e}"),
            },
            Ok(None) => {}
            Err(e) => self.status = e.to_string(),
        }
    }

    fn note_sharing_html(&self, note: &DesktopNote) -> Result<String, String> {
        use gridtimer_native::desktop_note_media::{
            ExistingBoundNoteMediaProbe, ReadOnlyDesktopNoteMediaStore,
        };
        let workspace = self.ai_workspace_identity();
        let identity = workspace_note_media_identity(&workspace);
        let root = workspace_note_media_root(
            &workspace.namespace_root,
            &stable_workspace_note_media_key(&identity),
        );
        let mut store = None;
        complete_note_html(note, "", |a| {
            if store.is_none() {
                store = match ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&root, &identity)
                    .map_err(|e| e.to_string())?
                {
                    ExistingBoundNoteMediaProbe::Present(s) => Some(s),
                    ExistingBoundNoteMediaProbe::Missing => return Err("本机缺少文档图片".into()),
                };
            }
            store
                .as_ref()
                .unwrap()
                .read_blob(&a.id, &a.sha256, a.size_bytes)
                .map_err(|e| e.to_string())
        })
    }

    fn start_note_image_export(&mut self) {
        if self.desktop_ui.parity.image_export.busy() {
            return;
        }
        let html = match self
            .note_sharing_snapshot()
            .and_then(|n| self.note_sharing_html(&n))
        {
            Ok(h) => h,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        match desktop_transfer_file_dialog(
            true,
            "导出便签长图",
            &format!("note_{}.png", now_millis()),
            "png",
            "PNG 图片",
        ) {
            Ok(Some(path)) => {
                self.desktop_ui.parity.image_export = NoteImageExport {
                    page: Some(note_image_page(&html)),
                    path: Some(path),
                    workspace: self.background_job_workspace_fingerprint(),
                    started: Some(Instant::now()),
                    ..Default::default()
                };
                self.status = "正在生成长图…".into();
            }
            Ok(None) => {}
            Err(e) => self.status = e.to_string(),
        }
    }

    fn poll_note_image_export(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.desktop_ui.parity.image_export.busy() {
            return;
        }
        if self.desktop_ui.parity.image_export.workspace
            != self.background_job_workspace_fingerprint()
        {
            self.desktop_ui.parity.image_export = NoteImageExport::default();
            self.status = "账户已切换，长图导出已取消".into();
            return;
        }
        let export = &mut self.desktop_ui.parity.image_export;
        if export
            .started
            .is_some_and(|t| t.elapsed() > Duration::from_secs(45))
        {
            *export = NoteImageExport::default();
            self.status = "长图生成超时，可重试或导出 HTML".into();
            return;
        }
        if let Some(raw) = export.receiver.as_ref().and_then(|r| r.try_recv().ok()) {
            let path = export.path.clone().unwrap();
            let result = decode_note_image(&raw).and_then(|bytes| {
                atomic_replace_bytes_no_backup(&path, &bytes).map_err(|e| e.to_string())?;
                let written = fs::read(&path).map_err(|e| e.to_string())?;
                if Sha256::digest(&written) != Sha256::digest(&bytes) {
                    return Err("导出文件校验失败".into());
                }
                Ok(())
            });
            *export = NoteImageExport::default();
            match result {
                Ok(()) => {
                    self.status = format!("长图已导出：{}", path.display());
                    self.transfers.last_note_export = Some(path);
                }
                Err(e) => self.status = format!("长图未导出：{e}"),
            }
            return;
        }
        #[cfg(target_os = "windows")]
        if let Some(page) = export.page.take() {
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            let cache = self
                .state_path
                .parent()
                .unwrap_or(Path::new("."))
                .join("note_export_cache");
            let mut context = wry::WebContext::new(Some(cache));
            let built = wry::WebViewBuilder::new_with_web_context(&mut context)
                .with_url("http://note-export.localhost/")
                .with_custom_protocol("note-export".into(), move |_, request| {
                    let valid =
                        request.uri().path() == "/" && request.method() == wry::http::Method::GET;
                    wry::http::Response::builder()
                        .status(if valid { 200 } else { 404 })
                        .header("Content-Type", "text/html; charset=utf-8")
                        .header("Cache-Control", "no-store")
                        .body(std::borrow::Cow::Owned(if valid {
                            page.as_bytes().to_vec()
                        } else {
                            Vec::new()
                        }))
                        .unwrap()
                })
                .with_visible(false)
                .with_focused(false)
                .with_incognito(true)
                .with_devtools(false)
                .with_general_autofill_enabled(false)
                .with_permission_handler(|_| wry::PermissionResponse::Deny)
                .with_navigation_handler(|url| {
                    url == "about:blank" || url == "http://note-export.localhost/"
                })
                .with_on_page_load_handler(|event, _| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        append_client_runtime_log("NOTE_PNG_PAGE_READY");
                    }
                })
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false)
                .with_ipc_handler(move |request| {
                    append_client_runtime_log(&format!(
                        "NOTE_PNG_RECEIVED bytes={}",
                        request.body().len()
                    ));
                    if request.body().len() <= 48 * 1024 * 1024 {
                        let _ = tx.send(request.body().clone());
                        repaint.request_repaint();
                    }
                })
                .build_as_child(frame);
            match built {
                Ok(view) => {
                    export.view = Some(view);
                    export.context = Some(context);
                    export.receiver = Some(rx);
                }
                Err(e) => {
                    *export = NoteImageExport::default();
                    self.status = format!("长图渲染器无法启动：{e}");
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (ctx, frame);
            *export = NoteImageExport::default();
            self.status = "此平台不支持长图渲染".into();
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn decode_note_image(raw: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    if raw.len() > 48 * 1024 * 1024 {
        return Err("长图文件超过上限".into());
    }
    let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if let Some(e) = value.get("error").and_then(Value::as_str) {
        return Err(e.chars().take(300).collect());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(
            value
                .get("png")
                .and_then(Value::as_str)
                .ok_or("渲染器未返回图片")?,
        )
        .map_err(|e| e.to_string())?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("图片格式无效".into());
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if w != 1080 || !(1..=20128).contains(&h) {
        return Err("图片尺寸超出上限".into());
    }
    Ok(bytes)
}
