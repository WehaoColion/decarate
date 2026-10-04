// v0.0.1 - Verify persisted canvas operations and capacity barriers with isolated data.
mod android_canvas;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

struct Workspace {
    root: PathBuf,
    token: String,
    path: PathBuf,
}
impl Workspace {
    fn new() -> (Self, Value) {
        let root = std::env::temp_dir().join(format!("tenfold-canvas-scenarios-{:032x}", rand::random::<u128>()));
        let opened = android_canvas::open(&root, "scenario-account").unwrap();
        let digest: String = Sha256::digest(b"scenario-account").iter().map(|b| format!("{b:02x}")).collect();
        let path = root.join("knowledge_canvases").join(digest).join("canvases.json");
        (Self { root, path, token: opened["token"].as_str().unwrap().into() }, opened)
    }
    fn apply(&self, request: Value) -> Value {
        android_canvas::command(&self.token, &request.to_string()).unwrap()
    }
    fn reopen(&mut self) -> Value {
        android_canvas::close(&self.token);
        let result = android_canvas::open(&self.root, "scenario-account").unwrap();
        self.token = result["token"].as_str().unwrap().into();
        result
    }
    fn load_fixture(&mut self, store: &Value) -> Value {
        android_canvas::close(&self.token);
        fs::write(&self.path, serde_json::to_vec(store).unwrap()).unwrap();
        self.reopen()
    }
    fn reject(&self, request: Value) {
        let before = fs::read(&self.path).unwrap();
        assert!(android_canvas::command(&self.token, &request.to_string()).is_err());
        assert_eq!(before, fs::read(&self.path).unwrap(), "rejected operation changed persisted data");
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        android_canvas::close(&self.token);
        let temp = std::env::temp_dir();
        assert!(self.root.starts_with(&temp));
        assert!(self.root.file_name().unwrap().to_string_lossy().starts_with("tenfold-canvas-scenarios-"));
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn node_id(state: &Value, i: usize) -> String { state["canvas"]["nodes"][i]["id"].as_str().unwrap().into() }
fn main() {
    let (mut workspace, initial) = Workspace::new();
    let mut scenarios = vec![];
    let original = initial["canvas"]["id"].clone();
    workspace.apply(json!({"op":"rename","title":"Project map"}));
    let state = workspace.apply(json!({"op":"addNote","text":"Draft"}));
    let note = node_id(&state, 0);
    workspace.apply(json!({"op":"text","id":note,"text":"Final decision"}));
    workspace.apply(json!({"op":"color","id":note,"color":"purple"}));
    workspace.apply(json!({"op":"resize","id":note,"width":384,"height":252}));
    workspace.apply(json!({"op":"move","id":note,"x":123,"y":-456}));
    let state = workspace.reopen();
    assert_eq!(state["canvas"]["title"], "Project map");
    let card = &state["canvas"]["nodes"][0];
    assert_eq!(card["text"], "Final decision");
    assert_eq!(card["color"], "purple");
    assert_eq!(card["width"], 384.0); assert_eq!(card["height"], 252.0);
    assert_eq!(card["x"], 123.0); assert_eq!(card["y"], -456.0);
    scenarios.push("note_edit_color_resize_move_and_title_survive_reopen");

    let state = workspace.apply(json!({"op":"addPage","pageId":"source-page"}));
    let page = node_id(&state, 1);
    workspace.apply(json!({"op":"addNote","text":"Third"}));
    workspace.apply(json!({"op":"connect","from":note,"to":page}));
    let arranged = workspace.apply(json!({"op":"arrange"}));
    let cards = arranged["canvas"]["nodes"].as_array().unwrap();
    for (i, a) in cards.iter().enumerate() {
        for b in &cards[i+1..] {
            let f = |n: &Value, k: &str| n[k].as_f64().unwrap();
            assert!(f(a,"x")+f(a,"width") <= f(b,"x") || f(b,"x")+f(b,"width") <= f(a,"x") || f(a,"y")+f(a,"height") <= f(b,"y") || f(b,"y")+f(b,"height") <= f(a,"y"));
        }
    }
    let restored = workspace.apply(json!({"op":"undo"}));
    assert_eq!(restored["canvas"]["nodes"][0]["x"], 123.0);
    let redone = workspace.apply(json!({"op":"redo"}));
    assert_eq!(redone["canvas"], arranged["canvas"]);
    assert_eq!(workspace.reopen()["canvas"], arranged["canvas"]);
    scenarios.push("mixed_size_auto_arrangement_undo_redo_and_reopen");

    let edge = arranged["canvas"]["edges"][0]["id"].clone();
    assert_eq!(workspace.apply(json!({"op":"removeEdge","id":edge}))["canvas"]["edges"],json!([]));
    workspace.apply(json!({"op":"undo"}));
    workspace.apply(json!({"op":"removeNode","id":page}));
    let state = workspace.reopen();
    assert_eq!(state["canvas"]["edges"],json!([]));
    assert_eq!(state["canvas"]["nodes"].as_array().unwrap().len(),2);
    scenarios.push("page_removal_and_incident_edge_cleanup_survive_reopen");

    workspace.apply(json!({"op":"new","title":"Second board"}));
    let second = workspace.apply(json!({"op":"addNote","text":"Other board"}));
    workspace.apply(json!({"op":"switch","id":original}));
    let original_state = workspace.reopen();
    assert_eq!(original_state["canvas"]["title"],"Project map");
    let second_state = workspace.apply(json!({"op":"deleteCanvas"}));
    assert_eq!(second_state["canvas"],second["canvas"]);
    assert_eq!(workspace.reopen()["canvases"].as_array().unwrap().len(),1);
    scenarios.push("board_switch_delete_and_other_board_preservation");

    workspace.apply(json!({"op":"view","x":10,"y":20,"zoom":0.5}));
    let state = workspace.apply(json!({"op":"gesture","viewX":40,"viewY":50,"zoom":1.5,"id":node_id(&second,0),"x":60,"y":70}));
    assert_eq!(workspace.reopen()["canvas"],state["canvas"]);
    scenarios.push("combined_gesture_commits_view_and_card_together");

    let template: Value = serde_json::from_slice(&fs::read(&workspace.path).unwrap()).unwrap();
    let mut node_limit = template.clone();
    let mut nodes = vec![];
    for i in 0..1000 {
        let mut node = template["canvases"][0]["nodes"][0].clone();
        node["id"]=json!(format!("node-{i}")); nodes.push(node);
    }
    node_limit["canvases"][0]["nodes"]=json!(nodes);
    workspace.load_fixture(&node_limit);
    workspace.reject(json!({"op":"addNote","text":"overflow"}));
    workspace.reject(json!({"op":"addPage","pageId":"overflow-page"}));
    scenarios.push("one_thousand_card_limit_rejects_new_cards_without_writing");

    let mut edge_limit=node_limit.clone();
    let mut edges=vec![];
    for from in 0..100 {
        for to in 0..100 {
            if from!=to && edges.len()<4000 { edges.push(json!({"id":format!("edge-{from}-{to}"),"from":format!("node-{from}"),"to":format!("node-{to}")})); }
        }
    }
    edge_limit["canvases"][0]["edges"]=json!(edges);
    workspace.load_fixture(&edge_limit);
    workspace.reject(json!({"op":"connect","from":"node-99","to":"node-98"}));
    scenarios.push("four_thousand_edge_limit_preserves_existing_graph");

    let mut board_limit=template.clone();
    let boards: Vec<_>=(0..128).map(|i| { let mut board=template["canvases"][0].clone(); board["id"]=json!(format!("board-{i}")); board }).collect();
    board_limit["canvases"]=json!(boards); board_limit["activeCanvasId"]=json!("board-0");
    workspace.load_fixture(&board_limit);
    workspace.reject(json!({"op":"new","title":"overflow"}));
    scenarios.push("one_hundred_twenty_eight_board_limit_preserves_workspace");

    workspace.load_fixture(&template);
    workspace.reject(json!({"op":"text","id":node_id(&state,0),"text":"x".repeat(8001)}));
    workspace.reject(json!({"op":"rename","title":"x".repeat(81)}));
    workspace.reject(json!({"op":"color","id":node_id(&state,0),"color":"unknown"}));
    workspace.reject(json!({"op":"view","x":0,"y":0,"zoom":2.6}));
    workspace.reject(json!({"op":"gesture","viewX":0,"viewY":0,"zoom":1,"id":node_id(&state,0),"x":250001,"y":0}));
    scenarios.push("text_title_color_zoom_and_coordinate_barriers_preserve_disk");

    println!("{}",json!({"passed":true,"noDeviceOperations":true,"scenarios":scenarios}));
}
