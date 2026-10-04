// Windows - Rehearse distinct versions with explicit isolated supervisor boundaries.
// v0.0.3 - Send a valid empty snapshot when requesting a server-only download.
// v0.0.2 - Verify new writes survive publication finalization and another real runtime restart.
// v0.0.1 - Exercise real isolated runtime stop, publication, lock handoff and authenticated recovery.

#[test]
fn changed_retained_android_is_rejected_before_sync_runtime_quiesce() {
    let project = TestDirectory::new("preflight-before-sync-quiesce");
    let current = project.path().join("release_artifacts/current");
    fs::create_dir_all(&current).unwrap();
    fs::write(
        current.join(product_identity::RELEASE_MANIFEST_FILE_NAME),
        b"invalid release manifest",
    )
    .unwrap();
    assert!(optional_release_set_descriptor(&current).is_err());

    let action_called = Cell::new(false);
    let error = with_windows_only_publication_guard(
        project.path(),
        Path::new("unused-android-sdk"),
        || {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "retained Android changed",
            ))
        },
        || {
            action_called.set(true);
            Ok(())
        },
    )
    .expect_err("a changed APK must fail before inspecting live sync processes");
    assert!(error.to_string().contains("retained Android changed"));
    assert!(!action_called.get());
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimePublicationInputs {
    current: PathBuf,
    stable: PathBuf,
    old_supervisor: PathBuf,
    new_supervisor: PathBuf,
    candidate_source_root: PathBuf,
    android_sdk: PathBuf,
    evidence: PathBuf,
    port: u16,
}

fn runtime_rehearsal_artifacts(input: &RuntimePublicationInputs) -> WindowsReleaseArtifacts {
    let mut artifacts = load_prepared_windows(&input.candidate_source_root).unwrap();
    // Only the supervisor is an explicitly isolated harness build. It must keep
    // the correct candidate version and exact server-hash binding. The actual
    // server, desktop and stable entry remain the prepared images.
    let launcher = artifacts
        .current
        .iter_mut()
        .find(|item| item.public_base_name == "grid_timer_sync_launcher")
        .unwrap();
    require_real_regular_file(&input.new_supervisor, "isolated candidate supervisor").unwrap();
    launcher.source = input.new_supervisor.clone();
    artifacts
}

fn runtime_owned_root(root: &Path) {
    let canonical = fs::canonicalize(root).unwrap();
    assert!(canonical.starts_with(fs::canonicalize(env::temp_dir()).unwrap()));
    assert!(canonical
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("grid-timer-packager-runtime-publish-"));
    assert_eq!(
        fs::read(root.join("runtime.marker")).unwrap(),
        b"runtime-publication-v1"
    );
}

pub(super) fn runtime_publication_checkpoint(point: &str) {
    let Some(root) = env::var_os("TENRATE_RUNTIME_PUBLISH_CHILD") else {
        return;
    };
    if env::var("TENRATE_RUNTIME_POINT").ok().as_deref() != Some(point) {
        return;
    }
    let root = PathBuf::from(root);
    runtime_owned_root(&root);
    fs::write(root.join("publisher.paused"), point).unwrap();
    let started = Instant::now();
    while !root.join("publisher.exit").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "parent never authorized fixture exit"
        );
        thread::sleep(Duration::from_millis(20));
    }
    std::process::exit(86);
}

struct RuntimeOwnedChild(std::process::Child);
impl RuntimeOwnedChild {
    fn wait(&mut self, seconds: u64) -> std::process::ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(seconds),
                "owned runtime child timed out"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for RuntimeOwnedChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct RuntimeFixture(TestDirectory, Vec<String>);
impl RuntimeFixture {
    fn root(&self) -> &Path {
        self.0.path()
    }
    fn paths(&self) -> Vec<PathBuf> {
        // Cleanup must still recognize owned children when current is between
        // renames or its manifest is intentionally incomplete at a crash point.
        let current = self.root().join("release_artifacts/current");
        self.1.iter().map(|name| current.join(name)).collect()
    }
    fn processes(&self) -> Vec<(u32, PathBuf)> {
        let paths = self.paths();
        let names = paths
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase()
            })
            .collect();
        windows_release_process_api::processes_with_names(&names)
            .unwrap()
            .into_iter()
            .filter(|(_, path)| {
                paths.iter().any(|expected| {
                    normalize_release_root_text(&expected.to_string_lossy())
                        == normalize_release_root_text(&path.to_string_lossy())
                })
            })
            .collect()
    }
}
impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        runtime_owned_root(self.root());
        let mut processes = self.processes();
        processes.sort_by_key(|(_, path)| !path.to_string_lossy().contains("sync_launcher"));
        for (pid, path) in processes {
            let _ = windows_release_process_api::terminate_exact_process(pid, &path);
        }
    }
}

fn runtime_spawn(command: &mut Command, root: &Path, label: &str) -> RuntimeOwnedChild {
    use std::os::windows::process::CommandExt;
    runtime_owned_root(root);
    command
        .env("LOCALAPPDATA", root.join("localappdata"))
        .env("GRID_TIMER_LAUNCHER_DISABLE_AUTOSTART", "1")
        .env_remove("GRID_TIMER_SYNC_BACKUP_DIR")
        .env_remove("GRID_TIMER_WINDOWS_CLIENT_DIR")
        .env_remove("GRID_TIMER_PUBLIC_SERVER_URL")
        .env_remove("GRID_TIMER_PUBLIC_SERVER_URL_FILE")
        .env_remove("GRID_TIMER_STABLE_PUBLIC_SERVER_URL")
        .env_remove("GRID_TIMER_CLOUDFLARED_TUNNEL_TOKEN")
        .stdin(Stdio::null())
        .stdout(File::create(root.join(format!("{label}.out"))).unwrap())
        .stderr(File::create(root.join(format!("{label}.err"))).unwrap())
        .creation_flags(0x0800_0000);
    RuntimeOwnedChild(command.spawn().unwrap())
}

fn runtime_http(
    port: u16,
    path: &str,
    body: Option<&serde_json::Value>,
    token: &str,
) -> Result<serde_json::Value, String> {
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(20))
        .redirects(0)
        .build();
    let url = format!("http://127.0.0.1:{port}{path}");
    let response = if let Some(body) = body {
        agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("Authorization", &format!("Bearer {token}"))
            .send_string(&body.to_string())
    } else {
        agent.get(&url).call()
    };
    let text = response
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn runtime_wait_health(
    input: &RuntimePublicationInputs,
    fixture: &RuntimeFixture,
) -> serde_json::Value {
    let started = Instant::now();
    loop {
        if let Ok(health) = runtime_http(input.port, "/health", None, "") {
            let pid = health["serverProcessId"].as_u64().unwrap() as u32;
            let current = fixture.root().join("release_artifacts/current");
            let descriptor = derive_release_set_descriptor(&current).unwrap();
            let server = descriptor
                .files
                .iter()
                .find(|file| file.role == "sync_server")
                .unwrap();
            assert!(fixture
                .processes()
                .iter()
                .any(|(actual, path)| *actual == pid
                    && normalize_release_root_text(&path.to_string_lossy())
                        == normalize_release_root_text(
                            &release_set_file_path(&current, server).to_string_lossy()
                        )));
            let manifest = read_release_manifest_file(
                &current.join(product_identity::RELEASE_MANIFEST_FILE_NAME),
            )
            .unwrap();
            assert_eq!(health["serverBuildId"], descriptor.version);
            assert_eq!(
                health["serverSourceSnapshotSha256"],
                manifest.source_snapshot_sha256
            );
            assert_eq!(health["productId"], "gridtimer");
            assert_eq!(health["backupStatus"], "ok");
            return health;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "isolated server failed to become healthy; logs at {}",
            fixture.root().display()
        );
        thread::sleep(Duration::from_millis(100));
    }
}

fn runtime_result_has_note(result: &serde_json::Value, id: &str) -> bool {
    let value: serde_json::Value =
        serde_json::from_str(result["appDataJson"].as_str().unwrap()).unwrap();
    value["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note["id"] == id)
}

fn runtime_sync(
    input: &RuntimePublicationInputs,
    token: &str,
    snapshot: &str,
    previous: &serde_json::Value,
) -> serde_json::Value {
    let empty_snapshot =
        gridtimer_native::app_data::default_app_data_json(current_time_millis() as i64);
    let wire_snapshot = if snapshot.is_empty() {
        empty_snapshot.as_str()
    } else {
        snapshot
    };
    let result = runtime_http(input.port, "/v1/sync", Some(&serde_json::json!({
        "requestId":format!("runtime-{}",current_time_millis()), "appDataJson":wire_snapshot,
        "clientUpdatedAtEpochMillis":current_time_millis(), "deviceName":"isolated-runtime-rehearsal",
        "forceDownload":snapshot.is_empty(),
        "acknowledgedGeneration":previous["currentGeneration"].as_i64().unwrap_or(0),
        "serverInstanceId":previous["serverInstanceId"].as_str().unwrap_or(""),
        "accountNamespace":previous["accountNamespace"].as_str().unwrap_or(""),
        "workspaceId":previous["workspaceId"].as_str().unwrap_or(""),
        "workspaceProof":previous["workspaceProof"].as_str().unwrap_or("")
    })), token).unwrap();
    assert_eq!(result["ok"], true, "{result}");
    result
}

struct RuntimeReleaseHandle(*mut std::ffi::c_void);
impl RuntimeReleaseHandle {
    fn retained_while_owned(root: &Path) -> Self {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, timeout: u32) -> u32;
        }
        let name = release_mutex_name(&root.join("release_artifacts"))
            .unwrap()
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let handle = Self(unsafe { OpenMutexW(0x0010_0001, 0, name.as_ptr()) });
        assert!(
            !handle.0.is_null(),
            "publisher must keep its release mutex available"
        );
        assert_eq!(
            unsafe { WaitForSingleObject(handle.0, 0) },
            0x102,
            "publisher must own the release mutex until publication stops"
        );
        handle
    }
}
impl Drop for RuntimeReleaseHandle {
    fn drop(&mut self) {
        unsafe extern "system" {
            fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        if !self.0.is_null() {
            unsafe {
                ReleaseMutex(self.0);
                CloseHandle(self.0);
            }
        }
    }
}

#[test]
#[ignore = "Explicit real supervisor/server publication rehearsal with isolated port, locks and data"]
fn runtime_publication_stops_recovers_and_hands_off_with_data_intact() {
    let input: RuntimePublicationInputs =
        serde_json::from_slice(&fs::read(env::var_os("TENRATE_RUNTIME_INPUTS").unwrap()).unwrap())
            .unwrap();
    if let Some(root) = env::var_os("TENRATE_RUNTIME_PUBLISH_CHILD") {
        let root = PathBuf::from(root);
        runtime_owned_root(&root);
        let point = env::var("TENRATE_RUNTIME_POINT").unwrap();
        let retained = RetainedAndroidRelease::capture_published(&root).unwrap().0;
        let result = with_windows_only_publication_guard(
            &root,
            &input.android_sdk,
            || retained.verify_unchanged(),
            || {
                assert!(
                    enumerate_current_release_processes(
                        &root.join("release_artifacts/current"),
                        &derive_release_set_descriptor(&root.join("release_artifacts/current"))?
                    )
                    .unwrap()
                    .is_empty(),
                    "runtime must be stopped before publication starts"
                );
                if point == "action_error" {
                    return Err(io::Error::other("synthetic publication failure"));
                }
                if point == "finalize" {
                    return Ok(());
                }
                publish_release_transaction_with_scope(
                    &root,
                    &sample_version(product_identity::WINDOWS_APP_VERSION),
                    &input.android_sdk,
                    retained.current_apk(),
                    &runtime_rehearsal_artifacts(&input),
                    true,
                )
            },
        );
        if point == "action_error" {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("synthetic publication failure"));
        } else {
            result.unwrap();
        }
        retained.verify_unchanged().unwrap();
        return;
    }
    assert!(
        input.port >= 49152,
        "isolated rehearsal requires an allocated high port"
    );
    let prepared = load_prepared_windows(&input.candidate_source_root).unwrap();
    fs::create_dir_all(&input.evidence).unwrap();
    let installed = derive_release_set_descriptor(&input.current).unwrap();
    assert_ne!(installed.version, product_identity::WINDOWS_APP_VERSION);
    let old_launcher = installed
        .files
        .iter()
        .find(|file| file.role == "sync_launcher")
        .unwrap();
    let old_server = installed
        .files
        .iter()
        .find(|file| file.role == "sync_server")
        .unwrap();
    let old_supervisor_hash = sha256_file_hex(&input.old_supervisor).unwrap().1;
    let new_supervisor_hash = sha256_file_hex(&input.new_supervisor).unwrap().1;
    assert_ne!(old_supervisor_hash, old_launcher.sha256,
        "runtime rehearsal requires a reviewed isolated old supervisor, never the production supervisor");
    let prepared_supervisor = prepared
        .current
        .iter()
        .find(|item| item.public_base_name == "grid_timer_sync_launcher")
        .unwrap();
    assert_ne!(new_supervisor_hash, sha256_file_hex(&prepared_supervisor.source).unwrap().1,
        "runtime rehearsal requires a reviewed isolated new supervisor, never the production supervisor");
    let selected = env::var("TENRATE_RUNTIME_CASE").ok();
    assert!(
        selected.as_deref().is_none_or(|point| [
            "normal",
            "action_error",
            "current_removed",
            "phase_Committed"
        ]
        .contains(&point)),
        "unknown runtime case must not produce an empty success report"
    );
    let mut results = Vec::new();
    for point in [
        "normal",
        "action_error",
        "current_removed",
        "phase_Committed",
    ] {
        if selected.as_deref().is_some_and(|name| name != point) {
            continue;
        }
        let fixture = RuntimeFixture(
            TestDirectory::new("runtime-publish"),
            vec![
                old_launcher.file_name.clone(),
                old_server.file_name.clone(),
                product_identity::SYNC_LAUNCHER_FILE_NAME.to_string(),
                product_identity::SYNC_SERVER_FILE_NAME.to_string(),
            ],
        );
        let root = fixture.root();
        fs::write(root.join("runtime.marker"), b"runtime-publication-v1").unwrap();
        let current = root.join("release_artifacts/current");
        for directory in [
            current.join("tools"),
            root.join("release_artifacts/desktop_entry"),
            root.join("tools"),
            root.join("APK"),
            root.join("inputs"),
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        for file in &installed.files {
            fs::copy(
                release_set_file_path(&input.current, file),
                release_set_file_path(&current, file),
            )
            .unwrap();
        }
        fs::copy(
            &input.old_supervisor,
            release_set_file_path(&current, old_launcher),
        )
        .unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(current.join(product_identity::RELEASE_MANIFEST_FILE_NAME)).unwrap(),
        )
        .unwrap();
        let (bytes, digest) = sha256_file_hex(&input.old_supervisor).unwrap();
        let entry = manifest["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|item| item["role"] == "sync_launcher")
            .unwrap();
        entry["size"] = bytes.into();
        entry["sha256"] = digest.into();
        fs::write(
            current.join(product_identity::RELEASE_MANIFEST_FILE_NAME),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let apk = release_apk_descriptor(&installed).unwrap();
        fs::copy(current.join(&apk.file_name), root.join(&apk.file_name)).unwrap();
        fs::copy(
            current.join(&apk.file_name),
            root.join("APK").join(&apk.file_name),
        )
        .unwrap();
        fs::copy(
            current.join("tools/cloudflared.exe"),
            root.join("tools/cloudflared.exe"),
        )
        .unwrap();
        fs::copy(&input.stable, stable_desktop_entry_path(root)).unwrap();
        let retained = RetainedAndroidRelease::capture_published(root).unwrap().0;
        let mut start = Command::new(stable_desktop_entry_path(root));
        start.arg("--ensure-sync");
        assert!(runtime_spawn(&mut start, root, "initial_start")
            .wait(30)
            .success());
        let before_health = runtime_wait_health(&input, &fixture);
        let old_pid = before_health["serverProcessId"].as_u64().unwrap();
        let registered = runtime_http(input.port,"/v1/register",Some(&serde_json::json!({"email":"runtime@example.invalid","password":"SyntheticRuntimePassword123","deviceName":"fixture","requestId":"runtime-register"})),"").unwrap();
        assert_eq!(registered["ok"], true);
        let token = registered["token"].as_str().unwrap();
        let snapshot = gridtimer_native::app_data::upsert_note_app_data_json(&gridtimer_native::app_data::default_app_data_json(100), r#"{"id":"before-runtime-update","title":"Before update","content":"Preserve this note","updatedAtEpochMillis":100}"#,100).unwrap();
        let seeded = runtime_sync(&input, token, &snapshot, &serde_json::Value::Null);
        let stale_before_delete = gridtimer_native::app_data::upsert_note_app_data_json(
            seeded["appDataJson"].as_str().unwrap(),
            r#"{"id":"deleted-before-update","title":"Delete before migration","content":"Must never return","updatedAtEpochMillis":200}"#, 200).unwrap();
        let with_deleted_record = runtime_sync(&input, token, &stale_before_delete, &seeded);
        let deleted_snapshot = gridtimer_native::app_data::delete_note_permanently_app_data_json(
            with_deleted_record["appDataJson"].as_str().unwrap(),
            "deleted-before-update",
            300,
        )
        .unwrap();
        let seeded = runtime_sync(&input, token, &deleted_snapshot, &with_deleted_record);
        assert!(!runtime_result_has_note(&seeded, "deleted-before-update"));
        for key in ["userId", "serverInstanceId", "accountNamespace", "tokenId"] {
            assert!(
                !seeded[key].as_str().unwrap().is_empty(),
                "missing identity: {key}"
            );
        }
        assert!(seeded["appDataJson"]
            .as_str()
            .unwrap()
            .contains("before-runtime-update"));
        let database = root.join("localappdata/GridTimerSync/server_store.sqlite3");
        let schema = || {
            rusqlite::Connection::open_with_flags(
                &database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap()
            .pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
            .unwrap()
        };
        assert_eq!(schema(), 15);
        let mut publisher = Command::new(env::current_exe().unwrap());
        publisher
            .args([
                "--exact",
                "tests::runtime_publication_stops_recovers_and_hands_off_with_data_intact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TENRATE_RUNTIME_PUBLISH_CHILD", root)
            .env("TENRATE_RUNTIME_POINT", point);
        let mut publisher = runtime_spawn(&mut publisher, root, "publisher");
        let crash = matches!(point, "current_removed" | "phase_Committed");
        if crash {
            let started = Instant::now();
            while !root.join("publisher.paused").exists() {
                assert!(
                    publisher.0.try_wait().unwrap().is_none(),
                    "publisher exited before checkpoint: {}",
                    fs::read_to_string(root.join("publisher.err")).unwrap()
                );
                assert!(started.elapsed() < Duration::from_secs(180));
                thread::sleep(Duration::from_millis(25));
            }
            assert!(
                fixture.processes().is_empty(),
                "old runtime must be quiescent at the publication checkpoint"
            );
            let _retained_mutex = RuntimeReleaseHandle::retained_while_owned(root);
            let mut resolver = Command::new(stable_desktop_entry_path(root));
            resolver.args(["--sync-supervisor", "--resolve-only"]);
            let mut resolver = runtime_spawn(&mut resolver, root, "waiting_resolver");
            thread::sleep(Duration::from_millis(300));
            assert!(
                resolver.0.try_wait().unwrap().is_none(),
                "stable entry must wait for active publication"
            );
            fs::write(
                root.join("publisher.exit"),
                b"exit while holding release mutex",
            )
            .unwrap();
            assert_eq!(publisher.wait(30).code(), Some(86));
            assert!(
                resolver.wait(60).success(),
                "{}",
                fs::read_to_string(root.join("waiting_resolver.err")).unwrap()
            );
            assert!(runtime_spawn(&mut start, root, "recovery_start")
                .wait(30)
                .success());
        } else {
            assert!(
                publisher.wait(180).success(),
                "{}",
                fs::read_to_string(root.join("publisher.err")).unwrap()
            );
        }
        let after_health = runtime_wait_health(&input, &fixture);
        assert_ne!(
            old_pid,
            after_health["serverProcessId"].as_u64().unwrap(),
            "restart must replace the stopped server process"
        );
        let committed = matches!(point, "normal" | "phase_Committed");
        assert_eq!(schema(), if committed { 16 } else { 15 });
        assert_eq!(
            sha256_file_hex(&current.join(versioned_artifact_name(
                "grid_timer_sync_server",
                if committed {
                    product_identity::WINDOWS_APP_VERSION
                } else {
                    &installed.version
                },
                "exe"
            )))
            .unwrap(),
            sha256_file_hex(&if committed {
                rehearsal_server_path(&prepared).to_path_buf()
            } else {
                release_set_file_path(&input.current, old_server)
            })
            .unwrap()
        );
        let restored = runtime_sync(&input, token, &stale_before_delete, &seeded);
        assert!(
            !runtime_result_has_note(&restored, "deleted-before-update"),
            "migration/recovery must retain the old server's permanent deletion fence"
        );
        for key in ["userId", "serverInstanceId", "accountNamespace", "tokenId"] {
            assert_eq!(
                restored[key], seeded[key],
                "identity changed at {point}: {key}"
            );
        }
        assert!(restored["appDataJson"]
            .as_str()
            .unwrap()
            .contains("before-runtime-update"));
        let now = current_time_millis() as i64;
        let appended=gridtimer_native::app_data::upsert_note_app_data_json(restored["appDataJson"].as_str().unwrap(), &serde_json::json!({"id":"after-runtime-update","title":"After update","content":"New write after restart","updatedAtEpochMillis":now}).to_string(),now).unwrap();
        let written = runtime_sync(&input, token, &appended, &restored);
        assert!(written["appDataJson"]
            .as_str()
            .unwrap()
            .contains("after-runtime-update"));
        let finalized_health = if crash {
            let mut finalize = Command::new(env::current_exe().unwrap());
            finalize
                .args([
                    "--exact",
                    "tests::runtime_publication_stops_recovers_and_hands_off_with_data_intact",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("TENRATE_RUNTIME_PUBLISH_CHILD", root)
                .env("TENRATE_RUNTIME_POINT", "finalize");
            assert!(
                runtime_spawn(&mut finalize, root, "finalize")
                    .wait(180)
                    .success(),
                "{}",
                fs::read_to_string(root.join("finalize.err")).unwrap()
            );
            assert!(load_transaction_journal(root).unwrap().is_none());
            let health = runtime_wait_health(&input, &fixture);
            assert_ne!(health["serverProcessId"], after_health["serverProcessId"]);
            health
        } else {
            after_health.clone()
        };
        let persisted = runtime_sync(&input, token, "", &written);
        for key in ["userId", "serverInstanceId", "accountNamespace", "tokenId"] {
            assert_eq!(
                persisted[key], seeded[key],
                "finalization changed identity: {key}"
            );
        }
        assert!(
            !runtime_result_has_note(&persisted, "deleted-before-update"),
            "finalization and restart must not revive deleted content"
        );
        for note in ["before-runtime-update", "after-runtime-update"] {
            assert!(
                persisted["appDataJson"].as_str().unwrap().contains(note),
                "server-only read lost {note}"
            );
        }
        assert!(runtime_spawn(&mut start, root, "idempotent_start")
            .wait(30)
            .success());
        let final_health = runtime_wait_health(&input, &fixture);
        assert_eq!(
            final_health["serverProcessId"],
            finalized_health["serverProcessId"]
        );
        assert_eq!(
            fixture.processes().len(),
            2,
            "exactly one supervisor and server must remain"
        );
        retained.verify_unchanged().unwrap();
        let evidence = input.evidence.join(point);
        fs::create_dir_all(&evidence).unwrap();
        for item in fs::read_dir(root).unwrap().flatten() {
            if item
                .path()
                .extension()
                .is_some_and(|ext| ext == "out" || ext == "err")
            {
                fs::copy(item.path(), evidence.join(item.file_name())).unwrap();
            }
        }
        let record = serde_json::json!({"point":point,"oldWindowsVersion":installed.version,"candidateWindowsVersion":product_identity::WINDOWS_APP_VERSION,
            "candidateServerSource":prepared.build_identity.source_snapshot_sha256,
            "runtimeBoundary":"isolated supervisors; exact prepared candidate server and stable entry; not full unmodified release binaries",
            "oldSupervisorSha256":sha256_file_hex(&input.old_supervisor).unwrap().1,
            "newSupervisorSha256":sha256_file_hex(&input.new_supervisor).unwrap().1,"oldPid":old_pid,"newPid":after_health["serverProcessId"],"finalPid":finalized_health["serverProcessId"],"publisherExit":if crash {86}else{0},"committed":committed,"schema":schema(),"identitiesPreserved":true,"existingNotePreserved":true,"permanentDeletionFencePreserved":true,"newWriteSucceeded":true,"serverOnlyReadVerified":true,"newWriteSurvivesFinalization":crash,"idempotentStart":true,"apkUnchanged":true,"releaseLockHandoff":crash});
        fs::write(
            evidence.join("result.json"),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        results.push(record);
        println!("runtime publication {point}: real supervisor/server replacement, authenticated data and ownership verified");
    }
    fs::write(
        input.evidence.join("results.json"),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
