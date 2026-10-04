// v2.22.49.5 Android - Encode borrowed replies and send only viewport deltas for pan/zoom.
// v2.22.49.4 Android - Account-scoped, transactional knowledge canvases.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

// Inactive boards and unchanged graph data are shared between transactions and
// undo entries. Panning a large board does not clone all its cards or strings.
#[derive(Clone, Debug)]
pub struct Shared<T>(Arc<T>);
impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}
impl<T> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self(Arc::new(value))
    }
}
impl<T> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: Clone> std::ops::DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}
impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_ref().serialize(serializer)
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Into::into)
    }
}

const MAX_BYTES: usize = 16 * 1024 * 1024;
const HISTORY_BYTES: usize = 2 * 1024 * 1024;
const MAX_NODES: usize = 1000;
const MAX_EDGES: usize = 4000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub page_id: String,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Canvas {
    pub id: String,
    pub title: String,
    pub center_x: f32,
    pub center_y: f32,
    pub zoom: f32,
    pub nodes: Shared<Vec<Node>>,
    pub edges: Shared<Vec<Edge>>,
}

impl Canvas {
    fn new(title: &str) -> Self {
        Self {
            id: identifier(),
            title: title.into(),
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
            nodes: vec![].into(),
            edges: vec![].into(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id)
            || self.title.trim().is_empty()
            || self.title.chars().count() > 80
            || !coordinate(self.center_x)
            || !coordinate(self.center_y)
            || !self.zoom.is_finite()
            || !(0.35..=2.5).contains(&self.zoom)
            || self.nodes.len() > MAX_NODES
            || self.edges.len() > MAX_EDGES
        {
            return Err("画布数据超出允许范围".into());
        }
        let mut ids = HashSet::with_capacity(self.nodes.len());
        let mut pages = HashSet::new();
        for node in self.nodes.iter() {
            if !valid_id(&node.id)
                || !ids.insert(node.id.as_str())
                || !coordinate(node.x)
                || !coordinate(node.y)
                || !node.width.is_finite()
                || !(160.0..=720.0).contains(&node.width)
                || !node.height.is_finite()
                || !(120.0..=720.0).contains(&node.height)
                || node.text.chars().count() > 8000
                || !["blue", "green", "amber", "pink", "purple"].contains(&node.color.as_str())
                || !["page", "note"].contains(&node.kind.as_str())
                || (node.kind == "page"
                    && (!valid_id(&node.page_id)
                        || !pages.insert(node.page_id.as_str())
                        || !node.text.is_empty()))
                || (node.kind == "note" && !node.page_id.is_empty())
            {
                return Err("卡片数据无效".into());
            }
        }
        let mut edge_ids = HashSet::new();
        let mut pairs = HashSet::new();
        for edge in self.edges.iter() {
            if !valid_id(&edge.id)
                || !edge_ids.insert(&edge.id)
                || edge.from == edge.to
                || !ids.contains(edge.from.as_str())
                || !ids.contains(edge.to.as_str())
                || !pairs.insert((&edge.from, &edge.to))
            {
                return Err("连线数据无效".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Store {
    version: u32,
    revision: u64,
    active_canvas_id: String,
    canvases: Vec<Shared<Canvas>>,
}

impl Store {
    fn initial() -> Self {
        let canvas = Canvas::new("知识画布");
        Self {
            version: 1,
            revision: 0,
            active_canvas_id: canvas.id.clone(),
            canvases: vec![canvas.into()],
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.canvases.is_empty() || self.canvases.len() > 128 {
            return Err("画布文件版本或数量不受支持".into());
        }
        let mut ids = HashSet::new();
        for canvas in &self.canvases {
            canvas.validate()?;
            if !ids.insert(&canvas.id) {
                return Err("画布标识重复".into());
            }
        }
        if !ids.contains(&self.active_canvas_id) {
            return Err("当前画布不存在".into());
        }
        Ok(())
    }
    fn active_index(&self) -> usize {
        self.canvases
            .iter()
            .position(|c| c.id == self.active_canvas_id)
            .expect("validated active canvas")
    }
}

struct Session {
    path: PathBuf,
    store: Store,
    fingerprint: [u8; 32],
    undo: VecDeque<Shared<Canvas>>,
    redo: VecDeque<Shared<Canvas>>,
    warning: String,
}

fn identifier() -> String {
    format!("{:032x}", rand::random::<u128>())
}
fn valid_id(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256
}
fn coordinate(v: f32) -> bool {
    v.is_finite() && v.abs() <= 250_000.0
}
fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scoped_path(root: &Path, workspace: &str) -> Result<PathBuf, String> {
    if !root.is_absolute() || !valid_id(workspace) {
        return Err("画布账户未准备好".into());
    }
    let digest = Sha256::digest(workspace.as_bytes());
    let name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Ok(root
        .join("knowledge_canvases")
        .join(name)
        .join("canvases.json"))
}

fn read_store_with_digest(path: &Path) -> Result<(Store, [u8; 32]), String> {
    let file = File::open(path).map_err(|_| "无法读取画布文件".to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "画布读取失败")?;
    if bytes.len() > MAX_BYTES {
        return Err("画布文件过大".into());
    }
    let store: Store = serde_json::from_slice(&bytes).map_err(|_| "画布文件损坏或版本不受支持")?;
    store.validate()?;
    Ok((store, Sha256::digest(&bytes).into()))
}

#[cfg(test)]
fn read_store(path: &Path) -> Result<Store, String> {
    read_store_with_digest(path).map(|(store, _)| store)
}

fn file_digest(path: &Path) -> Result<[u8; 32], String> {
    let mut file = File::open(path).map_err(|_| "无法核对画布文件")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0;
    loop {
        let count = file.read(&mut buffer).map_err(|_| "画布核对失败")?;
        if count == 0 {
            break;
        }
        total += count;
        if total > MAX_BYTES {
            return Err("画布文件过大".into());
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

// The backup remains valid across interruption before, during or after replacement.
// All callers hold the session mutex; revision checks reject a second stale editor.
fn persist(path: &Path, store: &Store, expected: Option<[u8; 32]>) -> Result<[u8; 32], String> {
    store.validate()?;
    if let Some(fingerprint) = expected {
        if file_digest(path)? != fingerprint {
            return Err("画布已在另一窗口修改，请返回后重新打开".into());
        }
    } else if path.exists() {
        return Err("画布文件已存在，已停止覆盖".into());
    }
    let bytes = serde_json::to_vec(store).map_err(|_| "画布编码失败")?;
    if bytes.len() > MAX_BYTES {
        return Err("画布总容量已达上限，请精简卡片内容".into());
    }
    let parent = path.parent().ok_or("画布保存路径无效")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建画布目录")?;
    let pending = parent.join(format!("{}.pending", identifier()));
    let staged_backup = parent.join(format!("{}.backup", identifier()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)
            .map_err(|_| "无法写入画布")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "画布保存失败，请检查存储空间")?;
        drop(file);
        if path.exists() {
            let backup = path.with_extension("bak");
            fs::copy(path, &staged_backup).map_err(|_| "画布备份失败")?;
            OpenOptions::new()
                .write(true)
                .open(&staged_backup)
                .and_then(|f| f.sync_all())
                .map_err(|_| "画布备份写入失败")?;
            // Windows host tests need remove-before-rename; Android rename replaces atomically.
            #[cfg(target_os = "windows")]
            if backup.exists() {
                fs::remove_file(&backup).map_err(|_| "画布备份更新失败")?;
            }
            fs::rename(&staged_backup, &backup).map_err(|_| "画布备份提交失败")?;
            #[cfg(target_os = "windows")]
            fs::remove_file(path).map_err(|_| "画布文件正在使用")?;
        }
        fs::rename(&pending, path).map_err(|_| "画布提交失败")?;
        #[cfg(unix)]
        let _ = File::open(parent).and_then(|f| f.sync_all());
        Ok(Sha256::digest(&bytes).into())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&pending);
        let _ = fs::remove_file(&staged_backup);
    }
    result
}

fn response(token: &str, session: &Session) -> Value {
    json!({"ok":true,"token":token,"revision":session.store.revision,
        "canvas":session.store.canvases[session.store.active_index()],
        "canvases":session.store.canvases.iter().map(|c| json!({"id":c.id,"title":c.title})).collect::<Vec<_>>(),
        "canUndo":!session.undo.is_empty(),"canRedo":!session.redo.is_empty(),"warning":session.warning})
}

#[derive(Serialize)]
struct CanvasSummary<'a> {
    id: &'a str,
    title: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewportReply<'a> {
    id: &'a str,
    center_x: f32,
    center_y: f32,
    zoom: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EncodedReply<'a> {
    ok: bool,
    token: &'a str,
    revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canvas: Option<&'a Canvas>,
    #[serde(skip_serializing_if = "Option::is_none")]
    viewport: Option<ViewportReply<'a>>,
    canvases: Vec<CanvasSummary<'a>>,
    can_undo: bool,
    can_redo: bool,
    warning: &'a str,
}

fn response_json(
    token: &str,
    session: &Session,
    viewport_only: bool,
    base_revision: u64,
) -> Result<String, String> {
    let canvas = &session.store.canvases[session.store.active_index()];
    serde_json::to_string(&EncodedReply {
        ok: true,
        token,
        revision: session.store.revision,
        base_revision: viewport_only.then_some(base_revision),
        canvas: (!viewport_only).then_some(canvas),
        viewport: viewport_only.then_some(ViewportReply {
            id: &canvas.id,
            center_x: canvas.center_x,
            center_y: canvas.center_y,
            zoom: canvas.zoom,
        }),
        canvases: session
            .store
            .canvases
            .iter()
            .map(|c| CanvasSummary {
                id: &c.id,
                title: &c.title,
            })
            .collect(),
        can_undo: !session.undo.is_empty(),
        can_redo: !session.redo.is_empty(),
        warning: &session.warning,
    })
    .map_err(|_| "画布响应编码失败".into())
}

pub fn open(root: &Path, workspace: &str) -> Result<Value, String> {
    open_with(root, workspace, |token, session| {
        Ok(response(token, session))
    })
}

pub fn open_json(root: &Path, workspace: &str) -> Result<String, String> {
    open_with(root, workspace, |token, session| {
        response_json(token, session, false, 0)
    })
}

fn open_with<R>(
    root: &Path,
    workspace: &str,
    encode: impl FnOnce(&str, &Session) -> Result<R, String>,
) -> Result<R, String> {
    let path = scoped_path(root, workspace)?;
    let mut map = sessions().lock().map_err(|_| "画布服务暂不可用")?;
    if map.len() >= 8 {
        return Err("打开的画布过多，请先关闭其他画布".into());
    }
    let mut warning = String::new();
    let (store, fingerprint) = if path.exists() {
        // Unknown schema and corruption are never silently replaced by an old backup.
        read_store_with_digest(&path)?
    } else if path.with_extension("bak").exists() {
        let (recovered, _) = read_store_with_digest(&path.with_extension("bak"))?;
        let fingerprint = persist(&path, &recovered, None)?;
        warning = "已从本机备份恢复画布".into();
        (recovered, fingerprint)
    } else {
        let initial = Store::initial();
        let fingerprint = persist(&path, &initial, None)?;
        (initial, fingerprint)
    };
    let token = identifier();
    let session = Session {
        path,
        store,
        fingerprint,
        undo: VecDeque::new(),
        redo: VecDeque::new(),
        warning,
    };
    let result = encode(&token, &session)?;
    map.insert(token, session);
    Ok(result)
}

pub fn close(token: &str) {
    if let Ok(mut map) = sessions().lock() {
        map.remove(token);
    }
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺少画布参数：{key}"))
}
fn number(request: &Value, key: &str) -> Result<f32, String> {
    request
        .get(key)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("画布数值无效：{key}"))
}

fn edit(canvas: &mut Canvas, request: &Value) -> Result<(), String> {
    match text(request, "op")? {
        "rename" => canvas.title = text(request, "title")?.trim().into(),
        "addNote" | "addPage" => {
            if canvas.nodes.len() >= MAX_NODES {
                return Err("当前画布已达到 1000 张卡片".into());
            }
            let page = text(request, "op")? == "addPage";
            let page_id = if page { text(request, "pageId")? } else { "" };
            if page
                && canvas
                    .nodes
                    .iter()
                    .any(|n| n.kind == "page" && n.page_id == page_id)
            {
                return Err("此页面已在画布中".into());
            }
            canvas.nodes.push(Node {
                id: identifier(),
                kind: if page { "page" } else { "note" }.into(),
                page_id: page_id.into(),
                text: if page {
                    String::new()
                } else {
                    text(request, "text")?.into()
                },
                x: canvas.center_x - 128.0,
                y: canvas.center_y - 84.0,
                width: 256.0,
                height: 168.0,
                color: "blue".into(),
            });
        }
        "move" | "resize" | "text" | "color" => {
            let node = canvas
                .nodes
                .iter_mut()
                .find(|n| Some(n.id.as_str()) == request["id"].as_str())
                .ok_or("卡片不存在")?;
            match text(request, "op")? {
                "move" => {
                    node.x = number(request, "x")?;
                    node.y = number(request, "y")?;
                }
                "resize" => {
                    node.width = number(request, "width")?;
                    node.height = number(request, "height")?;
                }
                "text" if node.kind == "note" => node.text = text(request, "text")?.into(),
                "color" => node.color = text(request, "color")?.into(),
                _ => return Err("页面卡片正文不能在画布中修改".into()),
            }
        }
        "connect" => {
            if canvas.edges.len() >= MAX_EDGES {
                return Err("当前画布已达到 4000 条连线".into());
            }
            let from = text(request, "from")?;
            let to = text(request, "to")?;
            if from == to
                || !canvas.nodes.iter().any(|n| n.id == from)
                || !canvas.nodes.iter().any(|n| n.id == to)
            {
                return Err("请选择两张不同的卡片".into());
            }
            if canvas.edges.iter().any(|e| e.from == from && e.to == to) {
                return Err("这两张卡片已经连接".into());
            }
            canvas.edges.push(Edge {
                id: identifier(),
                from: from.into(),
                to: to.into(),
            });
        }
        "removeNode" => {
            let id = text(request, "id")?;
            if !canvas.nodes.iter().any(|n| n.id == id) {
                return Err("卡片不存在".into());
            }
            canvas.nodes.retain(|n| n.id != id);
            canvas.edges.retain(|e| e.from != id && e.to != id);
        }
        "removeEdge" => {
            let id = text(request, "id")?;
            if !canvas.edges.iter().any(|e| e.id == id) {
                return Err("连线不存在".into());
            }
            canvas.edges.retain(|e| e.id != id);
        }
        "view" => {
            canvas.center_x = number(request, "x")?;
            canvas.center_y = number(request, "y")?;
            canvas.zoom = number(request, "zoom")?;
        }
        "gesture" => {
            canvas.center_x = number(request, "viewX")?;
            canvas.center_y = number(request, "viewY")?;
            canvas.zoom = number(request, "zoom")?;
            if let Some(id) = request["id"].as_str() {
                let node = canvas
                    .nodes
                    .iter_mut()
                    .find(|n| n.id == id)
                    .ok_or("卡片不存在")?;
                node.x = number(request, "x")?;
                node.y = number(request, "y")?;
            }
        }
        "arrange" => {
            let columns = (canvas.nodes.len() as f32).sqrt().ceil().max(1.0) as usize;
            let width = canvas.nodes.iter().map(|n| n.width).fold(256.0, f32::max) + 36.0;
            let height = canvas.nodes.iter().map(|n| n.height).fold(168.0, f32::max) + 36.0;
            for (i, node) in canvas.nodes.iter_mut().enumerate() {
                node.x = (i % columns) as f32 * width;
                node.y = (i / columns) as f32 * height;
            }
        }
        _ => return Err("未知画布操作".into()),
    }
    canvas.validate()
}

fn trim_history(history: &mut VecDeque<Shared<Canvas>>) {
    let mut bytes: usize = history.iter().map(|c| canvas_memory(c)).sum();
    while history.len() > 20 || bytes > HISTORY_BYTES {
        if let Some(old) = history.pop_front() {
            bytes = bytes.saturating_sub(canvas_memory(&old));
        } else {
            break;
        }
    }
}
fn canvas_memory(c: &Canvas) -> usize {
    std::mem::size_of::<Canvas>()
        + c.title.len()
        + c.nodes
            .iter()
            .map(|n| {
                std::mem::size_of::<Node>()
                    + n.text.len()
                    + n.id.len()
                    + n.page_id.len()
                    + n.color.len()
                    + n.kind.len()
            })
            .sum::<usize>()
        + c.edges
            .iter()
            .map(|e| std::mem::size_of::<Edge>() + e.id.len() + e.from.len() + e.to.len())
            .sum::<usize>()
}

pub fn command(token: &str, raw: &str) -> Result<Value, String> {
    command_with(token, raw, |token, session, _, _| {
        Ok(response(token, session))
    })
}

pub fn command_json(token: &str, raw: &str) -> Result<String, String> {
    command_with(token, raw, response_json)
}

fn command_with<R>(
    token: &str,
    raw: &str,
    encode: impl FnOnce(&str, &Session, bool, u64) -> Result<R, String>,
) -> Result<R, String> {
    if raw.len() > 64 * 1024 {
        return Err("画布操作数据过大".into());
    }
    let request: Value = serde_json::from_str(raw).map_err(|_| "画布操作无效")?;
    let mut map = sessions().lock().map_err(|_| "画布服务暂不可用")?;
    let session = map.get_mut(token).ok_or("画布已关闭，请重新打开")?;
    let op = text(&request, "op")?;
    let viewport_only = op == "view" || (op == "gesture" && request["id"].as_str().is_none());
    let base_revision = session.store.revision;
    let index = session.store.active_index();
    let before = session.store.canvases[index].clone();
    let mut next = session.store.clone();
    match op {
        "new" => {
            if next.canvases.len() >= 128 {
                return Err("画布数量已达 128 个".into());
            }
            let canvas = Canvas::new(text(&request, "title")?.trim());
            next.active_canvas_id = canvas.id.clone();
            next.canvases.push(canvas.into());
        }
        "switch" => {
            let id = text(&request, "id")?;
            if !next.canvases.iter().any(|c| c.id == id) {
                return Err("画布不存在".into());
            }
            next.active_canvas_id = id.into();
        }
        "deleteCanvas" => {
            next.canvases.remove(index);
            if next.canvases.is_empty() {
                next.canvases.push(Canvas::new("知识画布").into());
            }
            next.active_canvas_id = next.canvases[0].id.clone();
        }
        "undo" => {
            next.canvases[index] = session.undo.back().ok_or("没有可撤销的操作")?.clone();
        }
        "redo" => {
            next.canvases[index] = session.redo.back().ok_or("没有可重做的操作")?.clone();
        }
        _ => edit(&mut next.canvases[index], &request)?,
    }
    if next.active_canvas_id == session.store.active_canvas_id
        && next.canvases == session.store.canvases
    {
        return encode(token, session, viewport_only, base_revision);
    }
    next.revision = next.revision.checked_add(1).ok_or("画布修订次数已达上限")?;
    let fingerprint = persist(&session.path, &next, Some(session.fingerprint))?;
    session.store = next;
    session.fingerprint = fingerprint;
    session.warning.clear();
    match op {
        "new" | "switch" | "deleteCanvas" => {
            session.undo.clear();
            session.redo.clear();
        }
        "undo" => {
            session.undo.pop_back();
            session.redo.push_back(before);
            trim_history(&mut session.redo);
        }
        "redo" => {
            session.redo.pop_back();
            session.undo.push_back(before);
            trim_history(&mut session.undo);
        }
        "view" => (),
        "gesture" if before.nodes == session.store.canvases[index].nodes => (),
        _ => {
            session.undo.push_back(before);
            trim_history(&mut session.undo);
            session.redo.clear();
        }
    }
    encode(token, session, viewport_only, base_revision)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_ui_KnowledgeCanvasNative_open(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    root: jni::objects::JString,
    workspace: jni::objects::JString,
) -> jni::sys::jstring {
    let result = (|| {
        let root: String = env.get_string(&root).map_err(|_| "画布路径无效")?.into();
        let workspace: String = env
            .get_string(&workspace)
            .map_err(|_| "画布账户无效")?
            .into();
        open_json(Path::new(&root), &workspace)
    })();
    jni_result(&env, result)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_ui_KnowledgeCanvasNative_command(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    token: jni::objects::JString,
    request: jni::objects::JString,
) -> jni::sys::jstring {
    let result = (|| {
        let token: String = env.get_string(&token).map_err(|_| "画布会话无效")?.into();
        let request: String = env.get_string(&request).map_err(|_| "画布操作无效")?.into();
        command_json(&token, &request)
    })();
    jni_result(&env, result)
}

#[no_mangle]
pub extern "system" fn Java_com_ofairyo_gridtimer_ui_KnowledgeCanvasNative_close(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    token: jni::objects::JString,
) {
    if let Ok(token) = env.get_string(&token) {
        close(&String::from(token));
    }
}

fn jni_result(env: &jni::JNIEnv, result: Result<String, String>) -> jni::sys::jstring {
    let value = result.unwrap_or_else(|message| json!({"ok":false,"error":message}).to_string());
    env.new_string(value)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
#[path = "android_canvas_tests.rs"]
mod tests;
