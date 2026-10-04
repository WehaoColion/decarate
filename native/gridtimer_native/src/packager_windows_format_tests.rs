// v1.0.3.1 - Formatting scope is a Windows release gate, including shared and included sources.

fn windows_format_fixture() -> (TestDirectory, serde_json::Value) {
    let project = TestDirectory::new("windows-format-scope");
    fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname='gridtimer_native'\nversion='0.0.0'\nedition='2021'\n",
    )
    .unwrap();
    for directory in ["src/bin", "src/desktop", "src/runtime", "src/sourcegen"] {
        fs::create_dir_all(project.path().join(directory)).unwrap();
    }
    let mut targets = vec![serde_json::json!({
        "name": "gridtimer_native", "kind": ["cdylib", "rlib"], "edition": "2021",
        "src_path": project.path().join("src/lib.rs"),
    })];
    fs::write(project.path().join("src/lib.rs"), "pub mod shared;\n").unwrap();
    for name in WINDOWS_FORMAT_BIN_NAMES {
        let path = project.path().join(format!("src/bin/{name}.rs"));
        fs::write(&path, "fn main() {}\n").unwrap();
        targets.push(serde_json::json!({
            "name": name, "kind": ["bin"], "edition": "2021", "src_path": path,
        }));
    }
    for path in [
        "src/shared.rs",
        "src/android_snapshot_verifier.rs",
        "src/desktop/included_editor.rs",
        "src/desktop/android_parity.rs",
        "src/runtime/mod.rs",
        "src/runtime/ownership_tests.rs",
        "src/packager_windows_format_tests.rs",
    ] {
        fs::write(project.path().join(path), "fn covered() {}\n").unwrap();
    }
    // Neither file is part of a Windows target. Even a syntax error in an
    // Android generator cannot authorize changing that platform's sources.
    fs::write(
        project.path().join("src/sourcegen/kotlin_sources.rs"),
        "fn deliberately incomplete Android source",
    )
    .unwrap();
    fs::write(
        project.path().join("src/bin/gridtimer_sourcegen.rs"),
        "mod deliberately_incomplete {",
    )
    .unwrap();
    targets.push(serde_json::json!({
        "name": "gridtimer_sourcegen", "kind": ["bin"], "edition": "2021",
        "src_path": project.path().join("src/bin/gridtimer_sourcegen.rs"),
    }));
    let metadata = serde_json::json!({"packages": [{
        "name": "gridtimer_native", "edition": "2021",
        "manifest_path": project.path().join("Cargo.toml"), "targets": targets,
    }]});
    (project, metadata)
}

#[test]
fn windows_format_scope_covers_shared_includes_and_excludes_android_generators() {
    let (project, metadata) = windows_format_fixture();
    let sourcegen = project.path().join("src/sourcegen/kotlin_sources.rs");
    let before = fs::read(&sourcegen).unwrap();
    let actual = collect_windows_format_sources(project.path(), &metadata).unwrap();
    let mut expected = vec![
        PathBuf::from("src/lib.rs"),
        PathBuf::from("src/shared.rs"),
        PathBuf::from("src/android_snapshot_verifier.rs"),
        PathBuf::from("src/desktop/included_editor.rs"),
        PathBuf::from("src/desktop/android_parity.rs"),
        PathBuf::from("src/runtime/mod.rs"),
        PathBuf::from("src/runtime/ownership_tests.rs"),
        PathBuf::from("src/packager_windows_format_tests.rs"),
    ];
    expected.extend(
        WINDOWS_FORMAT_BIN_NAMES
            .iter()
            .map(|name| PathBuf::from(format!("src/bin/{name}.rs"))),
    );
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(fs::read(&sourcegen).unwrap(), before);
    let batches = windows_format_batches(&actual).unwrap();
    assert_eq!(batches.into_iter().flatten().collect::<Vec<_>>(), actual);
}

#[test]
fn windows_format_scope_rejects_missing_roots_and_untrusted_metadata() {
    let (project, original) = windows_format_fixture();
    let mut missing = original.clone();
    missing["packages"][0]["targets"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    assert!(collect_windows_format_sources(project.path(), &missing).is_err());
    let outside = TestDirectory::new("windows-format-outside");
    let foreign = outside.path().join("foreign.rs");
    fs::write(&foreign, "fn main() {}\n").unwrap();
    let mut escaped = original.clone();
    escaped["packages"][0]["targets"][1]["src_path"] = serde_json::json!(foreign);
    assert!(collect_windows_format_sources(project.path(), &escaped).is_err());
    let mut wrong_edition = original.clone();
    wrong_edition["packages"][0]["targets"][1]["edition"] = serde_json::json!("2024");
    assert!(collect_windows_format_sources(project.path(), &wrong_edition).is_err());
    let mut duplicated = original.clone();
    let extra = duplicated["packages"][0]["targets"][1].clone();
    duplicated["packages"][0]["targets"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    assert!(collect_windows_format_sources(project.path(), &duplicated).is_err());
    let mut custom_build = original.clone();
    custom_build["packages"][0]["targets"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "build-script-build", "kind": ["custom-build"], "edition": "2021",
            "src_path": project.path().join("build.rs"),
        }));
    assert!(collect_windows_format_sources(project.path(), &custom_build).is_err());
    fs::remove_file(project.path().join("src/bin/timer_sync_server.rs")).unwrap();
    assert!(collect_windows_format_sources(project.path(), &original).is_err());
}

#[test]
fn windows_format_batches_cover_every_source_and_stop_before_tests_on_failure() {
    let paths = (0..500)
        .map(|index| PathBuf::from(format!("src/desktop/{}_{index}.rs", "长文件名".repeat(12))))
        .collect::<Vec<_>>();
    let batches = windows_format_batches(&paths).unwrap();
    assert!(batches.len() > 1);
    assert!(batches.iter().all(|batch| batch
        .iter()
        .map(|path| path.as_os_str().to_string_lossy().encode_utf16().count() * 2 + 3)
        .sum::<usize>()
        <= 16_000));
    assert_eq!(batches.into_iter().flatten().collect::<Vec<_>>(), paths);
    assert!(windows_format_batches(&[PathBuf::from("x".repeat(16_001))]).is_err());

    let mut visited = Vec::new();
    let error =
        run_windows_release_command_sequence(&WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE, |command| {
            visited.push(command);
            if command == WindowsReleaseCommand::FormatCheck {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows source is not formatted",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(visited, [WindowsReleaseCommand::FormatCheck]);
}

#[test]
fn windows_format_scope_includes_the_verified_icon_build_root() {
    let (project, mut metadata) = windows_format_fixture();
    let build = project.path().join("build.rs");
    fs::write(&build, "fn main() {}\n").unwrap();
    metadata["packages"][0]["targets"].as_array_mut().unwrap().push(serde_json::json!({
        "name":"build-script-build", "kind":["custom-build"], "edition":"2021", "src_path":build,
    }));
    let sources = collect_windows_format_sources(project.path(), &metadata).unwrap();
    assert_eq!(
        sources
            .iter()
            .filter(|path| **path == PathBuf::from("build.rs"))
            .count(),
        1
    );
    let outside = TestDirectory::new("foreign-build-script");
    fs::write(outside.path().join("build.rs"), "fn main() {}\n").unwrap();
    let index = metadata["packages"][0]["targets"].as_array().unwrap().len() - 1;
    metadata["packages"][0]["targets"][index]["src_path"] =
        serde_json::json!(outside.path().join("build.rs"));
    assert!(collect_windows_format_sources(project.path(), &metadata).is_err());
}
