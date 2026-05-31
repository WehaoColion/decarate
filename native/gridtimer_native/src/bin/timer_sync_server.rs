use gridtimer_native::sync_core::{default_server_store_path, run_sync_server};
use std::env;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("sync server failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> std::io::Result<()> {
    let mut args = env::args().skip(1);
    let bind_addr = args.next().unwrap_or_else(|| "0.0.0.0:8917".to_string());
    let store_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_server_store_path);
    run_sync_server(&bind_addr, &store_path)
}
