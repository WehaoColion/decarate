#![cfg_attr(windows, windows_subsystem = "windows")]

use std::cmp::Ordering;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SYNC_BIND_ADDR: &str = "0.0.0.0:8917";
const LOCAL_HEALTH_ADDR: &str = "127.0.0.1:8917";

fn main() {
    if TcpStream::connect(LOCAL_HEALTH_ADDR).is_ok() {
        return;
    }

    match launch_sync_server() {
        Ok(()) => {}
        Err(error) => write_log(&format!("launcher failed: {error}")),
    }
}

fn launch_sync_server() -> Result<(), String> {
    let current_exe = env::current_exe().map_err(|error| error.to_string())?;
    let Some(app_dir) = current_exe.parent() else {
        return Err("could not resolve launcher directory".to_string());
    };
    let server_path = find_sync_server(app_dir)
        .ok_or_else(|| "could not find sync server executable".to_string())?;

    Command::new(&server_path)
        .arg(SYNC_BIND_ADDR)
        .current_dir(app_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start {}: {error}", server_path.display()))
}

fn find_sync_server(app_dir: &Path) -> Option<PathBuf> {
    fs::read_dir(app_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_sync_server_executable(path))
        .max_by(|left, right| compare_sync_server_candidates(left, right))
}

fn is_sync_server_executable(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("grid_timer_sync_server_v") && name.ends_with(".exe"))
        .unwrap_or(false)
}

fn compare_sync_server_candidates(left: &Path, right: &Path) -> Ordering {
    match (
        sync_server_version_parts(left),
        sync_server_version_parts(right),
    ) {
        (Some(left_parts), Some(right_parts)) => compare_version_parts(&left_parts, &right_parts)
            .then_with(|| compare_modified_time(left, right)),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => compare_modified_time(left, right),
    }
}

fn sync_server_version_parts(path: &Path) -> Option<Vec<u32>> {
    let name = path.file_name()?.to_str()?;
    let version = name
        .strip_prefix("grid_timer_sync_server_v")?
        .strip_suffix(".exe")?;
    let mut parts = Vec::new();
    for part in version.split('.') {
        parts.push(part.parse::<u32>().ok()?);
    }
    Some(parts)
}

fn compare_version_parts(left: &[u32], right: &[u32]) -> Ordering {
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_value = *left.get(index).unwrap_or(&0);
        let right_value = *right.get(index).unwrap_or(&0);
        match left_value.cmp(&right_value) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn compare_modified_time(left: &Path, right: &Path) -> Ordering {
    left.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .cmp(
            &right
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok(),
        )
}

fn write_log(message: &str) {
    let base_dir = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base_dir.join("GridTimerSync");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("sync_launcher.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}
