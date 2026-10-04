// v2.22.49.6 - Verify independent boards reopen correctly and failed creation preserves saved state.
// v2.22.49.5 - Verify compact replies preserve durable state and graph edits.
// v2.22.49.4 - Exercise real canvas transactions, recovery and edit barriers.
use super::*;
use std::sync::{Mutex, MutexGuard};

#[test]
fn view_transactions_share_card_text_and_inactive_boards() {
    let mut store = Store::initial();
    let mut canvas = Canvas::new("large");
    for i in 0..1000 {
        canvas.nodes.push(Node {
            id: format!("n{i}"),
            kind: "note".into(),
            page_id: String::new(),
            text: "content".repeat(100),
            x: 0.0,
            y: 0.0,
            width: 256.0,
            height: 168.0,
            color: "blue".into(),
        });
    }
    store.canvases.push(canvas.into());
    let mut next = store.clone();
    edit(
        &mut next.canvases[1],
        &json!({"op":"view","x":30,"y":20,"zoom":0.8}),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&store.canvases[0].0, &next.canvases[0].0));
    assert!(Arc::ptr_eq(
        &store.canvases[1].nodes.0,
        &next.canvases[1].nodes.0
    ));
    assert!(Arc::ptr_eq(
        &store.canvases[1].edges.0,
        &next.canvases[1].edges.0
    ));
    assert_eq!(store.canvases[1].center_x, 0.0);
    assert_eq!(next.canvases[1].center_x, 30.0);
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn compact_view_replies_keep_graph_and_history_and_persist_before_success() {
    let mut f = Fixture::new();
    let opened: Value = serde_json::from_str(&open_json(&f.root, "compact").unwrap()).unwrap();
    f.tokens.push(opened["token"].as_str().unwrap().into());
    let saved = add(&opened, "retained content");
    let token = saved["token"].as_str().unwrap();
    let encoded = command_json(
        token,
        r#"{"op":"gesture","viewX":20,"viewY":30,"zoom":0.8}"#,
    )
    .unwrap();
    let view: Value = serde_json::from_str(&encoded).unwrap();
    assert!(view.get("canvas").is_none());
    assert_eq!(view["baseRevision"], saved["revision"]);
    assert_eq!(view["revision"], saved["revision"].as_u64().unwrap() + 1);
    assert_eq!(view["viewport"]["id"], saved["canvas"]["id"]);
    assert_eq!(view["canUndo"], true);
    let reopened = f.open("compact");
    assert_eq!(reopened["canvas"]["nodes"], saved["canvas"]["nodes"]);
    assert_eq!(reopened["canvas"]["centerX"], 20.0);
    assert_eq!(reopened["revision"], view["revision"]);
    let undo = apply(&saved, json!({"op":"undo"}));
    assert!(undo["canvas"]["nodes"].as_array().unwrap().is_empty());
}

#[test]
fn encoded_graph_edits_remain_full_and_equivalent_to_the_committed_store() {
    let mut f = Fixture::new();
    let saved = add(&f.open("graph"), "card");
    let token = saved["token"].as_str().unwrap();
    let request =
        json!({"op":"gesture","id":id(&saved,0),"x":5,"y":6,"viewX":20,"viewY":30,"zoom":1});
    let reply: Value =
        serde_json::from_str(&command_json(token, &request.to_string()).unwrap()).unwrap();
    assert!(reply.get("viewport").is_none());
    assert_eq!(reply["canvas"]["nodes"][0]["x"], 5.0);
    let reopened = f.open("graph");
    assert_eq!(reply["canvas"], reopened["canvas"]);
    let map = sessions().lock().unwrap();
    assert_eq!(reply, response(token, &map[token]));
}

#[test]
fn compact_replies_cannot_acknowledge_invalid_or_stale_writes() {
    let mut f = Fixture::new();
    let a = f.open("stale-compact");
    let b = f.open("stale-compact");
    let token = a["token"].as_str().unwrap();
    assert!(command_json(token, r#"{"op":"view","x":0,"y":0,"zoom":0}"#).is_err());
    assert_eq!(f.open("stale-compact")["revision"], 0);
    let newer = add(&b, "newer");
    assert!(command_json(token, r#"{"op":"view","x":20,"y":0,"zoom":1}"#).is_err());
    assert_eq!(f.open("stale-compact")["canvas"], newer["canvas"]);
}

struct Fixture {
    root: PathBuf,
    tokens: Vec<String>,
    _lock: MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Self {
            root: std::env::temp_dir().join(format!("tenfold-canvas-{}", identifier())),
            tokens: vec![],
            _lock: lock,
        }
    }
    fn open(&mut self, account: &str) -> Value {
        let result = open(&self.root, account).unwrap();
        self.tokens.push(result["token"].as_str().unwrap().into());
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for token in &self.tokens {
            close(token);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn apply(value: &Value, request: Value) -> Value {
    command(value["token"].as_str().unwrap(), &request.to_string()).unwrap()
}
fn add(value: &Value, text: &str) -> Value {
    apply(value, json!({"op":"addNote","text":text}))
}
fn id(value: &Value, index: usize) -> &str {
    value["canvas"]["nodes"][index]["id"].as_str().unwrap()
}

#[test]
fn account_scope_survives_reopen_and_cannot_traverse_directories() {
    let mut f = Fixture::new();
    let a = f.open("../account-a");
    let a = add(&a, "private-a");
    let b = f.open("account-b");
    assert_eq!(b["canvas"]["nodes"].as_array().unwrap().len(), 0);
    close(a["token"].as_str().unwrap());
    let reopened = f.open("../account-a");
    assert_eq!(reopened["canvas"]["nodes"][0]["text"], "private-a");
    assert!(scoped_path(&f.root, "../account-a")
        .unwrap()
        .starts_with(f.root.join("knowledge_canvases")));
    assert!(open(&f.root, "").is_err());
}
#[test]
fn delete_removes_incident_edges_and_undo_restores_the_complete_graph() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = add(&a, "first");
    let a = add(&a, "second");
    let first = id(&a, 0).to_string();
    let second = id(&a, 1).to_string();
    let a = apply(&a, json!({"op":"connect","from":first,"to":second}));
    let a = apply(&a, json!({"op":"removeNode","id":first}));
    assert_eq!(a["canvas"]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(a["canvas"]["edges"], json!([]));
    let a = apply(&a, json!({"op":"undo"}));
    assert_eq!(a["canvas"]["edges"].as_array().unwrap().len(), 1);
    let a = apply(&a, json!({"op":"redo"}));
    assert_eq!(a["canvas"]["edges"], json!([]));
    let disk = read_store(&scoped_path(&f.root, "a").unwrap()).unwrap();
    assert_eq!(disk.canvases[0].nodes.len(), 1);
}
#[test]
fn invalid_commands_never_change_the_durable_revision_or_valid_state() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = add(&a, "keep");
    let path = scoped_path(&f.root, "a").unwrap();
    let bytes = fs::read(&path).unwrap();
    for request in [
        json!({"op":"move","id":id(&a,0),"x":1e100,"y":0}),
        json!({"op":"connect","from":id(&a,0),"to":id(&a,0)}),
        json!({"op":"connect","from":id(&a,0),"to":"missing"}),
        json!({"op":"rename","title":" "}),
        json!({"op":"resize","id":id(&a,0),"width":0,"height":120}),
        json!({"op":"switch","id":"missing"}),
        json!({"op":"wrong"}),
    ] {
        assert!(command(a["token"].as_str().unwrap(), &request.to_string()).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}
#[test]
fn page_cards_hold_only_references_and_duplicate_pages_are_rejected() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = apply(
        &a,
        json!({"op":"addPage","pageId":"encrypted-page","text":"secret"}),
    );
    assert_eq!(a["canvas"]["nodes"][0]["text"], "");
    for request in [
        json!({"op":"addPage","pageId":"encrypted-page"}),
        json!({"op":"text","id":id(&a,0),"text":"secret"}),
    ] {
        assert!(command(a["token"].as_str().unwrap(), &request.to_string()).is_err());
    }
    assert!(!fs::read_to_string(scoped_path(&f.root, "a").unwrap())
        .unwrap()
        .contains("secret"));
}
#[test]
fn duplicate_edges_are_rejected_and_removing_an_edge_is_undoable() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = add(&a, "one");
    let a = add(&a, "two");
    let request = json!({"op":"connect","from":id(&a,0),"to":id(&a,1)});
    let a = apply(&a, request.clone());
    assert!(command(a["token"].as_str().unwrap(), &request.to_string()).is_err());
    let a = apply(
        &a,
        json!({"op":"removeEdge","id":a["canvas"]["edges"][0]["id"]}),
    );
    assert_eq!(a["canvas"]["edges"], json!([]));
    let a = apply(&a, json!({"op":"undo"}));
    assert_eq!(a["canvas"]["edges"].as_array().unwrap().len(), 1);
}
#[test]
fn another_editor_cannot_overwrite_a_newer_saved_revision() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let stale = f.open("a");
    let a = add(&a, "newer");
    assert!(command(
        stale["token"].as_str().unwrap(),
        r#"{"op":"addNote","text":"stale"}"#
    )
    .is_err());
    let disk = read_store(&scoped_path(&f.root, "a").unwrap()).unwrap();
    assert_eq!(disk.revision, a["revision"].as_u64().unwrap());
    assert_eq!(disk.canvases[0].nodes[0].text, "newer");
}
#[test]
fn corrupt_and_future_files_are_not_reset_or_replaced() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let _a = add(&a, "backup");
    let path = scoped_path(&f.root, "a").unwrap();
    fs::write(&path, b"broken").unwrap();
    assert!(open(&f.root, "a").is_err());
    assert_eq!(fs::read(&path).unwrap(), b"broken");
    let mut future = Store::initial();
    future.version = 2;
    let bytes = serde_json::to_vec(&future).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(open(&f.root, "a").is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}
#[test]
fn missing_primary_recovers_a_valid_backup_without_changing_the_account() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = add(&a, "retained");
    let _a = add(&a, "last");
    let path = scoped_path(&f.root, "a").unwrap();
    fs::remove_file(&path).unwrap();
    let recovered = f.open("a");
    assert_eq!(recovered["canvas"]["nodes"][0]["text"], "retained");
    assert!(!recovered["warning"].as_str().unwrap().is_empty());
}
#[test]
fn switching_boards_clears_history_and_last_board_deletion_keeps_a_valid_workspace() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = add(&a, "first");
    let original = a["canvas"]["id"].clone();
    let a = apply(&a, json!({"op":"new","title":"second"}));
    assert_eq!(a["canUndo"], false);
    let a = apply(&a, json!({"op":"switch","id":original}));
    assert_eq!(a["canvas"]["nodes"][0]["text"], "first");
    let a = apply(&a, json!({"op":"deleteCanvas"}));
    let a = apply(&a, json!({"op":"deleteCanvas"}));
    assert_eq!(a["canvases"].as_array().unwrap().len(), 1);
    assert_eq!(a["canvas"]["nodes"], json!([]));
}
#[test]
fn multiple_boards_keep_independent_graphs_views_and_the_last_selection_after_reopen() {
    let mut f = Fixture::new();
    let first = add(&f.open("multi-board"), "first card");
    let first = add(&first, "second card");
    let first = apply(
        &first,
        json!({"op":"connect","from":id(&first,0),"to":id(&first,1)}),
    );
    let first = apply(&first, json!({"op":"view","x":20,"y":30,"zoom":0.5}));
    let first_board = first["canvas"].clone();
    let token = first["token"].as_str().unwrap();
    let second: Value = serde_json::from_str(
        &command_json(token, r#"{"op":"new","title":"Second board"}"#).unwrap(),
    )
    .unwrap();
    assert_ne!(second["canvas"]["id"], first_board["id"]);
    assert_eq!(second["canvases"].as_array().unwrap().len(), 2);
    assert_eq!(second["canvas"]["nodes"], json!([]));
    assert_eq!(second["canvas"]["edges"], json!([]));
    assert_eq!(second["canUndo"], false);
    let second = add(&second, "second board only");
    let second = apply(&second, json!({"op":"view","x":-40,"y":-50,"zoom":2}));
    let second_board = second["canvas"].clone();

    close(token);
    let reopened = f.open("multi-board");
    assert_eq!(reopened["canvas"], second_board);
    assert_eq!(reopened["canvases"], second["canvases"]);
    let switched: Value = serde_json::from_str(
        &command_json(
            reopened["token"].as_str().unwrap(),
            &json!({"op":"switch","id":first_board["id"]}).to_string(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(switched["canvas"], first_board);
    close(reopened["token"].as_str().unwrap());
    let reopened = f.open("multi-board");
    assert_eq!(reopened["canvas"], first_board);
    let switched = apply(&reopened, json!({"op":"switch","id":second_board["id"]}));
    assert_eq!(switched["canvas"], second_board);
}
#[test]
fn failed_board_creation_keeps_current_board_history_and_disk_then_allows_retry() {
    let mut f = Fixture::new();
    let saved = add(&f.open("create-failure"), "keep");
    let saved = add(&saved, "redo target");
    let saved = apply(&saved, json!({"op":"undo"}));
    assert_eq!(saved["canUndo"], true);
    assert_eq!(saved["canRedo"], true);
    let token = saved["token"].as_str().unwrap();
    let path = scoped_path(&f.root, "create-failure").unwrap();
    let before = fs::read(&path).unwrap();
    for title in [" ".to_string(), "x".repeat(81)] {
        assert!(command_json(token, &json!({"op":"new","title":title}).to_string()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(response(token, &sessions().lock().unwrap()[token]), saved);
    }

    // A directory blocks backup replacement while leaving the primary store readable.
    let backup = path.with_extension("bak");
    fs::remove_file(&backup).unwrap();
    fs::create_dir(&backup).unwrap();
    let request = r#"{"op":"new","title":"Retry board"}"#;
    assert!(command_json(token, request).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(response(token, &sessions().lock().unwrap()[token]), saved);

    fs::remove_dir(&backup).unwrap();
    let created: Value = serde_json::from_str(&command_json(token, request).unwrap()).unwrap();
    assert_eq!(created["canvases"].as_array().unwrap().len(), 2);
    assert_eq!(created["revision"], saved["revision"].as_u64().unwrap() + 1);
    assert_eq!(created["canUndo"], false);
    assert_eq!(created["canRedo"], false);
    assert_eq!(f.open("create-failure")["canvas"], created["canvas"]);
    let original = apply(&created, json!({"op":"switch","id":saved["canvas"]["id"]}));
    assert_eq!(original["canvas"], saved["canvas"]);
}
#[test]
fn view_changes_do_not_consume_undo_and_drag_commits_one_atomic_gesture() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let a = add(&a, "card");
    let a = apply(
        &a,
        json!({"op":"gesture","viewX":100,"viewY":30,"zoom":0.5,"id":id(&a,0),"x":60,"y":20}),
    );
    assert_eq!(a["canvas"]["nodes"][0]["x"], 60.0);
    let a = apply(
        &a,
        json!({"op":"gesture","viewX":200,"viewY":60,"zoom":0.8}),
    );
    let a = apply(&a, json!({"op":"undo"}));
    assert_eq!(a["canvas"]["nodes"][0]["x"], -128.0);
}
#[test]
fn history_is_bounded_and_new_edits_clear_redo() {
    let mut f = Fixture::new();
    let mut a = f.open("a");
    for i in 0..24 {
        a = add(&a, &format!("{i}"));
    }
    for _ in 0..20 {
        a = apply(&a, json!({"op":"undo"}));
    }
    assert_eq!(a["canUndo"], false);
    assert_eq!(a["canRedo"], true);
    let a = add(&a, "new branch");
    assert_eq!(a["canRedo"], false);
}
#[test]
fn save_failure_does_not_publish_an_uncommitted_canvas_or_history() {
    let mut f = Fixture::new();
    let a = f.open("a");
    let path = scoped_path(&f.root, "a").unwrap();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(command(
        a["token"].as_str().unwrap(),
        r#"{"op":"addNote","text":"unsaved"}"#
    )
    .is_err());
    let map = sessions().lock().unwrap();
    let session = &map[a["token"].as_str().unwrap()];
    assert!(session.store.canvases[0].nodes.is_empty());
    assert!(session.undo.is_empty());
}
