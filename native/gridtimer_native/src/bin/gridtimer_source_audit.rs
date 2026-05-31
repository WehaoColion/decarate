use gridtimer_native::tooling::project_root;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("source audit failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let root = project_root();
    let source_roots = [
        root.join("app/src"),
        root.join("native/gridtimer_native/src"),
    ];
    let mut violations = Vec::<PathBuf>::new();

    for source_root in source_roots {
        if source_root.exists() {
            scan_for_non_rust_sources(&source_root, &mut violations)?;
        }
    }

    if violations.is_empty() {
        println!("source audit ok");
        return Ok(());
    }

    eprintln!("found non-Rust source files:");
    for path in &violations {
        eprintln!("{}", path.display());
    }
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!("{} non-Rust source file(s) found", violations.len()),
    ))
}

fn scan_for_non_rust_sources(root: &Path, violations: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if is_non_rust_source(&path) {
                violations.push(path);
            }
        }
    }
    Ok(())
}

fn is_non_rust_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("kt" | "kts" | "java" | "groovy" | "scala")
    )
}
