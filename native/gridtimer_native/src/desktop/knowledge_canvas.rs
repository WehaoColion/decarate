// v1.0.3.13 Windows - Make multiple canvases manageable and keep canvas titles valid.
// v1.0.3.10 Windows - Honor the editing barrier before handling canvas shortcuts.
// v1.0.3.3 Windows - Add a persistent infinite canvas for connected knowledge pages.

const KNOWLEDGE_CANVAS_FILE: &str = "knowledge_canvases.json";
const KNOWLEDGE_CANVAS_VERSION: u32 = 1;
const KNOWLEDGE_CANVAS_NODE_LIMIT: usize = 1_000;
const KNOWLEDGE_CANVAS_EDGE_LIMIT: usize = 4_000;
const KNOWLEDGE_CANVAS_SAVE_DEBOUNCE_MILLIS: i64 = 650;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum KnowledgeCanvasNodeKind {
    #[default]
    Page,
    Note,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct KnowledgeCanvasNode {
    id: String,
    kind: KnowledgeCanvasNodeKind,
    page_id: String,
    text: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: String,
}

impl Default for KnowledgeCanvasNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: KnowledgeCanvasNodeKind::Page,
            page_id: String::new(),
            text: String::new(),
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 168.0,
            color: "blue".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct KnowledgeCanvasEdge {
    id: String,
    from: String,
    to: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct KnowledgeCanvas {
    id: String,
    title: String,
    center_x: f32,
    center_y: f32,
    zoom: f32,
    nodes: Vec<KnowledgeCanvasNode>,
    edges: Vec<KnowledgeCanvasEdge>,
}

impl Default for KnowledgeCanvas {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: "知识画布".into(),
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct KnowledgeCanvasStore {
    version: u32,
    active_canvas_id: String,
    canvases: Vec<KnowledgeCanvas>,
}

impl Default for KnowledgeCanvasStore {
    fn default() -> Self {
        Self {
            version: KNOWLEDGE_CANVAS_VERSION,
            active_canvas_id: String::new(),
            canvases: Vec::new(),
        }
    }
}

impl KnowledgeCanvasStore {
    fn active(&self) -> Option<&KnowledgeCanvas> {
        self.canvases
            .iter()
            .find(|canvas| canvas.id == self.active_canvas_id)
    }

    fn active_mut(&mut self) -> Option<&mut KnowledgeCanvas> {
        self.canvases
            .iter_mut()
            .find(|canvas| canvas.id == self.active_canvas_id)
    }

    fn unique_title(&self, title: &str) -> String {
        let base: String = title.trim().chars().take(80).collect();
        let base = if base.is_empty() { "新画布" } else { &base };
        if !self.canvases.iter().any(|canvas| canvas.title == base) {
            return base.to_owned();
        }
        for number in 2..=129 {
            let suffix = format!(" {number}");
            let stem: String = base.chars().take(80 - suffix.chars().count()).collect();
            let candidate = format!("{stem}{suffix}");
            if !self.canvases.iter().any(|canvas| canvas.title == candidate) {
                return candidate;
            }
        }
        base.to_owned()
    }

    fn new_canvas(&mut self, title: &str) -> bool {
        if self.canvases.len() >= 128 {
            return false;
        }
        let canvas = KnowledgeCanvas {
            id: random_desktop_identifier("knowledge-canvas"),
            title: self.unique_title(title),
            ..Default::default()
        };
        self.active_canvas_id = canvas.id.clone();
        self.canvases.push(canvas);
        true
    }

    fn duplicate_active(&mut self) -> bool {
        if self.canvases.len() >= 128 {
            return false;
        }
        let Some(source) = self.active().cloned() else {
            return false;
        };
        let mut duplicate = source.clone();
        duplicate.id = random_desktop_identifier("knowledge-canvas");
        let title_stem: String = source.title.chars().take(77).collect();
        duplicate.title = self.unique_title(&format!("{title_stem} 副本"));
        let mut node_ids = BTreeMap::new();
        for node in &mut duplicate.nodes {
            let next = random_desktop_identifier("knowledge-canvas-node");
            node_ids.insert(node.id.clone(), next.clone());
            node.id = next;
        }
        for edge in &mut duplicate.edges {
            edge.id = random_desktop_identifier("knowledge-canvas-edge");
            edge.from = node_ids[&edge.from].clone();
            edge.to = node_ids[&edge.to].clone();
        }
        self.active_canvas_id = duplicate.id.clone();
        self.canvases.push(duplicate);
        true
    }

    fn delete_canvas(&mut self, id: &str) -> bool {
        if self.canvases.len() <= 1 {
            return false;
        }
        let Some(index) = self.canvases.iter().position(|canvas| canvas.id == id) else {
            return false;
        };
        self.canvases.remove(index);
        if self.active_canvas_id == id {
            self.active_canvas_id = self.canvases[index.min(self.canvases.len() - 1)].id.clone();
        }
        true
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != KNOWLEDGE_CANVAS_VERSION {
            return Err("画布数据版本不受支持".into());
        }
        if self.canvases.len() > 128 {
            return Err("画布数量超出上限".into());
        }
        let mut canvas_ids = HashSet::new();
        let mut active_exists = self.active_canvas_id.is_empty();
        for canvas in &self.canvases {
            if !knowledge_canvas_valid_id(&canvas.id) || !canvas_ids.insert(&canvas.id) {
                return Err("画布标识无效或重复".into());
            }
            active_exists |= canvas.id == self.active_canvas_id;
            if canvas.title.trim().is_empty()
                || canvas.title.chars().count() > 80
                || !canvas.center_x.is_finite()
                || !canvas.center_y.is_finite()
                || canvas.center_x.abs() > 250_000.0
                || canvas.center_y.abs() > 250_000.0
                || !canvas.zoom.is_finite()
                || !(0.35..=2.5).contains(&canvas.zoom)
            {
                return Err("画布视图数据无效".into());
            }
            if canvas.nodes.len() > KNOWLEDGE_CANVAS_NODE_LIMIT
                || canvas.edges.len() > KNOWLEDGE_CANVAS_EDGE_LIMIT
            {
                return Err("画布元素数量超出上限".into());
            }
            let mut node_ids = HashSet::new();
            for node in &canvas.nodes {
                if !knowledge_canvas_valid_id(&node.id)
                    || !node_ids.insert(&node.id)
                    || !node.x.is_finite()
                    || !node.y.is_finite()
                    || node.x.abs() > 250_000.0
                    || node.y.abs() > 250_000.0
                    || !node.width.is_finite()
                    || !node.height.is_finite()
                    || !(160.0..=720.0).contains(&node.width)
                    || !(120.0..=720.0).contains(&node.height)
                    || node.text.chars().count() > 8_000
                    || (node.kind == KnowledgeCanvasNodeKind::Page
                        && !knowledge_canvas_valid_id(&node.page_id))
                {
                    return Err("画布卡片数据无效".into());
                }
            }
            let mut edge_ids = HashSet::new();
            for edge in &canvas.edges {
                if !knowledge_canvas_valid_id(&edge.id)
                    || !edge_ids.insert(&edge.id)
                    || edge.from == edge.to
                    || !node_ids.contains(&edge.from)
                    || !node_ids.contains(&edge.to)
                {
                    return Err("画布关系数据无效".into());
                }
            }
        }
        if !active_exists {
            return Err("当前画布不存在".into());
        }
        Ok(())
    }
}

fn knowledge_canvas_valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256
}

#[derive(Default)]
struct KnowledgeCanvasWorkspaceState {
    store: KnowledgeCanvasStore,
    scope_path: Option<PathBuf>,
    selected_nodes: Vec<String>,
    selected_edge: Option<String>,
    connect_from: Option<String>,
    connect_mode: bool,
    add_page_search: String,
    title_draft: String,
    title_canvas_id: String,
    pending_delete_canvas_id: Option<String>,
    dirty: bool,
    save_due_epoch_millis: i64,
    save_error: Option<String>,
}

fn knowledge_canvas_path(state_path: &Path) -> PathBuf {
    state_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(KNOWLEDGE_CANVAS_FILE)
}

fn knowledge_canvas_json_is_valid(raw: &str) -> Option<String> {
    let mut store = serde_json::from_str::<KnowledgeCanvasStore>(raw).ok()?;
    // Older Windows builds could save a blank title. Repair only that legacy
    // field so its cards and relationships remain available after upgrading.
    let mut used = store
        .canvases
        .iter()
        .filter(|canvas| !canvas.title.trim().is_empty())
        .map(|canvas| canvas.title.clone())
        .collect::<HashSet<_>>();
    let mut next_number = 1usize;
    for canvas in &mut store.canvases {
        if canvas.title.trim().is_empty() {
            loop {
                let candidate = format!("未命名画布 {next_number}");
                next_number += 1;
                if used.insert(candidate.clone()) {
                    canvas.title = candidate;
                    break;
                }
            }
        }
    }
    store.validate().ok()?;
    serde_json::to_string_pretty(&store).ok()
}

fn knowledge_canvas_load(path: &Path) -> PersistenceLoad<KnowledgeCanvasStore> {
    let fallback = serde_json::to_string_pretty(&KnowledgeCanvasStore::default())
        .unwrap_or_else(|_| "{}".into());
    let loaded = load_recoverable_text(
        path,
        "knowledge_canvases.json",
        knowledge_canvas_json_is_valid,
        fallback,
    );
    PersistenceLoad {
        value: serde_json::from_str(&loaded.value).unwrap_or_default(),
        message: loaded.message,
    }
}

impl TimerWindowsClient {
    fn mark_knowledge_canvas_dirty(&mut self) {
        let state = &mut self.desktop_ui.knowledge_canvas;
        state.dirty = true;
        state.save_due_epoch_millis =
            now_millis().saturating_add(KNOWLEDGE_CANVAS_SAVE_DEBOUNCE_MILLIS);
        state.save_error = None;
    }

    fn persist_knowledge_canvas(&mut self, force: bool) -> io::Result<()> {
        let now = now_millis();
        let state = &self.desktop_ui.knowledge_canvas;
        if !state.dirty || (!force && now < state.save_due_epoch_millis) {
            return Ok(());
        }
        let path = state
            .scope_path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "画布工作区尚未载入"))?;
        let content = serde_json::to_string_pretty(&state.store)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let result = atomic_save_text(&path, &content, |raw| knowledge_canvas_json_is_valid(raw));
        let state = &mut self.desktop_ui.knowledge_canvas;
        match result {
            Ok(()) => {
                state.dirty = false;
                state.save_error = None;
                Ok(())
            }
            Err(error) => {
                state.save_error = Some(error.to_string());
                state.save_due_epoch_millis = now.saturating_add(2_000);
                Err(error)
            }
        }
    }

    fn ensure_knowledge_canvas_loaded(&mut self) -> bool {
        let path = knowledge_canvas_path(&self.state_path);
        if self.desktop_ui.knowledge_canvas.scope_path.as_deref() == Some(path.as_path()) {
            return true;
        }
        if self.persist_knowledge_canvas(true).is_err() {
            return false;
        }
        let loaded = knowledge_canvas_load(&path);
        if let Some(message) = loaded.message {
            self.status = message;
        }
        let state = &mut self.desktop_ui.knowledge_canvas;
        state.store = loaded.value;
        state.scope_path = Some(path);
        state.selected_nodes.clear();
        state.selected_edge = None;
        state.connect_from = None;
        state.connect_mode = false;
        state.add_page_search.clear();
        state.title_draft.clear();
        state.title_canvas_id.clear();
        state.pending_delete_canvas_id = None;
        state.dirty = false;
        state.save_due_epoch_millis = 0;
        state.save_error = None;
        if state.store.canvases.is_empty() {
            state.store.new_canvas("知识画布");
            state.dirty = true;
            state.save_due_epoch_millis = now_millis();
        } else if state.store.active().is_none() {
            state.store.active_canvas_id = state.store.canvases[0].id.clone();
            state.dirty = true;
            state.save_due_epoch_millis = now_millis();
        }
        true
    }

    fn flush_due_knowledge_canvas(&mut self) {
        if self.desktop_ui.knowledge_canvas.dirty
            && now_millis() >= self.desktop_ui.knowledge_canvas.save_due_epoch_millis
        {
            if let Err(error) = self.persist_knowledge_canvas(false) {
                self.status = format!("画布保存失败，修改仍保留在当前工作区：{error}");
            }
        }
    }

    fn ui_knowledge_canvas(&mut self, ui: &mut egui::Ui) {
        if !self.ensure_knowledge_canvas_loaded() {
            ui.centered_and_justified(|ui| {
                ui.colored_label(
                    palette().danger,
                    "当前账户的画布修改无法安全保存，画布已暂停切换。",
                );
            });
            return;
        }

        self.ui_knowledge_canvas_toolbar(ui);
        self.ui_knowledge_canvas_delete_dialog(ui.ctx());
        if self
            .desktop_ui
            .knowledge_canvas
            .pending_delete_canvas_id
            .is_some()
        {
            ui.set_enabled(false);
        }
        ui.add_space(8.0);
        let available = ui.available_size().max(egui::vec2(320.0, 260.0));
        let (board_rect, board_response) =
            ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        if !ui.is_rect_visible(board_rect) {
            return;
        }

        let p = palette();
        let canvas_index = match self
            .desktop_ui
            .knowledge_canvas
            .store
            .canvases
            .iter()
            .position(|canvas| canvas.id == self.desktop_ui.knowledge_canvas.store.active_canvas_id)
        {
            Some(index) => index,
            None => return,
        };

        let mut view_changed = false;
        let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
        let scroll_y = ui.ctx().input(|input| input.raw_scroll_delta.y);
        if board_response.hovered() && scroll_y.abs() > 0.1 {
            let canvas = &mut self.desktop_ui.knowledge_canvas.store.canvases[canvas_index];
            let anchor = pointer.unwrap_or(board_rect.center());
            let world_before = egui::vec2(
                canvas.center_x + (anchor.x - board_rect.center().x) / canvas.zoom,
                canvas.center_y + (anchor.y - board_rect.center().y) / canvas.zoom,
            );
            let next_zoom = (canvas.zoom * (scroll_y * 0.0015).exp()).clamp(0.35, 2.5);
            if (next_zoom - canvas.zoom).abs() > f32::EPSILON {
                canvas.zoom = next_zoom;
                canvas.center_x = world_before.x - (anchor.x - board_rect.center().x) / next_zoom;
                canvas.center_y = world_before.y - (anchor.y - board_rect.center().y) / next_zoom;
                view_changed = true;
            }
        }
        let space_pan = ui.ctx().input(|input| input.key_down(egui::Key::Space));
        if board_response.dragged_by(egui::PointerButton::Middle)
            || (space_pan && board_response.dragged_by(egui::PointerButton::Primary))
        {
            let delta = ui.ctx().input(|input| input.pointer.delta());
            let canvas = &mut self.desktop_ui.knowledge_canvas.store.canvases[canvas_index];
            canvas.center_x =
                (canvas.center_x - delta.x / canvas.zoom).clamp(-200_000.0, 200_000.0);
            canvas.center_y =
                (canvas.center_y - delta.y / canvas.zoom).clamp(-200_000.0, 200_000.0);
            view_changed = true;
        } else if board_response.clicked_by(egui::PointerButton::Primary) {
            let ctrl = ui
                .ctx()
                .input(|input| input.modifiers.ctrl || input.modifiers.command);
            if !ctrl {
                self.desktop_ui.knowledge_canvas.selected_nodes.clear();
                self.desktop_ui.knowledge_canvas.selected_edge = None;
            }
        }

        if view_changed {
            self.mark_knowledge_canvas_dirty();
        }
        let fit_requested = self.draw_knowledge_canvas_background(ui, board_rect);
        if fit_requested {
            self.fit_knowledge_canvas_to_rect(board_rect);
        }
        self.draw_knowledge_canvas_edges(ui, board_rect, canvas_index);
        self.draw_knowledge_canvas_nodes(ui, board_rect, canvas_index);
        if ui.is_enabled() {
            self.handle_knowledge_canvas_shortcuts(ui.ctx());
        }

        let state = &self.desktop_ui.knowledge_canvas;
        if state
            .store
            .active()
            .is_some_and(|canvas| canvas.nodes.is_empty())
        {
            ui.painter().text(
                board_rect.center() - egui::vec2(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                "从一页开始，把想法和资料连起来",
                egui::FontId::proportional(17.0),
                p.muted,
            );
            ui.painter().text(
                board_rect.center() + egui::vec2(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                "添加页面或便签 · 拖动卡片排列 · 滚轮缩放 · 按住空格平移",
                egui::FontId::proportional(12.0),
                p.muted.linear_multiply(0.8),
            );
        }
        if state.connect_mode {
            let from = state
                .connect_from
                .as_ref()
                .and_then(|id| {
                    state
                        .store
                        .active()?
                        .nodes
                        .iter()
                        .find(|node| &node.id == id)
                })
                .and_then(|node| match node.kind {
                    KnowledgeCanvasNodeKind::Page => self
                        .canvas_page_title(&node.page_id)
                        .map(|(title, _, _)| title),
                    KnowledgeCanvasNodeKind::Note => Some("便签".into()),
                });
            ui.painter().text(
                board_rect.left_top() + egui::vec2(14.0, 12.0),
                egui::Align2::LEFT_TOP,
                from.map_or_else(
                    || "连接模式 · 选择一张卡片作为起点".into(),
                    |title| format!("连接到另一张卡片 · 起点：{title}"),
                ),
                egui::FontId::proportional(12.0),
                p.accent,
            );
        }
    }

    fn ui_knowledge_canvas_toolbar(&mut self, ui: &mut egui::Ui) {
        let p = palette();
        ui.horizontal(|ui| {
            for view in [
                KnowledgeCollectionView::Recent,
                KnowledgeCollectionView::Board,
                KnowledgeCollectionView::Library,
                KnowledgeCollectionView::Canvas,
            ] {
                desktop_segment(ui, &mut self.knowledge_view, view, view.label());
            }
            if self.knowledge_view != KnowledgeCollectionView::Canvas {
                self.desktop_ui.knowledge_canvas.connect_mode = false;
                self.desktop_ui.knowledge_canvas.connect_from = None;
            }
        });
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            let (active_id, active_title, canvas_count) = {
                let store = &self.desktop_ui.knowledge_canvas.store;
                (
                    store.active_canvas_id.clone(),
                    store
                        .active()
                        .map_or("知识画布", |canvas| canvas.title.as_str())
                        .to_owned(),
                    store.canvases.len(),
                )
            };
            ui.label(egui::RichText::new(&active_title).strong().color(p.text));
            ui.menu_button(format!("全部画布 · {canvas_count}"), |ui| {
                ui.set_min_width(270.0);
                let options = self
                    .desktop_ui
                    .knowledge_canvas
                    .store
                    .canvases
                    .iter()
                    .map(|canvas| (canvas.id.clone(), canvas.title.clone(), canvas.nodes.len()))
                    .collect::<Vec<_>>();
                egui::ScrollArea::vertical()
                    .max_height(290.0)
                    .show(ui, |ui| {
                        for (id, title, cards) in options {
                            if ui
                                .selectable_label(
                                    id == active_id,
                                    format!("{title}    ·    {cards} 张卡片"),
                                )
                                .clicked()
                            {
                                if id != active_id {
                                    self.desktop_ui.knowledge_canvas.store.active_canvas_id = id;
                                    self.reset_knowledge_canvas_interaction();
                                    self.mark_knowledge_canvas_dirty();
                                }
                                ui.close_menu();
                            }
                        }
                    });
                ui.separator();
                if ui
                    .add_enabled(canvas_count < 128, egui::Button::new("复制当前画布"))
                    .clicked()
                {
                    if self.desktop_ui.knowledge_canvas.store.duplicate_active() {
                        self.reset_knowledge_canvas_interaction();
                        self.mark_knowledge_canvas_dirty();
                    }
                    ui.close_menu();
                }
                if ui
                    .add_enabled(canvas_count > 1, egui::Button::new("删除当前画布…"))
                    .clicked()
                {
                    self.desktop_ui.knowledge_canvas.pending_delete_canvas_id = Some(
                        self.desktop_ui
                            .knowledge_canvas
                            .store
                            .active_canvas_id
                            .clone(),
                    );
                    ui.close_menu();
                }
            });
            if ui
                .add_enabled(
                    self.desktop_ui.knowledge_canvas.store.canvases.len() < 128,
                    egui::Button::new("＋ 新画布"),
                )
                .on_hover_text("创建一个独立的知识画布")
                .clicked()
            {
                if self.desktop_ui.knowledge_canvas.store.new_canvas("新画布") {
                    self.reset_knowledge_canvas_interaction();
                    self.mark_knowledge_canvas_dirty();
                }
            }

            let current_id = self
                .desktop_ui
                .knowledge_canvas
                .store
                .active_canvas_id
                .clone();
            if self.desktop_ui.knowledge_canvas.title_canvas_id != current_id {
                self.desktop_ui.knowledge_canvas.title_draft = self
                    .desktop_ui
                    .knowledge_canvas
                    .store
                    .active()
                    .map_or("知识画布", |canvas| canvas.title.as_str())
                    .to_owned();
                self.desktop_ui.knowledge_canvas.title_canvas_id = current_id;
            }
            if self.desktop_ui.knowledge_canvas.store.active().is_some() {
                let response = ui
                    .add(
                        egui::TextEdit::singleline(
                            &mut self.desktop_ui.knowledge_canvas.title_draft,
                        )
                        .desired_width(150.0)
                        .font(egui::TextStyle::Button)
                        .hint_text("画布名称"),
                    )
                    .on_hover_text("编辑名称后按回车或点击其他位置保存");
                if response.changed() {
                    self.desktop_ui.knowledge_canvas.title_draft = self
                        .desktop_ui
                        .knowledge_canvas
                        .title_draft
                        .chars()
                        .take(80)
                        .collect();
                    let title = self.desktop_ui.knowledge_canvas.title_draft.trim();
                    if !title.is_empty() {
                        let mut changed = false;
                        if let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() {
                            if canvas.title != title {
                                canvas.title = title.to_owned();
                                changed = true;
                            }
                        }
                        if changed {
                            self.mark_knowledge_canvas_dirty();
                        }
                    }
                }
                if response.lost_focus()
                    || (response.has_focus()
                        && ui.ctx().input(|input| input.key_pressed(egui::Key::Enter)))
                {
                    let title = self
                        .desktop_ui
                        .knowledge_canvas
                        .title_draft
                        .trim()
                        .to_owned();
                    if title.is_empty() {
                        self.desktop_ui.knowledge_canvas.title_draft = self
                            .desktop_ui
                            .knowledge_canvas
                            .store
                            .active()
                            .map_or("知识画布", |canvas| canvas.title.as_str())
                            .to_owned();
                        self.status = "画布名称不能为空".into();
                    } else {
                        self.desktop_ui.knowledge_canvas.title_draft = title;
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            self.ui_knowledge_canvas_add_page_menu(ui);
            if ui
                .button("便签")
                .on_hover_text("在画布上写下一个想法")
                .clicked()
            {
                self.knowledge_canvas_add_note();
            }
            self.ui_knowledge_canvas_card_navigator(ui);
            let selected_note_ids = self
                .desktop_ui
                .knowledge_canvas
                .store
                .active()
                .map(|canvas| {
                    canvas
                        .nodes
                        .iter()
                        .filter(|node| {
                            node.kind == KnowledgeCanvasNodeKind::Note
                                && self
                                    .desktop_ui
                                    .knowledge_canvas
                                    .selected_nodes
                                    .contains(&node.id)
                        })
                        .map(|node| node.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !selected_note_ids.is_empty() {
                ui.menu_button("颜色", |ui| {
                    for (label, color) in [
                        ("暖黄", "amber"),
                        ("雾蓝", "blue"),
                        ("浅绿", "sage"),
                        ("柔粉", "rose"),
                    ] {
                        if ui.selectable_label(false, label).clicked() {
                            if let Some(canvas) =
                                self.desktop_ui.knowledge_canvas.store.active_mut()
                            {
                                for node in &mut canvas.nodes {
                                    if selected_note_ids.contains(&node.id) {
                                        node.color = color.into();
                                    }
                                }
                            }
                            self.mark_knowledge_canvas_dirty();
                            ui.close_menu();
                        }
                    }
                });
            }
            let connect_label = if self.desktop_ui.knowledge_canvas.connect_mode {
                "完成连接"
            } else {
                "连接"
            };
            if ui
                .add(egui::Button::new(connect_label).fill(
                    if self.desktop_ui.knowledge_canvas.connect_mode {
                        p.accent_soft
                    } else {
                        p.panel_alt
                    },
                ))
                .on_hover_text("选择两张卡片，在它们之间建立关系")
                .clicked()
            {
                let state = &mut self.desktop_ui.knowledge_canvas;
                state.connect_mode = !state.connect_mode;
                state.connect_from = None;
            }
            if ui
                .button("自动排布")
                .on_hover_text("按阅读顺序整理卡片")
                .clicked()
            {
                self.knowledge_canvas_auto_layout();
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("knowledge_canvas_fit_requested"), true)
                });
            }
            if ui
                .button("适应画布")
                .on_hover_text("缩放并居中显示所有内容")
                .clicked()
            {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new("knowledge_canvas_fit_requested"), true)
                });
            }
            if ui.small_button("−").on_hover_text("缩小").clicked() {
                self.knowledge_canvas_zoom_by(0.85);
            }
            let zoom = self
                .desktop_ui
                .knowledge_canvas
                .store
                .active()
                .map_or(100, |canvas| (canvas.zoom * 100.0).round() as i32);
            ui.label(
                egui::RichText::new(format!("{zoom}%"))
                    .small()
                    .color(p.muted),
            );
            if ui.small_button("＋").on_hover_text("放大").clicked() {
                self.knowledge_canvas_zoom_by(1.18);
            }

            let has_selection = !self.desktop_ui.knowledge_canvas.selected_nodes.is_empty()
                || self.desktop_ui.knowledge_canvas.selected_edge.is_some();
            if ui
                .add_enabled(has_selection, egui::Button::new("删除"))
                .on_hover_text("删除选中的卡片或关系")
                .clicked()
            {
                self.knowledge_canvas_delete_selection();
            }
            let item_count = self
                .desktop_ui
                .knowledge_canvas
                .store
                .active()
                .map_or((0, 0), |canvas| (canvas.nodes.len(), canvas.edges.len()));
            ui.label(
                egui::RichText::new(format!("{} 张卡片 · {} 条关系", item_count.0, item_count.1))
                    .small()
                    .color(p.muted),
            );
            let save_label = if self.desktop_ui.knowledge_canvas.save_error.is_some() {
                "保存失败"
            } else if self.desktop_ui.knowledge_canvas.dirty {
                "保存中"
            } else {
                "已保存"
            };
            ui.label(egui::RichText::new(save_label).small().color(
                if self.desktop_ui.knowledge_canvas.save_error.is_some() {
                    p.danger
                } else {
                    p.muted
                },
            ))
            .on_hover_text(
                self.desktop_ui
                    .knowledge_canvas
                    .save_error
                    .as_deref()
                    .unwrap_or("画布布局保存在当前 Windows 账户的本地工作区"),
            );
        });
    }

    fn reset_knowledge_canvas_interaction(&mut self) {
        let state = &mut self.desktop_ui.knowledge_canvas;
        state.selected_nodes.clear();
        state.selected_edge = None;
        state.connect_from = None;
        state.connect_mode = false;
        state.title_canvas_id.clear();
    }

    fn ui_knowledge_canvas_card_navigator(&mut self, ui: &mut egui::Ui) {
        let count = self
            .desktop_ui
            .knowledge_canvas
            .store
            .active()
            .map_or(0, |canvas| canvas.nodes.len());
        ui.menu_button(format!("卡片 · {count}"), |ui| {
            ui.set_min_width(270.0);
            if count == 0 {
                ui.label("当前画布还没有卡片");
                return;
            }
            let canvas = self.desktop_ui.knowledge_canvas.store.active().unwrap();
            let page_ids = canvas
                .nodes
                .iter()
                .filter(|node| node.kind == KnowledgeCanvasNodeKind::Page)
                .map(|node| node.page_id.as_str())
                .collect::<HashSet<_>>();
            let page_titles = self
                .data
                .notes
                .iter()
                .filter(|note| page_ids.contains(note.id.as_str()))
                .filter_map(|note| {
                    let unlocked = self
                        .note_unlocked_record
                        .as_ref()
                        .filter(|unlocked| unlocked.id == note.id);
                    knowledge_canvas_page_display(note, unlocked, &self.data.note_folders)
                        .map(|(title, _, _)| (note.id.as_str(), title))
                })
                .collect::<BTreeMap<_, _>>();
            let cards = canvas
                .nodes
                .iter()
                .map(|node| {
                    let label = if node.kind == KnowledgeCanvasNodeKind::Page {
                        page_titles
                            .get(node.page_id.as_str())
                            .map_or("页面已移除", String::as_str)
                    } else if node.text.trim().is_empty() {
                        "空白便签"
                    } else {
                        node.text.trim()
                    };
                    (
                        node.id.clone(),
                        knowledge_canvas_ellipsis(label, 28),
                        node.x + node.width * 0.5,
                        node.y + node.height * 0.5,
                    )
                })
                .collect::<Vec<_>>();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (id, label, x, y) in cards {
                        if ui.button(label).clicked() {
                            if let Some(canvas) =
                                self.desktop_ui.knowledge_canvas.store.active_mut()
                            {
                                canvas.center_x = x;
                                canvas.center_y = y;
                                canvas.zoom = 1.0;
                            }
                            let state = &mut self.desktop_ui.knowledge_canvas;
                            state.selected_nodes = vec![id];
                            state.selected_edge = None;
                            state.connect_from = None;
                            state.connect_mode = false;
                            self.mark_knowledge_canvas_dirty();
                            ui.close_menu();
                        }
                    }
                });
        });
    }

    fn ui_knowledge_canvas_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self
            .desktop_ui
            .knowledge_canvas
            .pending_delete_canvas_id
            .clone()
        else {
            return;
        };
        let Some(canvas) = self
            .desktop_ui
            .knowledge_canvas
            .store
            .canvases
            .iter()
            .find(|canvas| canvas.id == id)
        else {
            self.desktop_ui.knowledge_canvas.pending_delete_canvas_id = None;
            return;
        };
        let title = canvas.title.clone();
        let card_count = canvas.nodes.len();
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("删除画布")
            .id(egui::Id::new("knowledge_canvas_delete_dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("确定删除“{title}”？"));
                ui.label(format!("其中 {card_count} 张卡片及其关系会一并删除。"));
                ui.label("原知识页面会保留。");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                    if ui.button("删除画布").clicked() {
                        confirm = true;
                    }
                });
            });
        if confirm {
            if self.desktop_ui.knowledge_canvas.store.delete_canvas(&id) {
                self.reset_knowledge_canvas_interaction();
                self.mark_knowledge_canvas_dirty();
            }
            self.desktop_ui.knowledge_canvas.pending_delete_canvas_id = None;
        } else if cancel {
            self.desktop_ui.knowledge_canvas.pending_delete_canvas_id = None;
        }
    }

    fn ui_knowledge_canvas_add_page_menu(&mut self, ui: &mut egui::Ui) {
        let mut add_page = None;
        ui.menu_button("＋ 页面", |ui| {
            ui.set_min_width(280.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.desktop_ui.knowledge_canvas.add_page_search)
                    .desired_width(f32::INFINITY)
                    .hint_text("搜索知识页面"),
            );
            ui.separator();
            let query = self
                .desktop_ui
                .knowledge_canvas
                .add_page_search
                .trim()
                .to_lowercase();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    let mut pages = self
                        .data
                        .notes
                        .iter()
                        .filter(|note| {
                            desktop_note_kind(note) == DesktopNoteKind::Document
                                && note.deleted_at_epoch_millis.is_none()
                        })
                        .collect::<Vec<_>>();
                    pages.sort_by(|a, b| {
                        b.updated_at_epoch_millis
                            .cmp(&a.updated_at_epoch_millis)
                            .then_with(|| a.title.cmp(&b.title))
                    });
                    let mut shown = 0usize;
                    for note in pages {
                        let unlocked = self
                            .note_unlocked_record
                            .as_ref()
                            .is_some_and(|unlocked| unlocked.id == note.id);
                        let locked = note.encryption.is_some() && !unlocked;
                        let title = if locked {
                            "🔒 加密页面".to_string()
                        } else if unlocked {
                            self.note_unlocked_record
                                .as_ref()
                                .map(|value| value.title.clone())
                                .unwrap_or_else(|| "未命名页面".into())
                        } else if note.title.trim().is_empty() {
                            "未命名页面".into()
                        } else {
                            note.title.clone()
                        };
                        if !query.is_empty() && (locked || !title.to_lowercase().contains(&query)) {
                            continue;
                        }
                        let id = note.id.clone();
                        if ui
                            .button(format!("{}  {}", if locked { "▣" } else { "▤" }, title))
                            .on_hover_text(if locked {
                                "页面内容保持加密；加入画布只会显示锁定状态"
                            } else {
                                "加入当前画布"
                            })
                            .clicked()
                        {
                            add_page = Some(id);
                            ui.close_menu();
                        }
                        shown += 1;
                        if shown >= 40 {
                            break;
                        }
                    }
                    if shown == 0 {
                        ui.label(egui::RichText::new("没有匹配的页面").color(palette().muted));
                    }
                });
        });
        if let Some(page_id) = add_page {
            self.knowledge_canvas_add_page(&page_id);
        }
    }

    fn draw_knowledge_canvas_background(&mut self, ui: &mut egui::Ui, rect: egui::Rect) -> bool {
        let p = palette();
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 12.0, p.bg);
        painter.rect_stroke(rect, 12.0, egui::Stroke::new(1.0, p.line));
        let canvas = match self.desktop_ui.knowledge_canvas.store.active() {
            Some(canvas) => canvas,
            None => return false,
        };
        let spacing = (36.0 * canvas.zoom).max(14.0);
        let left_world = canvas.center_x + (rect.left() - rect.center().x) / canvas.zoom;
        let right_world = canvas.center_x + (rect.right() - rect.center().x) / canvas.zoom;
        let top_world = canvas.center_y + (rect.top() - rect.center().y) / canvas.zoom;
        let bottom_world = canvas.center_y + (rect.bottom() - rect.center().y) / canvas.zoom;
        let step_world = spacing / canvas.zoom;
        let line_color = p.line.linear_multiply(0.38);
        let x_start = (left_world / step_world).floor() as i32;
        let x_end = (right_world / step_world).ceil() as i32;
        for step in x_start..=x_end {
            let world = step as f32 * step_world;
            let x = rect.center().x + (world - canvas.center_x) * canvas.zoom;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(0.65, line_color),
            );
        }
        let y_start = (top_world / step_world).floor() as i32;
        let y_end = (bottom_world / step_world).ceil() as i32;
        for step in y_start..=y_end {
            let world = step as f32 * step_world;
            let y = rect.center().y + (world - canvas.center_y) * canvas.zoom;
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                egui::Stroke::new(0.65, line_color),
            );
        }
        let fit_requested = ui.ctx().data_mut(|data| {
            data.get_temp::<bool>(egui::Id::new("knowledge_canvas_fit_requested"))
                .unwrap_or(false)
        });
        if fit_requested {
            ui.ctx().data_mut(|data| {
                data.remove::<bool>(egui::Id::new("knowledge_canvas_fit_requested"));
            });
        }
        fit_requested
    }

    fn draw_knowledge_canvas_edges(
        &mut self,
        ui: &mut egui::Ui,
        board_rect: egui::Rect,
        canvas_index: usize,
    ) {
        let canvas = &self.desktop_ui.knowledge_canvas.store.canvases[canvas_index];
        let mut rects = std::collections::HashMap::with_capacity(canvas.nodes.len());
        for node in &canvas.nodes {
            rects.insert(
                node.id.as_str(),
                knowledge_canvas_screen_rect(node, canvas, board_rect),
            );
        }
        let zoom = canvas.zoom;
        let selected_edge = self.desktop_ui.knowledge_canvas.selected_edge.as_deref();
        let mut clicked_edge = None;
        let painter = ui.painter().with_clip_rect(board_rect);
        for edge in &canvas.edges {
            let (Some(from), Some(to)) =
                (rects.get(edge.from.as_str()), rects.get(edge.to.as_str()))
            else {
                continue;
            };
            let from_center = from.center();
            let to_center = to.center();
            let direction = to_center - from_center;
            if direction.length_sq() < 1.0 {
                continue;
            }
            let start = knowledge_canvas_edge_point(*from, direction);
            let tip = knowledge_canvas_edge_point(*to, -direction);
            if (tip - start).length_sq() < 1.0 {
                continue;
            }
            let hit = egui::Rect::from_two_pos(start, tip).expand(12.0);
            if !hit.intersects(board_rect) {
                continue;
            }
            let selected = selected_edge == Some(edge.id.as_str());
            let color = if selected {
                palette().accent
            } else {
                palette().muted
            };
            let stroke = egui::Stroke::new(if selected { 2.3 } else { 1.55 }, color);
            painter.line_segment([start, tip], stroke);
            let arrow = (tip - start).normalized();
            let side = egui::vec2(-arrow.y, arrow.x);
            let head = (8.0 * zoom).clamp(5.0, 12.0);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip,
                    tip - arrow * head + side * head * 0.48,
                    tip - arrow * head - side * head * 0.48,
                ],
                color,
                egui::Stroke::NONE,
            ));
            let response = ui.interact(
                hit.intersect(board_rect),
                egui::Id::new(("knowledge_canvas_edge", &edge.id)),
                egui::Sense::click(),
            );
            let pointer = ui.ctx().input(|input| input.pointer.interact_pos());
            let on_line = pointer.is_some_and(|point| {
                knowledge_canvas_point_segment_distance(point, start, tip) <= 8.0
            });
            if response.clicked() && on_line {
                clicked_edge = Some(edge.id.clone());
            }
        }
        if let Some(id) = clicked_edge {
            self.desktop_ui.knowledge_canvas.selected_edge = Some(id);
            self.desktop_ui.knowledge_canvas.selected_nodes.clear();
        }
    }

    fn draw_knowledge_canvas_nodes(
        &mut self,
        ui: &mut egui::Ui,
        board_rect: egui::Rect,
        canvas_index: usize,
    ) {
        let (node_count, zoom) = {
            let canvas = &self.desktop_ui.knowledge_canvas.store.canvases[canvas_index];
            (canvas.nodes.len(), canvas.zoom)
        };
        let page_lookup = self.knowledge_note_indices();
        let ctrl = ui
            .ctx()
            .input(|input| input.modifiers.ctrl || input.modifiers.command);
        let p = palette();
        let mut changed = false;
        let mut open_page = None;
        let mut node_clicks = Vec::new();

        for index in 0..node_count {
            let snapshot = {
                let canvas = &self.desktop_ui.knowledge_canvas.store.canvases[canvas_index];
                let node = &canvas.nodes[index];
                let rect = knowledge_canvas_screen_rect(node, canvas, board_rect);
                rect.intersects(board_rect).then(|| {
                    // The note body can be thousands of characters. Keep only the
                    // small display fields here and edit the stored text in place.
                    let display = KnowledgeCanvasNode {
                        id: node.id.clone(),
                        kind: node.kind,
                        page_id: node.page_id.clone(),
                        text: String::new(),
                        x: node.x,
                        y: node.y,
                        width: node.width,
                        height: node.height,
                        color: node.color.clone(),
                    };
                    (display, rect)
                })
            };
            let Some((node, rect)) = snapshot else {
                continue;
            };
            let (fill, accent) = knowledge_canvas_node_color(&node.color, p);
            let selected = self
                .desktop_ui
                .knowledge_canvas
                .selected_nodes
                .iter()
                .any(|id| id == &node.id);
            let painter = ui.painter().with_clip_rect(board_rect);
            painter.rect_filled(
                rect.translate(egui::vec2(0.0, 3.0)),
                10.0,
                p.line.linear_multiply(0.2),
            );
            painter.rect_filled(rect, 10.0, fill);
            painter.rect_stroke(
                rect,
                10.0,
                egui::Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if selected { p.accent } else { p.line },
                ),
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    rect.left_top(),
                    egui::pos2(rect.right(), rect.top() + 5.0 * zoom),
                ),
                3.0,
                accent,
            );
            let painter = ui.painter().with_clip_rect(rect.intersect(board_rect));

            let header_height = (38.0 * zoom).clamp(30.0, 46.0);
            let header_rect = egui::Rect::from_min_max(
                rect.left_top() + egui::vec2(0.0, 3.0),
                egui::pos2(rect.right(), rect.top() + header_height),
            );
            let header_response = ui
                .interact(
                    header_rect,
                    egui::Id::new(("knowledge_canvas_node_header", &node.id)),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("拖动卡片 · 双击页面卡片打开文档");
            if header_response.drag_started_by(egui::PointerButton::Primary) && !selected {
                self.desktop_ui.knowledge_canvas.selected_nodes = vec![node.id.clone()];
                self.desktop_ui.knowledge_canvas.selected_edge = None;
            }
            if header_response.dragged_by(egui::PointerButton::Primary) {
                let delta = ui.ctx().input(|input| input.pointer.delta()) / zoom;
                if delta.length_sq() > 0.0 {
                    let state = &mut self.desktop_ui.knowledge_canvas;
                    let canvas = &mut state.store.canvases[canvas_index];
                    if selected && state.selected_nodes.len() > 1 {
                        let selected_ids = state
                            .selected_nodes
                            .iter()
                            .map(String::as_str)
                            .collect::<HashSet<_>>();
                        for item in &mut canvas.nodes {
                            if selected_ids.contains(item.id.as_str()) {
                                item.x = (item.x + delta.x).clamp(-200_000.0, 200_000.0);
                                item.y = (item.y + delta.y).clamp(-200_000.0, 200_000.0);
                            }
                        }
                    } else {
                        let item = &mut canvas.nodes[index];
                        item.x = (item.x + delta.x).clamp(-200_000.0, 200_000.0);
                        item.y = (item.y + delta.y).clamp(-200_000.0, 200_000.0);
                    }
                    changed = true;
                }
            }
            if header_response.clicked() {
                node_clicks.push((
                    node.id.clone(),
                    node.page_id.clone(),
                    header_response.double_clicked(),
                ));
            }

            match node.kind {
                KnowledgeCanvasNodeKind::Page => {
                    let display = page_lookup
                        .get(&node.page_id)
                        .and_then(|index| self.data.notes.get(*index))
                        .and_then(|note| {
                            let unlocked = self
                                .note_unlocked_record
                                .as_ref()
                                .filter(|unlocked| unlocked.id == note.id);
                            knowledge_canvas_page_display(note, unlocked, &self.data.note_folders)
                        });
                    let Some((title, detail, locked)) = display else {
                        painter.text(
                            rect.min + egui::vec2(16.0, header_height + 10.0),
                            egui::Align2::LEFT_TOP,
                            "页面已移除",
                            egui::FontId::proportional((14.0 * zoom).clamp(12.0, 17.0)),
                            p.muted,
                        );
                        continue;
                    };
                    let title_font_size = (15.0 * zoom).clamp(12.0, 17.0);
                    let title_start = 38.0 * zoom;
                    let title_char_limit = ((rect.width() - title_start - 14.0) / title_font_size)
                        .floor()
                        .max(1.0) as usize;
                    let title = knowledge_canvas_ellipsis(&title, title_char_limit);
                    painter.text(
                        rect.min + egui::vec2(14.0 * zoom, 16.0 * zoom + 5.0),
                        egui::Align2::LEFT_CENTER,
                        if locked { "▣" } else { "▤" },
                        egui::FontId::proportional(title_font_size.min(18.0)),
                        accent,
                    );
                    painter.text(
                        rect.min + egui::vec2(38.0 * zoom, 16.0 * zoom + 5.0),
                        egui::Align2::LEFT_CENTER,
                        title,
                        egui::FontId::proportional(title_font_size),
                        p.text,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + 14.0, rect.top() + header_height),
                            egui::pos2(rect.right() - 14.0, rect.top() + header_height),
                        ],
                        egui::Stroke::new(1.0, p.line),
                    );
                    painter.text(
                        rect.min + egui::vec2(16.0, header_height + 15.0),
                        egui::Align2::LEFT_TOP,
                        detail,
                        egui::FontId::proportional((12.0 * zoom).clamp(10.5, 13.0)),
                        p.muted,
                    );
                    if !locked {
                        painter.text(
                            rect.min + egui::vec2(16.0, header_height + 42.0 * zoom),
                            egui::Align2::LEFT_TOP,
                            "双击打开页面",
                            egui::FontId::proportional((11.0 * zoom).clamp(10.0, 12.0)),
                            p.muted.linear_multiply(0.85),
                        );
                    }
                }
                KnowledgeCanvasNodeKind::Note => {
                    let body_rect = egui::Rect::from_min_max(
                        rect.min + egui::vec2(13.0, header_height + 4.0),
                        rect.max - egui::vec2(13.0, 10.0),
                    );
                    let item = &mut self.desktop_ui.knowledge_canvas.store.canvases[canvas_index]
                        .nodes[index];
                    let response = ui.allocate_ui_at_rect(body_rect, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut item.text)
                                .id(egui::Id::new(("knowledge_canvas_note", &node.id)))
                                .desired_width(f32::INFINITY)
                                .desired_rows(4)
                                .font(egui::FontId::proportional((13.0 * zoom).clamp(11.0, 15.0)))
                                .text_color(p.text)
                                .frame(false)
                                .hint_text("写下你的想法…"),
                        )
                    });
                    if response.inner.changed() {
                        if let Some((end, _)) = item.text.char_indices().nth(8_000) {
                            item.text.truncate(end);
                        }
                        changed = true;
                    }
                }
            }

            let handle_size = (14.0 * zoom).clamp(10.0, 17.0);
            let handle_rect = egui::Rect::from_min_size(
                rect.right_bottom() - egui::vec2(handle_size, handle_size),
                egui::vec2(handle_size, handle_size),
            );
            painter.line_segment(
                [
                    handle_rect.right_bottom() - egui::vec2(2.0, handle_size * 0.62),
                    handle_rect.right_bottom() - egui::vec2(handle_size * 0.62, 2.0),
                ],
                egui::Stroke::new(1.2, p.muted),
            );
            painter.line_segment(
                [
                    handle_rect.right_bottom() - egui::vec2(2.0, handle_size * 0.34),
                    handle_rect.right_bottom() - egui::vec2(handle_size * 0.34, 2.0),
                ],
                egui::Stroke::new(1.2, p.muted),
            );
            let resize_response = ui
                .interact(
                    handle_rect.intersect(board_rect),
                    egui::Id::new(("knowledge_canvas_resize", &node.id)),
                    egui::Sense::drag(),
                )
                .on_hover_text("拖动调整卡片大小");
            if resize_response.dragged_by(egui::PointerButton::Primary) {
                let delta = ui.ctx().input(|input| input.pointer.delta()) / zoom;
                let item =
                    &mut self.desktop_ui.knowledge_canvas.store.canvases[canvas_index].nodes[index];
                item.width = (item.width + delta.x).clamp(180.0, 600.0);
                item.height = (item.height + delta.y).clamp(140.0, 520.0);
                changed = true;
            }
        }

        for (id, page_id, double_clicked) in node_clicks {
            self.desktop_ui.knowledge_canvas.selected_edge = None;
            if self.desktop_ui.knowledge_canvas.connect_mode {
                if let Some(source) = self.desktop_ui.knowledge_canvas.connect_from.take() {
                    if source != id {
                        let edges =
                            &self.desktop_ui.knowledge_canvas.store.canvases[canvas_index].edges;
                        let duplicate = edges.iter().any(|edge| {
                            (edge.from == source && edge.to == id)
                                || (edge.from == id && edge.to == source)
                        });
                        if !duplicate && edges.len() >= KNOWLEDGE_CANVAS_EDGE_LIMIT {
                            self.status = "当前画布已达到关系上限".into();
                        } else if !duplicate {
                            self.desktop_ui.knowledge_canvas.store.canvases[canvas_index]
                                .edges
                                .push(KnowledgeCanvasEdge {
                                    id: random_desktop_identifier("knowledge-canvas-edge"),
                                    from: source,
                                    to: id.clone(),
                                });
                            changed = true;
                        }
                    }
                } else {
                    self.desktop_ui.knowledge_canvas.connect_from = Some(id.clone());
                    self.desktop_ui.knowledge_canvas.selected_nodes = vec![id.clone()];
                }
            } else if ctrl {
                if let Some(index) = self
                    .desktop_ui
                    .knowledge_canvas
                    .selected_nodes
                    .iter()
                    .position(|selected| selected == &id)
                {
                    self.desktop_ui
                        .knowledge_canvas
                        .selected_nodes
                        .remove(index);
                } else {
                    self.desktop_ui
                        .knowledge_canvas
                        .selected_nodes
                        .push(id.clone());
                }
            } else {
                self.desktop_ui.knowledge_canvas.selected_nodes = vec![id.clone()];
            }
            if double_clicked
                && !self.desktop_ui.knowledge_canvas.connect_mode
                && !page_id.is_empty()
            {
                open_page = Some(page_id);
            }
        }
        if changed {
            self.mark_knowledge_canvas_dirty();
        }
        if let Some(page_id) = open_page {
            self.select_note_by_id(&page_id);
        }
    }

    fn canvas_page_title(&self, page_id: &str) -> Option<(String, String, bool)> {
        let note = self.data.notes.iter().find(|note| {
            note.id == page_id && desktop_note_kind(note) == DesktopNoteKind::Document
        })?;
        let unlocked = self
            .note_unlocked_record
            .as_ref()
            .filter(|item| item.id == note.id);
        knowledge_canvas_page_display(note, unlocked, &self.data.note_folders)
    }

    fn knowledge_canvas_add_page(&mut self, page_id: &str) {
        let Some((_, _, _)) = self.canvas_page_title(page_id) else {
            self.status = "此页面不可加入画布".into();
            return;
        };
        let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() else {
            return;
        };
        if canvas.nodes.len() >= KNOWLEDGE_CANVAS_NODE_LIMIT {
            self.status = "当前画布已达到卡片上限".into();
            return;
        }
        if canvas
            .nodes
            .iter()
            .any(|node| node.kind == KnowledgeCanvasNodeKind::Page && node.page_id == page_id)
        {
            self.status = "该页面已在当前画布中".into();
            return;
        }
        let (x, y) = knowledge_canvas_next_position(canvas);
        let node = KnowledgeCanvasNode {
            id: random_desktop_identifier("knowledge-canvas-node"),
            kind: KnowledgeCanvasNodeKind::Page,
            page_id: page_id.into(),
            x,
            y,
            ..Default::default()
        };
        let node_id = node.id.clone();
        canvas.nodes.push(node);
        self.desktop_ui.knowledge_canvas.selected_nodes = vec![node_id];
        self.desktop_ui.knowledge_canvas.selected_edge = None;
        self.desktop_ui.knowledge_canvas.connect_from = None;
        self.desktop_ui.knowledge_canvas.connect_mode = false;
        self.mark_knowledge_canvas_dirty();
    }

    fn knowledge_canvas_add_note(&mut self) {
        let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() else {
            return;
        };
        if canvas.nodes.len() >= KNOWLEDGE_CANVAS_NODE_LIMIT {
            self.status = "当前画布已达到卡片上限".into();
            return;
        }
        let (x, y) = knowledge_canvas_next_position(canvas);
        let node = KnowledgeCanvasNode {
            id: random_desktop_identifier("knowledge-canvas-note"),
            kind: KnowledgeCanvasNodeKind::Note,
            text: String::new(),
            x,
            y,
            width: 248.0,
            height: 204.0,
            color: "amber".into(),
            ..Default::default()
        };
        let node_id = node.id.clone();
        canvas.nodes.push(node);
        self.desktop_ui.knowledge_canvas.selected_nodes = vec![node_id];
        self.desktop_ui.knowledge_canvas.selected_edge = None;
        self.desktop_ui.knowledge_canvas.connect_from = None;
        self.desktop_ui.knowledge_canvas.connect_mode = false;
        self.mark_knowledge_canvas_dirty();
    }

    fn knowledge_canvas_delete_selection(&mut self) {
        let selected = self.desktop_ui.knowledge_canvas.selected_nodes.clone();
        let selected_edge = self.desktop_ui.knowledge_canvas.selected_edge.clone();
        if let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() {
            if !selected.is_empty() {
                canvas.nodes.retain(|node| !selected.contains(&node.id));
                canvas
                    .edges
                    .retain(|edge| !selected.contains(&edge.from) && !selected.contains(&edge.to));
            } else if let Some(edge_id) = selected_edge.as_deref() {
                canvas.edges.retain(|edge| edge.id != edge_id);
            }
        }
        self.desktop_ui.knowledge_canvas.selected_nodes.clear();
        self.desktop_ui.knowledge_canvas.selected_edge = None;
        self.desktop_ui.knowledge_canvas.connect_from = None;
        self.mark_knowledge_canvas_dirty();
    }

    fn knowledge_canvas_zoom_by(&mut self, factor: f32) {
        if let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() {
            canvas.zoom = (canvas.zoom * factor).clamp(0.35, 2.5);
            self.mark_knowledge_canvas_dirty();
        }
    }

    fn knowledge_canvas_auto_layout(&mut self) {
        if let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() {
            let count = canvas.nodes.len();
            if count == 0 {
                return;
            }
            let mut order = Vec::with_capacity(count);
            order.extend(
                canvas
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.kind == KnowledgeCanvasNodeKind::Page)
                    .map(|(index, _)| index),
            );
            order.extend(
                canvas
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.kind == KnowledgeCanvasNodeKind::Note)
                    .map(|(index, _)| index),
            );
            let columns = (count as f32).sqrt().ceil() as usize;
            let rows = count.div_ceil(columns);
            let cell_width = canvas
                .nodes
                .iter()
                .map(|node| node.width)
                .fold(0.0_f32, f32::max)
                + 36.0;
            let cell_height = canvas
                .nodes
                .iter()
                .map(|node| node.height)
                .fold(0.0_f32, f32::max)
                + 36.0;
            let base_x = (canvas.center_x - columns as f32 * cell_width * 0.5)
                .clamp(-200_000.0, 200_000.0 - (columns - 1) as f32 * cell_width);
            let base_y = (canvas.center_y - rows as f32 * cell_height * 0.5)
                .clamp(-200_000.0, 200_000.0 - (rows - 1) as f32 * cell_height);
            for (position, node_index) in order.into_iter().enumerate() {
                let node = &mut canvas.nodes[node_index];
                node.x = base_x + (position % columns) as f32 * cell_width;
                node.y = base_y + (position / columns) as f32 * cell_height;
            }
            self.mark_knowledge_canvas_dirty();
        }
    }

    fn fit_knowledge_canvas_to_rect(&mut self, rect: egui::Rect) {
        let Some(canvas) = self.desktop_ui.knowledge_canvas.store.active_mut() else {
            return;
        };
        if canvas.nodes.is_empty() {
            canvas.center_x = 0.0;
            canvas.center_y = 0.0;
            canvas.zoom = 1.0;
        } else {
            let left = canvas
                .nodes
                .iter()
                .map(|node| node.x)
                .fold(f32::INFINITY, f32::min);
            let top = canvas
                .nodes
                .iter()
                .map(|node| node.y)
                .fold(f32::INFINITY, f32::min);
            let right = canvas
                .nodes
                .iter()
                .map(|node| node.x + node.width)
                .fold(f32::NEG_INFINITY, f32::max);
            let bottom = canvas
                .nodes
                .iter()
                .map(|node| node.y + node.height)
                .fold(f32::NEG_INFINITY, f32::max);
            let width = (right - left).max(1.0);
            let height = (bottom - top).max(1.0);
            canvas.center_x = (left + right) * 0.5;
            canvas.center_y = (top + bottom) * 0.5;
            canvas.zoom = ((rect.width() - 100.0) / width)
                .min((rect.height() - 100.0) / height)
                .clamp(0.35, 1.7);
        }
        self.mark_knowledge_canvas_dirty();
    }

    fn handle_knowledge_canvas_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            let state = &mut self.desktop_ui.knowledge_canvas;
            state.connect_mode = false;
            state.connect_from = None;
        } else if ctx.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        }) && (!self.desktop_ui.knowledge_canvas.selected_nodes.is_empty()
            || self.desktop_ui.knowledge_canvas.selected_edge.is_some())
        {
            self.knowledge_canvas_delete_selection();
        }
    }
}

fn knowledge_canvas_page_display(
    note: &DesktopNote,
    unlocked: Option<&DesktopNote>,
    folders: &[DesktopNoteFolder],
) -> Option<(String, String, bool)> {
    if desktop_note_kind(note) != DesktopNoteKind::Document
        || note.deleted_at_epoch_millis.is_some()
    {
        return None;
    }
    let locked = note.encryption.is_some() && unlocked.is_none();
    let title = if locked {
        "加密页面".into()
    } else if let Some(unlocked) = unlocked {
        if unlocked.title.trim().is_empty() {
            "未命名页面".into()
        } else {
            unlocked.title.clone()
        }
    } else if note.title.trim().is_empty() {
        "未命名页面".into()
    } else {
        note.title.clone()
    };
    let detail = if locked {
        "页面内容保持加密".into()
    } else {
        note.folder_id
            .as_ref()
            .and_then(|folder_id| folders.iter().find(|folder| &folder.id == folder_id))
            .map(|folder| folder.name.clone())
            .unwrap_or_else(|| {
                if note
                    .document
                    .knowledge
                    .as_ref()
                    .is_some_and(|page| page.database.is_some())
                {
                    "知识数据库".into()
                } else {
                    "知识页面".into()
                }
            })
    };
    Some((title, detail, locked))
}
fn knowledge_canvas_next_position(canvas: &KnowledgeCanvas) -> (f32, f32) {
    let index = canvas.nodes.len();
    let column = index % 4;
    let row = index / 4;
    (
        (canvas.center_x - 500.0 + column as f32 * 300.0).clamp(-200_000.0, 200_000.0),
        (canvas.center_y - 180.0 + row as f32 * 230.0).clamp(-200_000.0, 200_000.0),
    )
}

fn knowledge_canvas_screen_rect(
    node: &KnowledgeCanvasNode,
    canvas: &KnowledgeCanvas,
    board: egui::Rect,
) -> egui::Rect {
    let min = board.center()
        + egui::vec2(
            (node.x - canvas.center_x) * canvas.zoom,
            (node.y - canvas.center_y) * canvas.zoom,
        );
    egui::Rect::from_min_size(min, egui::vec2(node.width, node.height) * canvas.zoom)
}

fn knowledge_canvas_edge_point(rect: egui::Rect, direction: egui::Vec2) -> egui::Pos2 {
    let half = rect.size() * 0.5;
    let dx = direction.x.abs().max(0.001);
    let dy = direction.y.abs().max(0.001);
    let scale = (half.x / dx).min(half.y / dy);
    rect.center() + direction * scale
}

fn knowledge_canvas_point_segment_distance(
    point: egui::Pos2,
    start: egui::Pos2,
    end: egui::Pos2,
) -> f32 {
    let segment = end - start;
    let denominator = segment.length_sq();
    if denominator <= f32::EPSILON {
        return point.distance(start);
    }
    let along = ((point - start).dot(segment) / denominator).clamp(0.0, 1.0);
    point.distance(start + segment * along)
}

fn knowledge_canvas_ellipsis(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        value.into()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(maximum.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn knowledge_canvas_node_color(color: &str, palette: Palette) -> (egui::Color32, egui::Color32) {
    match color {
        "amber" => (
            if palette.is_dark {
                egui::Color32::from_rgb(54, 46, 33)
            } else {
                egui::Color32::from_rgb(255, 248, 229)
            },
            if palette.is_dark {
                egui::Color32::from_rgb(224, 177, 92)
            } else {
                egui::Color32::from_rgb(210, 151, 49)
            },
        ),
        "sage" => (
            if palette.is_dark {
                egui::Color32::from_rgb(37, 52, 47)
            } else {
                egui::Color32::from_rgb(235, 247, 241)
            },
            if palette.is_dark {
                egui::Color32::from_rgb(112, 196, 160)
            } else {
                egui::Color32::from_rgb(65, 151, 119)
            },
        ),
        "rose" => (
            if palette.is_dark {
                egui::Color32::from_rgb(55, 40, 47)
            } else {
                egui::Color32::from_rgb(255, 239, 243)
            },
            if palette.is_dark {
                egui::Color32::from_rgb(217, 133, 158)
            } else {
                egui::Color32::from_rgb(191, 95, 123)
            },
        ),
        _ => (
            if palette.is_dark {
                egui::Color32::from_rgb(38, 47, 65)
            } else {
                egui::Color32::from_rgb(238, 243, 255)
            },
            palette.accent,
        ),
    }
}
