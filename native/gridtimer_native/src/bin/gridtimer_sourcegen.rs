use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[path = "../sourcegen/android_sources.rs"]
mod android_sources;
#[path = "../sourcegen/kotlin_sources.rs"]
mod kotlin_sources;

fn main() {
    if let Err(error) = run() {
        eprintln!("source generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let output_root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output directory"))?;

    if output_root.exists() {
        fs::remove_dir_all(&output_root)?;
    }
    fs::create_dir_all(&output_root)?;

    for source in kotlin_sources::SOURCES {
        write_source(&output_root, source.path, source.contents)?;
    }

    for source in android_sources::SOURCES {
        write_source(&output_root, source.path, source.contents)?;
    }

    Ok(())
}

fn write_source(output_root: &Path, relative_path: &str, contents: &str) -> io::Result<()> {
    let destination = relative_path
        .split('/')
        .fold(output_root.to_path_buf(), |path, segment| {
            path.join(segment)
        });

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(destination, contents.trim_start())
}
