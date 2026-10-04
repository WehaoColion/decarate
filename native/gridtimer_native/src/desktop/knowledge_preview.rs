// v2.22.52 - Offline reading view with real math, nested content and sandboxed embeds.

fn knowledge_complete_html(
    note: &DesktopNote,
    mut read_image: impl FnMut(&DesktopNoteAttachment) -> Result<Vec<u8>, String>,
) -> Result<String, String> {
    use base64::Engine as _;
    let mut images = BTreeMap::new();
    let mut total = 0_u64;
    for attachment in &note.attachments {
        if attachment.kind.eq_ignore_ascii_case("IMAGE")
            && !matches!(
                attachment.mime_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/bmp"
            )
        {
            return Err("图片格式不受支持".into());
        }
        if !attachment
            .mime_type
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/.-+".contains(&b))
        {
            return Err("附件类型无效".into());
        }
        let bytes = read_image(attachment)?;
        if bytes.len() as i64 != attachment.size_bytes
            || format!("{:x}", Sha256::digest(&bytes)) != attachment.sha256.to_ascii_lowercase()
        {
            return Err("附件内容与记录不一致，未生成不完整文档".into());
        }
        total = total.saturating_add(bytes.len() as u64);
        if total > NOTE_HTML_MAX_IMAGE_BYTES {
            return Err("导出图片超过容量限制".into());
        }
        images.insert(
            attachment.id.clone(),
            format!(
                "data:{};base64,{}",
                attachment.mime_type,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
        );
    }
    let blocks = note_canvas_blocks(note);
    if note.document.rich_text_enabled {
        for block in blocks.iter().filter(|b| b.knowledge.is_none()) {
            for id in gridtimer_native::desktop_rich_text_attachment_ids(&block.text) {
                if !images.contains_key(&id) {
                    return Err("富文本引用了缺失的图片".into());
                }
            }
        }
    }
    let mut body = format!(
        "<h1 class=page-title>{}</h1>",
        escape_html_text(&note.title)
    );
    let mut seen = HashSet::new();
    let mut rendered_images = HashSet::new();
    body.push_str(&knowledge_blocks_html(
        &blocks,
        note.document.rich_text_enabled,
        None,
        None,
        &images,
        &mut rendered_images,
        &mut seen,
        0,
    ));
    for id in gridtimer_native::desktop_rich_text_attachment_ids(&body) {
        let src = images.get(&id).ok_or("图片未嵌入")?;
        body = body.replace(
            &format!("src=\"note-image://{id}\""),
            &format!("src=\"{src}\""),
        );
        rendered_images.insert(id);
    }
    if body.contains("data-gridtimer-security-blocked=\"oversized\"") {
        return Err("文档内容超过安全渲染上限".into());
    }
    for attachment in &note.attachments {
        if attachment.kind.eq_ignore_ascii_case("IMAGE")
            && !rendered_images.contains(&attachment.id)
        {
            if let Some(src) = images.get(&attachment.id) {
                body.push_str(&format!(
                    "<figure><img src=\"{}\"><figcaption>{}</figcaption></figure>",
                    src,
                    escape_html_text(&attachment.display_name)
                ));
            }
        }
    }
    Ok(knowledge_preview_shell(&note.title, &body, false))
}

fn knowledge_blocks_html(
    blocks: &[DesktopNoteBlock],
    rich_text: bool,
    parent: Option<&str>,
    column: Option<u8>,
    images: &BTreeMap<String, String>,
    rendered_images: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    depth: usize,
) -> String {
    if depth > 32 {
        return String::new();
    }
    let mut html = String::new();
    for block in knowledge_block_children(blocks, parent, column) {
        if !seen.insert(block.id.clone()) {
            continue;
        }
        let m = block.knowledge.clone().unwrap_or_default();
        let text = escape_html_text(&block.text);
        let inline = || {
            if rich_text && block.knowledge.is_none() {
                gridtimer_native::sanitize_desktop_rich_text_html(&block.text)
            } else {
                gridtimer_native::render_desktop_rich_text_body(&block.text, true)
            }
        };
        let children = |seen: &mut HashSet<String>, rendered: &mut HashSet<String>| {
            knowledge_blocks_html(
                blocks,
                rich_text,
                Some(&block.id),
                None,
                images,
                rendered,
                seen,
                depth + 1,
            )
        };
        html.push_str(&format!(
            "<section id=\"block-{}\" class=block>",
            escape_html_text(&block.id)
        ));
        if block.block_type.eq_ignore_ascii_case("IMAGE") {
            if let Some(id) = &block.attachment_id {
                if let Some(src) = images.get(id) {
                    rendered_images.insert(id.clone());
                    html.push_str(&format!(
                        "<figure><img src=\"{src}\"><figcaption>{}</figcaption></figure>",
                        escape_html_text(&block.caption)
                    ));
                }
            }
        } else if block.block_type.eq_ignore_ascii_case("CONTACT") {
            html.push_str(&format!(
                "<aside><strong>{}</strong><p>{}</p>",
                escape_html_text(&block.contact_name),
                escape_html_text(&block.contact_organization)
            ));
            for phone in &block.contact_phones {
                html.push_str(&format!(
                    "<p>{} {}</p>",
                    escape_html_text(phone["label"].as_str().unwrap_or("")),
                    escape_html_text(phone["number"].as_str().unwrap_or(""))
                ));
            }
            html.push_str("</aside>");
        } else if block.block_type.eq_ignore_ascii_case("CALL") {
            html.push_str(&format!(
                "<aside>{} {}<p>{}</p></aside>",
                escape_html_text(&block.call_contact_name),
                escape_html_text(&block.call_phone_number),
                text
            ));
        } else {
            use knowledge::BlockKind as K;
            match m.kind{
                K::Heading1|K::Heading2|K::Heading3=>{let level=match m.kind{K::Heading1=>1,K::Heading2=>2,_=>3};html.push_str(&format!("<h{level}>{text}</h{level}>"));},
                K::Todo=>html.push_str(&format!("<div class=todo><input type=checkbox disabled {}><div>{}</div></div>",if m.checked{"checked"}else{""},inline())),
                K::BulletedList|K::NumberedList=>{let tag=if m.kind==K::BulletedList{"ul".into()}else{format!("ol start=\"{}\"",knowledge_numbered_position(blocks,&block))};let end=if m.kind==K::BulletedList{"ul"}else{"ol"};html.push_str(&format!("<{tag}><li>{}{}</li></{end}>",inline(),children(seen,rendered_images)));},
                K::Quote=>html.push_str(&format!("<blockquote>{}</blockquote>",inline())),K::Callout=>html.push_str(&format!("<aside>{}</aside>",inline())),K::Divider=>html.push_str("<hr>"),
                K::Toggle=>html.push_str(&format!("<details {}><summary>{text}</summary>{}</details>",if m.collapsed{""}else{"open"},children(seen,rendered_images))),
                K::Code=>html.push_str(&format!("<pre><code>{text}</code></pre>")),K::Equation=>html.push_str(&format!("<div class=math data-latex=\"{}\"></div>",escape_html_text(&block.text))),
                K::Columns=>{html.push_str("<div class=columns>");for col in 0..m.columns{html.push_str("<div>");html.push_str(&knowledge_blocks_html(blocks,rich_text,Some(&block.id),Some(col),images,rendered_images,seen,depth+1));html.push_str("</div>");}html.push_str("</div>");},
                K::Table=>{html.push_str("<div class=table-scroll><table>");for(i,row)in m.table.iter().enumerate(){html.push_str("<tr>");let tag=if i==0&&m.table_header{"th"}else{"td"};for cell in row{html.push_str(&format!("<{tag}>{}</{tag}>",escape_html_text(cell).replace('\n',"<br>")));}html.push_str("</tr>");}html.push_str("</table></div>");},
                K::PageLink|K::BlockLink=>html.push_str(&format!("<a class=page-link href=\"#block-{}\" data-page=\"{}\" data-block=\"{}\">↗ {text}</a>",escape_html_text(&m.target_block_id),escape_html_text(&m.target_page_id),escape_html_text(&m.target_block_id))),
                K::TableOfContents=>{html.push_str("<nav class=toc>");for heading in blocks.iter().filter(|b|b.knowledge.as_ref().is_some_and(|m|matches!(m.kind,K::Heading1|K::Heading2|K::Heading3))){html.push_str(&format!("<a href=\"#block-{}\">{}</a>",escape_html_text(&heading.id),escape_html_text(&heading.text)));}html.push_str("</nav>");},
                K::Embed=>{if knowledge::safe_web_url(&m.url){html.push_str(&format!("<iframe sandbox=\"allow-scripts allow-same-origin\" referrerpolicy=no-referrer loading=lazy src=\"{}\" title=\"{}\"></iframe><a href=\"{}\" target=_blank rel=noopener>打开原网页</a>",escape_html_text(&m.url),text,escape_html_text(&m.url)));}},
                K::Audio|K::Video=>{let source = block.attachment_id.as_ref().and_then(|id|images.get(id)).cloned().or_else(||knowledge::safe_web_url(&m.url).then(||m.url.clone()));if let Some(src)=source{let tag=if m.kind==K::Audio{"audio"}else{"video"};html.push_str(&format!("<{tag} controls preload=none src=\"{}\"></{tag}>",escape_html_text(&src)));}},
                K::File if block.attachment_id.as_ref().is_some_and(|id|images.contains_key(id))=>{let id=block.attachment_id.as_ref().unwrap();html.push_str(&format!("<a class=bookmark download=\"{}\" data-attachment=\"{}\" href=\"{}\">{text}</a>",escape_html_text(&block.text),escape_html_text(id),images[id]));},
                K::Bookmark|K::File=>{if knowledge::safe_web_url(&m.url){html.push_str(&format!("<a class=bookmark href=\"{}\" target=_blank rel=noopener>{text}<small>{}</small></a>",escape_html_text(&m.url),escape_html_text(&m.url)));}else{html.push_str(&inline());}},
                _=>html.push_str(&inline()),
            }
        }
        html.push_str("</section>");
    }
    html
}

fn knowledge_preview_shell(title: &str, body: &str, hosted: bool) -> String {
    let theme = if palette().is_dark { "dark" } else { "light" };
    let css = include_str!("assets/katex_v0.18.7_embedded.css");
    let script = include_str!("assets/katex_v0.18.7.min.js");
    let toolbar = if hosted {
        "<header class=reader-bar><span>阅读视图</span><button id=reader-close>返回编辑</button></header>"
    } else {
        ""
    };
    let html = format!(
        r#"<!doctype html><html lang="zh-CN" data-theme="{theme}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-knowledge-preview'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src data: https: http:; frame-src https: http:; connect-src 'none'; base-uri 'none'; form-action 'none'; object-src 'none'"><title>{}</title><style>{css}
body{{margin:0;background:#fffdf9;color:#24272a;font:16px/1.75 'Segoe UI','Microsoft YaHei',sans-serif}}html[data-theme=dark] body{{background:#16191d;color:#e5e8ec}}main{{max-width:880px;margin:32px auto 100px;padding:0 36px}}.page-title{{font-size:36px;margin-bottom:30px}}h1,h2,h3{{line-height:1.3;margin:24px 0 12px}}.block{{margin:8px 0;scroll-margin-top:75px}}p{{margin:5px 0}}a{{color:#497992;text-decoration:none}}a:hover{{text-decoration:underline}}img,video{{max-width:100%;border-radius:8px}}figure{{margin:20px 0}}figcaption,small{{color:#7c838a;font-size:13px}}small{{display:block}}.columns{{display:flex;gap:22px}}.columns>div{{min-width:0;flex:1}}.todo{{display:flex;align-items:start;gap:10px}}input[type=checkbox]{{margin-top:9px}}blockquote{{border-left:3px solid #b6c5c9;padding:6px 18px;margin:12px 0}}aside,.bookmark{{display:block;background:#b6c5c91a;border:1px solid #b6c5c944;border-radius:8px;padding:16px}}pre{{overflow:auto;background:#b6c5c91a;padding:18px;border-radius:8px;line-height:1.5}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #b6c5c977;padding:8px 14px;min-width:90px;text-align:left}}th{{background:#b6c5c922}}.table-scroll,.math{{overflow-x:auto}}summary{{cursor:pointer}}details>section{{margin-left:20px}}.toc a{{display:block}}iframe{{width:100%;height:450px;border:1px solid #b6c5c944;border-radius:8px}}hr{{border:0;border-top:1px solid #b6c5c944;margin:25px 0}}.reader-bar{{position:sticky;top:0;z-index:5;background:inherit;backdrop-filter:blur(16px);border-bottom:1px solid #b6c5c944;padding:12px 25px;display:flex;justify-content:space-between}}button{{font:inherit;border:1px solid #b6c5c966;border-radius:6px;padding:5px 14px;background:transparent;color:inherit;cursor:pointer}}@media(max-width:600px){{main{{padding:0 20px}}.columns{{flex-direction:column}}}}
</style></head><body>{toolbar}<main>{body}</main><script nonce="knowledge-preview">{script}</script><script nonce="knowledge-preview">
for(const el of document.querySelectorAll('[data-latex]')){{try{{katex.render(el.dataset.latex,el,{{displayMode:true,throwOnError:true,trust:false,maxSize:20,maxExpand:1000}})}}catch(error){{el.textContent='公式无法解析：'+error.message;el.style.color='#bc4242'}}}}
const send=value=>{{if(window.ipc)window.ipc.postMessage(JSON.stringify(value));}};
document.getElementById('reader-close')?.addEventListener('click',()=>send({{kind:'reader-close'}}));
document.addEventListener('keydown',event=>{{if(event.key==='Escape'&&window.ipc){{event.preventDefault();send({{kind:'reader-close'}})}}}});
document.querySelectorAll('[data-page]').forEach(link=>link.addEventListener('click',event=>{{if(window.ipc){{event.preventDefault();send({{kind:'reader-open',title:link.dataset.page,request_id:link.dataset.block}})}}}}));
document.querySelectorAll('a[href^="http"]').forEach(link=>link.addEventListener('click',event=>{{if(window.ipc){{event.preventDefault();send({{kind:'reader-external',title:link.href}})}}}}));
window.requestHostSnapshot=()=>send({{kind:'reader-close'}});window.SmartisanRichText={{focusEditor:()=>{{}}}};
document.querySelectorAll('[data-attachment]').forEach(link=>link.addEventListener('click',event=>{{if(window.ipc){{event.preventDefault();send({{kind:'reader-file',title:link.dataset.attachment}})}}}}));
send({{kind:'ready'}});
</script></body></html>"#,
        escape_html_text(title)
    );
    html
}

impl TimerWindowsClient {
    fn open_knowledge_reading_view(&mut self, only_block: Option<&str>) {
        if self.selected_note_is_locked() || self.flush_document_operation_drafts().is_err() {
            return;
        }
        let Some(mut note) = self.selected_note() else {
            return;
        };
        if let Some(id) = only_block {
            let ids = knowledge_block_subtree(&note.document.blocks, id);
            note.document.blocks.retain(|b| ids.contains(&b.id));
            if let Some(root) = note.document.blocks.iter_mut().find(|b| b.id == id) {
                if let Some(meta) = &mut root.knowledge {
                    meta.parent_id = None;
                }
            }
        }
        if only_block.is_some() {
            let ids = note
                .document
                .blocks
                .iter()
                .filter_map(|b| b.attachment_id.as_ref())
                .collect::<HashSet<_>>();
            note.attachments.retain(|a| ids.contains(&a.id));
        }
        let workspace = self.ai_workspace_identity();
        let identity = workspace_note_media_identity(&workspace);
        let root = workspace_note_media_root(
            &workspace.namespace_root,
            &stable_workspace_note_media_key(&identity),
        );
        let mut store = None;
        let html = knowledge_complete_html(&note, |attachment| {
            use gridtimer_native::desktop_note_media::{
                ExistingBoundNoteMediaProbe, ReadOnlyDesktopNoteMediaStore,
            };
            if store.is_none() {
                store = match ReadOnlyDesktopNoteMediaStore::probe_existing_bound(&root, &identity)
                    .map_err(|e| e.to_string())?
                {
                    ExistingBoundNoteMediaProbe::Present(store) => Some(store),
                    ExistingBoundNoteMediaProbe::Missing => {
                        return Err("本机尚未下载页面图片".into())
                    }
                };
            }
            store
                .as_ref()
                .unwrap()
                .read_blob(&attachment.id, &attachment.sha256, attachment.size_bytes)
                .map_err(|e| e.to_string())
        });
        let html=match html{Ok(html)=>html.replace("<body>","<body><header class=reader-bar><span>阅读视图</span><button id=reader-close>返回编辑</button></header>"),Err(error)=>{self.status=error;return;}};
        self.rich_editor = DesktopRichEditor {
            active: true,
            read_only: true,
            started: Some(Instant::now()),
            note_id: self.selected_note_id.clone(),
            workspace: self.background_job_workspace_fingerprint(),
            page: Some(html),
            ..Default::default()
        };
        self.status = "阅读视图".into();
    }
}
