// v1.0.3.1 - Windows receipt coverage for independently retained APK updates.

fn windows_receipt_fixture(label: &str) -> (TestDirectory, PathBuf) {
    let project = TestDirectory::new(label);
    let current = project.path().join("release_artifacts/current");
    write_current_release_set(&current, PRODUCT.app_version);
    let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let mut manifest = read_release_manifest_file(&path).unwrap();
    manifest.windows_only = true;
    manifest.passed_checks = release_passed_checks(true);
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let inputs = project.path().join("inputs");
    fs::create_dir(&inputs).unwrap();
    write_windows_build_verification(
        project.path(),
        &sample_version(PRODUCT.app_version),
        &test_windows_artifacts(&inputs),
    )
    .unwrap();
    validate_windows_only_current_release(
        project.path(),
        &sample_version(PRODUCT.app_version),
        PRODUCT.app_version,
    )
    .unwrap();
    (project, current)
}

fn replace_fixture_retained_apk(current: &Path) -> ReleaseManifest {
    let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let mut manifest = read_release_manifest_file(&path).unwrap();
    let file = manifest
        .files
        .iter_mut()
        .find(|file| file.role == "apk")
        .unwrap();
    fs::remove_file(current.join(&file.file_name)).unwrap();
    let version = "9.8.7.1";
    let name = format!("tenfold_v{version}.apk");
    fs::write(current.join(&name), b"independently-published-retained-apk").unwrap();
    *file = describe_release_file("apk", &current.join(&name), &name).unwrap();
    manifest.android_release = Some(AndroidReleaseProvenance {
        version: version.to_string(),
        version_code: 9871,
        android_only: true,
        verification: format!("release_artifacts/verification/v{version}"),
        sha256: file.sha256.clone(),
        source_snapshot_sha256: Some("b".repeat(64)),
        created_at: "2026-09-22T12:00:00Z".to_string(),
    });
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    manifest
}

fn check_fixture_windows_receipt(project: &TestDirectory, current: &Path) -> io::Result<()> {
    let release = derive_release_set_descriptor(current)?;
    validate_windows_only_current_release(
        project.path(),
        &sample_version(PRODUCT.app_version),
        release_apk_version(&release)?,
    )
}

#[test]
fn windows_verification_accepts_independent_apk_change_with_unchanged_windows() {
    let (project, current) = windows_receipt_fixture("windows-receipt-independent-apk");
    let report_path = project
        .path()
        .join("release_artifacts/verification/windows_build_verification.json");
    let original_receipt = fs::read(&report_path).unwrap();
    replace_fixture_retained_apk(&current);
    check_fixture_windows_receipt(&project, &current)
        .expect("an independently published APK must not invalidate verified Windows binaries");
    assert_eq!(
        fs::read(&report_path).unwrap(),
        original_receipt,
        "validation must not rewrite historical build evidence"
    );

    // Canonical manifest identity does not depend on JSON indentation or ordering.
    let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let manifest = read_release_manifest_file(&path).unwrap();
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    check_fixture_windows_receipt(&project, &current).unwrap();
}

#[test]
fn windows_verification_rejects_windows_payload_or_manifest_identity_changes() {
    let (project, current) = windows_receipt_fixture("windows-receipt-tampering");
    let manifest = replace_fixture_retained_apk(&current);
    check_fixture_windows_receipt(&project, &current).unwrap();
    let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let original = serde_json::to_value(&manifest).unwrap();
    for (field, value) in [
        ("productId", serde_json::json!("another-product")),
        ("displayName", serde_json::json!("another-title")),
        ("version", serde_json::json!("1.0.3.999")),
        ("windowsOnly", serde_json::json!(false)),
        ("schemaVersion", serde_json::json!(999)),
        (
            "syncProtocolVersion",
            serde_json::json!(PRODUCT.sync_protocol_version + 1),
        ),
        ("gitCommit", serde_json::json!("d".repeat(40))),
        ("sourceWorktreeDirty", serde_json::json!(false)),
        ("sourceSnapshotSha256", serde_json::json!("c".repeat(64))),
        (
            "createdAtEpochMillis",
            serde_json::json!(manifest.created_at_epoch_millis + 1),
        ),
        ("passedChecks", serde_json::json!(["cargo_fmt"])),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            check_fixture_windows_receipt(&project, &current).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("rustc", "rustc 1.96.0 (other)"),
        ("cargo", "cargo 1.96.0 (other)"),
        ("host", "aarch64-pc-windows-msvc"),
        ("target", "aarch64-pc-windows-msvc"),
        ("linker", r"C:\other\link.exe"),
    ] {
        let mut changed = original.clone();
        changed["toolchain"][field] = serde_json::json!(value);
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            check_fixture_windows_receipt(&project, &current).is_err(),
            "toolchain.{field}"
        );
    }
    for role in [
        "sync_server",
        "sync_launcher",
        "windows_client",
        "cloudflared",
    ] {
        let mut changed = manifest.clone();
        let file = changed
            .files
            .iter_mut()
            .find(|file| file.role == role)
            .unwrap();
        let artifact_path = release_set_file_path(&current, file);
        let original_bytes = fs::read(&artifact_path).unwrap();
        fs::write(&artifact_path, b"replaced-Windows-executable").unwrap();
        *file = describe_release_file(role, &artifact_path, &file.file_name).unwrap();
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        // Its manifest now agrees with the replaced file, so the original
        // Windows build receipt must be what rejects this otherwise coherent set.
        derive_release_set_descriptor(&current).unwrap();
        assert!(
            check_fixture_windows_receipt(&project, &current).is_err(),
            "{role}"
        );
        fs::write(&artifact_path, original_bytes).unwrap();
    }
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    check_fixture_windows_receipt(&project, &current).unwrap();
}

#[test]
fn windows_verification_rejects_missing_or_inconsistent_independent_provenance() {
    let (project, current) = windows_receipt_fixture("windows-receipt-provenance");
    let manifest = replace_fixture_retained_apk(&current);
    let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let original = serde_json::to_value(&manifest).unwrap();
    for (field, value) in [
        ("version", serde_json::json!("9.8.7.2")),
        ("versionCode", serde_json::json!(0)),
        ("androidOnly", serde_json::json!(false)),
        ("verification", serde_json::json!("../untrusted")),
        ("sha256", serde_json::json!("0".repeat(64))),
        ("sourceSnapshotSha256", serde_json::json!("invalid")),
        ("createdAt", serde_json::json!("")),
        ("unknownOverride", serde_json::json!(true)),
    ] {
        let mut changed = original.clone();
        changed["androidRelease"][field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            check_fixture_windows_receipt(&project, &current).is_err(),
            "{field}"
        );
    }
    let mut missing = original.clone();
    missing.as_object_mut().unwrap().remove("androidRelease");
    fs::write(&path, serde_json::to_vec(&missing).unwrap()).unwrap();
    assert!(
        check_fixture_windows_receipt(&project, &current).is_err(),
        "a changed APK requires independent provenance"
    );
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let apk = manifest
        .files
        .iter()
        .find(|file| file.role == "apk")
        .unwrap();
    fs::write(current.join(&apk.file_name), b"unverified-replacement-apk").unwrap();
    assert!(
        check_fixture_windows_receipt(&project, &current).is_err(),
        "APK bytes must still agree with the independent release identity"
    );
}

#[test]
fn legacy_windows_verification_remains_bound_to_entire_release() {
    let (project, current) = windows_receipt_fixture("windows-receipt-legacy");
    let path = project
        .path()
        .join("release_artifacts/verification/windows_build_verification.json");
    let mut report: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    report["schemaVersion"] = serde_json::json!(2);
    report
        .as_object_mut()
        .unwrap()
        .remove("windowsManifestSha256");
    fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
    check_fixture_windows_receipt(&project, &current).unwrap();
    replace_fixture_retained_apk(&current);
    assert!(
        check_fixture_windows_receipt(&project, &current).is_err(),
        "a legacy receipt cannot prove a Windows scope that it never recorded"
    );
}

#[test]
fn full_platform_windows_verification_remains_bound_to_entire_release() {
    let project = TestDirectory::new("windows-receipt-full-platform");
    let current = project.path().join("release_artifacts/current");
    write_current_release_set(&current, PRODUCT.app_version);
    let inputs = project.path().join("inputs");
    fs::create_dir(&inputs).unwrap();
    let version = sample_version(PRODUCT.app_version);
    write_windows_build_verification(project.path(), &version, &test_windows_artifacts(&inputs))
        .unwrap();
    validate_windows_build_verification(project.path(), &current, &version).unwrap();
    let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let manifest = read_release_manifest_file(&path).unwrap();
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    derive_release_set_descriptor(&current).unwrap();
    assert!(
        validate_windows_build_verification(project.path(), &current, &version).is_err(),
        "a full-platform receipt keeps its exact original manifest binding"
    );
}

#[test]
fn windows_verification_requires_its_recorded_scope_and_known_schema() {
    let (project, current) = windows_receipt_fixture("windows-receipt-scope-proof");
    let path = project
        .path()
        .join("release_artifacts/verification/windows_build_verification.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (field, value) in [
        ("windowsManifestSha256", serde_json::Value::Null),
        ("windowsManifestSha256", serde_json::json!("0".repeat(64))),
        ("schemaVersion", serde_json::json!(99)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            check_fixture_windows_receipt(&project, &current).is_err(),
            "{field}"
        );
    }
}
