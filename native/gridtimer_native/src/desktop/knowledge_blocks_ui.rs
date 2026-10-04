// v1.0.3 Windows - Keep backlink rows limited to page identifiers and titles.
// v2.22.54 - Avoid discarded undo copies and full-library clones in child navigation.
// v2.22.53 - Focused knowledge workspace, contextual controls and safe navigation.
// v2.22.52 - Structured blocks, hierarchy-aware editing and direct references.

#[derive(Clone)]
struct KnowledgeBlockDrag {
    page: String,
    block: String,
}

fn knowledge_block_children(
    blocks: &[DesktopNoteBlock],
    parent: Option<&str>,
    column: Option<u8>,
) -> Vec<DesktopNoteBlock> {
    blocks
        .iter()
        .filter(|b| {
            b.knowledge.as_ref().and_then(|m| m.parent_id.as_deref()) == parent
                && column.map_or(true, |c| b.knowledge.as_ref().map_or(0, |m| m.column) == c)
        })
        .cloned()
        .collect()
}

fn knowledge_block_sibling_ids(
    blocks: &[DesktopNoteBlock],
    parent: Option<&str>,
    column: Option<u8>,
) -> Vec<String> {
    blocks
        .iter()
        .filter(|block| {
            block
                .knowledge
                .as_ref()
                .and_then(|meta| meta.parent_id.as_deref())
                == parent
                && column.map_or(true, |wanted| {
                    block.knowledge.as_ref().map_or(0, |meta| meta.column) == wanted
                })
        })
        .map(|block| block.id.clone())
        .collect()
}

fn knowledge_numbered_position(blocks: &[DesktopNoteBlock], block: &DesktopNoteBlock) -> usize {
    let parent_id = block
        .knowledge
        .as_ref()
        .and_then(|meta| meta.parent_id.as_ref());
    let column = block.knowledge.as_ref().map_or(0, |meta| meta.column);
    let mut number = 0;
    for item in blocks.iter().filter(|b| {
        b.knowledge.as_ref().and_then(|m| m.parent_id.as_ref()) == parent_id
            && b.knowledge.as_ref().map_or(0, |m| m.column) == column
    }) {
        number = if item
            .knowledge
            .as_ref()
            .is_some_and(|m| m.kind == knowledge::BlockKind::NumberedList)
        {
            number + 1
        } else {
            0
        };
        if item.id == block.id {
            return number.max(1);
        }
    }
    1
}

fn knowledge_block_subtree(blocks: &[DesktopNoteBlock], root: &str) -> HashSet<String> {
    let mut ids = HashSet::from([root.to_string()]);
    let mut pending = vec![root.to_string()];
    while let Some(id) = pending.pop() {
        for child in blocks
            .iter()
            .filter(|b| b.knowledge.as_ref().and_then(|m| m.parent_id.as_deref()) == Some(&id))
        {
            if ids.insert(child.id.clone()) {
                pending.push(child.id.clone());
            }
        }
    }
    ids
}

impl TimerWindowsClient {
    fn ensure_knowledge_page(&mut self) {
        self.desktop_ui
            .knowledge
            .page
            .get_or_insert_with(Default::default);
    }

    fn insert_knowledge_block(
        &mut self,
        kind: knowledge::BlockKind,
        parent: Option<String>,
        column: u8,
        after: Option<&str>,
    ) -> bool {
        if self.knowledge_page_locked() {
            return false;
        }
        let before = self.current_note_draft_snapshot();
        self.ensure_knowledge_page();
        let mut meta = knowledge::KnowledgeBlock {
            kind,
            parent_id: parent,
            column,
            ..Default::default()
        };
        if kind == knowledge::BlockKind::Table {
            meta.table = vec![
                vec!["列一".into(), "列二".into()],
                vec![String::new(), String::new()],
            ];
        }
        let mut block = new_desktop_text_block("");
        block.knowledge = Some(meta);
        self.note_active_block_id = block.id.clone();
        self.desktop_ui.parity.outline_target = Some(block.id.clone());
        let index = after
            .and_then(|id| self.note_blocks_draft.iter().position(|b| b.id == id))
            .map_or(self.note_blocks_draft.len(), |i| i + 1);
        self.note_blocks_draft.insert(index, block);
        self.finish_note_structural_change(before);
        true
    }

    fn convert_knowledge_block(&mut self, id: &str, kind: knowledge::BlockKind, slash: bool) {
        if self.knowledge_page_locked() {
            return;
        }
        let before = self.current_note_draft_snapshot();
        self.ensure_knowledge_page();
        let Some(index) = self.note_blocks_draft.iter().position(|b| b.id == id) else {
            return;
        };
        let old = self.note_blocks_draft[index]
            .knowledge
            .clone()
            .unwrap_or_default();
        if !matches!(
            kind,
            knowledge::BlockKind::Toggle
                | knowledge::BlockKind::Columns
                | knowledge::BlockKind::BulletedList
                | knowledge::BlockKind::NumberedList
        ) {
            for child in &mut self.note_blocks_draft {
                if let Some(meta) = &mut child.knowledge {
                    if meta.parent_id.as_deref() == Some(id) {
                        meta.parent_id = old.parent_id.clone();
                        meta.column = old.column;
                    }
                }
            }
        }
        let block = &mut self.note_blocks_draft[index];
        let meta = block.knowledge.get_or_insert_with(Default::default);
        meta.kind = kind;
        if kind == knowledge::BlockKind::Table && meta.table.is_empty() {
            meta.table = vec![
                vec!["列一".into(), "列二".into()],
                vec![String::new(), String::new()],
            ];
        }
        if slash {
            block.text.clear();
        }
        self.note_active_block_id = id.into();
        self.finish_note_structural_change(before);
    }

    fn update_knowledge_block(&mut self, block: DesktopNoteBlock, text_only: bool) {
        if self.knowledge_page_locked() {
            return;
        }
        if block
            .knowledge
            .as_ref()
            .is_some_and(|m| m.validate().is_err())
        {
            self.status = "内容块未通过校验".into();
            return;
        }
        let id = block.id.clone();
        let edit_key = format!("block:{id}");
        let before = if text_only {
            self.begin_note_text_edit(&edit_key)
        } else {
            Some(self.current_note_draft_snapshot())
        };
        self.ensure_knowledge_page();
        let columns = block
            .knowledge
            .as_ref()
            .filter(|m| m.kind == knowledge::BlockKind::Columns)
            .map(|m| m.columns);
        if let Some(old) = self.note_blocks_draft.iter_mut().find(|b| b.id == id) {
            *old = block;
        } else {
            return;
        }
        if let Some(columns) = columns {
            for child in &mut self.note_blocks_draft {
                if let Some(meta) = child
                    .knowledge
                    .as_mut()
                    .filter(|m| m.parent_id.as_deref() == Some(id.as_str()))
                {
                    meta.column = meta.column.min(columns.saturating_sub(1));
                }
            }
        }
        if text_only {
            self.finish_note_text_edit(before, &edit_key);
        } else if let Some(before) = before {
            self.finish_note_structural_change(before);
        }
    }

    fn delete_knowledge_block(&mut self, id: &str) {
        if self.knowledge_page_locked() {
            return;
        }
        let before = self.current_note_draft_snapshot();
        let ids = knowledge_block_subtree(&self.note_blocks_draft, id);
        self.note_blocks_draft.retain(|b| !ids.contains(&b.id));
        if self.note_blocks_draft.is_empty() {
            self.note_blocks_draft.push(new_desktop_text_block(""));
        }
        self.note_active_block_id = self.note_blocks_draft[0].id.clone();
        self.finish_note_structural_change(before);
    }

    fn duplicate_knowledge_block(&mut self, id: &str) {
        if self.knowledge_page_locked() {
            return;
        }
        let before = self.current_note_draft_snapshot();
        let ids = knowledge_block_subtree(&self.note_blocks_draft, id);
        let map = ids
            .iter()
            .map(|id| (id.clone(), new_desktop_note_block_id()))
            .collect::<BTreeMap<_, _>>();
        let mut copied = self
            .note_blocks_draft
            .iter()
            .filter(|b| ids.contains(&b.id))
            .cloned()
            .collect::<Vec<_>>();
        for block in &mut copied {
            block.id = map[&block.id].clone();
            if let Some(meta) = &mut block.knowledge {
                if let Some(next) = meta.parent_id.as_ref().and_then(|id| map.get(id)) {
                    meta.parent_id = Some(next.clone());
                }
                if meta.target_page_id == self.selected_note_id {
                    if let Some(next) = map.get(&meta.target_block_id) {
                        meta.target_block_id = next.clone();
                    }
                }
            }
        }
        let index = self
            .note_blocks_draft
            .iter()
            .rposition(|b| ids.contains(&b.id))
            .map_or(self.note_blocks_draft.len(), |i| i + 1);
        self.note_blocks_draft.splice(index..index, copied);
        self.note_active_block_id = map.get(id).cloned().unwrap_or_default();
        self.finish_note_structural_change(before);
    }

    fn move_knowledge_block(&mut self, id: &str, target: &str, after: bool) -> bool {
        if self.knowledge_page_locked() || id == target {
            return false;
        }
        let ids = knowledge_block_subtree(&self.note_blocks_draft, id);
        if ids.contains(target) {
            return false;
        }
        let Some(destination) = self
            .note_blocks_draft
            .iter()
            .find(|b| b.id == target)
            .cloned()
        else {
            return false;
        };
        let before = self.current_note_draft_snapshot();
        let mut moving = self
            .note_blocks_draft
            .iter()
            .filter(|b| ids.contains(&b.id))
            .cloned()
            .collect::<Vec<_>>();
        let dest = destination.knowledge.unwrap_or_default();
        if let Some(root) = moving.iter_mut().find(|b| b.id == id) {
            let m = root.knowledge.get_or_insert_with(Default::default);
            m.parent_id = dest.parent_id;
            m.column = dest.column;
        }
        self.note_blocks_draft.retain(|b| !ids.contains(&b.id));
        let Some(mut index) = self.note_blocks_draft.iter().position(|b| b.id == target) else {
            return false;
        };
        if after {
            let target_ids = knowledge_block_subtree(&self.note_blocks_draft, target);
            index = self
                .note_blocks_draft
                .iter()
                .rposition(|b| target_ids.contains(&b.id))
                .unwrap_or(index)
                + 1;
        }
        self.note_blocks_draft.splice(index..index, moving);
        self.finish_note_structural_change(before);
        true
    }

    fn ui_knowledge_blocks(&mut self, ui: &mut egui::Ui) {
        let locked = self.knowledge_page_locked();
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(!locked, |ui| {
                ui.menu_button("＋ 内容块", |ui| {
                    for kind in knowledge::BlockKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            self.insert_knowledge_block(kind, None, 0, None);
                            ui.close_menu();
                        }
                    }
                });
                if knowledge_quiet_button(ui, "图片", "插入图片").clicked() {
                    self.note_insert_menu_open = true;
                }
                if ui
                    .add_enabled(
                        !self.note_undo_stack.is_empty(),
                        egui::Button::new("撤销").frame(false),
                    )
                    .clicked()
                {
                    self.undo_note_draft();
                }
                if ui
                    .add_enabled(
                        !self.note_redo_stack.is_empty(),
                        egui::Button::new("重做").frame(false),
                    )
                    .clicked()
                {
                    self.redo_note_draft();
                }
            });
        });
        if self.note_insert_menu_open {
            self.ui_note_insert_menu(ui);
        }
        let editing_page = self.selected_note_id.clone();
        let roots = knowledge_block_children(&self.note_blocks_draft, None, None);
        for (index, block) in roots.iter().enumerate() {
            self.ui_knowledge_block(ui, block, index, 0, locked);
            if self.selected_note_id != editing_page || self.rich_editor.active {
                return;
            }
        }
        if !locked
            && ui
                .add_sized(
                    [ui.available_width(), 28.0],
                    egui::Button::new("＋").frame(false),
                )
                .clicked()
        {
            self.insert_knowledge_block(knowledge::BlockKind::Paragraph, None, 0, None);
        }
        self.ui_knowledge_child_pages(ui);
    }

    fn ui_knowledge_block(
        &mut self,
        ui: &mut egui::Ui,
        block: &DesktopNoteBlock,
        _index: usize,
        depth: usize,
        locked: bool,
    ) {
        if depth > 32 {
            return;
        }
        if !block_is_plain_text(block) {
            let siblings = knowledge_block_sibling_ids(
                &self.note_blocks_draft,
                block
                    .knowledge
                    .as_ref()
                    .and_then(|m| m.parent_id.as_deref()),
                block.knowledge.as_ref().map(|m| m.column),
            );
            let position = siblings.iter().position(|id| id == &block.id).unwrap_or(0);
            if let Some(action) =
                self.ui_note_canvas_block(ui, block, position, siblings.len(), locked)
            {
                match action {
                    1 if position > 0 => {
                        self.move_knowledge_block(&block.id, &siblings[position - 1], false);
                    }
                    2 if position + 1 < siblings.len() => {
                        self.move_knowledge_block(&block.id, &siblings[position + 1], true);
                    }
                    3 => self.duplicate_knowledge_block(&block.id),
                    4 => self.delete_knowledge_block(&block.id),
                    _ => {}
                }
            }
            return;
        }
        let mut edited = block.clone();
        let mut meta = block.knowledge.clone().unwrap_or_default();
        let old_meta = meta.clone();
        let mut remove = false;
        let mut duplicate = false;
        let mut convert = None;
        let mut append = false;
        let mut command = None;
        let mut preview = false;
        let mut import_file = false;
        let mut export_file = None;
        let editing_page = self.selected_note_id.clone();
        let response=ui.push_id(&block.id,|ui|{
            let width=ui.available_width();let available=if self.desktop_ui.knowledge.page.as_ref().is_some_and(|p|p.full_width){width}else{width.min(900.0)};ui.set_max_width(available);
            let text_color=match meta.color.as_str(){"红"=>palette().danger,"蓝"=>palette().blue,"绿"=>palette().good,"灰"=>palette().muted,_=>palette().text};
            let frame=egui::Frame::none().inner_margin(egui::Margin::symmetric(5.0,4.0)).fill(match meta.background.as_str(){"柔和"=>palette().panel_alt,"强调"=>palette().accent_soft,_ if matches!(meta.kind,knowledge::BlockKind::Callout|knowledge::BlockKind::Code)=>palette().panel_alt,_=>egui::Color32::TRANSPARENT}).rounding(5.0);
            frame.show(ui,|ui|{
                ui.horizontal_top(|ui|{
                    let previous_rect=ui.ctx().data(|d|d.get_temp::<egui::Rect>(egui::Id::new(("knowledge_block_rect",&block.id))));
                    let show_handle=self.note_active_block_id==block.id||previous_rect.is_some_and(|r|ui.rect_contains_pointer(r));
                    let (handle_rect,handle)=ui.allocate_exact_size(egui::vec2(16.0,24.0),egui::Sense::click_and_drag());if show_handle {for x in [5.0,10.0]{for y in [7.0,12.0,17.0]{ui.painter().circle_filled(handle_rect.min+egui::vec2(x,y),1.1,palette().muted);}}}let handle=handle.on_hover_text("拖动排序，右键设置");if !locked{handle.dnd_set_drag_payload(KnowledgeBlockDrag{page:self.selected_note_id.clone(),block:block.id.clone()});}
                    handle.context_menu(|ui|{ui.add_enabled_ui(!locked,|ui|{
                        ui.menu_button("转换为",|ui|{for kind in knowledge::BlockKind::ALL{if ui.button(kind.label()).clicked(){convert=Some(kind);ui.close_menu();}}});
                        ui.menu_button("颜色",|ui|{for name in ["默认","红","蓝","绿","灰"]{ui.selectable_value(&mut meta.color,name.into(),name);}ui.separator();for name in ["无背景","柔和","强调"]{ui.selectable_value(&mut meta.background,name.into(),name);}});
                        if ui.button("下方插入段落").clicked(){append=true;ui.close_menu();}if ui.button("复制块").clicked(){duplicate=true;ui.close_menu();}if ui.button("删除块及子内容").clicked(){remove=true;ui.close_menu();}
                    });if ui.button("复制块引用").clicked(){ui.output_mut(|o|o.copied_text=format!("[[{}#{}]]",self.selected_note_id,block.id));ui.close_menu();}if ui.button("批注此块").clicked(){self.desktop_ui.knowledge.comment_block=block.id.clone();self.desktop_ui.experience.inspector=Some(KnowledgeInspector::Discussion);ui.close_menu();}});
                    ui.vertical(|ui|{ui.set_width((available-44.0).max(80.0));ui.add_enabled_ui(!locked,|ui|{
                        use knowledge::BlockKind as K;
                        match meta.kind{
                            K::Divider=>{ui.separator();},
                            K::Table=>{
                                let mut cell_changed=false;egui::ScrollArea::horizontal().id_source("block_table_scroll").show(ui,|ui|{egui::Grid::new("simple_table").striped(true).spacing([6.0,4.0]).show(ui,|ui|{for(row_index,row)in meta.table.iter_mut().enumerate(){for cell in row{let response=ui.add(egui::TextEdit::singleline(cell).desired_width(125.0).font(if row_index==0&&meta.table_header{egui::TextStyle::Button}else{egui::TextStyle::Body}));self.desktop_ui.navigation.track_document_text_edit(&response);cell_changed|=response.changed();}ui.end_row();}});});
                                ui.horizontal(|ui|{if ui.small_button("＋ 行").clicked(){let width=meta.table.first().map_or(2,Vec::len);meta.table.push(vec![String::new();width]);}if ui.small_button("＋ 列").clicked()&&meta.table.first().map_or(0,Vec::len)<128{for row in &mut meta.table{row.push(String::new());}}if ui.add_enabled(meta.table.len()>1,egui::Button::new("移除末行").small()).clicked(){meta.table.pop();}if ui.add_enabled(meta.table.first().is_some_and(|r|r.len()>1),egui::Button::new("移除末列").small()).clicked(){for row in &mut meta.table{row.pop();}}ui.checkbox(&mut meta.table_header,"标题行");});
                                if cell_changed||meta.table!=old_meta.table{edited.text=meta.table.iter().map(|r|r.join(" | ")).collect::<Vec<_>>().join("\n");}
                            },
                            K::TableOfContents=>{for heading in self.note_blocks_draft.iter().filter(|b|b.knowledge.as_ref().is_some_and(|m|matches!(m.kind,K::Heading1|K::Heading2|K::Heading3))){let level=heading.knowledge.as_ref().map_or(0,|m|match m.kind{K::Heading2=>1,K::Heading3=>2,_=>0});ui.horizontal(|ui|{ui.add_space(level as f32*14.0);if ui.link(&heading.text).clicked(){self.desktop_ui.parity.outline_target=Some(heading.id.clone());}});}},
                            K::PageLink|K::BlockLink=>{
                                let pages=self.knowledge_records();let target=pages.iter().find(|p|p.id==meta.target_page_id&&!p.deleted&&!p.encrypted);let target_title=target.map_or("选择页面",|p|p.title.as_str());
                                egui::ComboBox::from_id_source("block_page_target").selected_text(target_title).show_ui(ui,|ui|{for p in pages.iter().filter(|p|!p.deleted&&!p.encrypted){if ui.selectable_value(&mut meta.target_page_id,p.id.clone(),&p.title).changed(){edited.text=p.title.clone();meta.target_block_id.clear();}}});
                                if meta.kind==K::BlockLink{if let Some(note)=self.data.notes.iter().find(|p|p.id==meta.target_page_id&&p.encryption.is_none()&&p.deleted_at_epoch_millis.is_none()){let blocks=note.document.blocks.clone();egui::ComboBox::from_id_source("block_link_target").selected_text(blocks.iter().find(|b|b.id==meta.target_block_id).map_or("选择内容块",|b|b.text.as_str())).show_ui(ui,|ui|{for b in blocks{if ui.selectable_value(&mut meta.target_block_id,b.id.clone(),b.text.chars().take(64).collect::<String>()).changed(){edited.text=b.text;}}});}}
                                if ui.add_enabled(target.is_some(),egui::Button::new("打开引用").small()).clicked(){command=Some((meta.target_page_id.clone(),meta.target_block_id.clone()));}
                            },
                            K::Columns=>{ui.horizontal(|ui|{ui.label("分栏");ui.add(egui::DragValue::new(&mut meta.columns).clamp_range(2..=4));});},
                            K::Bookmark|K::Embed|K::File|K::Audio|K::Video=>{
                                if matches!(meta.kind,K::File|K::Audio|K::Video) {
                                    ui.horizontal(|ui|{if ui.small_button("选择本地文件").clicked(){import_file=true;}if let Some(id)=&block.attachment_id { if ui.small_button("另存为").clicked(){export_file=Some(id.clone());} }});
                                }
                                let name_response=ui.add(egui::TextEdit::singleline(&mut edited.text).hint_text("名称").desired_width(f32::INFINITY));self.desktop_ui.navigation.track_document_text_edit(&name_response);let url_response=ui.add(egui::TextEdit::singleline(&mut meta.url).hint_text("https://").desired_width(f32::INFINITY));self.desktop_ui.navigation.track_document_text_edit(&url_response);
                                if matches!(meta.kind,K::Embed|K::Audio|K::Video) && ui.small_button("预览").clicked(){preview=true;}
                                if knowledge::safe_web_url(&meta.url){ui.hyperlink_to(if edited.text.is_empty(){"打开链接"}else{&edited.text},&meta.url);}else if !meta.url.is_empty(){ui.colored_label(palette().danger,"请输入完整的 http 或 https 地址");}
                            },
                            _=>{
                                ui.horizontal_top(|ui|{
                                    if meta.kind==K::Todo{ui.checkbox(&mut meta.checked,"");}else if meta.kind==K::BulletedList{ui.label("•");}else if meta.kind==K::NumberedList{ui.label(format!("{}.", knowledge_numbered_position(&self.note_blocks_draft, block)));}else if meta.kind==K::Quote{ui.label(egui::RichText::new("▎").color(palette().muted));}else if meta.kind==K::Callout{ui.label("◈");}else if meta.kind==K::Toggle{if ui.small_button(if meta.collapsed{"▸"}else{"▾"}).clicked(){meta.collapsed=!meta.collapsed;}}
                                    let small=self.desktop_ui.knowledge.page.as_ref().is_some_and(|p|p.small_text);let size=match meta.kind{K::Heading1=>26.0,K::Heading2=>22.0,K::Heading3=>18.0,_=>if small{13.0}else{15.0}};
                                    let mut editor=egui::TextEdit::multiline(&mut edited.text).id(egui::Id::new(("knowledge_block_editor",self.desktop_ui.navigation.editor_epoch,&block.id))).desired_width(f32::INFINITY).desired_rows(1).font(egui::FontId::proportional(size)).text_color(text_color).frame(false).hint_text(if meta.kind==K::Code{"代码"}else if meta.kind==K::Equation{"LaTeX 公式"}else{"输入内容，或输入 / 选择块类型"});
                                    if matches!(meta.kind,K::Code|K::Equation){editor=editor.code_editor();}
                                    let response=ui.add(editor);self.desktop_ui.navigation.track_document_text_edit(&response);if response.has_focus(){self.note_active_block_id=block.id.clone();}
                                    if self.desktop_ui.parity.outline_target.as_deref()==Some(&block.id){response.scroll_to_me(Some(egui::Align::Center));response.request_focus();self.desktop_ui.parity.outline_target=None;}
                                    if response.has_focus()&&ui.input(|i|i.modifiers.command&&i.key_pressed(egui::Key::Enter)){append=true;}
                                    });
                                    if self.note_active_block_id == block.id && edited.text.starts_with('/')&&!edited.text.contains('\n')&&edited.text.len()<=64{
                                        let query=edited.text[1..].trim().to_lowercase();let choices=K::ALL.iter().copied().filter(|k|query.is_empty()||k.label().contains(&query)||k.search_terms().contains(&query)).collect::<Vec<_>>();
                                        if !choices.is_empty(){egui::Frame::popup(ui.style()).show(ui,|ui|{ui.set_max_width(220.0);egui::ScrollArea::vertical().max_height(250.0).show(ui,|ui|{for kind in choices{if ui.button(kind.label()).clicked(){convert=Some(kind);}}});});}
                                    }
                                if meta.kind==K::Code{ui.horizontal(|ui|{ui.label("语言");let response=ui.add(egui::TextEdit::singleline(&mut meta.language).desired_width(100.0));self.desktop_ui.navigation.track_document_text_edit(&response);if ui.small_button("复制代码").clicked(){ui.output_mut(|o|o.copied_text=edited.text.clone());}});}
                                if matches!(meta.kind,K::Equation|K::Embed|K::Audio|K::Video) && ui.small_button("预览").clicked(){preview=true;}
                            }
                        }
                    });});
                });
            }).response
        }).inner;
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new(("knowledge_block_rect", &block.id)),
                response.rect,
            )
        });
        if !locked {
            if let Some(payload) = response.dnd_release_payload::<KnowledgeBlockDrag>() {
                if payload.page == self.selected_note_id {
                    self.move_knowledge_block(&payload.block, &block.id, false);
                }
            }
        }
        if !matches!(
            meta.kind,
            knowledge::BlockKind::Code | knowledge::BlockKind::Equation
        ) {
            let mut links = knowledge::wiki_references(&edited.text)
                .iter()
                .map(|r| (r.page.to_string(), r.block.to_string(), r.label.to_string()))
                .collect::<Vec<_>>();
            if locked
                && matches!(
                    meta.kind,
                    knowledge::BlockKind::PageLink | knowledge::BlockKind::BlockLink
                )
            {
                links.push((
                    meta.target_page_id.clone(),
                    meta.target_block_id.clone(),
                    edited.text.clone(),
                ));
            }
            if !links.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (page, target, label) in links {
                        if let Some(note) = self.data.notes.iter().find(|p| {
                            p.id == page
                                && p.encryption.is_none()
                                && p.deleted_at_epoch_millis.is_none()
                        }) {
                            if ui
                                .small_button(format!(
                                    "↗ {}",
                                    if label.is_empty() {
                                        &note.title
                                    } else {
                                        &label
                                    }
                                ))
                                .clicked()
                            {
                                command = Some((page, target));
                            }
                        }
                    }
                });
            }
        }
        let changed_text = edited.text != block.text;
        let changed_meta = meta != old_meta;
        if changed_meta || changed_text {
            edited.knowledge = Some(meta.clone());
            self.update_knowledge_block(edited, changed_text && !changed_meta);
        }
        if let Some(kind) = convert {
            self.convert_knowledge_block(&block.id, kind, block.text.trim_start().starts_with('/'));
        }
        if duplicate {
            self.duplicate_knowledge_block(&block.id);
        }
        if remove {
            self.delete_knowledge_block(&block.id);
            return;
        }
        if append {
            self.insert_knowledge_block(
                knowledge::BlockKind::Paragraph,
                meta.parent_id.clone(),
                meta.column,
                Some(&block.id),
            );
        }
        if import_file {
            self.import_knowledge_file(&block.id);
            return;
        }
        if let Some(id) = export_file {
            self.export_knowledge_attachment(&id);
            return;
        }
        if preview {
            self.open_knowledge_reading_view(Some(&block.id));
            return;
        }
        if let Some((page, target)) = command {
            self.select_note_by_id(&page);
            if !target.is_empty() {
                self.desktop_ui.parity.outline_target = Some(target);
            }
            return;
        }
        if matches!(
            meta.kind,
            knowledge::BlockKind::Toggle
                | knowledge::BlockKind::Columns
                | knowledge::BlockKind::BulletedList
                | knowledge::BlockKind::NumberedList
        ) && !(meta.kind == knowledge::BlockKind::Toggle && meta.collapsed)
        {
            if meta.kind == knowledge::BlockKind::Columns {
                ui.columns(meta.columns as usize, |columns| {
                    for (col, ui) in columns.iter_mut().enumerate() {
                        let children = knowledge_block_children(
                            &self.note_blocks_draft,
                            Some(&block.id),
                            Some(col as u8),
                        );
                        for (i, child) in children.iter().enumerate() {
                            self.ui_knowledge_block(ui, child, i, depth + 1, locked);
                            if self.selected_note_id != editing_page {
                                return;
                            }
                        }
                        if !locked && ui.small_button("＋ 此栏内容").clicked() {
                            self.insert_knowledge_block(
                                knowledge::BlockKind::Paragraph,
                                Some(block.id.clone()),
                                col as u8,
                                None,
                            );
                        }
                    }
                });
            } else {
                ui.indent((&block.id, "children"), |ui| {
                    let children =
                        knowledge_block_children(&self.note_blocks_draft, Some(&block.id), None);
                    for (i, child) in children.iter().enumerate() {
                        self.ui_knowledge_block(ui, child, i, depth + 1, locked);
                        if self.selected_note_id != editing_page || self.rich_editor.active {
                            return;
                        }
                    }
                    if !locked && ui.small_button("＋ 子内容").clicked() {
                        self.insert_knowledge_block(
                            knowledge::BlockKind::Paragraph,
                            Some(block.id.clone()),
                            0,
                            None,
                        );
                    }
                });
            }
        }
    }

    fn ui_knowledge_child_pages(&mut self, ui: &mut egui::Ui) {
        let id = self.selected_note_id.clone();
        if id.is_empty() {
            return;
        }
        let navigation = self.knowledge_navigation();
        let children = navigation
            .children_of(Some(&id))
            .iter()
            .map(|index| &navigation.pages[*index])
            .filter(|p| !p.encrypted)
            .collect::<Vec<_>>();
        if !children.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("子页面").small().color(palette().muted));
            for p in children {
                if ui.link(format!("{} {}", p.icon, p.title)).clicked() {
                    self.select_note_by_id(&p.id);
                    return;
                }
            }
        }
    }

    fn ui_knowledge_references_and_comments(&mut self, ui: &mut egui::Ui) {
        let id = self.selected_note_id.clone();
        egui::CollapsingHeader::new("引用此页")
            .id_source("knowledge_backlinks")
            .default_open(true)
            .show(ui, |ui| {
                let backlinks = self
                    .data
                    .notes
                    .iter()
                    .filter(|p| {
                        desktop_note_kind(p) == DesktopNoteKind::Document
                            && p.id != id
                            && p.encryption.is_none()
                            && p.deleted_at_epoch_millis.is_none()
                            && p.document.blocks.iter().any(|b| {
                                b.knowledge.as_ref().is_some_and(|m| m.target_page_id == id)
                                    || (!b.knowledge.as_ref().is_some_and(|m| {
                                        matches!(
                                            m.kind,
                                            knowledge::BlockKind::Code
                                                | knowledge::BlockKind::Equation
                                        )
                                    }) && knowledge::wiki_references(&b.text)
                                        .iter()
                                        .any(|r| r.page == id))
                            })
                    })
                    .map(|page| (page.id.clone(), page.title.clone()))
                    .collect::<Vec<_>>();
                if backlinks.is_empty() {
                    ui.label(
                        egui::RichText::new("暂无引用")
                            .small()
                            .color(palette().muted),
                    );
                }
                for (page_id, title) in backlinks {
                    if ui.link(&title).clicked() {
                        self.select_note_by_id(&page_id);
                    }
                }
            });
        if self.selected_note_id != id {
            return;
        }
        let mut meta = self.desktop_ui.knowledge.page.clone().unwrap_or_default();
        let original = meta.clone();
        egui::CollapsingHeader::new("批注")
            .id_source("knowledge_comments")
            .default_open(true)
            .show(ui, |ui| {
                for comment in meta.comments.iter_mut().filter(|c| c.deleted_at.is_none()) {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(if comment.author.is_empty() {
                                "我"
                            } else {
                                &comment.author
                            })
                            .strong(),
                        );
                        ui.label(
                            egui::RichText::new(desktop_local_timestamp(comment.created_at))
                                .small()
                                .color(palette().muted),
                        );
                        if ui.checkbox(&mut comment.resolved, "已解决").changed() {
                            comment.updated_at = now_millis();
                        }
                        if let Some(block) = &comment.block_id {
                            if ui.small_button("定位").clicked() {
                                self.desktop_ui.parity.outline_target = Some(block.clone());
                            }
                        }
                    });
                    ui.label(if comment.resolved {
                        egui::RichText::new(&comment.body)
                            .strikethrough()
                            .color(palette().muted)
                    } else {
                        egui::RichText::new(&comment.body)
                    });
                    ui.add_space(8.0);
                }
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.desktop_ui.knowledge.comment)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("写一条批注"),
                );
                self.desktop_ui
                    .navigation
                    .track_document_text_edit(&response);
                if !self.desktop_ui.knowledge.comment_block.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("批注当前内容块");
                        if ui.small_button("改为整页批注").clicked() {
                            self.desktop_ui.knowledge.comment_block.clear();
                        }
                    });
                }
                if ui
                    .add_enabled(
                        !self.knowledge_page_locked()
                            && !self.desktop_ui.knowledge.comment.trim().is_empty(),
                        egui::Button::new("添加批注"),
                    )
                    .clicked()
                {
                    let now = now_millis();
                    meta.comments.push(knowledge::PageComment {
                        id: random_desktop_identifier("comment"),
                        block_id: non_empty_string(&self.desktop_ui.knowledge.comment_block),
                        body: self.desktop_ui.knowledge.comment.trim().into(),
                        author: "我".into(),
                        created_at: now,
                        updated_at: now,
                        ..Default::default()
                    });
                    self.desktop_ui.knowledge.comment.clear();
                    self.desktop_ui.knowledge.comment_block.clear();
                }
            });
        if meta != original && !self.knowledge_page_locked() {
            self.update_knowledge_page(meta);
        }
    }
}
