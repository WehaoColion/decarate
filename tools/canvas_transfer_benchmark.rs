// v0.0.1 - Measure real canvas command encoding, allocation traffic and persistence.
mod android_canvas;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

struct MeasuredAllocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Relaxed) { CALLS.fetch_add(1, Relaxed); BYTES.fetch_add(layout.size() as u64, Relaxed); }
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Relaxed) { CALLS.fetch_add(1, Relaxed); BYTES.fetch_add(layout.size() as u64, Relaxed); }
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if TRACK.load(Relaxed) { CALLS.fetch_add(1, Relaxed); BYTES.fetch_add(size as u64, Relaxed); }
        System.realloc(ptr,layout,size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { System.dealloc(ptr,layout) }
}
#[global_allocator] static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

fn open(root: &Path) -> String {
    #[cfg(feature="optimized")] { android_canvas::open_json(root,"benchmark").unwrap() }
    #[cfg(not(feature="optimized"))] { android_canvas::open(root,"benchmark").unwrap().to_string() }
}
fn command(token: &str, request: &str) -> String {
    #[cfg(feature="optimized")] { android_canvas::command_json(token,request).unwrap() }
    #[cfg(not(feature="optimized"))] { android_canvas::command(token,request).unwrap().to_string() }
}
fn measured(f: impl FnOnce() -> String) -> (String, Value) {
    CALLS.store(0, Relaxed); BYTES.store(0, Relaxed);
    let started=Instant::now(); TRACK.store(true,Relaxed);
    let reply=f(); TRACK.store(false,Relaxed);
    let elapsed=started.elapsed().as_micros();
    let calls=CALLS.load(Relaxed); let bytes=BYTES.load(Relaxed);
    let result=json!({"responseBytes":reply.len(),"allocationCalls":calls,"allocatedBytes":bytes,"elapsedMicros":elapsed});
    (reply,result)
}
fn main() {
    let root=std::path::PathBuf::from(std::env::args_os().nth(1).expect("fixture root"));
    assert!(root.is_absolute());
    let name: String=Sha256::digest(b"benchmark").iter().map(|b|format!("{b:02x}")).collect();
    let path=root.join("knowledge_canvases").join(name).join("canvases.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let nodes: Vec<Value>=(0..1000).map(|i|json!({"id":format!("n{i}"),"kind":"note","pageId":"","text":"content".repeat(100),"x":0.0,"y":0.0,"width":256.0,"height":168.0,"color":"blue"})).collect();
    let edges: Vec<Value>=(0..1000).flat_map(|i|(1..=4).map(move |j|json!({"id":format!("e{i}_{j}"),"from":format!("n{i}"),"to":format!("n{}",(i+j)%1000)}))).collect();
    let store=json!({"version":1,"revision":0,"activeCanvasId":"board","canvases":[{"id":"board","title":"Benchmark","centerX":0.0,"centerY":0.0,"zoom":1.0,"nodes":nodes,"edges":edges}]});
    std::fs::write(&path,serde_json::to_vec(&store).unwrap()).unwrap();
    let (initial, open_measurement)=measured(||open(&root));
    let initial: Value=serde_json::from_str(&initial).unwrap();
    let token=initial["token"].as_str().unwrap();
    let mut samples=Vec::new();
    for i in 1..=30 {
        let request=format!(r#"{{"op":"view","x":{i},"y":20,"zoom":0.8}}"#);
        let (reply, measurement)=measured(||command(token,&request));
        let value: Value=serde_json::from_str(&reply).unwrap();
        assert_eq!(value["revision"],i);
        samples.push(measurement);
    }
    android_canvas::close(token);
    let reopened: Value=serde_json::from_str(&open(&root)).unwrap();
    assert_eq!(reopened["canvas"]["nodes"],store["canvases"][0]["nodes"]);
    assert_eq!(reopened["canvas"]["edges"],store["canvases"][0]["edges"]);
    assert_eq!(reopened["canvas"]["centerX"],30.0);
    android_canvas::close(reopened["token"].as_str().unwrap());
    println!("{}",json!({"passed":true,"optimized":cfg!(feature="optimized"),"fixture":{"nodes":1000,"edges":4000,"cardTextBytes":700,"iterations":30},"metricNote":"Host release build; cumulative allocator bytes, not peak RSS; command includes durable save; excludes Java decode","open":open_measurement,"panSamples":samples}));
}
