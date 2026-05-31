use gridtimer_native::tooling::{
    apksigner_path, archive_directory, archive_existing_root_packages, cargo_toolchain_exe,
    ensure_directory, gradle_home, move_file, project_root, read_project_version_info,
    replace_file_with_copy, resolve_android_sdk_dir, resolve_latest_ndk_directory, spawn_and_check,
    versioned_artifact_name,
};
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if let Err(error) = run() {
        eprintln!("packager failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let command = if args.is_empty() {
        "build".to_string()
    } else {
        args.remove(0)
    };

    match command.as_str() {
        "build" => run_build(&args),
        "finish" => run_finish(&args),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown command: {other}"),
        )),
    }
}

fn run_build(args: &[String]) -> io::Result<()> {
    let configuration =
        flag_value(args, "--configuration").unwrap_or_else(|| "Release".to_string());
    let keep_previous_root_packages = flag_present(args, "--keep-previous-root-packages");
    let project_root = project_root();
    let version = read_project_version_info(&project_root)?;
    let gradle_user_home = flag_value(args, "--gradle-user-home")
        .or_else(|| env::var("GRIDTIMER_GRADLE_USER_HOME").ok())
        .unwrap_or_else(|| r"C:\tools\gradle-home".to_string());
    let cargo_home = env::var("CARGO_HOME").unwrap_or_else(|_| r"C:\tools\cargo".to_string());
    let rustup_home = env::var("RUSTUP_HOME").unwrap_or_else(|_| r"C:\tools\rustup".to_string());
    let android_sdk = resolve_android_sdk_dir(&project_root)
        .unwrap_or_else(|| PathBuf::from(r"C:\tools\android-sdk"));
    let ndk_dir = resolve_latest_ndk_directory(&android_sdk)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Android NDK not found"))?;

    ensure_directory(Path::new(&gradle_user_home))?;
    ensure_directory(&archive_directory(&project_root))?;
    if let Some(signing) = gridtimer_native::tooling::load_signing_properties(&project_root)? {
        let store_file = resolve_project_path(&project_root, &signing.store_file);
        if !store_file.exists() && configuration.eq_ignore_ascii_case("release") {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("release keystore not found at {}", store_file.display()),
            ));
        }
    }

    let tasks = match configuration.as_str() {
        "Release" => vec![
            "clean",
            "testDebugUnitTest",
            "lintDebug",
            "assembleRelease",
        ],
        "XiaomiDebug" => vec![
            "clean",
            "testDebugUnitTest",
            "lintDebug",
            "assembleXiaomiDebug",
        ],
        _ => vec!["clean", "testDebugUnitTest", "lintDebug", "assembleDebug"],
    };
    invoke_gradle(
        &project_root,
        &tasks,
        &gradle_user_home,
        &cargo_home,
        &rustup_home,
        &android_sdk,
        &ndk_dir,
    )?;

    match configuration.as_str() {
        "Release" => publish_release_artifacts(
            &project_root,
            &version,
            &android_sdk,
            keep_previous_root_packages,
        )?,
        "XiaomiDebug" => publish_debug_artifact(
            &project_root,
            &version,
            "grid_timer_app_xiaomi_debug",
            "app/build/outputs/apk/xiaomiDebug/app-xiaomiDebug.apk",
            keep_previous_root_packages,
        )?,
        _ => publish_debug_artifact(
            &project_root,
            &version,
            "grid_timer_app_debug",
            "app/build/outputs/apk/debug/app-debug.apk",
            keep_previous_root_packages,
        )?,
    }

    run_status_generator(&project_root)?;
    Ok(())
}

fn run_finish(args: &[String]) -> io::Result<()> {
    let project_root = project_root();
    let expected_version_name = flag_value(args, "--expected-version-name").unwrap_or_else(|| {
        read_project_version_info(&project_root)
            .map(|v| v.version_name)
            .unwrap_or_else(|_| "2.16.9".to_string())
    });
    let expected_version_code = flag_value(args, "--expected-version-code")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or_else(|| {
            read_project_version_info(&project_root)
                .map(|v| v.version_code)
                .unwrap_or(2169)
        });
    let validate_only = flag_present(args, "--validate-only");
    let archive_old_packages = flag_present(args, "--archive-old-packages");

    let version = read_project_version_info(&project_root)?;
    if version.version_name != expected_version_name
        || version.version_code != expected_version_code
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "version mismatch: found {} ({}) but expected {} ({})",
                version.version_name,
                version.version_code,
                expected_version_name,
                expected_version_code
            ),
        ));
    }

    if validate_only {
        validate_expected_packages(&project_root, &version)?;
    }

    if archive_old_packages {
        archive_existing_root_packages(&project_root, "grid_timer_app")?;
        archive_existing_root_packages(&project_root, "grid_timer_app_debug")?;
        archive_existing_root_packages(&project_root, "grid_timer_app_xiaomi_debug")?;
    }

    Ok(())
}

fn publish_release_artifacts(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    android_sdk: &Path,
    keep_previous_root_packages: bool,
) -> io::Result<()> {
    let archive_dir = archive_directory(project_root);
    let release_apk = first_existing_path(&[
        project_root.join("app/build/outputs/apk/release/app-release.apk"),
        project_root
            .join("app/build/outputs/apk/release")
            .join(versioned_artifact_name(
                "grid_timer_app",
                &version.version_name,
                "apk",
            )),
    ])?;
    let root_apk = project_root.join(versioned_artifact_name(
        "grid_timer_app",
        &version.version_name,
        "apk",
    ));

    if !keep_previous_root_packages {
        archive_existing_root_packages(project_root, "grid_timer_app")?;
        remove_existing_root_packages(project_root, "app-release")?;
        remove_existing_root_packages(project_root, "grid_timer_app_debug")?;
        remove_existing_root_packages(project_root, "grid_timer_app_xiaomi_debug")?;
    }
    replace_file_with_copy(&release_apk, &root_apk, &archive_dir)?;
    verify_release_apk(&root_apk, android_sdk)?;
    Ok(())
}

fn verify_release_apk(apk_path: &Path, android_sdk: &Path) -> io::Result<()> {
    let apksigner = apksigner_path(android_sdk);
    let mut command = Command::new(apksigner);
    command.arg("verify");
    command.arg("--verbose");
    command.arg("--print-certs");
    command.arg(apk_path);
    spawn_and_check(command, "apksigner verify")
}

fn publish_debug_artifact(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    base_name: &str,
    source_relative: &str,
    keep_previous_root_packages: bool,
) -> io::Result<()> {
    let archive_dir = archive_directory(project_root);
    let default_source = project_root.join(source_relative);
    let source = first_existing_path(&[
        default_source.clone(),
        default_source
            .parent()
            .unwrap_or(project_root)
            .join(versioned_artifact_name(
                base_name,
                &version.version_name,
                "apk",
            )),
    ])?;
    let destination = project_root.join(versioned_artifact_name(
        base_name,
        &version.version_name,
        "apk",
    ));

    if !keep_previous_root_packages {
        archive_existing_root_packages(project_root, base_name)?;
    }
    replace_file_with_copy(&source, &destination, &archive_dir)?;
    if source.exists() {
        let source_stem = source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("app");
        if !source_stem.ends_with(&format!("_v{}", version.version_name)) {
            let versioned_output =
                source
                    .parent()
                    .unwrap_or(project_root)
                    .join(versioned_artifact_name(
                        source_stem,
                        &version.version_name,
                        "apk",
                    ));
            if versioned_output != source {
                move_file(&source, &versioned_output)?;
            }
        }
    }
    Ok(())
}

fn first_existing_path(candidates: &[PathBuf]) -> io::Result<PathBuf> {
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .ok_or_else(|| {
            let joined = candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("expected build artifact missing; checked {joined}"),
            )
        })
}

fn validate_expected_packages(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<()> {
    let expected = project_root.join(versioned_artifact_name(
        "grid_timer_app",
        &version.version_name,
        "apk",
    ));
    if !expected.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("expected package missing: {}", expected.display()),
        ));
    }

    let forbidden = [
        project_root.join(versioned_artifact_name(
            "grid_timer_app",
            &version.version_name,
            "aab",
        )),
        project_root.join(versioned_artifact_name(
            "grid_timer_app_debug",
            &version.version_name,
            "apk",
        )),
        project_root.join(versioned_artifact_name(
            "grid_timer_app_xiaomi_debug",
            &version.version_name,
            "apk",
        )),
        project_root.join(versioned_artifact_name(
            "app-release",
            &version.version_name,
            "apk",
        )),
        project_root.join(versioned_artifact_name(
            "app-release",
            &version.version_name,
            "aab",
        )),
    ];
    for path in forbidden {
        if path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("forbidden package present: {}", path.display()),
            ));
        }
    }
    Ok(())
}

fn remove_existing_root_packages(project_root: &Path, base_name: &str) -> io::Result<()> {
    for entry in std::fs::read_dir(project_root)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with(base_name) {
            continue;
        }
        if name == format!("{base_name}.apk")
            || name == format!("{base_name}.aab")
            || (name.starts_with(&format!("{base_name}_v"))
                && (name.ends_with(".apk") || name.ends_with(".aab")))
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn run_status_generator(project_root: &Path) -> io::Result<()> {
    let cargo = cargo_toolchain_exe();
    let mut command = Command::new(cargo);
    command.current_dir(project_root);
    command.args([
        "run",
        "--offline",
        "--quiet",
        "--manifest-path",
        "native/gridtimer_native/Cargo.toml",
        "--bin",
        "gridtimer_setup_status",
        "--",
        "--output-path",
        "xiaomi_setup_status.md",
    ]);
    spawn_and_check(command, "status generation")
}

fn invoke_gradle(
    project_root: &Path,
    tasks: &[&str],
    gradle_user_home: &str,
    cargo_home: &str,
    rustup_home: &str,
    sdk_dir: &Path,
    ndk_dir: &Path,
) -> io::Result<()> {
    let mut command = Command::new(gradle_home().join("bin/gradle.bat"));
    command.current_dir(project_root);
    command.arg("--no-daemon");
    command.arg("--console=plain");
    command.arg("--offline");
    command.args(tasks);
    command.env("JAVA_HOME", gridtimer_native::tooling::java_home());
    command.env("ANDROID_HOME", sdk_dir);
    command.env("ANDROID_SDK_ROOT", sdk_dir);
    command.env("GRADLE_USER_HOME", gradle_user_home);
    command.env("CARGO_HOME", cargo_home);
    command.env("RUSTUP_HOME", rustup_home);
    command.env("ANDROID_NDK_HOME", ndk_dir);
    command.env("ANDROID_NDK_ROOT", ndk_dir);
    command.env("ANDROID_NDK", ndk_dir);
    spawn_and_check(command, "gradle build")
}

fn resolve_project_path(project_root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

fn flag_present(args: &[String], name: &str) -> bool {
    args.iter().any(|value| value == name)
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}
