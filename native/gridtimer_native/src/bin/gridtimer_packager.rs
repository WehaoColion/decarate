// v1.1.0.3 Windows - Keep one stable desktop entry and verify embedded icon sources.
// v1.0.3.16 Windows - Keep Windows publication independent of an unpublished Android source version.
// v1.0.3.1 - Check all Windows and shared Rust sources without formatting Android generators.
// v1.0.3.1 - Keep Windows verification bound to Windows content across independent APK releases.
// v1.0.1 - Preserve independent Android provenance with an optional validated source fingerprint.
// v1.0.1 - Prepare verified Windows binaries without publishing or changing the running service.
// v2.22.56 - Limit package test symbols while preserving every release gate.
// v2.22.56 - Bind the Windows client to the packaged sync service image.
// v2.22.41 - Retain verified published Android binaries during independent Windows updates.
// v2.22.38 Android - Publish tenfold names and recognize historical APK names.
// v2.22.39 - Reuse standard Cargo caches for direct Windows packaging.
#[path = "../windows_desktop_entry.rs"]
mod windows_desktop_entry;
#[cfg(test)]
#[path = "../windows_icon_resource.rs"]
mod windows_icon_resource;
use gridtimer_native::product_identity::{self, PRODUCT};
use gridtimer_native::sync_core::{SyncClientResult, SYNC_SERVER_BUILD_ID};
use gridtimer_native::tooling::{
    apksigner_path, archive_directory, archive_file, current_time_millis, ensure_directory,
    gradle_home, project_root, read_project_version_info, resolve_android_sdk_dir,
    resolve_latest_ndk_directory, spawn_and_check, versioned_artifact_name,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const WINDOWS_MSVC_TARGET: &str = "x86_64-pc-windows-msvc";
const WINDOWS_MSVC_TOOLCHAIN: &str = "stable-x86_64-pc-windows-msvc";
const MINIMUM_WINDOWS_RUSTC_VERSION: (u32, u32, u32) = (1, 95, 0);
const FALLBACK_RUSTUP_HOME: &str = r"C:\tools\rustup";
const RELEASE_BINARY_NAMES: [(&str, &str); 3] = [
    ("timer_sync_server", "grid_timer_sync_server"),
    ("timer_sync_launcher", "grid_timer_sync_launcher"),
    ("timer_windows_client", "grid_timer_windows_client"),
];
const STABLE_DESKTOP_ENTRY_BINARY_NAME: &str = "tenrate_desktop_launcher";
const STABLE_DESKTOP_ENTRY_FILE_NAME: &str = "TenRate_Desktop_Launcher.exe";
const RELEASE_TRANSACTION_MAGIC: &str = "TenRateReleaseTransaction";
const RELEASE_TRANSACTION_SCHEMA_VERSION: u32 = 1;
// Test-only process loss hooks; formal binaries have no interruption controls.
#[cfg(test)]
thread_local! { static RELEASE_PROCESS_INTERRUPTION: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
fn release_process_checkpoint(point: &str) {
    #[cfg(windows)]
    tests::runtime_publication_checkpoint(point);
    if RELEASE_PROCESS_INTERRUPTION.with(|value| value.borrow().as_deref() == Some(point)) {
        std::process::exit(86);
    }
}

const RELEASE_TRANSACTION_JOURNAL_NAME: &str = ".release-transaction-v1.json";
const MAX_RELEASE_TRANSACTION_JOURNAL_BYTES: u64 = 1024 * 1024;
const RELEASE_MANIFEST_SCHEMA_VERSION: u32 = 1;
const WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION: u32 = 3;
const LEGACY_WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION: u32 = 2;
const MAX_RELEASE_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_WINDOWS_BUILD_VERIFICATION_BYTES: u64 = 1024 * 1024;
const RELEASE_MUTEX_WAIT_MILLIS: u32 = 30_000;
const WINDOWS_SERVER_SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
const WINDOWS_SERVER_SMOKE_RESPONSE_MAX_BYTES: u64 = 64 * 1024;
const RELEASE_PASSED_CHECKS: [&str; 13] = [
    "cargo_fmt",
    "core_library_tests",
    "desktop_media_tests",
    "windows_client_tests",
    "sync_launcher_tests",
    "packager_tests",
    "stable_desktop_entry_tests",
    "android_unit_tests",
    "android_lint",
    "android_release_build",
    "android_signature_verification",
    "windows_executable_integrity_chain",
    "windows_server_runtime_smoke",
];
const WINDOWS_ONLY_PASSED_CHECKS: [&str; 11] = [
    "cargo_fmt",
    "core_library_tests",
    "desktop_media_tests",
    "windows_client_tests",
    "sync_launcher_tests",
    "packager_tests",
    "stable_desktop_entry_tests",
    "retained_android_signature_verification",
    "retained_android_bytes_and_modified_time",
    "windows_executable_integrity_chain",
    "windows_server_runtime_smoke",
];

fn is_false(value: &bool) -> bool {
    !*value
}

fn release_passed_checks(windows_only: bool) -> Vec<String> {
    let checks: &[&str] = if windows_only {
        &WINDOWS_ONLY_PASSED_CHECKS
    } else {
        &RELEASE_PASSED_CHECKS
    };
    checks.iter().map(|value| (*value).to_string()).collect()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseFileDescriptor {
    role: String,
    file_name: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseSetDescriptor {
    version: String,
    files: Vec<ReleaseFileDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AndroidReleaseProvenance {
    version: String,
    version_code: i32,
    android_only: bool,
    verification: String,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_snapshot_sha256: Option<String>,
    created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseManifest {
    schema_version: u32,
    product_id: String,
    display_name: String,
    version: String,
    #[serde(default, skip_serializing_if = "is_false")]
    windows_only: bool,
    sync_protocol_version: i64,
    git_commit: String,
    source_worktree_dirty: bool,
    source_snapshot_sha256: String,
    toolchain: ReleaseToolchainIdentity,
    passed_checks: Vec<String>,
    created_at_epoch_millis: i64,
    files: Vec<ReleaseFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    android_release: Option<AndroidReleaseProvenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseToolchainIdentity {
    rustc: String,
    cargo: String,
    host: String,
    target: String,
    linker: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WindowsBuildVerification {
    schema_version: u32,
    passed: bool,
    verified_at_epoch_millis: i64,
    product_id: String,
    app_version: String,
    #[serde(default, skip_serializing_if = "is_false")]
    windows_only: bool,
    sync_protocol_version: i64,
    git_commit: String,
    source_worktree_dirty: bool,
    source_snapshot_sha256: String,
    toolchain: ReleaseToolchainIdentity,
    artifacts: Vec<ReleaseFileDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    windows_manifest_sha256: Option<String>,
    passed_checks: Vec<String>,
    publication_performed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchivePlanEntry {
    source_kind: String,
    source_role: String,
    source_file_name: String,
    destination_kind: String,
    destination_file_name: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum ReleaseTransactionPhase {
    Prepared,
    StableSwitching,
    CurrentSwitching,
    RootPublishing,
    Committed,
    RollingBack,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseTransactionJournal {
    magic: String,
    schema_version: u32,
    transaction_id: String,
    project_identity: String,
    phase: ReleaseTransactionPhase,
    new_version: String,
    #[serde(default, skip_serializing_if = "is_false")]
    windows_only: bool,
    old_version: Option<String>,
    current_staging: String,
    current_backup: String,
    stable_staging: String,
    stable_backup: String,
    transaction_root: String,
    new_current: ReleaseSetDescriptor,
    old_current: Option<ReleaseSetDescriptor>,
    stable_new: ReleaseFileDescriptor,
    stable_old: Option<ReleaseFileDescriptor>,
    root_new: ReleaseFileDescriptor,
    legacy_new: ReleaseFileDescriptor,
    archive_plan: Vec<ArchivePlanEntry>,
    journal_checksum: String,
}

#[derive(Debug)]
struct ReleaseTransactionPaths {
    journal: PathBuf,
    current: PathBuf,
    current_staging: PathBuf,
    current_backup: PathBuf,
    stable: PathBuf,
    stable_staging: PathBuf,
    stable_backup: PathBuf,
    transaction_root: PathBuf,
    root_staging: PathBuf,
    root_same_name_backup: PathBuf,
    legacy_staging: PathBuf,
    legacy_same_name_backup: PathBuf,
}

#[derive(Default)]
struct PreparedTransactionOwnership {
    current_staging: bool,
    transaction_root: bool,
    stable_staging: bool,
}

#[derive(Debug)]
struct WindowsReleaseArtifact {
    public_base_name: &'static str,
    source: PathBuf,
}

#[derive(Debug)]
struct WindowsReleaseArtifacts {
    current: Vec<WindowsReleaseArtifact>,
    stable_desktop_entry: PathBuf,
    build_identity: BuildIdentity,
}

#[derive(Debug)]
struct WindowsBuildEnvironment {
    cargo: PathBuf,
    rustc: PathBuf,
    linker: PathBuf,
    library_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutableIntegrityBinding {
    size: u64,
    sha256: String,
}

impl ExecutableIntegrityBinding {
    fn marker(&self, role: &str) -> String {
        format!(
            "gridtimer-executable-binding-v1|{role}|{}|{}",
            self.size, self.sha256
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BuildIdentity {
    git_commit: String,
    source_worktree_dirty: bool,
    source_snapshot_sha256: String,
    toolchain: ReleaseToolchainIdentity,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum CurrentReleaseProcessRole {
    WindowsClient,
    SyncLauncher,
    SyncServer,
    Cloudflared,
}

impl CurrentReleaseProcessRole {
    fn from_descriptor_role(role: &str) -> Option<Self> {
        match role {
            "windows_client" => Some(Self::WindowsClient),
            "sync_launcher" => Some(Self::SyncLauncher),
            "sync_server" => Some(Self::SyncServer),
            "cloudflared" => Some(Self::Cloudflared),
            _ => None,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::WindowsClient => "Windows client",
            Self::SyncLauncher => "sync launcher",
            Self::SyncServer => "sync server",
            Self::Cloudflared => "cloudflared tunnel",
        }
    }

    fn termination_order(self) -> u8 {
        match self {
            Self::SyncLauncher => 0,
            Self::SyncServer => 1,
            Self::Cloudflared => 2,
            Self::WindowsClient => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CurrentReleaseProcess {
    process_id: u32,
    image_path: PathBuf,
    role: CurrentReleaseProcessRole,
}

struct OwnedSmokeProcess {
    child: Child,
}

impl Drop for OwnedSmokeProcess {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsReleaseCommand {
    LibraryTests,
    DesktopMediaTests,
    WindowsClientTests,
    SyncLauncherTests,
    PackagerTests,
    StableDesktopEntryTests,
    FormatCheck,
    ReleaseServerBuild,
    ReleaseLauncherBuild,
    ReleaseClientBuild,
}

const WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE: [WindowsReleaseCommand; 7] = [
    WindowsReleaseCommand::FormatCheck,
    WindowsReleaseCommand::LibraryTests,
    WindowsReleaseCommand::DesktopMediaTests,
    WindowsReleaseCommand::WindowsClientTests,
    WindowsReleaseCommand::SyncLauncherTests,
    WindowsReleaseCommand::PackagerTests,
    WindowsReleaseCommand::StableDesktopEntryTests,
];

const WINDOWS_FORMAT_BIN_NAMES: [&str; 5] = [
    "timer_windows_client",
    "timer_sync_server",
    "timer_sync_launcher",
    "tenrate_desktop_launcher",
    "gridtimer_packager",
];

fn default_rustup_home() -> PathBuf {
    let user_profile = env::var_os("USERPROFILE").map(PathBuf::from);
    select_default_rustup_home(
        user_profile.as_deref(),
        Path::new(FALLBACK_RUSTUP_HOME),
        rustup_home_has_complete_msvc_toolchain,
    )
}

fn resolved_cargo_home() -> String {
    select_cargo_home(
        env::var("CARGO_HOME").ok().as_deref(),
        env::var_os("USERPROFILE").map(PathBuf::from).as_deref(),
    )
    .to_string_lossy()
    .into_owned()
}

fn select_cargo_home(explicit: Option<&str>, user_profile: Option<&Path>) -> PathBuf {
    if let Some(explicit) = explicit.filter(|value| !value.trim().is_empty()) {
        return PathBuf::from(explicit);
    }
    user_profile
        .map(|profile| profile.join(".cargo"))
        .unwrap_or_else(|| PathBuf::from(r"C:\tools\cargo"))
}

fn windows_target_directory(
    project_root: &Path,
    test_build: bool,
    test_override: Option<&str>,
    release_override: Option<&str>,
    cargo_override: Option<&str>,
) -> PathBuf {
    if test_build {
        if let Some(value) = test_override.filter(|value| !value.trim().is_empty()) {
            return resolve_project_path(project_root, value);
        }
        let native_root = project_root.join("native/gridtimer_native");
        return cargo_override
            .filter(|value| !value.trim().is_empty())
            .map(|value| resolve_project_path(&native_root, value))
            .unwrap_or_else(|| native_root.join("target"));
    }
    release_override
        .filter(|value| !value.trim().is_empty())
        .map(|value| resolve_project_path(project_root, value))
        .unwrap_or_else(|| PathBuf::from(r"C:\gt\gridtimer-build\windows-release"))
}

fn rustup_home_has_complete_msvc_toolchain(rustup_home: &Path) -> bool {
    let toolchain = rustup_home.join("toolchains").join(WINDOWS_MSVC_TOOLCHAIN);
    [
        toolchain.join(r"bin\cargo.exe"),
        toolchain.join(r"bin\rustc.exe"),
        toolchain.join(r"lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe"),
    ]
    .iter()
    .all(|path| path.is_file())
}

fn select_default_rustup_home(
    user_profile: Option<&Path>,
    fallback: &Path,
    is_directory: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(user_rustup) = user_profile.map(|profile| profile.join(".rustup")) {
        if is_directory(&user_rustup) {
            return user_rustup;
        }
    }
    fallback.to_path_buf()
}

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
        "prepare-windows" => run_windows_preparation(&args),
        "publish-windows" => run_prepared_windows_publication(&args),
        "format-windows" => run_windows_format_only(&args),
        "finish" => run_finish(&args),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown command: {other}"),
        )),
    }
}

fn run_build(args: &[String]) -> io::Result<()> {
    let requested_configuration =
        flag_value(args, "--configuration").unwrap_or_else(|| "Release".to_string());
    require_formal_release_configuration(&requested_configuration)?;
    if flag_present(args, "--windows-only") {
        return run_windows_only_build(args);
    }
    let keep_previous_root_packages = flag_present(args, "--keep-previous-root-packages");
    let project_root = project_root();
    let version = read_project_version_info(&project_root)?;
    validate_source_release_identity(&version)?;
    if keep_previous_root_packages {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--keep-previous-root-packages is incompatible with the formal release gate",
        ));
    }
    let gradle_user_home = flag_value(args, "--gradle-user-home")
        .or_else(|| env::var("GRIDTIMER_GRADLE_USER_HOME").ok())
        .unwrap_or_else(|| r"C:\tools\gradle-home".to_string());
    let cargo_home = resolved_cargo_home();
    let rustup_home = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(default_rustup_home)
        .to_string_lossy()
        .into_owned();
    let android_sdk = resolve_android_sdk_dir(&project_root)
        .unwrap_or_else(|| PathBuf::from(r"C:\tools\android-sdk"));
    let ndk_dir = resolve_latest_ndk_directory(&android_sdk)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Android NDK not found"))?;

    ensure_directory(Path::new(&gradle_user_home))?;
    ensure_directory(&archive_directory(&project_root))?;
    if let Some(signing) = gridtimer_native::tooling::load_signing_properties(&project_root)? {
        let store_file = resolve_project_path(&project_root, &signing.store_file);
        if !store_file.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("release keystore not found at {}", store_file.display()),
            ));
        }
    }

    enforce_forbidden_delivery_package_policy(&project_root, false)?;
    let gradle_tasks = ["clean", "testDebugUnitTest", "lintDebug", "assembleRelease"];
    run_formal_release_pipeline(
        || run_windows_release_tests(&project_root, &cargo_home, &rustup_home),
        || {
            run_with_forbidden_package_cleanup(&project_root, || {
                invoke_gradle(
                    &project_root,
                    &gradle_tasks,
                    &gradle_user_home,
                    &cargo_home,
                    &rustup_home,
                    &android_sdk,
                    &ndk_dir,
                )
            })
        },
        || {
            run_with_forbidden_package_cleanup(&project_root, || {
                build_windows_release_artifacts(&project_root, &cargo_home, &rustup_home)
            })
        },
        |windows_artifacts| {
            run_with_forbidden_package_cleanup(&project_root, || {
                publish_release_artifacts(
                    &project_root,
                    &version,
                    &android_sdk,
                    &windows_artifacts,
                )?;
                validate_expected_packages(&project_root, &version)?;
                validate_current_release_artifacts(&project_root, &version)?;
                Ok(())
            })
        },
    )?;

    run_status_generator(&project_root, &cargo_home, &rustup_home)?;
    windows_desktop_entry::maintain_desktop_entry(&project_root)?;
    Ok(())
}

fn require_formal_release_configuration(requested_configuration: &str) -> io::Result<()> {
    if requested_configuration.eq_ignore_ascii_case("release") {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "formal delivery packager only supports Release; forbidden configuration requested: {requested_configuration}"
        ),
    ))
}

fn run_formal_release_pipeline<T>(
    required_tests: impl FnOnce() -> io::Result<()>,
    android_build: impl FnOnce() -> io::Result<()>,
    windows_build: impl FnOnce() -> io::Result<T>,
    publish: impl FnOnce(T) -> io::Result<()>,
) -> io::Result<()> {
    required_tests()?;
    android_build()?;
    let artifacts = windows_build()?;
    publish(artifacts)
}

fn run_finish(args: &[String]) -> io::Result<()> {
    if flag_present(args, "--windows-only") {
        return run_windows_only_finish(args);
    }
    let project_root = project_root();
    let version = read_project_version_info(&project_root)?;
    let expected_version_name =
        flag_value(args, "--expected-version-name").unwrap_or_else(|| version.version_name.clone());
    let expected_version_code = match flag_value(args, "--expected-version-code") {
        Some(value) => value.parse::<i32>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid --expected-version-code {value:?}: {error}"),
            )
        })?,
        None => version.version_code,
    };
    let validate_only = flag_present(args, "--validate-only");
    let archive_old_packages = flag_present(args, "--archive-old-packages");
    if validate_only && archive_old_packages {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--validate-only cannot be combined with --archive-old-packages",
        ));
    }

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
        validate_source_release_identity(&version)?;
        validate_expected_packages(&project_root, &version)?;
        validate_current_release_artifacts(&project_root, &version)?;
    }

    if archive_old_packages {
        enforce_forbidden_delivery_package_policy(&project_root, false)?;
        archive_existing_root_release_apks(&project_root)?;
    }

    Ok(())
}

#[derive(Debug)]
struct RetainedAndroidRelease {
    descriptor: ReleaseFileDescriptor,
    locations: Vec<(PathBuf, std::time::SystemTime)>,
}

impl RetainedAndroidRelease {
    fn capture_published(project_root: &Path) -> io::Result<(Self, String)> {
        // Pending Android source changes are independent of this Windows release.
        // Both descriptor reads and all three replica hashes must agree.
        let current = project_root.join("release_artifacts/current");
        let release = derive_release_set_descriptor(&current)?;
        let version = release_apk_version(&release)?.to_string();
        let retained = Self::capture(project_root, &version)?;
        Ok((retained, version))
    }

    fn capture(project_root: &Path, android_version: &str) -> io::Result<Self> {
        let current = project_root.join("release_artifacts/current");
        let release = derive_release_set_descriptor(&current)?;
        if release.files.len() != 6 || release_apk_version(&release)? != android_version {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "retained Android APK does not match Android source identity and release manifest",
            ));
        }
        let descriptor = release_apk_descriptor(&release)?.clone();
        let mut locations = Vec::new();
        for directory in [
            current,
            project_root.to_path_buf(),
            project_root.join("APK"),
        ] {
            require_real_directory(&directory, "existing Android delivery directory")?;
            let path = directory.join(&descriptor.file_name);
            verify_file_descriptor(&path, &descriptor)?;
            let modified = fs::metadata(&path)?.modified()?;
            locations.push((path, modified));
        }
        Ok(Self {
            descriptor,
            locations,
        })
    }

    fn current_apk(&self) -> &Path {
        &self.locations[0].0
    }

    fn verify_unchanged(&self) -> io::Result<()> {
        for (path, modified) in &self.locations {
            verify_file_descriptor(path, &self.descriptor)?;
            if fs::metadata(path)?.modified()? != *modified {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "retained Android APK modification time changed: {}",
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn windows_release_version(
    android: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<gridtimer_native::tooling::ProjectVersionInfo> {
    validate_version_component(&android.version_name)?;
    if android.version_code <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Android source version code must be positive",
        ));
    }
    validate_release_identity(PRODUCT.app_version, product_identity::WINDOWS_APP_VERSION)?;
    validate_release_identity(PRODUCT.app_version, SYNC_SERVER_BUILD_ID)?;
    Ok(gridtimer_native::tooling::ProjectVersionInfo {
        application_id: android.application_id.clone(),
        debug_suffix: android.debug_suffix.clone(),
        version_name: product_identity::WINDOWS_APP_VERSION.to_string(),
        version_code: android.version_code,
    })
}

fn reject_android_mutation_flags(args: &[String]) -> io::Result<()> {
    if [
        "--archive-old-packages",
        "--keep-previous-root-packages",
        "--gradle-user-home",
        "--expected-version-code",
    ]
    .iter()
    .any(|flag| flag_present(args, flag))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Windows-only publication does not accept Android build, version-code, or APK archive options"));
    }
    Ok(())
}

// Prepared images belong to the exact source snapshot that ran all Windows gates.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedWindowsFile {
    role: String,
    file_name: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedWindowsRelease {
    schema_version: u32,
    status: String,
    windows_version: String,
    created_at_epoch_millis: u128,
    source_root: PathBuf,
    git_commit: String,
    source_worktree_dirty: bool,
    source_snapshot_sha256: String,
    toolchain: ReleaseToolchainIdentity,
    checks: Vec<String>,
    android_publication_validated: bool,
    runtime_changed: bool,
    files: Vec<PreparedWindowsFile>,
}

fn load_prepared_windows(root: &Path) -> io::Result<WindowsReleaseArtifacts> {
    let prepared = root.join("release_artifacts/prepared");
    require_real_directory(&prepared, "prepared Windows directory")?;
    let path = prepared.join("prepared_windows.json");
    require_real_regular_file(&path, "prepared Windows verification")?;
    if fs::metadata(&path)?.len() > MAX_RELEASE_MANIFEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prepared verification is too large",
        ));
    }
    let report: PreparedWindowsRelease = serde_json::from_slice(&fs::read(&path)?)?;
    let expected_checks = [
        "cargo_fmt",
        "core_library_tests",
        "desktop_media_tests",
        "windows_client_tests",
        "sync_launcher_tests",
        "packager_tests",
        "stable_desktop_entry_tests",
        "windows_executable_integrity_chain",
        "windows_server_runtime_smoke",
    ];
    if report.schema_version != 1
        || report.status != "prepared_not_published"
        || report.windows_version != product_identity::WINDOWS_APP_VERSION
        || report.created_at_epoch_millis == 0
        || fs::canonicalize(&report.source_root)? != fs::canonicalize(root)?
        || report.source_snapshot_sha256 != source_snapshot_sha256(root)?
        || report.checks != expected_checks
        || report.android_publication_validated
        || report.runtime_changed
        || !valid_release_toolchain(&report.toolchain)
        || report.files.len() != 4
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prepared verification does not match the frozen Windows sources and gates",
        ));
    }
    let mut seen = BTreeSet::new();
    for file in &report.files {
        let expected_name = format!("{}_v{}.exe", file.role, report.windows_version);
        if !seen.insert(file.role.as_str())
            || file.file_name != expected_name
            || !(RELEASE_BINARY_NAMES
                .iter()
                .any(|(_, name)| *name == file.role)
                || file.role == STABLE_DESKTOP_ENTRY_BINARY_NAME)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prepared executable role or name is invalid",
            ));
        }
        let path = prepared.join(&file.file_name);
        require_real_regular_file(&path, "prepared Windows executable")?;
        if sha256_file_hex(&path)? != (file.size, file.sha256.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prepared executable changed after verification",
            ));
        }
    }
    let current = RELEASE_BINARY_NAMES
        .iter()
        .map(|(_, name)| WindowsReleaseArtifact {
            public_base_name: name,
            source: prepared.join(format!("{name}_v{}.exe", report.windows_version)),
        })
        .collect();
    let artifacts = WindowsReleaseArtifacts {
        current,
        stable_desktop_entry: prepared.join(format!(
            "{STABLE_DESKTOP_ENTRY_BINARY_NAME}_v{}.exe",
            report.windows_version
        )),
        build_identity: BuildIdentity {
            git_commit: report.git_commit,
            source_worktree_dirty: report.source_worktree_dirty,
            source_snapshot_sha256: report.source_snapshot_sha256,
            toolchain: report.toolchain,
        },
    };
    validate_windows_release_artifact_sources(&artifacts)?;
    Ok(artifacts)
}

fn run_prepared_windows_publication(args: &[String]) -> io::Result<()> {
    if args.len() != 2 || args[0] != "--project-root" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publish-windows requires --project-root <existing project>",
        ));
    }
    let source = project_root();
    let destination = fs::canonicalize(&args[1])?;
    require_real_directory(&destination, "publication project")?;
    require_real_regular_file(
        &destination.join("native/gridtimer_native/Cargo.toml"),
        "publication project manifest",
    )?;
    let windows = windows_release_version(&read_project_version_info(&source)?)?;
    let artifacts = load_prepared_windows(&source)?;
    let sdk = resolve_android_sdk_dir(&destination)
        .unwrap_or_else(|| PathBuf::from(r"C:\tools\android-sdk"));
    let (retained, retained_version) = RetainedAndroidRelease::capture_published(&destination)?;
    verify_release_apk(retained.current_apk(), &sdk)?;
    with_windows_only_publication_guard(
        &destination,
        &sdk,
        || {
            retained.verify_unchanged()?;
            load_prepared_windows(&source)?;
            Ok(())
        },
        || {
            retained.verify_unchanged()?;
            // Recheck after acquiring the shared release lock, before changing current.
            load_prepared_windows(&source)?;
            publish_release_transaction_with_scope(
                &destination,
                &windows,
                &sdk,
                retained.current_apk(),
                &artifacts,
                true,
            )?;
            retained.verify_unchanged()?;
            write_windows_build_verification(&destination, &windows, &artifacts)?;
            validate_windows_only_current_release(&destination, &windows, &retained_version)
        },
    )?;
    windows_desktop_entry::maintain_desktop_entry(&destination)?;
    println!(
        "Prepared Windows {} published; retained Android {}.",
        windows.version_name, retained_version
    );
    Ok(())
}

fn run_windows_preparation(args: &[String]) -> io::Result<()> {
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "prepare-windows accepts no options",
        ));
    }
    let root = project_root();
    let android = read_project_version_info(&root)?;
    let windows = windows_release_version(&android)?;
    let prepared = root.join("release_artifacts/prepared");
    if prepared.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "prepared output already exists",
        ));
    }
    let source_before = source_snapshot_sha256(&root)?;
    let cargo_home = resolved_cargo_home();
    let rustup_home = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(default_rustup_home)
        .to_string_lossy()
        .into_owned();
    run_windows_release_tests(&root, &cargo_home, &rustup_home)?;
    let artifacts = build_windows_release_artifacts(&root, &cargo_home, &rustup_home)?;
    validate_windows_release_artifact_sources(&artifacts)?;
    if source_before != artifacts.build_identity.source_snapshot_sha256
        || source_before != source_snapshot_sha256(&root)?
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sources changed across preparation gates",
        ));
    }
    ensure_directory(&prepared)?;
    let mut files = Vec::new();
    for (role, source) in artifacts
        .current
        .iter()
        .map(|artifact| (artifact.public_base_name, artifact.source.as_path()))
        .chain(std::iter::once((
            STABLE_DESKTOP_ENTRY_BINARY_NAME,
            artifacts.stable_desktop_entry.as_path(),
        )))
    {
        let name = format!("{role}_v{}.exe", windows.version_name);
        let destination = prepared.join(&name);
        let before = sha256_file_hex(source)?;
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        if sha256_file_hex(&destination)? != before || sha256_file_hex(source)? != before {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prepared executable changed while copying",
            ));
        }
        files.push(
            serde_json::json!({"role":role,"fileName":name,"size":before.0,"sha256":before.1}),
        );
    }
    let report = serde_json::json!({
        "schemaVersion":1,"status":"prepared_not_published","windowsVersion":windows.version_name,
        "createdAtEpochMillis":current_time_millis(),"sourceRoot":root,
        "gitCommit":artifacts.build_identity.git_commit,
        "sourceWorktreeDirty":artifacts.build_identity.source_worktree_dirty,
        "sourceSnapshotSha256":source_before,"toolchain":artifacts.build_identity.toolchain,
        "checks":["cargo_fmt","core_library_tests","desktop_media_tests","windows_client_tests",
            "sync_launcher_tests","packager_tests","stable_desktop_entry_tests",
            "windows_executable_integrity_chain","windows_server_runtime_smoke"],
        "androidPublicationValidated":false,"runtimeChanged":false,"files":files
    });
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(prepared.join("prepared_windows.json"))?;
    output.write_all(&serde_json::to_vec_pretty(&report)?)?;
    output.sync_all()?;
    println!(
        "Windows release prepared without publication: {}",
        prepared.display()
    );
    Ok(())
}

fn run_windows_only_build(args: &[String]) -> io::Result<()> {
    reject_android_mutation_flags(args)?;
    let project_root = project_root();
    let android = read_project_version_info(&project_root)?;
    let windows = windows_release_version(&android)?;
    let cargo_home = resolved_cargo_home();
    let rustup_home = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(default_rustup_home)
        .to_string_lossy()
        .into_owned();
    // Only apksigner is needed. No Gradle, NDK, keystore or Android build is invoked.
    let android_sdk = resolve_android_sdk_dir(&project_root)
        .unwrap_or_else(|| PathBuf::from(r"C:\tools\android-sdk"));
    if load_transaction_journal(&project_root)?.is_some() {
        with_windows_only_publication_guard(&project_root, &android_sdk, || Ok(()), || Ok(()))?;
    }
    let (retained, retained_android_version) =
        RetainedAndroidRelease::capture_published(&project_root)?;
    run_windows_only_release_pipeline(
        || verify_release_apk(retained.current_apk(), &android_sdk),
        || run_windows_release_tests(&project_root, &cargo_home, &rustup_home),
        || build_windows_release_artifacts(&project_root, &cargo_home, &rustup_home),
        |artifacts| {
            validate_windows_release_artifact_sources(&artifacts)?;
            with_windows_only_publication_guard(
                &project_root,
                &android_sdk,
                || retained.verify_unchanged(),
                || {
                    retained.verify_unchanged()?;
                    publish_release_transaction_with_scope(
                        &project_root,
                        &windows,
                        &android_sdk,
                        retained.current_apk(),
                        &artifacts,
                        true,
                    )?;
                    retained.verify_unchanged()?;
                    write_windows_build_verification(&project_root, &windows, &artifacts)?;
                    validate_windows_only_current_release(
                        &project_root,
                        &windows,
                        &retained_android_version,
                    )
                },
            )
        },
    )?;
    windows_desktop_entry::maintain_desktop_entry(&project_root)
}

fn run_windows_only_release_pipeline<T>(
    verify_retained_android: impl FnOnce() -> io::Result<()>,
    required_windows_tests: impl FnOnce() -> io::Result<()>,
    windows_build: impl FnOnce() -> io::Result<T>,
    publish: impl FnOnce(T) -> io::Result<()>,
) -> io::Result<()> {
    verify_retained_android()?;
    required_windows_tests()?;
    let artifacts = windows_build()?;
    publish(artifacts)
}

fn with_windows_only_publication_guard(
    project_root: &Path,
    android_sdk: &Path,
    preflight: impl FnOnce() -> io::Result<()>,
    action: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let release_root = project_root.join("release_artifacts");
    require_real_directory(&release_root, "existing release artifacts directory")?;
    let release_mutex = acquire_release_mutex(&release_root, RELEASE_MUTEX_WAIT_MILLIS)?;
    if load_transaction_journal(project_root)?.is_some_and(|journal| !journal.windows_only) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "an unfinished full-platform release exists; Windows-only mode cannot mutate its Android artifacts"));
    }
    // An APK may advance while Windows binaries are being built. Detect that
    // before stopping the live sync runtime, which would otherwise discard a
    // working quick tunnel even though publication cannot proceed.
    preflight()?;
    let mut runtime_quiesce = CurrentRuntimeQuiesceGuard::new(project_root);
    let publication_result = (|| {
        runtime_quiesce.quiesce()?;
        recover_or_finalize_existing_transaction(project_root, android_sdk)?;
        action()
    })();
    drop(release_mutex);
    let restart_result = runtime_quiesce.restart();
    match (publication_result, restart_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(restart)) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Windows publication failed ({error}); sync supervisor restart failed ({restart})"
            ),
        )),
    }
}

fn validate_windows_only_current_release(
    project_root: &Path,
    windows: &gridtimer_native::tooling::ProjectVersionInfo,
    android_version: &str,
) -> io::Result<()> {
    let current = project_root.join("release_artifacts/current");
    validate_current_release_directory_for_platforms(&current, windows, android_version, true)?;
    validate_windows_build_verification(project_root, &current, windows)
}

fn run_windows_only_finish(args: &[String]) -> io::Result<()> {
    reject_android_mutation_flags(args)?;
    if !flag_present(args, "--validate-only") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows-only finish requires --validate-only",
        ));
    }
    let project_root = project_root();
    let android = read_project_version_info(&project_root)?;
    let windows = windows_release_version(&android)?;
    if flag_value(args, "--expected-version-name")
        .is_some_and(|expected| expected != windows.version_name)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "requested Windows release version does not match source identity",
        ));
    }
    if load_transaction_journal(&project_root)?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release publication is unfinished",
        ));
    }
    let (retained, retained_android_version) =
        RetainedAndroidRelease::capture_published(&project_root)?;
    let android_sdk = resolve_android_sdk_dir(&project_root)
        .unwrap_or_else(|| PathBuf::from(r"C:\tools\android-sdk"));
    verify_release_apk(retained.current_apk(), &android_sdk)?;
    validate_windows_only_current_release(&project_root, &windows, &retained_android_version)?;
    retained.verify_unchanged()
}

fn publish_release_artifacts(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    android_sdk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
) -> io::Result<()> {
    let release_apk = resolve_formal_release_apk(project_root, version)?;
    verify_release_apk(&release_apk, android_sdk)?;
    enforce_forbidden_delivery_package_policy(project_root, true)?;
    validate_windows_release_artifact_sources(windows_artifacts)?;

    let release_root = project_root.join("release_artifacts");
    ensure_directory(&release_root)?;
    let release_mutex = acquire_release_mutex(&release_root, RELEASE_MUTEX_WAIT_MILLIS)?;
    let mut runtime_quiesce = CurrentRuntimeQuiesceGuard::new(project_root);
    let publication_result = (|| {
        runtime_quiesce.quiesce()?;
        recover_or_finalize_existing_transaction(project_root, android_sdk)?;
        publish_release_transaction(
            project_root,
            version,
            android_sdk,
            &release_apk,
            windows_artifacts,
        )?;
        write_windows_build_verification(project_root, version, windows_artifacts)
    })();
    drop(release_mutex);
    let restart_result = runtime_quiesce.restart();
    match (publication_result, restart_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(publication_error), Ok(())) => Err(publication_error),
        (Ok(()), Err(restart_error)) => Err(restart_error),
        (Err(publication_error), Err(restart_error)) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "release publication failed ({publication_error}); sync supervisor restart also failed ({restart_error})"
            ),
        )),
    }
}

fn write_windows_build_verification(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    windows_artifacts: &WindowsReleaseArtifacts,
) -> io::Result<()> {
    let current = project_root.join("release_artifacts/current");
    let descriptor = derive_release_set_descriptor(&current)?;
    if descriptor.version != version.version_name || descriptor.files.len() != 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published release is incomplete before verification report generation",
        ));
    }
    let manifest =
        read_release_manifest_file(&current.join(product_identity::RELEASE_MANIFEST_FILE_NAME))?;
    let report = WindowsBuildVerification {
        schema_version: WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION,
        passed: true,
        verified_at_epoch_millis: current_time_millis().min(i64::MAX as u128) as i64,
        product_id: PRODUCT.internal_id.to_string(),
        app_version: PRODUCT.app_version.to_string(),
        windows_only: manifest.windows_only,
        sync_protocol_version: PRODUCT.sync_protocol_version,
        git_commit: windows_artifacts.build_identity.git_commit.clone(),
        source_worktree_dirty: windows_artifacts.build_identity.source_worktree_dirty,
        source_snapshot_sha256: windows_artifacts
            .build_identity
            .source_snapshot_sha256
            .clone(),
        toolchain: windows_artifacts.build_identity.toolchain.clone(),
        artifacts: descriptor.files,
        windows_manifest_sha256: manifest
            .windows_only
            .then(|| windows_manifest_identity_sha256(&manifest))
            .transpose()?,
        passed_checks: release_passed_checks(manifest.windows_only),
        publication_performed: true,
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize Windows verification report: {error}"),
        )
    })?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_WINDOWS_BUILD_VERIFICATION_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows verification report size is invalid",
        ));
    }
    let directory = project_root.join("release_artifacts/verification");
    ensure_directory(&directory)?;
    require_real_directory(&directory, "Windows verification directory")?;
    let destination = directory.join("windows_build_verification.json");
    let staging = unique_unused_path(&directory, ".windows-verification-staging")?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    drop(output);

    let publish_result = if destination.exists() {
        require_real_regular_file(&destination, "existing Windows verification report")?;
        let backup = unique_unused_path(&directory, ".windows-verification-backup")?;
        replace_existing_file_atomically(&destination, &staging, &backup)?;
        remove_real_file_if_present(&backup)
    } else {
        fs::rename(&staging, &destination)
    };
    if publish_result.is_err() {
        let _ = remove_real_file_if_present(&staging);
        return publish_result;
    }
    if fs::read(&destination)? != bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "published Windows verification report changed during atomic replacement",
        ));
    }
    Ok(())
}

fn formal_release_apk_path(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> PathBuf {
    project_root
        .join("app/build/outputs/apk/release")
        .join(versioned_artifact_name(
            "tenfold",
            &version.version_name,
            "apk",
        ))
}

fn resolve_formal_release_apk(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<PathBuf> {
    let path = formal_release_apk_path(project_root, version);
    require_regular_file(&path, "versioned formal release APK")?;
    Ok(path)
}

impl ReleaseTransactionPaths {
    fn derive(project_root: &Path, journal: &ReleaseTransactionJournal) -> io::Result<Self> {
        validate_transaction_id(&journal.transaction_id)?;
        validate_safe_basename(&journal.root_new.file_name, "root APK")?;
        let release_root = project_root.join("release_artifacts");
        let desktop_entry = release_root.join("desktop_entry");
        let transaction_root = release_root.join(&journal.transaction_root);
        Ok(Self {
            journal: release_root.join(RELEASE_TRANSACTION_JOURNAL_NAME),
            current: release_root.join("current"),
            current_staging: release_root.join(&journal.current_staging),
            current_backup: release_root.join(&journal.current_backup),
            stable: desktop_entry.join(STABLE_DESKTOP_ENTRY_FILE_NAME),
            stable_staging: desktop_entry.join(&journal.stable_staging),
            stable_backup: desktop_entry.join(&journal.stable_backup),
            root_staging: transaction_root
                .join("root")
                .join(&journal.root_new.file_name),
            root_same_name_backup: transaction_root
                .join("root-backup")
                .join(&journal.root_new.file_name),
            legacy_staging: transaction_root
                .join("APK")
                .join(&journal.legacy_new.file_name),
            legacy_same_name_backup: transaction_root
                .join("APK-backup")
                .join(&journal.legacy_new.file_name),
            transaction_root,
        })
    }
}

fn expected_transaction_names(transaction_id: &str) -> (String, String, String, String, String) {
    (
        format!(".current-staging-{transaction_id}"),
        format!(".current-backup-{transaction_id}"),
        format!(".stable-staging-{transaction_id}.exe"),
        format!(".stable-backup-{transaction_id}.exe"),
        format!(".transaction-{transaction_id}"),
    )
}

fn validate_transaction_id(transaction_id: &str) -> io::Result<()> {
    let Some((millis, process_id)) = transaction_id
        .strip_prefix('t')
        .and_then(|value| value.split_once('p'))
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid release transaction id: {transaction_id:?}"),
        ));
    };
    if millis.is_empty()
        || process_id.is_empty()
        || !millis.bytes().all(|byte| byte.is_ascii_digit())
        || !process_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid release transaction id: {transaction_id:?}"),
        ));
    }
    Ok(())
}

fn validate_safe_basename(value: &str, label: &str) -> io::Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || Path::new(value).file_name().and_then(|name| name.to_str()) != Some(value)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} is not a safe basename: {value:?}"),
        ));
    }
    Ok(())
}

fn sha256_bytes_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file_hex(path: &Path) -> io::Result<(u64, String)> {
    require_real_regular_file(path, "release transaction file")?;
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "file size overflow"))?;
        digest.update(&buffer[..read]);
    }
    if size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release transaction file is empty: {}", path.display()),
        ));
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

fn verify_binary_contains_integrity_binding(
    path: &Path,
    binding: &ExecutableIntegrityBinding,
    bound_role: &str,
    label: &str,
) -> io::Result<()> {
    let marker = binding.marker(bound_role);
    if !file_contains_bytes(path, marker.as_bytes())? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} does not contain the expected downstream executable integrity binding"
            ),
        ));
    }
    Ok(())
}

fn file_contains_bytes(path: &Path, needle: &[u8]) -> io::Result<bool> {
    if needle.is_empty() {
        return Ok(true);
    }
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut carry = Vec::with_capacity(needle.len().saturating_sub(1));
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(false);
        }
        let mut window = Vec::with_capacity(carry.len() + read);
        window.extend_from_slice(&carry);
        window.extend_from_slice(&buffer[..read]);
        if window.windows(needle.len()).any(|value| value == needle) {
            return Ok(true);
        }
        let keep = needle.len().saturating_sub(1).min(window.len());
        carry.clear();
        carry.extend_from_slice(&window[window.len() - keep..]);
    }
}

fn capture_build_identity(
    project_root: &Path,
    build_environment: &WindowsBuildEnvironment,
) -> io::Result<BuildIdentity> {
    let git_commit = git_stdout(project_root, &["rev-parse", "HEAD"])
        .filter(|value| {
            matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let source_worktree_dirty = git_stdout(
        project_root,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .is_none_or(|status| !status.trim().is_empty());
    Ok(BuildIdentity {
        git_commit,
        source_worktree_dirty,
        source_snapshot_sha256: source_snapshot_sha256(project_root)?,
        toolchain: ReleaseToolchainIdentity {
            rustc: executable_version(&build_environment.rustc, "rustc")?,
            cargo: executable_version(&build_environment.cargo, "cargo")?,
            host: WINDOWS_MSVC_TARGET.to_string(),
            target: WINDOWS_MSVC_TARGET.to_string(),
            linker: fs::canonicalize(&build_environment.linker)?
                .to_string_lossy()
                .into_owned(),
        },
    })
}

fn executable_version(path: &Path, label: &str) -> io::Result<String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not run {label} version check: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("{label} version check failed with {}", output.status),
        ));
    }
    let version = String::from_utf8(output.stdout)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "tool version is not UTF-8"))?
        .trim()
        .to_string();
    if version.is_empty() || version.len() > 256 || version.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} returned an invalid version string"),
        ));
    }
    Ok(version)
}

fn git_stdout(project_root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
}

fn source_snapshot_sha256(project_root: &Path) -> io::Result<String> {
    let roots = [
        "settings.gradle",
        "gradle.properties",
        "app/build.gradle",
        "app/src",
        "tools/windows.ps1",
        "tools/windows_desktop_entry.ps1",
        "native/gridtimer_native/build.rs",
        "native/gridtimer_native/assets/windows",
        "native/gridtimer_native/Cargo.toml",
        "native/gridtimer_native/Cargo.lock",
        "native/gridtimer_native/src",
    ];
    let mut files = Vec::new();
    for relative in roots {
        collect_source_snapshot_files(project_root, &project_root.join(relative), &mut files)?;
    }
    files.sort();
    files.dedup();
    let mut digest = Sha256::new();
    for path in files {
        let relative = path.strip_prefix(project_root).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("source snapshot escaped project root: {}", path.display()),
            )
        })?;
        let normalized = relative.to_string_lossy().replace('\\', "/");
        digest.update((normalized.len() as u64).to_be_bytes());
        digest.update(normalized.as_bytes());
        let mut file = File::open(&path)?;
        let size = file.metadata()?.len();
        digest.update(size.to_be_bytes());
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_source_snapshot_files(
    project_root: &Path,
    path: &Path,
    files: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("required source snapshot path missing: {}", path.display()),
        )
    })?;
    if metadata_is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "source snapshot path is a reparse point: {}",
                path.display()
            ),
        ));
    }
    if metadata.is_file() {
        if !path.starts_with(project_root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source snapshot file escaped project root",
            ));
        }
        files.push(path.to_path_buf());
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "source snapshot path is not a file or directory: {}",
                path.display()
            ),
        ));
    }
    let mut children = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        collect_source_snapshot_files(project_root, &child.path(), files)?;
    }
    Ok(())
}

fn describe_release_file(
    role: &str,
    path: &Path,
    file_name: &str,
) -> io::Result<ReleaseFileDescriptor> {
    validate_safe_basename(file_name, "release file name")?;
    let (size, sha256) = sha256_file_hex(path)?;
    Ok(ReleaseFileDescriptor {
        role: role.to_string(),
        file_name: file_name.to_string(),
        size,
        sha256,
    })
}

fn validate_file_descriptor(descriptor: &ReleaseFileDescriptor) -> io::Result<()> {
    validate_safe_basename(&descriptor.file_name, "release descriptor file name")?;
    if descriptor.role.is_empty()
        || descriptor.size == 0
        || descriptor.sha256.len() != 64
        || !descriptor
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid release file descriptor for role {:?}",
                descriptor.role
            ),
        ));
    }
    Ok(())
}

fn verify_file_descriptor(path: &Path, descriptor: &ReleaseFileDescriptor) -> io::Result<()> {
    validate_file_descriptor(descriptor)?;
    let (size, sha256) = sha256_file_hex(path)?;
    if size != descriptor.size || sha256 != descriptor.sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "release file identity mismatch at {}: expected {} bytes {}, found {} bytes {}",
                path.display(),
                descriptor.size,
                descriptor.sha256,
                size,
                sha256
            ),
        ));
    }
    Ok(())
}

fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        windows_file_attributes_are_reparse_point(metadata.file_attributes())
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(windows)]
fn windows_file_attributes_are_reparse_point(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn require_real_regular_file(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{label} missing at {}: {error}", path.display()),
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata_is_reparse_point(&metadata)
        || metadata.len() == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} is not a non-empty real file: {}", path.display()),
        ));
    }
    Ok(())
}

fn require_real_directory(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{label} missing at {}: {error}", path.display()),
        )
    })?;
    if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} is not a real directory: {}", path.display()),
        ));
    }
    Ok(())
}

fn ensure_real_directory(path: &Path, label: &str) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => require_real_directory(path, label),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            ensure_directory(path)?;
            require_real_directory(path, label)
        }
        Err(error) => Err(error),
    }
}

fn require_path_absent(path: &Path, label: &str) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{label} must be absent before transaction prepare: {}",
                path.display()
            ),
        )),
        Err(error) => Err(error),
    }
}

fn release_project_identity(release_root: &Path) -> io::Result<String> {
    require_real_directory(release_root, "release artifacts directory")?;
    let normalized = normalized_release_root(release_root)?;
    Ok(sha256_bytes_hex(normalized.as_bytes()))
}

fn normalized_release_root(release_root: &Path) -> io::Result<String> {
    let canonical = fs::canonicalize(release_root)?;
    Ok(normalize_release_root_text(&canonical.to_string_lossy()))
}

fn normalize_release_root_text(value: &str) -> String {
    let windows_path = value.replace('/', "\\");
    let mut normalized = if windows_path
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &windows_path[8..])
    } else if windows_path
        .get(..4)
        .is_some_and(|prefix| prefix == r"\\?\")
    {
        windows_path[4..].to_string()
    } else {
        windows_path
    }
    .to_lowercase();
    while normalized.ends_with('\\') && normalized.len() > 3 {
        normalized.pop();
    }
    normalized
}

fn classify_current_release_process_path(
    current_directory: &Path,
    descriptor: &ReleaseSetDescriptor,
    process_path: &Path,
) -> Option<CurrentReleaseProcessRole> {
    let actual = normalize_release_root_text(&process_path.to_string_lossy());
    descriptor.files.iter().find_map(|file| {
        let role = CurrentReleaseProcessRole::from_descriptor_role(&file.role)?;
        let expected = release_set_file_path(current_directory, file);
        (actual == normalize_release_root_text(&expected.to_string_lossy())).then_some(role)
    })
}

fn expected_current_release_process_names(descriptor: &ReleaseSetDescriptor) -> BTreeSet<String> {
    descriptor
        .files
        .iter()
        .filter(|file| CurrentReleaseProcessRole::from_descriptor_role(&file.role).is_some())
        .map(|file| file.file_name.to_ascii_lowercase())
        .collect()
}

#[cfg(windows)]
struct CurrentRuntimeQuiesceGuard {
    project_root: PathBuf,
    restart_pending: bool,
}

#[cfg(windows)]
impl CurrentRuntimeQuiesceGuard {
    fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            restart_pending: false,
        }
    }

    fn quiesce(&mut self) -> io::Result<()> {
        const QUIESCE_TIMEOUT: Duration = Duration::from_secs(20);

        let current_directory = self.project_root.join("release_artifacts/current");
        let Some(descriptor) = optional_release_set_descriptor(&current_directory)? else {
            return Ok(());
        };
        let current_directory = fs::canonicalize(&current_directory)?;
        let started = Instant::now();

        loop {
            let mut processes =
                enumerate_current_release_processes(&current_directory, &descriptor)?;
            let clients = processes
                .iter()
                .filter(|process| process.role == CurrentReleaseProcessRole::WindowsClient)
                .map(|process| {
                    format!(
                        "PID {} ({})",
                        process.process_id,
                        process.image_path.display()
                    )
                })
                .collect::<Vec<_>>();
            if !clients.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "formal publication cannot replace a running Windows client; close it and retry: {}",
                        clients.join(", ")
                    ),
                ));
            }

            processes.retain(|process| process.role != CurrentReleaseProcessRole::WindowsClient);
            if processes.is_empty() {
                return Ok(());
            }
            if !self.restart_pending {
                self.restart_pending = true;
                println!(
                    "Temporarily stopping {} verified current-release sync process(es) for atomic publication...",
                    processes.len()
                );
            }
            processes.sort_by_key(|process| (process.role.termination_order(), process.process_id));
            let process = processes
                .into_iter()
                .next()
                .expect("non-empty verified release process list");
            windows_release_process_api::terminate_exact_process(
                process.process_id,
                &process.image_path,
            )?;
            println!(
                "Stopped {} PID {} ({})",
                process.role.description(),
                process.process_id,
                process.image_path.display()
            );

            if started.elapsed() >= QUIESCE_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out quiescing the verified current-release sync runtime",
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn restart(&mut self) -> io::Result<()> {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const START_TIMEOUT: Duration = Duration::from_secs(20);

        if !self.restart_pending {
            return Ok(());
        }
        let stable_entry = stable_desktop_entry_path(&self.project_root);
        require_real_regular_file(&stable_entry, "stable desktop entry for sync restart")?;
        let current_directory = self.project_root.join("release_artifacts/current");
        let mut command = Command::new(&stable_entry);
        command
            .arg("--sync-supervisor")
            .current_dir(&current_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "unable to restart sync supervisor through {}: {error}",
                    stable_entry.display()
                ),
            )
        })?;
        let started = Instant::now();
        loop {
            match child.try_wait()? {
                Some(status) if status.success() => {
                    self.restart_pending = false;
                    println!(
                        "Restarted the sync supervisor through {}",
                        stable_entry.display()
                    );
                    return Ok(());
                }
                Some(status) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "stable desktop entry failed to restart the sync supervisor: {status}"
                        ),
                    ));
                }
                None if started.elapsed() < START_TIMEOUT => {
                    thread::sleep(Duration::from_millis(100));
                }
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "stable desktop entry timed out while restarting the sync supervisor",
                    ));
                }
            }
        }
    }
}

#[cfg(windows)]
impl Drop for CurrentRuntimeQuiesceGuard {
    fn drop(&mut self) {
        if let Err(error) = self.restart() {
            eprintln!("WARNING: unable to restore the sync supervisor: {error}");
        }
    }
}

#[cfg(not(windows))]
struct CurrentRuntimeQuiesceGuard;

#[cfg(not(windows))]
impl CurrentRuntimeQuiesceGuard {
    fn new(_project_root: &Path) -> Self {
        Self
    }

    fn quiesce(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn restart(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn enumerate_current_release_processes(
    current_directory: &Path,
    descriptor: &ReleaseSetDescriptor,
) -> io::Result<Vec<CurrentReleaseProcess>> {
    let expected_names = expected_current_release_process_names(descriptor);
    let mut matching = Vec::new();
    for (process_id, image_path) in
        windows_release_process_api::processes_with_names(&expected_names)?
    {
        if let Some(role) =
            classify_current_release_process_path(current_directory, descriptor, &image_path)
        {
            matching.push(CurrentReleaseProcess {
                process_id,
                image_path,
                role,
            });
        }
    }
    Ok(matching)
}

#[cfg(windows)]
mod windows_release_process_api {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};

    type Handle = *mut c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const ERROR_NO_MORE_FILES: u32 = 18;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    const PROCESS_TERMINATE: u32 = 0x0000_0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x0000_1000;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0x0000_0000;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const WAIT_FAILED: u32 = 0xffff_ffff;
    const MAX_PROCESS_IMAGE_PATH_CHARS: usize = 32_768;
    const TERMINATION_WAIT_MILLIS: u32 = 10_000;
    const RELEASE_UPDATE_EXIT_CODE: u32 = 0xe000_0025;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        thread_count: u32,
        parent_process_id: u32,
        base_priority: i32,
        flags: u32,
        executable_file: [u16; 260],
    }

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "CreateToolhelp32Snapshot"]
        fn create_toolhelp32_snapshot(flags: u32, process_id: u32) -> Handle;
        #[link_name = "Process32FirstW"]
        fn process32_first(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        #[link_name = "Process32NextW"]
        fn process32_next(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        #[link_name = "OpenProcess"]
        fn open_process(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        #[link_name = "QueryFullProcessImageNameW"]
        fn query_full_process_image_name(
            process: Handle,
            flags: u32,
            executable_name: *mut u16,
            size: *mut u32,
        ) -> i32;
        #[link_name = "TerminateProcess"]
        fn terminate_process(process: Handle, exit_code: u32) -> i32;
        #[link_name = "WaitForSingleObject"]
        fn wait_for_single_object(handle: Handle, milliseconds: u32) -> u32;
        #[link_name = "GetLastError"]
        fn get_last_error() -> u32;
        #[link_name = "CloseHandle"]
        fn close_handle(handle: Handle) -> i32;
    }

    struct OwnedHandle(Handle);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = close_handle(self.0);
            }
        }
    }

    pub(super) fn processes_with_names(
        expected_names: &BTreeSet<String>,
    ) -> io::Result<Vec<(u32, PathBuf)>> {
        let invalid_handle = -1_isize as Handle;
        let snapshot = unsafe { create_toolhelp32_snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == invalid_handle {
            return Err(io::Error::last_os_error());
        }
        let _snapshot = OwnedHandle(snapshot);
        let mut entry: ProcessEntry32W = unsafe { zeroed() };
        entry.dw_size = size_of::<ProcessEntry32W>() as u32;
        if unsafe { process32_first(snapshot, &mut entry) } == 0 {
            let error = unsafe { get_last_error() };
            return if error == ERROR_NO_MORE_FILES {
                Ok(Vec::new())
            } else {
                Err(io::Error::from_raw_os_error(error as i32))
            };
        }

        let mut found = Vec::new();
        loop {
            let name_length = entry
                .executable_file
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(entry.executable_file.len());
            let name = String::from_utf16_lossy(&entry.executable_file[..name_length])
                .to_ascii_lowercase();
            if expected_names.contains(&name) {
                match query_process_image_path(entry.process_id) {
                    Ok(path) => found.push((entry.process_id, path)),
                    Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) => {}
                    Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                        eprintln!(
                            "Ignoring same-name PID {} ({name}) because its image path cannot be verified; it will never be terminated: {error}",
                            entry.process_id
                        );
                    }
                    Err(error) => {
                        return Err(io::Error::new(
                            error.kind(),
                            format!(
                                "unable to verify candidate release process PID {} ({name}): {error}",
                                entry.process_id
                            ),
                        ));
                    }
                }
            }

            entry = unsafe { zeroed() };
            entry.dw_size = size_of::<ProcessEntry32W>() as u32;
            if unsafe { process32_next(snapshot, &mut entry) } == 0 {
                let error = unsafe { get_last_error() };
                if error == ERROR_NO_MORE_FILES {
                    break;
                }
                return Err(io::Error::from_raw_os_error(error as i32));
            }
        }
        Ok(found)
    }

    pub(super) fn terminate_exact_process(process_id: u32, expected_path: &Path) -> io::Result<()> {
        let process = unsafe {
            open_process(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
                0,
                process_id,
            )
        };
        if process.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
                Ok(())
            } else {
                Err(error)
            };
        }
        let _process = OwnedHandle(process);
        let actual_path = query_process_image_path_from_handle(process)?;
        let expected = normalize_release_root_text(&expected_path.to_string_lossy());
        let actual = normalize_release_root_text(&actual_path.to_string_lossy());
        if actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "PID {process_id} changed identity before termination; expected {}, found {}",
                    expected_path.display(),
                    actual_path.display()
                ),
            ));
        }

        match unsafe { wait_for_single_object(process, 0) } {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT => {}
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            result => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("unexpected pre-termination wait result: 0x{result:08x}"),
                ));
            }
        }
        if unsafe { terminate_process(process, RELEASE_UPDATE_EXIT_CODE) } == 0 {
            let error = io::Error::last_os_error();
            if unsafe { wait_for_single_object(process, 0) } != WAIT_OBJECT_0 {
                return Err(error);
            }
        }
        match unsafe { wait_for_single_object(process, TERMINATION_WAIT_MILLIS) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("PID {process_id} did not exit after a verified release-update stop"),
            )),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            result => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("unexpected process termination wait result: 0x{result:08x}"),
            )),
        }
    }

    fn query_process_image_path(process_id: u32) -> io::Result<PathBuf> {
        let process = unsafe { open_process(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let _process = OwnedHandle(process);
        query_process_image_path_from_handle(process)
    }

    fn query_process_image_path_from_handle(process: Handle) -> io::Result<PathBuf> {
        let mut buffer = vec![0_u16; MAX_PROCESS_IMAGE_PATH_CHARS];
        let mut size = buffer.len() as u32;
        if unsafe { query_full_process_image_name(process, 0, buffer.as_mut_ptr(), &mut size) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        buffer.truncate(size as usize);
        Ok(PathBuf::from(String::from_utf16_lossy(&buffer)))
    }
}

fn release_mutex_name(release_root: &Path) -> io::Result<String> {
    Ok(format!(
        "Local\\TenRate.ReleaseArtifacts.{}",
        sha256_bytes_hex(normalized_release_root(release_root)?.as_bytes())
    ))
}

#[cfg(windows)]
struct ReleaseMutexGuard {
    handle: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl Drop for ReleaseMutexGuard {
    fn drop(&mut self) {
        unsafe extern "system" {
            #[link_name = "ReleaseMutex"]
            fn release_mutex(handle: *mut std::ffi::c_void) -> i32;
            #[link_name = "CloseHandle"]
            fn close_handle(handle: *mut std::ffi::c_void) -> i32;
        }
        unsafe {
            let _ = release_mutex(self.handle);
            let _ = close_handle(self.handle);
        }
    }
}

#[cfg(windows)]
fn acquire_release_mutex(release_root: &Path, wait_millis: u32) -> io::Result<ReleaseMutexGuard> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "CreateMutexW"]
        fn create_mutex_w(
            attributes: *mut c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut c_void;
        #[link_name = "WaitForSingleObject"]
        fn wait_for_single_object(handle: *mut c_void, milliseconds: u32) -> u32;
        #[link_name = "CloseHandle"]
        fn close_handle(handle: *mut c_void) -> i32;
    }

    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_ABANDONED: u32 = 0x0000_0080;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const WAIT_FAILED: u32 = 0xffff_ffff;

    let name = release_mutex_name(release_root)?;
    let mut wide = std::ffi::OsStr::new(&name)
        .encode_wide()
        .collect::<Vec<_>>();
    wide.push(0);
    let handle = unsafe { create_mutex_w(std::ptr::null_mut(), 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let wait_result = unsafe { wait_for_single_object(handle, wait_millis) };
    match wait_result {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(ReleaseMutexGuard { handle }),
        WAIT_TIMEOUT => {
            unsafe {
                let _ = close_handle(handle);
            }
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("timed out waiting {wait_millis} ms for the release transaction mutex"),
            ))
        }
        WAIT_FAILED => {
            let error = io::Error::last_os_error();
            unsafe {
                let _ = close_handle(handle);
            }
            Err(error)
        }
        other => {
            unsafe {
                let _ = close_handle(handle);
            }
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("unexpected release mutex wait result: 0x{other:08x}"),
            ))
        }
    }
}

#[cfg(not(windows))]
struct ReleaseMutexGuard;

#[cfg(not(windows))]
fn acquire_release_mutex(_release_root: &Path, _wait_millis: u32) -> io::Result<ReleaseMutexGuard> {
    Ok(ReleaseMutexGuard)
}

fn compute_journal_checksum(journal: &ReleaseTransactionJournal) -> io::Result<String> {
    let mut unsigned = journal.clone();
    unsigned.journal_checksum.clear();
    let encoded = serde_json::to_vec(&unsigned).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unable to encode release transaction journal: {error}"),
        )
    })?;
    Ok(sha256_bytes_hex(&encoded))
}

fn validate_transaction_journal(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
) -> io::Result<ReleaseTransactionPaths> {
    if journal.magic != RELEASE_TRANSACTION_MAGIC
        || journal.schema_version != RELEASE_TRANSACTION_SCHEMA_VERSION
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported release transaction journal identity",
        ));
    }
    validate_transaction_id(&journal.transaction_id)?;
    validate_version_component(&journal.new_version)?;
    if let Some(old_version) = &journal.old_version {
        validate_version_component(old_version)?;
    }
    let (current_staging, current_backup, stable_staging, stable_backup, transaction_root) =
        expected_transaction_names(&journal.transaction_id);
    if journal.current_staging != current_staging
        || journal.current_backup != current_backup
        || journal.stable_staging != stable_staging
        || journal.stable_backup != stable_backup
        || journal.transaction_root != transaction_root
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release transaction path names do not match the transaction id",
        ));
    }
    let release_root = project_root.join("release_artifacts");
    if journal.project_identity != release_project_identity(&release_root)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release transaction project identity mismatch",
        ));
    }
    if journal.journal_checksum != compute_journal_checksum(journal)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release transaction journal checksum mismatch",
        ));
    }
    validate_release_set_descriptor(&journal.new_current)?;
    if journal.new_current.version != journal.new_version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "new current descriptor version mismatch",
        ));
    }
    match (&journal.old_version, &journal.old_current) {
        (Some(version), Some(descriptor)) if version == &descriptor.version => {
            validate_release_set_descriptor(descriptor)?;
            if release_apk_version(descriptor)? != descriptor.version && descriptor.files.len() != 6
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mixed old release is missing its manifest identity",
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "old current descriptor version mismatch",
            ));
        }
    }
    for descriptor in [&journal.stable_new, &journal.root_new, &journal.legacy_new] {
        validate_file_descriptor(descriptor)?;
    }
    let apk = release_apk_descriptor(&journal.new_current)?;
    if journal.windows_only {
        let retained_apk = journal
            .old_current
            .as_ref()
            .map(release_apk_descriptor)
            .transpose()?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows-only journal requires an existing Android APK",
                )
            })?;
        if retained_apk != apk
            || journal.new_current.files.len() != 6
            || journal
                .archive_plan
                .iter()
                .any(|entry| entry.destination_kind != "old_exes")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows-only journal changes or archives Android artifacts",
            ));
        }
    } else if release_apk_version(&journal.new_current)? != journal.new_version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "full release journal contains mixed platform versions",
        ));
    }
    if journal.stable_new.role != "stable"
        || journal.stable_new.file_name != STABLE_DESKTOP_ENTRY_FILE_NAME
        || journal.root_new.role != "root_apk"
        || journal.root_new.file_name != apk.file_name
        || journal.root_new.size != apk.size
        || journal.root_new.sha256 != apk.sha256
        || journal.legacy_new.role != "legacy_apk"
        || journal.legacy_new.file_name != journal.root_new.file_name
        || journal.legacy_new.size != journal.root_new.size
        || journal.legacy_new.sha256 != journal.root_new.sha256
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release transaction contains an invalid public file role",
        ));
    }
    if let Some(descriptor) = &journal.stable_old {
        validate_file_descriptor(descriptor)?;
        if descriptor.role != "stable" || descriptor.file_name != STABLE_DESKTOP_ENTRY_FILE_NAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "old stable entry descriptor is invalid",
            ));
        }
    }
    validate_archive_plan(&journal.archive_plan, &journal.transaction_id)?;
    ReleaseTransactionPaths::derive(project_root, journal)
}

fn atomic_store_transaction_journal(
    project_root: &Path,
    journal: &mut ReleaseTransactionJournal,
) -> io::Result<()> {
    journal.journal_checksum = compute_journal_checksum(journal)?;
    validate_transaction_journal(project_root, journal)?;
    let path = project_root
        .join("release_artifacts")
        .join(RELEASE_TRANSACTION_JOURNAL_NAME);
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "journal has no parent directory",
        )
    })?;
    let staged = unique_unused_path(parent, ".release-transaction-v1-writing")?;
    let encoded = serde_json::to_vec(journal).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unable to encode release transaction journal: {error}"),
        )
    })?;
    if !transaction_journal_size_is_allowed(encoded.len() as u64) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "release transaction journal exceeds {} bytes",
                MAX_RELEASE_TRANSACTION_JOURNAL_BYTES
            ),
        ));
    }
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)?;
    let write_result = (|| {
        output.write_all(&encoded)?;
        output.sync_all()
    })();
    drop(output);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&staged);
        return Err(error);
    }
    let result = commit_staged_file(&staged, &path, "release transaction journal");
    if staged.exists() {
        let _ = fs::remove_file(staged);
    }
    result
}

fn load_transaction_journal(project_root: &Path) -> io::Result<Option<ReleaseTransactionJournal>> {
    let path = project_root
        .join("release_artifacts")
        .join(RELEASE_TRANSACTION_JOURNAL_NAME);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file()
        || metadata_is_reparse_point(&metadata)
        || !transaction_journal_size_is_allowed(metadata.len())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "release transaction journal is not a valid real file: {}",
                path.display()
            ),
        ));
    }
    let encoded = fs::read(&path)?;
    let journal =
        serde_json::from_slice::<ReleaseTransactionJournal>(&encoded).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid release transaction journal JSON: {error}"),
            )
        })?;
    validate_transaction_journal(project_root, &journal)?;
    Ok(Some(journal))
}

fn transaction_journal_size_is_allowed(size: u64) -> bool {
    size > 0 && size <= MAX_RELEASE_TRANSACTION_JOURNAL_BYTES
}

fn release_role_specifications() -> [(&'static str, &'static str, &'static str); 4] {
    [
        ("apk", "grid_timer_app_v", ".apk"),
        ("sync_server", "grid_timer_sync_server_v", ".exe"),
        ("sync_launcher", "grid_timer_sync_launcher_v", ".exe"),
        ("windows_client", "grid_timer_windows_client_v", ".exe"),
    ]
}

fn extract_release_version<'a>(name: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    // Continue recognizing archived and retained releases from before renaming.
    let prefixed = name.strip_prefix(prefix).or_else(|| {
        if prefix == "grid_timer_app_v" {
            name.strip_prefix("tenfold_v")
        } else if prefix == "tenfold_v" {
            name.strip_prefix("grid_timer_app_v")
        } else {
            None
        }
    })?;
    let version = prefixed.strip_suffix(suffix)?;
    (!version.is_empty()).then_some(version)
}

fn derive_release_set_descriptor(directory: &Path) -> io::Result<ReleaseSetDescriptor> {
    derive_release_set_descriptor_for_staging(directory, false)
}

fn derive_release_set_descriptor_for_staging(
    directory: &Path,
    windows_only_staging: bool,
) -> io::Result<ReleaseSetDescriptor> {
    require_real_directory(directory, "release set directory")?;
    let mut discovered = Vec::<(String, String, String, PathBuf)>::new();
    let mut tools_found = false;
    let mut manifest_path = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("non-Unicode entry in release set {}", directory.display()),
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if name == product_identity::RELEASE_MANIFEST_FILE_NAME {
            if manifest_path.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate release manifest",
                ));
            }
            require_real_regular_file(&path, "release manifest")?;
            manifest_path = Some(path);
            continue;
        }
        if name == "tools" {
            if tools_found {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate tools directory in release set",
                ));
            }
            require_real_directory(&path, "release tools directory")?;
            let mut tool_entries = fs::read_dir(&path)?;
            let cloudflared = tool_entries.next().transpose()?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "release tools directory is empty",
                )
            })?;
            if tool_entries.next().transpose()?.is_some()
                || cloudflared.file_name() != "cloudflared.exe"
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "release tools directory must contain only cloudflared.exe",
                ));
            }
            require_real_regular_file(&cloudflared.path(), "cloudflared runtime dependency")?;
            tools_found = true;
            continue;
        }
        if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected non-file release entry: {}", path.display()),
            ));
        }
        let mut matched = None;
        for (role, prefix, suffix) in release_role_specifications() {
            if let Some(version) = extract_release_version(&name, prefix, suffix) {
                matched = Some((role.to_string(), version.to_string()));
                break;
            }
        }
        let Some((role, version)) = matched else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected release artifact: {}", path.display()),
            ));
        };
        validate_version_component(&version)?;
        if discovered
            .iter()
            .any(|(found_role, _, _, _)| found_role == &role)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("duplicate {role} artifact in {}", directory.display()),
            ));
        }
        require_real_regular_file(&path, "release set artifact")?;
        discovered.push((role, version, name, path));
    }
    if !tools_found || discovered.len() != release_role_specifications().len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release set is incomplete at {}", directory.display()),
        ));
    }
    let versions = discovered
        .iter()
        .filter(|(role, _, _, _)| role != "apk")
        .map(|(_, version, _, _)| version.as_str())
        .collect::<BTreeSet<_>>();
    if versions.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "mixed release versions at {}: {versions:?}",
                directory.display()
            ),
        ));
    }
    let version = versions.iter().next().unwrap().to_string();
    let windows_only = match &manifest_path {
        Some(path) => read_release_manifest_file(path)?.windows_only,
        None => windows_only_staging,
    };
    let mut files = Vec::new();
    for (role, prefix, suffix) in release_role_specifications() {
        let (_, artifact_version, actual_name, path) = discovered
            .iter()
            .find(|(found_role, _, _, _)| found_role == role)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("missing {role} artifact"),
                )
            })?;
        let expected_version = if role == "apk" && windows_only {
            artifact_version
        } else {
            &version
        };
        let expected_name = format!("{prefix}{expected_version}{suffix}");
        if actual_name != &expected_name
            && !(role == "apk"
                && extract_release_version(actual_name, prefix, suffix)
                    == Some(expected_version.as_str()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected {role} artifact name: {actual_name}"),
            ));
        }
        files.push(describe_release_file(role, path, actual_name)?);
    }
    files.push(describe_release_file(
        "cloudflared",
        &directory.join("tools/cloudflared.exe"),
        "cloudflared.exe",
    )?);
    let mut descriptor = ReleaseSetDescriptor { version, files };
    validate_release_set_descriptor(&descriptor)?;
    if let Some(manifest_path) = manifest_path {
        validate_release_manifest_file(&manifest_path, &descriptor)?;
        descriptor.files.push(describe_release_file(
            "release_manifest",
            &manifest_path,
            product_identity::RELEASE_MANIFEST_FILE_NAME,
        )?);
        validate_release_set_descriptor(&descriptor)?;
    }
    Ok(descriptor)
}

fn validate_release_set_descriptor(descriptor: &ReleaseSetDescriptor) -> io::Result<()> {
    validate_version_component(&descriptor.version)?;
    if !matches!(descriptor.files.len(), 5 | 6) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release set descriptor must contain five payload files and an optional manifest",
        ));
    }
    release_apk_version(descriptor)?;
    let actual_apk_name = release_apk_descriptor(descriptor)?.file_name.clone();
    let mut expected = release_role_specifications()
        .into_iter()
        .map(|(role, prefix, suffix)| {
            (
                role.to_string(),
                if role == "apk" {
                    actual_apk_name.clone()
                } else {
                    format!("{prefix}{}{suffix}", descriptor.version)
                },
            )
        })
        .collect::<Vec<_>>();
    expected.push(("cloudflared".to_string(), "cloudflared.exe".to_string()));
    if descriptor.files.len() == 6 {
        expected.push((
            "release_manifest".to_string(),
            product_identity::RELEASE_MANIFEST_FILE_NAME.to_string(),
        ));
    }
    let actual = descriptor
        .files
        .iter()
        .map(|file| (file.role.clone(), file.file_name.clone()))
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release set descriptor mismatch: expected {expected:?}, found {actual:?}"),
        ));
    }
    for file in &descriptor.files {
        validate_file_descriptor(file)?;
    }
    Ok(())
}

fn write_release_manifest(
    directory: &Path,
    payload: &ReleaseSetDescriptor,
    build_identity: &BuildIdentity,
) -> io::Result<()> {
    write_release_manifest_with_scope(directory, payload, build_identity, false, None)
}

fn release_apk_descriptor(descriptor: &ReleaseSetDescriptor) -> io::Result<&ReleaseFileDescriptor> {
    descriptor
        .files
        .iter()
        .find(|file| file.role == "apk")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "release set has no Android APK identity",
            )
        })
}

fn release_apk_version(descriptor: &ReleaseSetDescriptor) -> io::Result<&str> {
    let file = release_apk_descriptor(descriptor)?;
    let version =
        extract_release_version(&file.file_name, "grid_timer_app_v", ".apk").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid Android APK artifact name",
            )
        })?;
    validate_version_component(version)?;
    Ok(version)
}

fn write_release_manifest_with_scope(
    directory: &Path,
    payload: &ReleaseSetDescriptor,
    build_identity: &BuildIdentity,
    windows_only: bool,
    android_release: Option<AndroidReleaseProvenance>,
) -> io::Result<()> {
    if payload.files.len() != 5
        || payload
            .files
            .iter()
            .any(|file| file.role == "release_manifest")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release manifest payload is not canonical",
        ));
    }
    let manifest = ReleaseManifest {
        schema_version: RELEASE_MANIFEST_SCHEMA_VERSION,
        product_id: PRODUCT.internal_id.to_string(),
        display_name: PRODUCT.display_name.to_string(),
        version: payload.version.clone(),
        windows_only,
        sync_protocol_version: PRODUCT.sync_protocol_version,
        git_commit: build_identity.git_commit.clone(),
        source_worktree_dirty: build_identity.source_worktree_dirty,
        source_snapshot_sha256: build_identity.source_snapshot_sha256.clone(),
        toolchain: build_identity.toolchain.clone(),
        passed_checks: release_passed_checks(windows_only),
        created_at_epoch_millis: current_time_millis().min(i64::MAX as u128) as i64,
        files: payload.files.clone(),
        android_release,
    };
    validate_release_manifest(&manifest, payload)?;
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize release manifest: {error}"),
        )
    })?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_RELEASE_MANIFEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release manifest size is invalid",
        ));
    }
    let path = directory.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()
}

fn validate_release_manifest_file(path: &Path, payload: &ReleaseSetDescriptor) -> io::Result<()> {
    let manifest = read_release_manifest_file(path)?;
    validate_release_manifest(&manifest, payload)
}

fn read_release_manifest_file(path: &Path) -> io::Result<ReleaseManifest> {
    require_real_regular_file(path, "release manifest")?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.len() == 0 || metadata.len() > MAX_RELEASE_MANIFEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release manifest size is invalid",
        ));
    }
    let raw = fs::read(path)?;
    if raw.is_empty() || raw.len() as u64 > MAX_RELEASE_MANIFEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release manifest size changed while it was being read",
        ));
    }
    serde_json::from_slice::<ReleaseManifest>(&raw).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release manifest JSON is invalid: {error}"),
        )
    })
}

fn validate_release_manifest(
    manifest: &ReleaseManifest,
    payload: &ReleaseSetDescriptor,
) -> io::Result<()> {
    if let Some(android) = &manifest.android_release {
        let apk = release_apk_descriptor(payload)?;
        if !manifest.windows_only
            || !android.android_only
            || android.version != release_apk_version(payload)?
            || android.version_code <= 0
            || android.sha256 != apk.sha256
            || android.source_snapshot_sha256.as_ref().is_some_and(|sha| {
                sha.len() != 64
                    || !sha
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
            || android.verification
                != format!("release_artifacts/verification/v{}", android.version)
            || android.created_at.trim().is_empty()
            || android.created_at.len() > 128
            || android.created_at.chars().any(char::is_control)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Android release provenance does not match the retained APK",
            ));
        }
    }
    let valid_git_commit = manifest.git_commit == "unknown"
        || (matches!(manifest.git_commit.len(), 40 | 64)
            && manifest
                .git_commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()));
    let valid_source_snapshot = manifest.source_snapshot_sha256.len() == 64
        && manifest
            .source_snapshot_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if manifest.schema_version != RELEASE_MANIFEST_SCHEMA_VERSION
        || manifest.product_id != PRODUCT.internal_id
        || manifest.display_name != PRODUCT.display_name
        || manifest.version != payload.version
        || manifest.sync_protocol_version != PRODUCT.sync_protocol_version
        || !valid_git_commit
        || !valid_source_snapshot
        || !valid_release_toolchain(&manifest.toolchain)
        || (!manifest.windows_only && release_apk_version(payload)? != payload.version)
        || manifest.passed_checks != release_passed_checks(manifest.windows_only)
        || manifest.created_at_epoch_millis <= 0
        || manifest.files != payload.files
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release manifest does not match the staged release payload",
        ));
    }
    Ok(())
}

fn valid_release_toolchain(toolchain: &ReleaseToolchainIdentity) -> bool {
    toolchain.rustc.starts_with("rustc ")
        && toolchain.cargo.starts_with("cargo ")
        && toolchain.host == WINDOWS_MSVC_TARGET
        && toolchain.target == WINDOWS_MSVC_TARGET
        && !toolchain.linker.trim().is_empty()
        && !toolchain.linker.chars().any(char::is_control)
}

fn release_set_file_path(directory: &Path, descriptor: &ReleaseFileDescriptor) -> PathBuf {
    if descriptor.role == "cloudflared" {
        directory.join("tools").join(&descriptor.file_name)
    } else {
        directory.join(&descriptor.file_name)
    }
}

fn verify_release_set_descriptor(
    directory: &Path,
    expected: &ReleaseSetDescriptor,
) -> io::Result<()> {
    let actual = derive_release_set_descriptor(directory)?;
    if &actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release set identity mismatch at {}", directory.display()),
        ));
    }
    Ok(())
}

fn optional_release_set_descriptor(directory: &Path) -> io::Result<Option<ReleaseSetDescriptor>> {
    match fs::symlink_metadata(directory) {
        Ok(_) => derive_release_set_descriptor(directory).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_formal_apk_basename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "grid_timer_app.apk" {
        return true;
    }
    let Some(version) = lower
        .strip_prefix("grid_timer_app_v")
        .or_else(|| lower.strip_prefix("tenfold_v"))
        .and_then(|value| value.strip_suffix(".apk"))
    else {
        return false;
    };
    !version.is_empty()
        && !lower.contains("debug")
        && !lower.contains("xiaomi")
        && validate_version_component(version).is_ok()
}

fn deterministic_archive_name(file_name: &str, source_tag: &str, txid: &str) -> io::Result<String> {
    validate_safe_basename(file_name, "archive source file")?;
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "archive source has no stem"))?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let name = format!("{stem}_{source_tag}_{txid}{extension}");
    validate_safe_basename(&name, "archive destination file")?;
    Ok(name)
}

fn archive_entry_from_descriptor(
    source_kind: &str,
    source_tag: &str,
    destination_kind: &str,
    descriptor: &ReleaseFileDescriptor,
    transaction_id: &str,
) -> io::Result<ArchivePlanEntry> {
    Ok(ArchivePlanEntry {
        source_kind: source_kind.to_string(),
        source_role: descriptor.role.clone(),
        source_file_name: descriptor.file_name.clone(),
        destination_kind: destination_kind.to_string(),
        destination_file_name: deterministic_archive_name(
            &descriptor.file_name,
            source_tag,
            transaction_id,
        )?,
        size: descriptor.size,
        sha256: descriptor.sha256.clone(),
    })
}

fn collect_formal_apk_descriptors(
    directory: &Path,
    role: &str,
) -> io::Result<Vec<ReleaseFileDescriptor>> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "formal APK scope is not a real directory: {}",
                directory.display()
            ),
        ));
    }
    let mut descriptors = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("non-Unicode APK name in {}", directory.display()),
            )
        })?;
        if !is_formal_apk_basename(&name) {
            continue;
        }
        require_real_regular_file(&entry.path(), "old formal APK")?;
        descriptors.push(describe_release_file(role, &entry.path(), &name)?);
    }
    descriptors.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    Ok(descriptors)
}

fn build_archive_plan(
    project_root: &Path,
    transaction_id: &str,
    old_current: Option<&ReleaseSetDescriptor>,
    old_stable: Option<&ReleaseFileDescriptor>,
) -> io::Result<Vec<ArchivePlanEntry>> {
    let mut plan = Vec::new();
    if let Some(old_current) = old_current {
        for file in &old_current.files {
            let destination_kind = if file.role == "apk" {
                "old_apks"
            } else {
                "old_exes"
            };
            plan.push(archive_entry_from_descriptor(
                "current_backup",
                "current",
                destination_kind,
                file,
                transaction_id,
            )?);
        }
    }
    if let Some(old_stable) = old_stable {
        plan.push(archive_entry_from_descriptor(
            "stable_backup",
            "stable",
            "old_exes",
            old_stable,
            transaction_id,
        )?);
    }
    for descriptor in collect_formal_apk_descriptors(project_root, "root_apk")? {
        plan.push(archive_entry_from_descriptor(
            "project_root",
            "root",
            "old_apks",
            &descriptor,
            transaction_id,
        )?);
    }
    for descriptor in collect_formal_apk_descriptors(&project_root.join("APK"), "legacy_apk")? {
        plan.push(archive_entry_from_descriptor(
            "legacy_apk",
            "legacy_APK",
            "old_apks",
            &descriptor,
            transaction_id,
        )?);
    }
    plan.sort_by(|left, right| {
        (&left.source_kind, &left.source_file_name)
            .cmp(&(&right.source_kind, &right.source_file_name))
    });
    validate_archive_plan(&plan, transaction_id)?;
    Ok(plan)
}

fn validate_archive_plan(plan: &[ArchivePlanEntry], transaction_id: &str) -> io::Result<()> {
    validate_transaction_id(transaction_id)?;
    let mut destinations = BTreeSet::new();
    for entry in plan {
        let (source_tag, valid_role, expected_destination_kind) = match entry.source_kind.as_str() {
            "current_backup" => (
                "current",
                matches!(
                    entry.source_role.as_str(),
                    "apk"
                        | "sync_server"
                        | "sync_launcher"
                        | "windows_client"
                        | "cloudflared"
                        | "release_manifest"
                ),
                if entry.source_role == "apk" {
                    "old_apks"
                } else {
                    "old_exes"
                },
            ),
            "stable_backup" => ("stable", entry.source_role == "stable", "old_exes"),
            "project_root" => ("root", entry.source_role == "root_apk", "old_apks"),
            "legacy_apk" => ("legacy_APK", entry.source_role == "legacy_apk", "old_apks"),
            _ => ("", false, ""),
        };
        if !valid_role || entry.destination_kind != expected_destination_kind {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "release archive plan contains an unsupported role",
            ));
        }
        validate_safe_basename(&entry.source_file_name, "archive source file")?;
        validate_safe_basename(&entry.destination_file_name, "archive destination file")?;
        if entry.size == 0
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || entry.destination_file_name
                != deterministic_archive_name(&entry.source_file_name, source_tag, transaction_id)?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "release archive plan contains an invalid file identity",
            ));
        }
        if !destinations.insert((
            entry.destination_kind.clone(),
            entry.destination_file_name.clone(),
        )) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "release archive plan contains a duplicate destination",
            ));
        }
    }
    Ok(())
}

fn prepare_release_transaction(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    release_apk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
) -> io::Result<ReleaseTransactionJournal> {
    let transaction_id = format!("t{}p{}", current_time_millis(), std::process::id());
    prepare_release_transaction_with_id(
        project_root,
        version,
        release_apk,
        windows_artifacts,
        &transaction_id,
    )
}

fn prepare_release_transaction_with_id(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    release_apk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
    transaction_id: &str,
) -> io::Result<ReleaseTransactionJournal> {
    prepare_release_transaction_with_scope(
        project_root,
        version,
        release_apk,
        windows_artifacts,
        transaction_id,
        false,
    )
}

fn prepare_release_transaction_with_scope(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    release_apk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
    transaction_id: &str,
    windows_only: bool,
) -> io::Result<ReleaseTransactionJournal> {
    let release_root = project_root.join("release_artifacts");
    require_real_directory(&release_root, "release artifacts directory")?;
    validate_transaction_id(transaction_id)?;
    let (current_staging, current_backup, stable_staging, stable_backup, transaction_root) =
        expected_transaction_names(transaction_id);
    let stable_path = stable_desktop_entry_path(project_root);
    let stable_parent = stable_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "stable entry has no parent"))?;
    ensure_real_directory(stable_parent, "stable desktop entry directory")?;
    require_path_absent(
        &release_root.join(&current_backup),
        "current transaction backup slot",
    )?;
    require_path_absent(
        &stable_parent.join(&stable_backup),
        "stable transaction backup slot",
    )?;
    let current_path = release_root.join("current");
    let old_current = optional_release_set_descriptor(&current_path)?;
    let retained_android_provenance = if windows_only {
        read_release_manifest_file(
            &current_path.join(product_identity::RELEASE_MANIFEST_FILE_NAME),
        )?
        .android_release
    } else {
        None
    };
    let old_version = old_current
        .as_ref()
        .map(|descriptor| descriptor.version.clone());

    let stable_old = match fs::symlink_metadata(&stable_path) {
        Ok(_) => Some(describe_release_file(
            "stable",
            &stable_path,
            STABLE_DESKTOP_ENTRY_FILE_NAME,
        )?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let root_file_name = if windows_only {
        let old = old_current.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows-only release requires a verified existing release",
            )
        })?;
        let apk = release_apk_descriptor(old)?;
        verify_file_descriptor(release_apk, apk)?;
        verify_file_descriptor(&project_root.join(&apk.file_name), apk)?;
        verify_file_descriptor(&project_root.join("APK").join(&apk.file_name), apk)?;
        apk.file_name.clone()
    } else {
        versioned_artifact_name("tenfold", &version.version_name, "apk")
    };
    let mut journal = ReleaseTransactionJournal {
        magic: RELEASE_TRANSACTION_MAGIC.to_string(),
        schema_version: RELEASE_TRANSACTION_SCHEMA_VERSION,
        transaction_id: transaction_id.to_string(),
        project_identity: release_project_identity(&release_root)?,
        phase: ReleaseTransactionPhase::Prepared,
        new_version: version.version_name.clone(),
        windows_only,
        old_version,
        current_staging,
        current_backup,
        stable_staging,
        stable_backup,
        transaction_root,
        new_current: ReleaseSetDescriptor {
            version: version.version_name.clone(),
            files: Vec::new(),
        },
        old_current,
        stable_new: ReleaseFileDescriptor {
            role: "stable".to_string(),
            file_name: STABLE_DESKTOP_ENTRY_FILE_NAME.to_string(),
            size: 1,
            sha256: "0".repeat(64),
        },
        stable_old,
        root_new: ReleaseFileDescriptor {
            role: "root_apk".to_string(),
            file_name: root_file_name.clone(),
            size: 1,
            sha256: "0".repeat(64),
        },
        legacy_new: ReleaseFileDescriptor {
            role: "legacy_apk".to_string(),
            file_name: root_file_name,
            size: 1,
            sha256: "0".repeat(64),
        },
        archive_plan: Vec::new(),
        journal_checksum: String::new(),
    };
    let paths = ReleaseTransactionPaths::derive(project_root, &journal)?;
    let mut ownership = PreparedTransactionOwnership::default();
    let prepare_result = (|| {
        require_real_directory(
            paths.stable.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "stable entry has no parent")
            })?,
            "stable desktop entry directory",
        )?;
        fs::create_dir(&paths.current_staging)?;
        ownership.current_staging = true;
        fs::create_dir(&paths.transaction_root)?;
        ownership.transaction_root = true;
        ensure_directory(paths.root_staging.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "root staging has no parent")
        })?)?;
        ensure_directory(paths.legacy_staging.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "APK staging has no parent")
        })?)?;

        let staged_apk = paths.current_staging.join(&journal.root_new.file_name);
        copy_file_create_new(release_apk, &staged_apk)?;
        if windows_only {
            let modified = fs::metadata(release_apk)?.modified()?;
            File::options()
                .write(true)
                .open(&staged_apk)?
                .set_times(fs::FileTimes::new().set_modified(modified))?;
            if fs::metadata(&staged_apk)?.modified()? != modified {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "retained Android APK modification time was not preserved",
                ));
            }
        }
        for artifact in &windows_artifacts.current {
            let file_name =
                versioned_artifact_name(artifact.public_base_name, &version.version_name, "exe");
            copy_file_create_new(&artifact.source, &paths.current_staging.join(file_name))?;
        }
        let tools = paths.current_staging.join("tools");
        fs::create_dir(&tools)?;
        copy_file_create_new(
            &project_root.join(r"tools\cloudflared.exe"),
            &tools.join("cloudflared.exe"),
        )?;
        let release_payload =
            derive_release_set_descriptor_for_staging(&paths.current_staging, windows_only)?;
        write_release_manifest_with_scope(
            &paths.current_staging,
            &release_payload,
            &windows_artifacts.build_identity,
            windows_only,
            retained_android_provenance.clone(),
        )?;
        journal.new_current = derive_release_set_descriptor(&paths.current_staging)?;
        if journal.new_current.version != version.version_name {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "staged current release version mismatch",
            ));
        }

        copy_file_create_new(
            &windows_artifacts.stable_desktop_entry,
            &paths.stable_staging,
        )?;
        ownership.stable_staging = true;
        journal.stable_new = describe_release_file(
            "stable",
            &paths.stable_staging,
            STABLE_DESKTOP_ENTRY_FILE_NAME,
        )?;
        if windows_only {
            journal.root_new = describe_release_file(
                "root_apk",
                &project_root.join(&journal.root_new.file_name),
                &journal.root_new.file_name,
            )?;
            journal.legacy_new = describe_release_file(
                "legacy_apk",
                &project_root.join("APK").join(&journal.legacy_new.file_name),
                &journal.legacy_new.file_name,
            )?;
        } else {
            copy_file_create_new(release_apk, &paths.root_staging)?;
            journal.root_new = describe_release_file(
                "root_apk",
                &paths.root_staging,
                &journal.root_new.file_name,
            )?;
            copy_file_create_new(release_apk, &paths.legacy_staging)?;
            journal.legacy_new = describe_release_file(
                "legacy_apk",
                &paths.legacy_staging,
                &journal.legacy_new.file_name,
            )?;
        }
        journal.archive_plan = build_archive_plan(
            project_root,
            &journal.transaction_id,
            journal.old_current.as_ref(),
            journal.stable_old.as_ref(),
        )?;
        if windows_only {
            journal
                .archive_plan
                .retain(|entry| entry.destination_kind == "old_exes");
        }
        journal.journal_checksum = compute_journal_checksum(&journal)?;
        validate_transaction_journal(project_root, &journal)?;
        Ok(())
    })();
    if let Err(error) = prepare_result {
        return match cleanup_owned_prepared_paths(&paths, &ownership) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("transaction prepare failed ({error}); owned staging cleanup failed ({cleanup_error})"),
            )),
        };
    }
    Ok(journal)
}

fn cleanup_owned_prepared_paths(
    paths: &ReleaseTransactionPaths,
    ownership: &PreparedTransactionOwnership,
) -> io::Result<()> {
    if ownership.stable_staging {
        remove_real_file_if_present(&paths.stable_staging)?;
    }
    if ownership.current_staging {
        remove_empty_directory_tree_if_present(&paths.current_staging)?;
    }
    if ownership.transaction_root {
        remove_empty_directory_tree_if_present(&paths.transaction_root)?;
    }
    Ok(())
}

fn cleanup_transaction_staging(
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    let remove_current_backup = match optional_release_set_descriptor(&paths.current_backup)? {
        None => false,
        Some(actual) if journal.old_current.as_ref() == Some(&actual) => true,
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "refusing to clean a current backup not owned by this transaction",
            ));
        }
    };
    let remove_stable_backup = match fs::symlink_metadata(&paths.stable_backup) {
        Ok(_) => {
            let Some(expected) = journal.stable_old.as_ref() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "refusing to clean an unexpected stable backup",
                ));
            };
            verify_file_descriptor(&paths.stable_backup, expected)?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    remove_empty_directory_tree_if_present(&paths.current_staging)?;
    remove_real_file_if_present(&paths.stable_staging)?;
    if remove_current_backup {
        remove_empty_directory_tree_if_present(&paths.current_backup)?;
    }
    if remove_stable_backup {
        remove_real_file_if_present(&paths.stable_backup)?;
    }
    remove_empty_directory_tree_if_present(&paths.transaction_root)
}

fn set_transaction_phase(
    project_root: &Path,
    journal: &mut ReleaseTransactionJournal,
    phase: ReleaseTransactionPhase,
) -> io::Result<()> {
    journal.phase = phase;
    atomic_store_transaction_journal(project_root, journal)?;
    #[cfg(test)]
    release_process_checkpoint(&format!("phase_{phase:?}"));
    Ok(())
}

fn switch_stable_for_transaction(
    paths: &ReleaseTransactionPaths,
    journal: &ReleaseTransactionJournal,
) -> io::Result<()> {
    verify_file_descriptor(&paths.stable_staging, &journal.stable_new)?;
    if let Some(old) = &journal.stable_old {
        verify_file_descriptor(&paths.stable, old)?;
        if paths.stable_backup.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "stable transaction backup already exists: {}",
                    paths.stable_backup.display()
                ),
            ));
        }
        if let Err(error) = replace_existing_file_atomically(
            &paths.stable,
            &paths.stable_staging,
            &paths.stable_backup,
        ) {
            let rollback = restore_old_file_after_failed_replace(
                &paths.stable,
                &paths.stable_backup,
                "stable desktop entry",
            );
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("stable switch failed ({error}); rollback failed ({rollback_error})"),
                )),
            };
        }
    } else {
        fs::rename(&paths.stable_staging, &paths.stable)?;
    }
    verify_file_descriptor(&paths.stable, &journal.stable_new)
}

fn switch_current_for_transaction(
    paths: &ReleaseTransactionPaths,
    journal: &ReleaseTransactionJournal,
) -> io::Result<()> {
    verify_release_set_descriptor(&paths.current_staging, &journal.new_current)?;
    if let Some(old) = &journal.old_current {
        verify_release_set_descriptor(&paths.current, old)?;
        if paths.current_backup.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "current transaction backup already exists: {}",
                    paths.current_backup.display()
                ),
            ));
        }
        fs::rename(&paths.current, &paths.current_backup)?;
        #[cfg(test)]
        release_process_checkpoint("current_removed");
        if let Err(error) = fs::rename(&paths.current_staging, &paths.current) {
            let rollback = fs::rename(&paths.current_backup, &paths.current);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("current switch failed ({error}); rollback failed ({rollback_error})"),
                )),
            };
        }
    } else {
        fs::rename(&paths.current_staging, &paths.current)?;
    }
    verify_release_set_descriptor(&paths.current, &journal.new_current)
}

fn publish_root_for_transaction(
    project_root: &Path,
    paths: &ReleaseTransactionPaths,
    journal: &ReleaseTransactionJournal,
) -> io::Result<PathBuf> {
    if journal.windows_only {
        let destination = project_root.join(&journal.root_new.file_name);
        verify_file_descriptor(&destination, &journal.root_new)?;
        return Ok(destination);
    }
    verify_file_descriptor(&paths.root_staging, &journal.root_new)?;
    let destination = project_root.join(&journal.root_new.file_name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            require_real_regular_file(&destination, "existing root release APK")?;
            ensure_directory(paths.root_same_name_backup.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "root backup has no parent")
            })?)?;
            replace_existing_file_atomically(
                &destination,
                &paths.root_staging,
                &paths.root_same_name_backup,
            )?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(&paths.root_staging, &destination)?;
        }
        Err(error) => return Err(error),
    }
    verify_file_descriptor(&destination, &journal.root_new)?;
    Ok(destination)
}

fn publish_legacy_apk_for_transaction(
    project_root: &Path,
    paths: &ReleaseTransactionPaths,
    journal: &ReleaseTransactionJournal,
) -> io::Result<PathBuf> {
    if journal.windows_only {
        let destination = project_root.join("APK").join(&journal.legacy_new.file_name);
        verify_file_descriptor(&destination, &journal.legacy_new)?;
        return Ok(destination);
    }
    verify_file_descriptor(&paths.legacy_staging, &journal.legacy_new)?;
    let apk_directory = project_root.join("APK");
    ensure_directory(&apk_directory)?;
    require_real_directory(&apk_directory, "formal APK delivery directory")?;
    let destination = apk_directory.join(&journal.legacy_new.file_name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            require_real_regular_file(&destination, "existing APK delivery artifact")?;
            ensure_directory(paths.legacy_same_name_backup.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "APK backup has no parent")
            })?)?;
            replace_existing_file_atomically(
                &destination,
                &paths.legacy_staging,
                &paths.legacy_same_name_backup,
            )?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::rename(&paths.legacy_staging, &destination)?;
        }
        Err(error) => return Err(error),
    }
    verify_file_descriptor(&destination, &journal.legacy_new)?;
    Ok(destination)
}

fn publish_release_transaction(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    android_sdk: &Path,
    release_apk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
) -> io::Result<()> {
    publish_release_transaction_with_scope(
        project_root,
        version,
        android_sdk,
        release_apk,
        windows_artifacts,
        false,
    )
}

fn publish_release_transaction_with_scope(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    android_sdk: &Path,
    release_apk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
    windows_only: bool,
) -> io::Result<()> {
    let transaction_id = format!("t{}p{}", current_time_millis(), std::process::id());
    let mut journal = prepare_release_transaction_with_scope(
        project_root,
        version,
        release_apk,
        windows_artifacts,
        &transaction_id,
        windows_only,
    )?;
    let paths = ReleaseTransactionPaths::derive(project_root, &journal)?;
    atomic_store_transaction_journal(project_root, &mut journal)?;
    #[cfg(test)]
    release_process_checkpoint("prepared");

    let publish_result = (|| {
        set_transaction_phase(
            project_root,
            &mut journal,
            ReleaseTransactionPhase::StableSwitching,
        )?;
        switch_stable_for_transaction(&paths, &journal)?;
        #[cfg(test)]
        release_process_checkpoint("stable_switched");
        set_transaction_phase(
            project_root,
            &mut journal,
            ReleaseTransactionPhase::CurrentSwitching,
        )?;
        switch_current_for_transaction(&paths, &journal)?;
        #[cfg(test)]
        release_process_checkpoint("current_switched");
        set_transaction_phase(
            project_root,
            &mut journal,
            ReleaseTransactionPhase::RootPublishing,
        )?;
        let root_apk = publish_root_for_transaction(project_root, &paths, &journal)?;
        let legacy_apk = publish_legacy_apk_for_transaction(project_root, &paths, &journal)?;
        verify_release_apk(&root_apk, android_sdk)?;
        verify_release_apk(&legacy_apk, android_sdk)?;
        set_transaction_phase(
            project_root,
            &mut journal,
            ReleaseTransactionPhase::Committed,
        )?;
        finalize_committed_transaction(project_root, android_sdk, &journal, &paths)
    })();

    if publish_result.is_ok() || journal.phase == ReleaseTransactionPhase::Committed {
        return publish_result;
    }
    let publish_error = publish_result.unwrap_err();
    if let Err(journal_error) = set_transaction_phase(
        project_root,
        &mut journal,
        ReleaseTransactionPhase::RollingBack,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "release publication failed ({publish_error}); unable to persist rollback phase ({journal_error})"
            ),
        ));
    }
    match rollback_release_transaction(project_root, &journal, &paths) {
        Ok(()) => Err(publish_error),
        Err(rollback_error) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "release publication failed ({publish_error}); rollback also failed ({rollback_error})"
            ),
        )),
    }
}

fn file_matches_descriptor(path: &Path, descriptor: &ReleaseFileDescriptor) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let (size, sha256) = sha256_file_hex(path)?;
            Ok(size == descriptor.size && sha256 == descriptor.sha256)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn archive_entry_descriptor(entry: &ArchivePlanEntry) -> ReleaseFileDescriptor {
    ReleaseFileDescriptor {
        role: entry.source_role.clone(),
        file_name: entry.source_file_name.clone(),
        size: entry.size,
        sha256: entry.sha256.clone(),
    }
}

fn archive_source_path(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
    entry: &ArchivePlanEntry,
) -> PathBuf {
    match entry.source_kind.as_str() {
        "current_backup" => {
            let descriptor = archive_entry_descriptor(entry);
            release_set_file_path(&paths.current_backup, &descriptor)
        }
        "stable_backup" => paths.stable_backup.clone(),
        "project_root"
            if entry
                .source_file_name
                .eq_ignore_ascii_case(&journal.root_new.file_name) =>
        {
            paths.root_same_name_backup.clone()
        }
        "project_root" => project_root.join(&entry.source_file_name),
        "legacy_apk"
            if entry
                .source_file_name
                .eq_ignore_ascii_case(&journal.legacy_new.file_name) =>
        {
            paths.legacy_same_name_backup.clone()
        }
        "legacy_apk" => project_root.join("APK").join(&entry.source_file_name),
        _ => project_root.join(".invalid-release-archive-source"),
    }
}

fn archive_destination_path(project_root: &Path, entry: &ArchivePlanEntry) -> PathBuf {
    let directory = match entry.destination_kind.as_str() {
        "old_apks" => archive_directory(project_root),
        "old_exes" => project_root.join("old_exes"),
        _ => project_root.join(".invalid-release-archive-destination"),
    };
    directory.join(&entry.destination_file_name)
}

fn finalize_archive_entry(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
    entry: &ArchivePlanEntry,
) -> io::Result<()> {
    let source = archive_source_path(project_root, journal, paths, entry);
    let destination = archive_destination_path(project_root, entry);
    let destination_parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "archive destination has no parent",
        )
    })?;
    ensure_real_directory(destination_parent, "release archive destination directory")?;
    let expected = archive_entry_descriptor(entry);
    if destination.exists() {
        verify_file_descriptor(&destination, &expected)?;
    } else {
        verify_file_descriptor(&source, &expected)?;
        copy_file_create_new(&source, &destination)?;
        verify_file_descriptor(&destination, &expected)?;
    }
    if source.exists() {
        verify_file_descriptor(&source, &expected)?;
        fs::remove_file(&source)?;
    }
    Ok(())
}

fn remove_empty_directory_tree_if_present(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "transaction cleanup target is not a real directory: {}",
                        path.display()
                    ),
                ));
            }
            fs::remove_dir_all(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_empty_real_directory_if_present(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "transaction cleanup target is not a real directory: {}",
                        path.display()
                    ),
                ));
            }
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let child = entry.path();
                let child_metadata = fs::symlink_metadata(&child)?;
                if !child_metadata.file_type().is_dir()
                    || metadata_is_reparse_point(&child_metadata)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::DirectoryNotEmpty,
                        format!(
                            "refusing to remove a transaction directory containing an unexpected entry: {}",
                            child.display()
                        ),
                    ));
                }
                remove_empty_real_directory_if_present(&child)?;
            }
            fs::remove_dir(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_real_file_if_present(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "transaction cleanup target is not a real file: {}",
                        path.display()
                    ),
                ));
            }
            fs::remove_file(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn finalize_committed_transaction(
    project_root: &Path,
    android_sdk: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    finalize_committed_transaction_with_verifier(project_root, journal, paths, |root_apk| {
        verify_release_apk(root_apk, android_sdk)
    })
}

fn finalize_committed_transaction_with_verifier(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
    mut verify_apk: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    verify_file_descriptor(&paths.stable, &journal.stable_new)?;
    verify_release_set_descriptor(&paths.current, &journal.new_current)?;
    let root_apk = project_root.join(&journal.root_new.file_name);
    verify_file_descriptor(&root_apk, &journal.root_new)?;
    verify_apk(&root_apk)?;
    let legacy_apk = project_root.join("APK").join(&journal.legacy_new.file_name);
    verify_file_descriptor(&legacy_apk, &journal.legacy_new)?;
    verify_apk(&legacy_apk)?;
    for entry in &journal.archive_plan {
        finalize_archive_entry(project_root, journal, paths, entry)?;
        #[cfg(test)]
        release_process_checkpoint(&format!("archived_{}", entry.source_role));
    }
    #[cfg(test)]
    release_process_checkpoint("archives_complete");
    if journal.windows_only {
        let apk = release_apk_descriptor(&journal.new_current)?;
        let backup_apk = paths.current_backup.join(&apk.file_name);
        if backup_apk.exists() {
            verify_file_descriptor(&backup_apk, apk)?;
            // The byte-identical APK remains in current and both public delivery locations.
            remove_real_file_if_present(&backup_apk)?;
        }
    }
    remove_empty_real_directory_if_present(&paths.current_backup)?;
    remove_empty_directory_tree_if_present(&paths.current_staging)?;
    require_path_absent(
        &paths.stable_backup,
        "committed stable backup after archive finalization",
    )?;
    remove_real_file_if_present(&paths.stable_staging)?;
    remove_empty_directory_tree_if_present(&paths.transaction_root)?;
    if !journal.windows_only {
        enforce_forbidden_delivery_package_policy(project_root, true)?;
    }
    remove_real_file_if_present(&paths.journal)?;
    #[cfg(test)]
    release_process_checkpoint("finalized");
    Ok(())
}

fn rollback_root_publication(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    rollback_published_apk(
        project_root.join(&journal.root_new.file_name),
        "project_root",
        &journal.root_new,
        &paths.root_same_name_backup,
        &paths.transaction_root.join("root-rollback-displaced.apk"),
        journal,
    )
}

fn rollback_legacy_apk_publication(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    rollback_published_apk(
        project_root.join("APK").join(&journal.legacy_new.file_name),
        "legacy_apk",
        &journal.legacy_new,
        &paths.legacy_same_name_backup,
        &paths.transaction_root.join("APK-rollback-displaced.apk"),
        journal,
    )
}

fn rollback_published_apk(
    destination: PathBuf,
    source_kind: &str,
    new_descriptor: &ReleaseFileDescriptor,
    same_name_backup: &Path,
    displaced: &Path,
    journal: &ReleaseTransactionJournal,
) -> io::Result<()> {
    let old_same_name = journal.archive_plan.iter().find(|entry| {
        entry.source_kind == source_kind
            && entry
                .source_file_name
                .eq_ignore_ascii_case(&new_descriptor.file_name)
    });
    if let Some(old_entry) = old_same_name {
        let old = archive_entry_descriptor(old_entry);
        if file_matches_descriptor(&destination, &old)? {
            remove_real_file_if_present(same_name_backup)?;
            return Ok(());
        }
        if !file_matches_descriptor(same_name_backup, &old)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cannot roll back same-name APK because the old backup is missing or invalid",
            ));
        }
        if !destination.exists() {
            fs::rename(same_name_backup, &destination)?;
        } else {
            verify_file_descriptor(&destination, new_descriptor)?;
            replace_existing_file_atomically(&destination, same_name_backup, displaced)?;
            remove_real_file_if_present(displaced)?;
        }
        return verify_file_descriptor(&destination, &old);
    }

    if file_matches_descriptor(&destination, new_descriptor)? {
        fs::remove_file(destination)?;
    } else if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected APK blocks transaction rollback",
        ));
    }
    Ok(())
}

fn rollback_current_publication(
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    let current = optional_release_set_descriptor(&paths.current)?;
    match &journal.old_current {
        Some(old) => {
            if current.as_ref() == Some(old) {
                return Ok(());
            }
            let backup = optional_release_set_descriptor(&paths.current_backup)?;
            if backup.as_ref() != Some(old) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "old current backup is missing or invalid during rollback",
                ));
            }
            match current {
                Some(actual) if actual == journal.new_current => {
                    if paths.current_staging.exists() {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "current staging path is occupied during rollback",
                        ));
                    }
                    fs::rename(&paths.current, &paths.current_staging)?;
                }
                None => {}
                Some(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected canonical current blocks rollback",
                    ));
                }
            }
            fs::rename(&paths.current_backup, &paths.current)?;
            verify_release_set_descriptor(&paths.current, old)
        }
        None => match current {
            Some(actual) if actual == journal.new_current => {
                if paths.current_staging.exists() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "current staging path is occupied during first-publication rollback",
                    ));
                }
                fs::rename(&paths.current, &paths.current_staging)?;
                Ok(())
            }
            None => Ok(()),
            Some(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected canonical current blocks first-publication rollback",
            )),
        },
    }
}

fn rollback_stable_publication(
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    match &journal.stable_old {
        Some(old) => {
            if file_matches_descriptor(&paths.stable, old)? {
                return Ok(());
            }
            if !file_matches_descriptor(&paths.stable_backup, old)? {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "old stable entry backup is missing or invalid during rollback",
                ));
            }
            if !paths.stable.exists() {
                fs::rename(&paths.stable_backup, &paths.stable)?;
            } else {
                verify_file_descriptor(&paths.stable, &journal.stable_new)?;
                if paths.stable_staging.exists() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "stable staging path is occupied during rollback",
                    ));
                }
                replace_existing_file_atomically(
                    &paths.stable,
                    &paths.stable_backup,
                    &paths.stable_staging,
                )?;
                remove_real_file_if_present(&paths.stable_staging)?;
            }
            verify_file_descriptor(&paths.stable, old)
        }
        None => {
            if file_matches_descriptor(&paths.stable, &journal.stable_new)? {
                fs::remove_file(&paths.stable)?;
            } else if paths.stable.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected stable entry blocks first-publication rollback",
                ));
            }
            Ok(())
        }
    }
}

fn rollback_release_transaction(
    project_root: &Path,
    journal: &ReleaseTransactionJournal,
    paths: &ReleaseTransactionPaths,
) -> io::Result<()> {
    if !journal.windows_only {
        rollback_legacy_apk_publication(project_root, journal, paths)?;
        rollback_root_publication(project_root, journal, paths)?;
    }
    rollback_current_publication(journal, paths)?;
    rollback_stable_publication(journal, paths)?;
    cleanup_transaction_staging(journal, paths)?;
    remove_real_file_if_present(&paths.journal)
}

fn recover_or_finalize_existing_transaction(
    project_root: &Path,
    android_sdk: &Path,
) -> io::Result<()> {
    let Some(mut journal) = load_transaction_journal(project_root)? else {
        return Ok(());
    };
    let paths = ReleaseTransactionPaths::derive(project_root, &journal)?;
    if journal.phase == ReleaseTransactionPhase::Committed {
        return finalize_committed_transaction(project_root, android_sdk, &journal, &paths);
    }
    if journal.phase != ReleaseTransactionPhase::RollingBack {
        set_transaction_phase(
            project_root,
            &mut journal,
            ReleaseTransactionPhase::RollingBack,
        )?;
    }
    rollback_release_transaction(project_root, &journal, &paths)
}

fn validate_source_release_identity(
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<()> {
    validate_release_identity(&version.version_name, PRODUCT.app_version)?;
    validate_release_identity(PRODUCT.app_version, SYNC_SERVER_BUILD_ID)
}

fn validate_release_identity(android_version: &str, windows_version: &str) -> io::Result<()> {
    validate_version_component(android_version)?;
    validate_version_component(windows_version)?;
    if android_version != windows_version {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "release identity mismatch: Android is {} but Server/Launcher source is {}",
                android_version, windows_version
            ),
        ));
    }
    Ok(())
}

fn validate_version_component(version: &str) -> io::Result<()> {
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release version is not a safe artifact-name component: {version:?}"),
        ));
    }
    Ok(())
}

fn build_windows_release_artifacts(
    project_root: &Path,
    cargo_home: &str,
    rustup_home: &str,
) -> io::Result<WindowsReleaseArtifacts> {
    let build_environment = resolve_windows_build_environment(rustup_home)?;
    let build_identity = capture_build_identity(project_root, &build_environment)?;
    let target_dir = execute_windows_release_commands(
        project_root,
        cargo_home,
        rustup_home,
        &[WindowsReleaseCommand::ReleaseServerBuild],
        Some(&build_identity),
        None,
        None,
    )?;
    let server_path = windows_release_binary_path(&target_dir, "timer_sync_server");
    require_regular_file(&server_path, "Windows sync server release artifact")?;
    run_sync_server_release_smoke(&server_path, &target_dir, &build_identity)?;
    let (size, sha256) = sha256_file_hex(&server_path)?;
    let server_binding = ExecutableIntegrityBinding { size, sha256 };
    let launcher_target_dir = execute_windows_release_commands(
        project_root,
        cargo_home,
        rustup_home,
        &[WindowsReleaseCommand::ReleaseLauncherBuild],
        Some(&build_identity),
        Some(&server_binding),
        None,
    )?;
    if launcher_target_dir != target_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows server and launcher stages resolved different target directories",
        ));
    }
    let launcher_path = windows_release_binary_path(&target_dir, "timer_sync_launcher");
    require_regular_file(&launcher_path, "Windows sync launcher release artifact")?;
    verify_binary_contains_integrity_binding(
        &launcher_path,
        &server_binding,
        "sync_server",
        "sync launcher",
    )?;
    let (size, sha256) = sha256_file_hex(&launcher_path)?;
    let launcher_binding = ExecutableIntegrityBinding { size, sha256 };
    let client_target_dir = execute_windows_release_commands(
        project_root,
        cargo_home,
        rustup_home,
        &[WindowsReleaseCommand::ReleaseClientBuild],
        Some(&build_identity),
        Some(&server_binding),
        Some(&launcher_binding),
    )?;
    if client_target_dir != target_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows release stages resolved different target directories",
        ));
    }
    let client_path = windows_release_binary_path(&target_dir, "timer_windows_client");
    require_regular_file(&client_path, "Windows client release artifact")?;
    verify_binary_contains_integrity_binding(
        &client_path,
        &launcher_binding,
        "sync_launcher",
        "Windows client",
    )?;
    verify_binary_contains_integrity_binding(
        &client_path,
        &server_binding,
        "sync_server",
        "Windows client direct service binding",
    )?;
    let source_snapshot_after_build = source_snapshot_sha256(project_root)?;
    if source_snapshot_after_build != build_identity.source_snapshot_sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source files changed during the three-stage Windows release build",
        ));
    }

    let current = RELEASE_BINARY_NAMES
        .iter()
        .map(|(binary_name, public_base_name)| {
            let source = windows_release_binary_path(&target_dir, binary_name);
            require_regular_file(&source, "Windows release artifact")?;
            Ok(WindowsReleaseArtifact {
                public_base_name,
                source,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let stable_desktop_entry =
        windows_release_binary_path(&target_dir, STABLE_DESKTOP_ENTRY_BINARY_NAME);
    require_regular_file(
        &stable_desktop_entry,
        "stable desktop entry release artifact",
    )?;
    Ok(WindowsReleaseArtifacts {
        current,
        stable_desktop_entry,
        build_identity,
    })
}

fn run_sync_server_release_smoke(
    server_path: &Path,
    target_dir: &Path,
    build_identity: &BuildIdentity,
) -> io::Result<()> {
    require_real_regular_file(server_path, "Windows sync server smoke-test artifact")?;
    require_real_directory(target_dir, "Windows release target directory")?;
    let port_probe = TcpListener::bind(("127.0.0.1", 0))?;
    let port = port_probe.local_addr()?.port();
    drop(port_probe);

    let smoke_dir = unique_unused_path(target_dir, ".sync-server-runtime-smoke")?;
    fs::create_dir(&smoke_dir)?;
    let smoke_result =
        run_sync_server_release_smoke_inner(server_path, &smoke_dir, port, build_identity);
    let cleanup_result = remove_owned_smoke_directory(target_dir, &smoke_dir);
    match (smoke_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(io::Error::new(
            error.kind(),
            format!("sync server runtime smoke test passed but cleanup failed: {error}"),
        )),
    }
}

fn run_sync_server_release_smoke_inner(
    server_path: &Path,
    smoke_dir: &Path,
    port: u16,
    build_identity: &BuildIdentity,
) -> io::Result<()> {
    let bind_address = format!("127.0.0.1:{port}");
    let store_path = smoke_dir.join("server_store.sqlite3");
    let backup_dir = smoke_dir.join("backups");
    let child = Command::new(server_path)
        .arg(&bind_address)
        .arg(&store_path)
        .env("GRID_TIMER_SYNC_BACKUP_DIR", &backup_dir)
        .env("GRID_TIMER_SYNC_BACKUP_INTERVAL_MILLIS", "604800000")
        .env_remove("GRID_TIMER_PUBLIC_SERVER_URL")
        .env_remove("GRID_TIMER_PUBLIC_SERVER_URL_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to start Windows sync server runtime smoke test: {error}"),
            )
        })?;
    let expected_process_id = child.id();
    let expected_process_name = server_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows sync server smoke-test executable name is not Unicode",
            )
        })?;
    let mut process = OwnedSmokeProcess { child };
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout_connect(Duration::from_secs(1))
        .timeout_read(Duration::from_secs(2))
        .build();
    let health_url = format!("http://{bind_address}/health");
    let deadline = Instant::now() + WINDOWS_SERVER_SMOKE_TIMEOUT;
    loop {
        if let Some(status) = process.child.try_wait()? {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Windows sync server exited before its runtime smoke test completed: {status}"
                ),
            ));
        }
        match agent
            .get(&health_url)
            .set("Accept", "application/json")
            .call()
        {
            Ok(response) if (200..=299).contains(&response.status()) => {
                let mut body = Vec::new();
                response
                    .into_reader()
                    .take(WINDOWS_SERVER_SMOKE_RESPONSE_MAX_BYTES + 1)
                    .read_to_end(&mut body)?;
                if body.len() as u64 <= WINDOWS_SERVER_SMOKE_RESPONSE_MAX_BYTES
                    && validate_sync_server_smoke_response(
                        &body,
                        expected_process_name,
                        expected_process_id,
                        build_identity,
                    )
                {
                    return Ok(());
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows sync server returned an unexpected runtime identity",
                ));
            }
            Ok(_) | Err(ureq::Error::Status(_, _)) | Err(ureq::Error::Transport(_)) => {}
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Windows sync server did not become healthy during the runtime smoke test",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn validate_sync_server_smoke_response(
    body: &[u8],
    expected_process_name: &str,
    expected_process_id: u32,
    build_identity: &BuildIdentity,
) -> bool {
    serde_json::from_slice::<SyncClientResult>(body)
        .ok()
        .is_some_and(|result| {
            result.ok
                && result.mode == "health"
                && result.product_id == PRODUCT.internal_id
                && result.service_role == "sync_server"
                && result.sync_protocol_version == PRODUCT.sync_protocol_version
                && result.server_build_id == SYNC_SERVER_BUILD_ID
                && result
                    .server_process_name
                    .eq_ignore_ascii_case(expected_process_name)
                && result.server_process_id == expected_process_id
                && result.server_git_commit == build_identity.git_commit
                && result.server_source_snapshot_sha256 == build_identity.source_snapshot_sha256
        })
}

fn remove_owned_smoke_directory(expected_parent: &Path, smoke_dir: &Path) -> io::Result<()> {
    let parent = fs::canonicalize(expected_parent)?;
    let directory = fs::canonicalize(smoke_dir)?;
    let safe_name = directory
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.starts_with(".sync-server-runtime-smoke-"));
    if directory.parent() != Some(parent.as_path()) || !safe_name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "refusing to remove unowned sync server smoke directory: {}",
                directory.display()
            ),
        ));
    }
    fs::remove_dir_all(directory)
}

fn run_windows_release_tests(
    project_root: &Path,
    cargo_home: &str,
    rustup_home: &str,
) -> io::Result<()> {
    execute_windows_release_commands(
        project_root,
        cargo_home,
        rustup_home,
        &WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE,
        None,
        None,
        None,
    )?;
    Ok(())
}

fn run_windows_format_only(args: &[String]) -> io::Result<()> {
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "format-windows takes no mutation options",
        ));
    }
    let rustup_home = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(default_rustup_home);
    let build_environment = resolve_windows_build_environment(&rustup_home.to_string_lossy())?;
    run_windows_format_check(
        &project_root(),
        &resolved_cargo_home(),
        &rustup_home.to_string_lossy(),
        &build_environment,
    )
}

fn windows_format_scope_error(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn collect_windows_format_sources(
    crate_root: &Path,
    metadata: &serde_json::Value,
) -> io::Result<Vec<PathBuf>> {
    require_real_directory(crate_root, "Windows crate root")?;
    let manifest_path = crate_root.join("Cargo.toml");
    require_real_regular_file(&manifest_path, "Windows crate manifest")?;
    let expected_manifest = fs::canonicalize(&manifest_path)?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| windows_format_scope_error("Cargo metadata has no packages"))?;
    let matching_packages = packages
        .iter()
        .filter(|package| {
            package["manifest_path"]
                .as_str()
                .and_then(|path| fs::canonicalize(path).ok())
                .is_some_and(|path| path == expected_manifest)
        })
        .collect::<Vec<_>>();
    if matching_packages.len() != 1 {
        return Err(windows_format_scope_error(
            "Cargo metadata does not uniquely identify the Windows crate",
        ));
    }
    let package = matching_packages[0];
    if package["name"].as_str() != Some("gridtimer_native")
        || package["edition"].as_str() != Some("2021")
    {
        return Err(windows_format_scope_error(
            "Windows crate identity or Rust edition is unsupported",
        ));
    }
    let targets = package["targets"]
        .as_array()
        .ok_or_else(|| windows_format_scope_error("Cargo metadata has no targets"))?;
    let required = std::iter::once(("gridtimer_native", PathBuf::from("src/lib.rs"), false)).chain(
        WINDOWS_FORMAT_BIN_NAMES
            .iter()
            .map(|name| (*name, PathBuf::from(format!("src/bin/{name}.rs")), true)),
    );
    for (name, relative, binary) in required {
        let expected = crate_root.join(&relative);
        require_real_regular_file(&expected, "required Windows format root")?;
        let target_matches = targets
            .iter()
            .filter(|target| {
                target["name"].as_str() == Some(name)
                    && target["kind"].as_array().is_some_and(|kinds| {
                        kinds.iter().any(|kind| {
                            if binary {
                                kind.as_str() == Some("bin")
                            } else {
                                matches!(kind.as_str(), Some("lib" | "rlib" | "cdylib"))
                            }
                        })
                    })
            })
            .collect::<Vec<_>>();
        if target_matches.len() != 1 {
            return Err(windows_format_scope_error(
                "Cargo metadata is missing or duplicates a required Windows target",
            ));
        }
        let target = target_matches[0];
        let declared = target["src_path"]
            .as_str()
            .ok_or_else(|| windows_format_scope_error("Windows target has no source path"))?;
        if target["edition"].as_str() != Some("2021")
            || fs::canonicalize(declared)? != fs::canonicalize(&expected)?
        {
            return Err(windows_format_scope_error(
                "Windows target edition or source root is outside the verified format scope",
            ));
        }
    }
    // The sole custom build root embeds the verified application ICO. It is
    // explicitly checked and formatted with the other Windows source roots.
    let build_targets = targets
        .iter()
        .filter(|target| {
            target["kind"].as_array().is_some_and(|kinds| {
                kinds
                    .iter()
                    .any(|kind| kind.as_str() == Some("custom-build"))
            })
        })
        .collect::<Vec<_>>();
    if build_targets.len() > 1 {
        return Err(windows_format_scope_error("Duplicate Windows build roots"));
    }
    let mut build_source = None;
    if let Some(target) = build_targets.first() {
        let expected = crate_root.join("build.rs");
        require_real_regular_file(&expected, "Windows icon build script")?;
        let declared = target["src_path"]
            .as_str()
            .ok_or_else(|| windows_format_scope_error("Windows build root has no source path"))?;
        if target["edition"].as_str() != Some("2021")
            || fs::canonicalize(declared)? != fs::canonicalize(&expected)?
        {
            return Err(windows_format_scope_error(
                "Windows build root escaped the explicit scope",
            ));
        }
        build_source = Some(PathBuf::from("build.rs"));
    }

    let source_root = crate_root.join("src");
    require_real_directory(&source_root, "Windows source root")?;
    let mut directories = vec![source_root];
    let mut sources = build_source.into_iter().collect::<Vec<_>>();
    while let Some(directory) = directories.pop() {
        require_real_directory(&directory, "Windows format source directory")?;
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(crate_root)
                .map_err(|_| windows_format_scope_error("Windows format path escaped its crate"))?;
            if relative == Path::new("src/sourcegen") {
                continue;
            }
            // Android build/source generators are separate Cargo binary roots.
            // Names such as desktop/android_parity.rs remain part of Windows.
            if relative.parent() == Some(Path::new("src/bin"))
                && !WINDOWS_FORMAT_BIN_NAMES.iter().any(|name| {
                    relative == Path::new("src/bin").join(format!("{name}.rs")).as_path()
                })
            {
                continue;
            }
            let info = fs::symlink_metadata(&path)?;
            if info.file_type().is_symlink() || metadata_is_reparse_point(&info) {
                return Err(windows_format_scope_error(
                    "Windows format sources cannot follow symlinks or reparse points",
                ));
            }
            if info.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                if !info.is_file() {
                    return Err(windows_format_scope_error(
                        "Windows Rust source is not a regular file",
                    ));
                }
                sources.push(relative.to_path_buf());
            }
        }
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn windows_format_batches(sources: &[PathBuf]) -> io::Result<Vec<Vec<PathBuf>>> {
    // Stay well below CreateProcess's 32767 UTF-16 limit, including executable,
    // fixed arguments and conservative quoting overhead for every source path.
    const MAX_SOURCE_ARGUMENT_UNITS: usize = 16_000;
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_units = 0;
    for source in sources {
        let units = source.as_os_str().to_string_lossy().encode_utf16().count() * 2 + 3;
        if units > MAX_SOURCE_ARGUMENT_UNITS {
            return Err(windows_format_scope_error(
                "Windows Rust source path exceeds the bounded formatter command size",
            ));
        }
        if current_units + units > MAX_SOURCE_ARGUMENT_UNITS {
            batches.push(std::mem::take(&mut current));
            current_units = 0;
        }
        current.push(source.clone());
        current_units += units;
    }
    if !current.is_empty() {
        batches.push(current);
    }
    Ok(batches)
}

fn run_windows_format_check(
    project_root: &Path,
    cargo_home: &str,
    rustup_home: &str,
    build_environment: &WindowsBuildEnvironment,
) -> io::Result<()> {
    let crate_root = project_root.join("native/gridtimer_native");
    let rustfmt = build_environment
        .rustc
        .parent()
        .ok_or_else(|| windows_format_scope_error("Rust compiler has no toolchain directory"))?
        .join("rustfmt.exe");
    require_real_regular_file(&rustfmt, "fixed-toolchain Rust formatter")?;
    let metadata = Command::new(&build_environment.cargo)
        .current_dir(&crate_root)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
            "--locked",
        ])
        .env("CARGO_HOME", cargo_home)
        .env("RUSTUP_HOME", rustup_home)
        .env("RUSTUP_TOOLCHAIN", WINDOWS_MSVC_TOOLCHAIN)
        .output()?;
    if !metadata.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Windows format metadata failed: {}",
                String::from_utf8_lossy(&metadata.stderr)
            ),
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
    let sources = collect_windows_format_sources(&crate_root, &metadata)?;
    let batches = windows_format_batches(&sources)?;
    println!("Windows formatting gate: {} shared/Windows Rust sources in {} bounded batches; Android source generators excluded.", sources.len(), batches.len());
    for batch in batches {
        let mut command = Command::new(&rustfmt);
        command
            .current_dir(&crate_root)
            .args(windows_release_command_args(
                WindowsReleaseCommand::FormatCheck,
            ))
            .args(batch);
        spawn_and_check(command, "Windows and shared Rust formatting check")?;
    }
    Ok(())
}

fn execute_windows_release_commands(
    project_root: &Path,
    cargo_home: &str,
    rustup_home: &str,
    commands: &[WindowsReleaseCommand],
    build_identity: Option<&BuildIdentity>,
    server_binding: Option<&ExecutableIntegrityBinding>,
    launcher_binding: Option<&ExecutableIntegrityBinding>,
) -> io::Result<PathBuf> {
    let target_dir = windows_target_directory(
        project_root,
        build_identity.is_none(),
        env::var("GRIDTIMER_WINDOWS_TEST_TARGET_DIR")
            .ok()
            .as_deref(),
        env::var("GRIDTIMER_WINDOWS_RELEASE_TARGET_DIR")
            .ok()
            .as_deref(),
        env::var("CARGO_TARGET_DIR").ok().as_deref(),
    );
    ensure_directory(&target_dir)?;
    let build_environment = resolve_windows_build_environment(rustup_home)?;
    let library_path = env::join_paths(&build_environment.library_paths).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid MSVC LIB path: {error}"),
        )
    })?;

    run_windows_release_command_sequence(commands, |kind| {
        if kind == WindowsReleaseCommand::FormatCheck {
            return run_windows_format_check(
                project_root,
                cargo_home,
                rustup_home,
                &build_environment,
            );
        }
        let mut command = Command::new(&build_environment.cargo);
        command.current_dir(project_root.join("native/gridtimer_native"));
        command.args(windows_release_command_args(kind));
        command.env("CARGO_HOME", cargo_home);
        command.env("RUSTUP_HOME", rustup_home);
        command.env("RUSTUP_TOOLCHAIN", WINDOWS_MSVC_TOOLCHAIN);
        command.env("CARGO_TARGET_DIR", &target_dir);
        command.env(
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
            &build_environment.linker,
        );
        command.env("LIB", &library_path);
        if matches!(
            kind,
            WindowsReleaseCommand::ReleaseServerBuild
                | WindowsReleaseCommand::ReleaseLauncherBuild
                | WindowsReleaseCommand::ReleaseClientBuild
        ) {
            let identity = build_identity.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Windows release build requires an exact source identity",
                )
            })?;
            command.env("GRIDTIMER_GIT_COMMIT", &identity.git_commit);
            command.env(
                "GRIDTIMER_SOURCE_SNAPSHOT_SHA256",
                &identity.source_snapshot_sha256,
            );
        }
        if matches!(
            kind,
            WindowsReleaseCommand::ReleaseLauncherBuild | WindowsReleaseCommand::ReleaseClientBuild
        ) {
            let binding = server_binding.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Windows launcher release build requires sync-server integrity binding",
                )
            })?;
            command.env(
                "GRIDTIMER_EXPECTED_SYNC_SERVER_SIZE",
                binding.size.to_string(),
            );
            command.env("GRIDTIMER_EXPECTED_SYNC_SERVER_SHA256", &binding.sha256);
            command.env(
                "GRIDTIMER_EXPECTED_SYNC_SERVER_BINDING_V1",
                binding.marker("sync_server"),
            );
        }
        if kind == WindowsReleaseCommand::ReleaseClientBuild {
            let binding = launcher_binding.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Windows client release build requires launcher integrity binding",
                )
            })?;
            command.env(
                "GRIDTIMER_EXPECTED_SYNC_LAUNCHER_SIZE",
                binding.size.to_string(),
            );
            command.env("GRIDTIMER_EXPECTED_SYNC_LAUNCHER_SHA256", &binding.sha256);
            command.env(
                "GRIDTIMER_EXPECTED_SYNC_LAUNCHER_BINDING_V1",
                binding.marker("sync_launcher"),
            );
        }
        spawn_and_check(command, windows_release_command_label(kind))
    })?;
    Ok(target_dir)
}

fn run_windows_release_command_sequence(
    commands: &[WindowsReleaseCommand],
    mut run_command: impl FnMut(WindowsReleaseCommand) -> io::Result<()>,
) -> io::Result<()> {
    for &kind in commands {
        run_command(kind)?;
    }
    Ok(())
}

fn windows_release_binary_path(target_dir: &Path, binary_name: &str) -> PathBuf {
    target_dir
        .join(WINDOWS_MSVC_TARGET)
        .join("release")
        .join(format!("{binary_name}.exe"))
}

fn windows_release_command_args(kind: WindowsReleaseCommand) -> Vec<&'static str> {
    let mut args = match kind {
        WindowsReleaseCommand::FormatCheck => vec![
            "--check",
            "--edition",
            "2021",
            "--config",
            "skip_children=true",
        ],
        WindowsReleaseCommand::LibraryTests => vec![
            "test",
            "--locked",
            "--offline",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
            "--lib",
        ],
        WindowsReleaseCommand::DesktopMediaTests => vec![
            "test",
            "--locked",
            "--offline",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
            "--lib",
            "desktop_note_media",
        ],
        WindowsReleaseCommand::WindowsClientTests => vec![
            "test",
            "--locked",
            "--offline",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
            "--bin",
            "timer_windows_client",
        ],
        WindowsReleaseCommand::SyncLauncherTests => vec![
            "test",
            "--locked",
            "--offline",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
            "--bin",
            "timer_sync_launcher",
        ],
        WindowsReleaseCommand::PackagerTests => vec![
            "test",
            "--locked",
            "--offline",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
            "--bin",
            "gridtimer_packager",
        ],
        WindowsReleaseCommand::StableDesktopEntryTests => vec![
            "test",
            "--locked",
            "--offline",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
            "--bin",
            STABLE_DESKTOP_ENTRY_BINARY_NAME,
        ],
        WindowsReleaseCommand::ReleaseServerBuild
        | WindowsReleaseCommand::ReleaseLauncherBuild
        | WindowsReleaseCommand::ReleaseClientBuild => vec![
            "build",
            "--locked",
            "--offline",
            "--release",
            "--features",
            "desktop",
            "--target",
            WINDOWS_MSVC_TARGET,
        ],
    };
    if kind == WindowsReleaseCommand::ReleaseServerBuild {
        args.extend(["--bin", "timer_sync_server"]);
        args.extend(["--bin", STABLE_DESKTOP_ENTRY_BINARY_NAME]);
    } else if kind == WindowsReleaseCommand::ReleaseLauncherBuild {
        args.extend(["--bin", "timer_sync_launcher"]);
    } else if kind == WindowsReleaseCommand::ReleaseClientBuild {
        args.extend(["--bin", "timer_windows_client"]);
    }
    if args.first() == Some(&"test") {
        args.splice(
            1..1,
            ["--config", "profile.test.package.gridtimer_native.debug=0"],
        );
    }
    args
}

fn windows_release_command_label(kind: WindowsReleaseCommand) -> &'static str {
    match kind {
        WindowsReleaseCommand::FormatCheck => "Rust formatting check",
        WindowsReleaseCommand::LibraryTests => "Windows core library and desktop integration tests",
        WindowsReleaseCommand::DesktopMediaTests => "Windows desktop media and media sync tests",
        WindowsReleaseCommand::WindowsClientTests => "Windows client tests",
        WindowsReleaseCommand::SyncLauncherTests => "Windows sync launcher tests",
        WindowsReleaseCommand::PackagerTests => "formal delivery packager tests",
        WindowsReleaseCommand::StableDesktopEntryTests => "stable desktop entry launcher tests",
        WindowsReleaseCommand::ReleaseServerBuild => "Windows sync server release build",
        WindowsReleaseCommand::ReleaseLauncherBuild => {
            "server-bound Windows sync launcher release build"
        }
        WindowsReleaseCommand::ReleaseClientBuild => "launcher-bound Windows client release build",
    }
}

fn resolve_windows_build_environment(rustup_home: &str) -> io::Result<WindowsBuildEnvironment> {
    let toolchain_dir = PathBuf::from(rustup_home)
        .join("toolchains")
        .join(WINDOWS_MSVC_TOOLCHAIN);
    let cargo = toolchain_dir.join(r"bin\cargo.exe");
    require_regular_file(&cargo, "MSVC Cargo executable")?;
    let rustc = toolchain_dir.join(r"bin\rustc.exe");
    require_regular_file(&rustc, "MSVC Rust compiler")?;
    ensure_minimum_windows_rustc_version(&rustc)?;

    let linker = env::var_os("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            toolchain_dir.join(r"lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe")
        });
    require_regular_file(&linker, "MSVC linker")?;

    let library_paths = env::var_os("LIB")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from(r"C:\tools\xwin\crt\lib\x86_64"),
                PathBuf::from(r"C:\tools\xwin\sdk\lib\ucrt\x86_64"),
                PathBuf::from(r"C:\tools\xwin\sdk\lib\um\x86_64"),
            ]
        });
    if library_paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "MSVC LIB path is empty",
        ));
    }
    for path in &library_paths {
        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("MSVC library directory missing: {}", path.display()),
            ));
        }
    }

    Ok(WindowsBuildEnvironment {
        cargo,
        rustc,
        linker,
        library_paths,
    })
}

fn ensure_minimum_windows_rustc_version(rustc: &Path) -> io::Result<()> {
    let output = Command::new(rustc).arg("--version").output()?;
    if !output.status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("failed to query Rust compiler version: {}", output.status),
        ));
    }
    let raw = String::from_utf8(output.stdout)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "rustc version is not UTF-8"))?;
    let version = parse_rustc_version(&raw).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("could not parse Rust compiler version: {raw:?}"),
        )
    })?;
    if version < MINIMUM_WINDOWS_RUSTC_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "Windows release requires rustc {}.{}.{} or newer; found {}.{}.{}",
                MINIMUM_WINDOWS_RUSTC_VERSION.0,
                MINIMUM_WINDOWS_RUSTC_VERSION.1,
                MINIMUM_WINDOWS_RUSTC_VERSION.2,
                version.0,
                version.1,
                version.2
            ),
        ));
    }
    Ok(())
}

fn parse_rustc_version(raw: &str) -> Option<(u32, u32, u32)> {
    let mut words = raw.split_whitespace();
    if words.next()? != "rustc" {
        return None;
    }
    let version = words.next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn require_regular_file(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{label} missing at {}: {error}", path.display()),
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} is not a non-empty regular file: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn publish_current_release_artifacts(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    root_apk: &Path,
    windows_artifacts: &WindowsReleaseArtifacts,
) -> io::Result<()> {
    validate_windows_release_artifact_sources(windows_artifacts)?;
    let current_dir = project_root.join("release_artifacts/current");
    let release_directory = current_dir.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "current release directory has no parent: {}",
                current_dir.display()
            ),
        )
    })?;
    ensure_directory(release_directory)?;
    let staging_dir = create_unique_directory(release_directory, ".current-staging")?;

    let publication_result = (|| {
        let staged_apk = staging_dir.join(versioned_artifact_name(
            "tenfold",
            &version.version_name,
            "apk",
        ));
        copy_non_empty_file(root_apk, &staged_apk, "release APK")?;
        for artifact in &windows_artifacts.current {
            let destination = staging_dir.join(versioned_artifact_name(
                artifact.public_base_name,
                &version.version_name,
                "exe",
            ));
            copy_non_empty_file(&artifact.source, &destination, "Windows release artifact")?;
        }
        let tools_dir = staging_dir.join("tools");
        ensure_directory(&tools_dir)?;
        copy_non_empty_file(
            &project_root.join(r"tools\cloudflared.exe"),
            &tools_dir.join("cloudflared.exe"),
            "cloudflared runtime dependency",
        )?;
        let payload = derive_release_set_descriptor(&staging_dir)?;
        write_release_manifest(&staging_dir, &payload, &windows_artifacts.build_identity)?;
        validate_current_release_directory(&staging_dir, version)?;
        publish_stable_entry_before_current(
            || publish_stable_desktop_entry(project_root, &windows_artifacts.stable_desktop_entry),
            || commit_staged_current_release(project_root, &current_dir, &staging_dir),
        )
    })();

    if staging_dir.exists() {
        if let Err(error) = fs::remove_dir_all(&staging_dir) {
            if publication_result.is_ok() {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "release published but staging cleanup failed at {}: {error}",
                        staging_dir.display()
                    ),
                ));
            }
        }
    }
    publication_result
}

fn validate_windows_release_artifact_sources(
    windows_artifacts: &WindowsReleaseArtifacts,
) -> io::Result<()> {
    let expected = RELEASE_BINARY_NAMES
        .iter()
        .map(|(_, public_name)| *public_name)
        .collect::<BTreeSet<_>>();
    let actual = windows_artifacts
        .current
        .iter()
        .map(|artifact| artifact.public_base_name)
        .collect::<BTreeSet<_>>();
    if windows_artifacts.current.len() != RELEASE_BINARY_NAMES.len() || actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "incomplete or duplicate Windows release artifact set: expected {expected:?}, found {actual:?}"
            ),
        ));
    }
    for artifact in &windows_artifacts.current {
        require_regular_file(&artifact.source, "Windows release artifact")?;
    }
    require_regular_file(
        &windows_artifacts.stable_desktop_entry,
        "stable desktop entry release artifact",
    )?;
    Ok(())
}

fn publish_stable_entry_before_current(
    publish_stable_entry: impl FnOnce() -> io::Result<()>,
    publish_current: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    publish_stable_entry()?;
    publish_current()
}

fn stable_desktop_entry_path(project_root: &Path) -> PathBuf {
    project_root
        .join("release_artifacts/desktop_entry")
        .join(STABLE_DESKTOP_ENTRY_FILE_NAME)
}

fn publish_stable_desktop_entry(project_root: &Path, source: &Path) -> io::Result<()> {
    require_regular_file(source, "stable desktop entry release artifact")?;
    let destination = stable_desktop_entry_path(project_root);
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "stable desktop entry has no parent directory: {}",
                destination.display()
            ),
        )
    })?;
    ensure_directory(parent)?;
    let parent_metadata = fs::symlink_metadata(parent)?;
    if !parent_metadata.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "stable desktop entry directory is not a real directory: {}",
                parent.display()
            ),
        ));
    }

    let staged = create_unique_staged_copy(source, parent, ".desktop-entry-staging")?;
    let result = commit_staged_file(&staged, &destination, "stable desktop entry");
    if staged.exists() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn create_unique_staged_copy(source: &Path, parent: &Path, label: &str) -> io::Result<PathBuf> {
    for suffix in 0_u32..1_000 {
        let candidate = parent.join(format!(
            "{label}-{}-{}-{suffix}.exe",
            std::process::id(),
            current_time_millis()
        ));
        match copy_file_create_new(source, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "unable to stage stable desktop entry under {}",
            parent.display()
        ),
    ))
}

fn commit_staged_file(staged: &Path, destination: &Path, label: &str) -> io::Result<()> {
    commit_staged_file_with_replace(staged, destination, label, replace_existing_file_atomically)
}

fn commit_staged_file_with_replace(
    staged: &Path,
    destination: &Path,
    label: &str,
    replace_existing: impl FnOnce(&Path, &Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    require_real_regular_file(staged, label)?;
    let destination_metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if destination_metadata.as_ref().is_some_and(|metadata| {
        !metadata.file_type().is_file() || metadata_is_reparse_point(metadata)
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{label} destination is not a real file: {}",
                destination.display()
            ),
        ));
    }

    if destination_metadata.is_none() {
        return fs::rename(staged, destination);
    }

    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} has no parent"),
        )
    })?;
    let backup = unique_unused_path(parent, ".desktop-entry-backup")?;
    if let Err(publish_error) = replace_existing(destination, staged, &backup) {
        return match restore_old_file_after_failed_replace(destination, &backup, label) {
            Ok(()) => Err(publish_error),
            Err(rollback_error) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "{label} switch failed ({publish_error}); rollback also failed ({rollback_error})"
                ),
            )),
        };
    }
    fs::remove_file(&backup).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{label} updated, but backup cleanup failed at {}: {error}",
                backup.display()
            ),
        )
    })?;
    Ok(())
}

fn restore_old_file_after_failed_replace(
    destination: &Path,
    backup: &Path,
    label: &str,
) -> io::Result<()> {
    let backup_metadata = match fs::symlink_metadata(backup) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(metadata) = &backup_metadata {
        if !metadata.file_type().is_file() || metadata_is_reparse_point(metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{label} rollback backup is not a real file: {}",
                    backup.display()
                ),
            ));
        }
    }

    let destination_metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(metadata) = &destination_metadata {
        if !metadata.file_type().is_file() || metadata_is_reparse_point(metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{label} rollback destination is not a real file: {}",
                    destination.display()
                ),
            ));
        }
    }

    match (backup_metadata.is_some(), destination_metadata.is_some()) {
        (false, true) => Ok(()),
        (false, false) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{label} replacement failure left neither destination nor backup"),
        )),
        (true, false) => fs::rename(backup, destination),
        (true, true) => {
            let parent = destination.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{label} rollback destination has no parent"),
                )
            })?;
            let displaced = unique_unused_path(parent, ".desktop-entry-rollback-displaced")?;
            replace_existing_file_atomically(destination, backup, &displaced)?;
            fs::remove_file(&displaced).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "{label} rollback restored the old entry, but could not remove {}: {error}",
                        displaced.display()
                    ),
                )
            })
        }
    }
}

#[cfg(windows)]
fn replace_existing_file_atomically(
    destination: &Path,
    replacement: &Path,
    backup: &Path,
) -> io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "ReplaceFileW"]
        fn replace_file_w(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
    }

    fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Windows path contains a NUL character: {}", path.display()),
            ));
        }
        wide.push(0);
        Ok(wide)
    }

    let destination = wide_path(destination)?;
    let replacement = wide_path(replacement)?;
    let backup = wide_path(backup)?;
    let replaced = unsafe {
        replace_file_w(
            destination.as_ptr(),
            replacement.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_existing_file_atomically(
    destination: &Path,
    replacement: &Path,
    backup: &Path,
) -> io::Result<()> {
    copy_file_create_new(destination, backup)?;
    if let Err(error) = fs::rename(replacement, destination) {
        let _ = fs::remove_file(backup);
        return Err(error);
    }
    Ok(())
}

fn copy_non_empty_file(source: &Path, destination: &Path, label: &str) -> io::Result<()> {
    require_regular_file(source, label)?;
    let expected_length = fs::metadata(source)?.len();
    let copied_length = fs::copy(source, destination)?;
    if copied_length != expected_length {
        let _ = fs::remove_file(destination);
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "short copy for {label}: copied {copied_length} of {expected_length} bytes from {}",
                source.display()
            ),
        ));
    }
    Ok(())
}

fn create_unique_directory(parent: &Path, label: &str) -> io::Result<PathBuf> {
    for suffix in 0_u32..1_000 {
        let candidate = parent.join(format!(
            "{label}-{}-{}-{suffix}",
            std::process::id(),
            current_time_millis()
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "unable to allocate staging directory under {}",
            parent.display()
        ),
    ))
}

fn unique_unused_path(parent: &Path, label: &str) -> io::Result<PathBuf> {
    for suffix in 0_u32..1_000 {
        let candidate = parent.join(format!(
            "{label}-{}-{}-{suffix}",
            std::process::id(),
            current_time_millis()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("unable to allocate backup path under {}", parent.display()),
    ))
}

fn commit_staged_current_release(
    project_root: &Path,
    current_dir: &Path,
    staging_dir: &Path,
) -> io::Result<()> {
    let current_metadata = match fs::symlink_metadata(current_dir) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if current_metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.file_type().is_dir())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "current release path is not a real directory: {}",
                current_dir.display()
            ),
        ));
    }

    let archived_copies = if current_metadata.is_some() {
        prepare_current_release_archives(project_root, current_dir)?
    } else {
        Vec::new()
    };
    let backup_dir = if current_metadata.is_some() {
        let Some(parent) = current_dir.parent() else {
            cleanup_archive_copies(&archived_copies);
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "current release path has no parent",
            ));
        };
        match unique_unused_path(parent, ".current-backup") {
            Ok(path) => Some(path),
            Err(error) => {
                cleanup_archive_copies(&archived_copies);
                return Err(error);
            }
        }
    } else {
        None
    };

    if let Some(backup_dir) = &backup_dir {
        if let Err(error) = fs::rename(current_dir, backup_dir) {
            cleanup_archive_copies(&archived_copies);
            return Err(error);
        }
    }
    if let Err(publish_error) = fs::rename(staging_dir, current_dir) {
        let rollback_result = backup_dir
            .as_ref()
            .map(|backup_dir| fs::rename(backup_dir, current_dir))
            .transpose();
        cleanup_archive_copies(&archived_copies);
        return match rollback_result {
            Ok(_) => Err(publish_error),
            Err(rollback_error) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "current release switch failed ({publish_error}); rollback also failed ({rollback_error})"
                ),
            )),
        };
    }

    if let Some(backup_dir) = backup_dir {
        if let Err(error) = fs::remove_dir_all(&backup_dir) {
            eprintln!(
                "warning: current release was published, but backup cleanup failed at {}: {error}",
                backup_dir.display()
            );
        }
    }
    Ok(())
}

fn prepare_current_release_archives(
    project_root: &Path,
    current_dir: &Path,
) -> io::Result<Vec<PathBuf>> {
    let descriptor = derive_release_set_descriptor(current_dir)?;
    let mut sources = descriptor
        .files
        .iter()
        .map(|file| release_set_file_path(current_dir, file))
        .collect::<Vec<_>>();
    sources.sort();

    let mut created = Vec::new();
    for source in sources {
        if is_forbidden_delivery_package(&source) {
            cleanup_archive_copies(&created);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "forbidden package cannot be archived from current release: {}",
                    source.display()
                ),
            ));
        }
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let archive_dir = match extension.as_str() {
            "apk" => archive_directory(project_root),
            "exe" | "json" => project_root.join("old_exes"),
            _ => {
                cleanup_archive_copies(&created);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported current release artifact: {}", source.display()),
                ));
            }
        };
        if let Err(error) = ensure_directory(&archive_dir) {
            cleanup_archive_copies(&created);
            return Err(error);
        }
        match copy_file_to_archive(&source, &archive_dir) {
            Ok(destination) => created.push(destination),
            Err(error) => {
                cleanup_archive_copies(&created);
                return Err(error);
            }
        }
    }
    Ok(created)
}

fn copy_file_to_archive(source: &Path, archive_dir: &Path) -> io::Result<PathBuf> {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let timestamp = current_time_millis();

    for suffix in 0_u32..1_000 {
        let collision_suffix = if suffix == 0 {
            String::new()
        } else {
            format!("_{suffix}")
        };
        let destination =
            archive_dir.join(format!("{stem}_{timestamp}{collision_suffix}{extension}"));
        match copy_file_create_new(source, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("unable to allocate archive name for {}", source.display()),
    ))
}

fn copy_file_create_new(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(source)?;
    let expected_length = input.metadata()?.len();
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let result = (|| {
        let copied_length = io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        if copied_length != expected_length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "short archive copy: copied {copied_length} of {expected_length} bytes from {}",
                    source.display()
                ),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        drop(output);
        let _ = fs::remove_file(destination);
    }
    result
}

fn cleanup_archive_copies(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn validate_current_release_artifacts(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<()> {
    let current_dir = project_root.join("release_artifacts/current");
    validate_current_release_directory(&current_dir, version)?;
    validate_windows_build_verification(project_root, &current_dir, version)
}

fn validate_windows_build_verification(
    project_root: &Path,
    current_dir: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<()> {
    let report_path =
        project_root.join("release_artifacts/verification/windows_build_verification.json");
    require_real_regular_file(&report_path, "Windows build verification report")?;
    let metadata = fs::symlink_metadata(&report_path)?;
    if metadata.len() == 0 || metadata.len() > MAX_WINDOWS_BUILD_VERIFICATION_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows build verification report size is invalid",
        ));
    }
    let raw = fs::read(&report_path)?;
    if raw.is_empty() || raw.len() as u64 > MAX_WINDOWS_BUILD_VERIFICATION_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows build verification report size changed while it was being read",
        ));
    }
    let report = serde_json::from_slice::<WindowsBuildVerification>(&raw).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Windows build verification report JSON is invalid: {error}"),
        )
    })?;
    let descriptor = derive_release_set_descriptor(current_dir)?;
    let manifest = read_release_manifest_file(
        &current_dir.join(product_identity::RELEASE_MANIFEST_FILE_NAME),
    )?;
    let expected_checks = release_passed_checks(manifest.windows_only);
    let artifacts_match = windows_verification_artifacts_match(&report, &manifest, &descriptor)?;
    if !matches!(
        report.schema_version,
        LEGACY_WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION
            | WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION
    ) || !report.passed
        || report.verified_at_epoch_millis <= 0
        || report.product_id != PRODUCT.internal_id
        || report.app_version != version.version_name
        || report.windows_only != manifest.windows_only
        || report.sync_protocol_version != PRODUCT.sync_protocol_version
        || report.git_commit != manifest.git_commit
        || report.source_worktree_dirty != manifest.source_worktree_dirty
        || report.source_snapshot_sha256 != manifest.source_snapshot_sha256
        || report.toolchain != manifest.toolchain
        || !artifacts_match
        || report.passed_checks != expected_checks
        || !report.publication_performed
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows build verification report does not match the current release",
        ));
    }
    Ok(())
}

fn windows_manifest_identity_sha256(manifest: &ReleaseManifest) -> io::Result<String> {
    // Preserve every Windows field, including creation time and all tool bindings.
    // Only the independently published APK and its provenance can change.
    let mut windows_manifest = manifest.clone();
    windows_manifest.android_release = None;
    windows_manifest.files.retain(|file| file.role != "apk");
    let canonical = serde_json::to_vec(&windows_manifest)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn windows_verification_artifacts_match(
    report: &WindowsBuildVerification,
    manifest: &ReleaseManifest,
    current: &ReleaseSetDescriptor,
) -> io::Result<bool> {
    // Schema 2 never recorded a Windows-only manifest identity. Its historical
    // proof remains valid only for the exact full release it originally checked.
    if report.schema_version == LEGACY_WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION
        || (report.schema_version == WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION
            && !report.windows_only)
    {
        return Ok(report.windows_manifest_sha256.is_none() && report.artifacts == current.files);
    }
    if report.schema_version != WINDOWS_BUILD_VERIFICATION_SCHEMA_VERSION
        || !report.windows_only
        || !manifest.windows_only
    {
        return Ok(false);
    }
    let recorded = ReleaseSetDescriptor {
        version: report.app_version.clone(),
        files: report.artifacts.clone(),
    };
    validate_release_set_descriptor(&recorded)?;
    if recorded.files.len() != 6
        || report.windows_manifest_sha256.as_deref()
            != Some(windows_manifest_identity_sha256(manifest)?.as_str())
    {
        return Ok(false);
    }
    if release_apk_descriptor(&recorded)? != release_apk_descriptor(current)?
        && manifest.android_release.is_none()
    {
        return Ok(false);
    }
    // The caller derives and validates the entire current set first. This does
    // not bypass APK bytes, manifest consistency, or independent provenance.
    // The Windows-only finish path also verifies its signature and replicas.
    let windows_files =
        |file: &&ReleaseFileDescriptor| file.role != "apk" && file.role != "release_manifest";
    Ok(recorded
        .files
        .iter()
        .filter(windows_files)
        .eq(current.files.iter().filter(windows_files)))
}

fn validate_current_release_directory(
    current_dir: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<()> {
    validate_current_release_directory_for_platforms(
        current_dir,
        version,
        &version.version_name,
        false,
    )
}

fn validate_current_release_directory_for_platforms(
    current_dir: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
    android_version: &str,
    windows_only: bool,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(current_dir).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "current release directory missing at {}: {error}",
                current_dir.display()
            ),
        )
    })?;
    if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "current release path is not a real directory: {}",
                current_dir.display()
            ),
        ));
    }

    let mut actual_files = BTreeSet::new();
    let mut tools_found = false;
    for entry in fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("non-Unicode artifact name in {}", current_dir.display()),
            )
        })?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() && name == "tools" {
            validate_current_tools_directory(&path)?;
            tools_found = true;
            continue;
        }
        if !file_type.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected entry in current release directory: {}",
                    path.display()
                ),
            ));
        }
        require_regular_file(&path, "current release artifact")?;
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".aab")
            || lower.starts_with("app-release")
            || lower.starts_with("grid_timer_app_debug")
            || lower.starts_with("grid_timer_app_xiaomi_debug")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("forbidden current release artifact: {}", path.display()),
            ));
        }
        actual_files.insert(name);
    }

    if !tools_found {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "current release runtime dependency missing: tools/cloudflared.exe",
        ));
    }
    let expected_files =
        expected_current_release_names_for_platforms(&version.version_name, android_version);
    if actual_files != expected_files {
        let missing = expected_files
            .difference(&actual_files)
            .cloned()
            .collect::<Vec<_>>();
        let unexpected = actual_files
            .difference(&expected_files)
            .cloned()
            .collect::<Vec<_>>();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "current release set mismatch for version {}: missing {missing:?}, unexpected {unexpected:?}",
                version.version_name
            ),
        ));
    }
    let descriptor = derive_release_set_descriptor(current_dir)?;
    if descriptor.version != version.version_name || descriptor.files.len() != 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "current release manifest or payload identity is incomplete",
        ));
    }
    let manifest = read_release_manifest_file(
        &current_dir.join(product_identity::RELEASE_MANIFEST_FILE_NAME),
    )?;
    if manifest.windows_only != windows_only {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release publication scope does not match the requested platform",
        ));
    }
    Ok(())
}

fn validate_current_tools_directory(tools_dir: &Path) -> io::Result<()> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(tools_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected entry in current tools directory: {}",
                    entry.path().display()
                ),
            ));
        }
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("non-Unicode dependency name in {}", tools_dir.display()),
            )
        })?;
        names.insert(name);
    }
    let expected = BTreeSet::from(["cloudflared.exe".to_string()]);
    if names != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("current tools set mismatch: expected {expected:?}, found {names:?}"),
        ));
    }
    require_regular_file(
        &tools_dir.join("cloudflared.exe"),
        "cloudflared runtime dependency",
    )
}

fn expected_current_release_names(version_name: &str) -> BTreeSet<String> {
    expected_current_release_names_for_platforms(version_name, version_name)
}

fn expected_current_release_names_for_platforms(
    version_name: &str,
    android_version: &str,
) -> BTreeSet<String> {
    let mut expected = BTreeSet::from([versioned_artifact_name("tenfold", android_version, "apk")]);
    for (_, public_base_name) in RELEASE_BINARY_NAMES {
        expected.insert(versioned_artifact_name(
            public_base_name,
            version_name,
            "exe",
        ));
    }
    expected.insert(product_identity::RELEASE_MANIFEST_FILE_NAME.to_string());
    expected
}

fn is_forbidden_delivery_package(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("aab") => true,
        Some("apk") => {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            name.contains("debug")
                || name.contains("xiaomi")
                || name.contains("app-release")
                || !(name.starts_with("grid_timer_app_v") || name.starts_with("tenfold_v"))
        }
        _ => false,
    }
}

fn collect_forbidden_delivery_packages(project_root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut forbidden = BTreeSet::new();
    collect_forbidden_packages_in_directory(project_root, false, &mut forbidden)?;
    for relative in [
        "APK",
        "old_apks",
        "app/build/outputs/apk",
        "app/build/outputs/bundle",
        "release_artifacts",
    ] {
        collect_forbidden_packages_in_directory(
            &project_root.join(relative),
            true,
            &mut forbidden,
        )?;
    }
    Ok(forbidden.into_iter().collect())
}

fn collect_forbidden_packages_in_directory(
    directory: &Path,
    recursive: bool,
    forbidden: &mut BTreeSet<PathBuf>,
) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() || metadata_is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "delivery package scope is not a real directory: {}",
                directory.display()
            ),
        ));
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata_is_reparse_point(&metadata) {
            let is_package_name = path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("apk") || extension.eq_ignore_ascii_case("aab")
                });
            if recursive || is_package_name {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "unsupported reparse point in delivery scope: {}",
                        path.display()
                    ),
                ));
            }
            continue;
        }
        if metadata.file_type().is_file() {
            if is_forbidden_delivery_package(&path) {
                forbidden.insert(path);
            }
            continue;
        }
        if metadata.file_type().is_dir() {
            if recursive {
                collect_forbidden_packages_in_directory(&path, true, forbidden)?;
            }
            continue;
        }
        if recursive || is_forbidden_delivery_package(&path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported reparse point or special entry in delivery scope: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn validate_no_forbidden_delivery_packages(project_root: &Path) -> io::Result<()> {
    let forbidden = collect_forbidden_delivery_packages(project_root)?;
    if forbidden.is_empty() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "forbidden delivery packages remain: {}",
            display_paths(&forbidden)
        ),
    ))
}

fn enforce_forbidden_delivery_package_policy(
    project_root: &Path,
    fail_if_found: bool,
) -> io::Result<()> {
    let forbidden = collect_forbidden_delivery_packages(project_root)?;
    if forbidden.is_empty() {
        return Ok(());
    }

    let current_dir = project_root.join("release_artifacts/current");
    let mut protected_current = Vec::new();
    let mut removed = Vec::new();
    for path in forbidden {
        if path.starts_with(&current_dir) {
            protected_current.push(path);
            continue;
        }
        fs::remove_file(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to remove forbidden delivery package {}: {error}",
                    path.display()
                ),
            )
        })?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "forbidden delivery package still exists after removal: {}",
                        path.display()
                    ),
                ));
            }
            Err(error) => return Err(error),
        }
        removed.push(path);
    }

    if !protected_current.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "current release contains forbidden packages and was left untouched: {}",
                display_paths(&protected_current)
            ),
        ));
    }
    if fail_if_found {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "forbidden packages were generated or retained and removed: {}",
                display_paths(&removed)
            ),
        ));
    }
    Ok(())
}

fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn run_with_forbidden_package_cleanup<T>(
    project_root: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let operation_result = operation();
    let cleanup_result = enforce_forbidden_delivery_package_policy(project_root, true);
    match (operation_result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "formal release operation failed ({error}); forbidden package cleanup also failed ({cleanup_error})"
            ),
        )),
    }
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

fn validate_expected_packages(
    project_root: &Path,
    version: &gridtimer_native::tooling::ProjectVersionInfo,
) -> io::Result<()> {
    validate_no_forbidden_delivery_packages(project_root)?;
    let expected = project_root.join(versioned_artifact_name(
        "tenfold",
        &version.version_name,
        "apk",
    ));
    require_real_regular_file(&expected, "expected formal release APK")?;

    let expected_name = expected
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let legacy_expected = project_root.join("APK").join(expected_name);
    require_real_regular_file(&legacy_expected, "expected APK delivery artifact")?;
    let expected_identity = sha256_file_hex(&expected)?;
    if sha256_file_hex(&legacy_expected)? != expected_identity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "root and APK delivery artifacts do not have the same identity",
        ));
    }
    for entry in fs::read_dir(project_root)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if path.is_file()
            && (lower.ends_with(".apk") || lower.ends_with(".aab"))
            && name != expected_name
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "only the formal versioned release APK may remain at project root; found {}",
                    path.display()
                ),
            ));
        }
    }

    for descriptor in collect_formal_apk_descriptors(&project_root.join("APK"), "legacy_apk")? {
        if descriptor.file_name != expected_name {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "only the formal versioned release APK may remain in APK; found {}",
                    descriptor.file_name
                ),
            ));
        }
    }

    Ok(())
}

fn archive_existing_root_release_apks(project_root: &Path) -> io::Result<()> {
    let archive_dir = archive_directory(project_root);
    ensure_directory(&archive_dir)?;
    let mut release_apks = Vec::new();
    for entry in fs::read_dir(project_root)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower == "grid_timer_app.apk"
            || ((lower.starts_with("grid_timer_app_v") || lower.starts_with("tenfold_v"))
                && lower.ends_with(".apk"))
        {
            if is_forbidden_delivery_package(&path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "forbidden APK cannot be archived as a formal release: {}",
                        path.display()
                    ),
                ));
            }
            release_apks.push(path);
        }
    }
    release_apks.sort();
    for path in release_apks {
        archive_file(&path, &archive_dir)?;
    }
    Ok(())
}

fn run_status_generator(
    project_root: &Path,
    cargo_home: &str,
    rustup_home: &str,
) -> io::Result<()> {
    let build_environment = resolve_windows_build_environment(rustup_home)?;
    let library_path = env::join_paths(&build_environment.library_paths).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid MSVC LIB path: {error}"),
        )
    })?;
    let mut command = Command::new(&build_environment.cargo);
    command.current_dir(project_root);
    command.args([
        "run",
        "--locked",
        "--offline",
        "--quiet",
        "--manifest-path",
        "native/gridtimer_native/Cargo.toml",
        "--target",
        WINDOWS_MSVC_TARGET,
        "--bin",
        "gridtimer_setup_status",
        "--",
        "--output-path",
        "xiaomi_setup_status.md",
    ]);
    command.env("CARGO_HOME", cargo_home);
    command.env("RUSTUP_HOME", rustup_home);
    command.env("RUSTUP_TOOLCHAIN", WINDOWS_MSVC_TOOLCHAIN);
    command.env(
        "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
        &build_environment.linker,
    );
    command.env("LIB", library_path);
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
    let build_environment = resolve_windows_build_environment(rustup_home)?;
    let library_path = env::join_paths(&build_environment.library_paths).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid MSVC LIB path: {error}"),
        )
    })?;
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
    command.env("RUSTUP_TOOLCHAIN", WINDOWS_MSVC_TOOLCHAIN);
    command.env(
        "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
        &build_environment.linker,
    );
    command.env("LIB", library_path);
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

#[cfg(test)]
mod tests {
    include!("../packager_windows_format_tests.rs");
    include!("../packager_windows_verification_tests.rs");
    #[cfg(windows)]
    include!("../packager_update_recovery_tests.rs");
    #[cfg(windows)]
    include!("../packager_runtime_publication_tests.rs");
    use super::*;
    use gridtimer_native::tooling::ProjectVersionInfo;
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_NONCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = TEST_NONCE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "grid-timer-packager-{label}-{}-{}-{nonce}",
                std::process::id(),
                current_time_millis()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cargo_home_defaults_to_the_standard_user_registry_and_preserves_overrides() {
        let profile = Path::new(r"C:\Users\tester");
        assert_eq!(
            profile.join(".cargo"),
            select_cargo_home(None, Some(profile))
        );
        assert_eq!(
            profile.join(".cargo"),
            select_cargo_home(Some("  "), Some(profile))
        );
        assert_eq!(
            Path::new(r"D:\shared-cargo"),
            select_cargo_home(Some(r"D:\shared-cargo"), Some(profile))
        );
        assert_eq!(
            Path::new("custom-cargo"),
            select_cargo_home(Some("custom-cargo"), None)
        );
    }

    #[test]
    fn windows_test_cache_matches_the_script_without_reusing_the_release_cache() {
        let project = Path::new(r"C:\projects\timer");
        let native = project.join("native/gridtimer_native");
        assert_eq!(
            native.join("target"),
            windows_target_directory(project, true, None, Some("release-cache"), None)
        );
        assert_eq!(
            native.join("cargo-cache"),
            windows_target_directory(project, true, None, None, Some("cargo-cache"))
        );
        assert_eq!(
            project.join("test-cache"),
            windows_target_directory(project, true, Some("test-cache"), None, Some("cargo-cache"))
        );
        assert_eq!(
            project.join("release-cache"),
            windows_target_directory(
                project,
                false,
                Some("test-cache"),
                Some("release-cache"),
                Some("cargo-cache")
            )
        );
        assert_eq!(
            native.join("target"),
            windows_target_directory(project, true, Some(" "), None, Some(" "))
        );
    }

    #[test]
    fn default_rustup_home_prefers_existing_user_directory_then_falls_back() {
        let user_profile = Path::new(r"C:\Users\tester");
        let user_rustup = user_profile.join(".rustup");
        let fallback = Path::new(FALLBACK_RUSTUP_HOME);

        assert_eq!(
            user_rustup,
            select_default_rustup_home(Some(user_profile), fallback, |path| path == user_rustup)
        );
        assert_eq!(
            fallback,
            select_default_rustup_home(Some(user_profile), fallback, |_| false)
        );
        assert_eq!(
            fallback,
            select_default_rustup_home(None, fallback, |_| true)
        );
    }

    #[test]
    fn windows_release_gate_runs_all_required_test_suites_before_build() {
        assert_eq!(
            [
                WindowsReleaseCommand::FormatCheck,
                WindowsReleaseCommand::LibraryTests,
                WindowsReleaseCommand::DesktopMediaTests,
                WindowsReleaseCommand::WindowsClientTests,
                WindowsReleaseCommand::SyncLauncherTests,
                WindowsReleaseCommand::PackagerTests,
                WindowsReleaseCommand::StableDesktopEntryTests,
            ],
            WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE
        );

        let format_args = windows_release_command_args(WindowsReleaseCommand::FormatCheck);
        assert_eq!(
            [
                "--check",
                "--edition",
                "2021",
                "--config",
                "skip_children=true"
            ],
            format_args.as_slice()
        );

        let library_args = windows_release_command_args(WindowsReleaseCommand::LibraryTests);
        assert_eq!("test", library_args[0]);
        assert!(library_args.contains(&"--lib"));
        assert!(library_args.contains(&"desktop"));

        let media_args = windows_release_command_args(WindowsReleaseCommand::DesktopMediaTests);
        assert!(media_args.contains(&"--lib"));
        assert!(media_args.contains(&"desktop_note_media"));

        let client_args = windows_release_command_args(WindowsReleaseCommand::WindowsClientTests);
        assert!(client_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_windows_client"]));

        let launcher_args = windows_release_command_args(WindowsReleaseCommand::SyncLauncherTests);
        assert!(launcher_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_sync_launcher"]));

        let packager_args = windows_release_command_args(WindowsReleaseCommand::PackagerTests);
        assert!(packager_args
            .windows(2)
            .any(|values| values == ["--bin", "gridtimer_packager"]));

        let stable_entry_args =
            windows_release_command_args(WindowsReleaseCommand::StableDesktopEntryTests);
        assert!(stable_entry_args
            .windows(2)
            .any(|values| values == ["--bin", STABLE_DESKTOP_ENTRY_BINARY_NAME]));

        let server_args = windows_release_command_args(WindowsReleaseCommand::ReleaseServerBuild);
        assert!(server_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_sync_server"]));
        assert!(server_args
            .windows(2)
            .any(|values| values == ["--bin", STABLE_DESKTOP_ENTRY_BINARY_NAME]));
        assert!(!server_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_sync_launcher"]));

        let launcher_build_args =
            windows_release_command_args(WindowsReleaseCommand::ReleaseLauncherBuild);
        assert!(launcher_build_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_sync_launcher"]));
        assert!(!launcher_build_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_sync_server"]));

        let client_build_args =
            windows_release_command_args(WindowsReleaseCommand::ReleaseClientBuild);
        assert!(client_build_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_windows_client"]));
        assert!(!client_build_args
            .windows(2)
            .any(|values| values == ["--bin", "timer_sync_launcher"]));
    }

    #[test]
    fn rustc_version_parser_enforces_the_release_floor() {
        assert_eq!(Some((1, 95, 0)), parse_rustc_version("rustc 1.95.0 (abc)"));
        assert_eq!(
            Some((1, 96, 0)),
            parse_rustc_version("rustc 1.96.0-nightly (abc)")
        );
        assert_eq!(None, parse_rustc_version("cargo 1.95.0"));
        assert!((1, 94, 1) < MINIMUM_WINDOWS_RUSTC_VERSION);
        assert!((1, 95, 0) >= MINIMUM_WINDOWS_RUSTC_VERSION);
    }

    #[test]
    fn current_release_process_classification_requires_the_exact_install_path() {
        let project = TestDirectory::new("release-process-classification");
        let current = project.path().join("release_artifacts/current");
        write_current_release_set(&current, "9.8.7-runtime");
        let current = fs::canonicalize(&current).unwrap();
        let descriptor = derive_release_set_descriptor(&current).unwrap();

        assert_eq!(4, expected_current_release_process_names(&descriptor).len());
        assert!(
            CurrentReleaseProcessRole::SyncLauncher.termination_order()
                < CurrentReleaseProcessRole::SyncServer.termination_order()
        );
        assert!(
            CurrentReleaseProcessRole::SyncServer.termination_order()
                < CurrentReleaseProcessRole::Cloudflared.termination_order()
        );
        for file in &descriptor.files {
            let Some(expected_role) = CurrentReleaseProcessRole::from_descriptor_role(&file.role)
            else {
                continue;
            };
            let exact_path = release_set_file_path(&current, file);
            assert_eq!(
                Some(expected_role),
                classify_current_release_process_path(&current, &descriptor, &exact_path)
            );

            let unrelated_path = project.path().join("unrelated").join(&file.file_name);
            assert_eq!(
                None,
                classify_current_release_process_path(&current, &descriptor, &unrelated_path)
            );
        }
    }

    #[test]
    fn downstream_integrity_binding_must_be_embedded_in_the_built_binary() {
        let directory = TestDirectory::new("embedded-integrity-binding");
        let path = directory.path().join("artifact.exe");
        let binding = ExecutableIntegrityBinding {
            size: 12_345_678,
            sha256: "a".repeat(64),
        };
        let mut bytes = vec![0_u8; 65_530];
        bytes.extend_from_slice(binding.marker("sync_server").as_bytes());
        fs::write(&path, bytes).unwrap();
        verify_binary_contains_integrity_binding(&path, &binding, "sync_server", "test artifact")
            .unwrap();

        let missing = ExecutableIntegrityBinding {
            size: binding.size + 1,
            sha256: binding.sha256,
        };
        assert!(verify_binary_contains_integrity_binding(
            &path,
            &missing,
            "sync_server",
            "test artifact"
        )
        .is_err());
    }

    #[test]
    fn formal_release_orders_every_windows_test_before_gradle_build_and_publication() {
        let events = RefCell::new(Vec::new());
        run_formal_release_pipeline(
            || {
                run_windows_release_command_sequence(
                    &WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE,
                    |kind| {
                        events.borrow_mut().push(format!("test:{kind:?}"));
                        Ok(())
                    },
                )
            },
            || {
                events.borrow_mut().push("gradle".to_string());
                Ok(())
            },
            || {
                events.borrow_mut().push("windows-build".to_string());
                Ok(())
            },
            |()| {
                events.borrow_mut().push("publish".to_string());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            vec![
                "test:FormatCheck",
                "test:LibraryTests",
                "test:DesktopMediaTests",
                "test:WindowsClientTests",
                "test:SyncLauncherTests",
                "test:PackagerTests",
                "test:StableDesktopEntryTests",
                "gradle",
                "windows-build",
                "publish",
            ],
            events.into_inner()
        );
    }

    #[test]
    fn each_required_windows_test_failure_blocks_every_later_step() {
        for failure_index in 0..WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE.len() {
            let command_index = Cell::new(0_usize);
            let later_steps = RefCell::new(Vec::new());
            let error = run_formal_release_pipeline(
                || {
                    run_windows_release_command_sequence(
                        &WINDOWS_RELEASE_TEST_COMMAND_SEQUENCE,
                        |_| {
                            let index = command_index.get();
                            command_index.set(index + 1);
                            if index == failure_index {
                                return Err(io::Error::new(
                                    io::ErrorKind::Other,
                                    "injected required test failure",
                                ));
                            }
                            Ok(())
                        },
                    )
                },
                || {
                    later_steps.borrow_mut().push("gradle");
                    Ok(())
                },
                || {
                    later_steps.borrow_mut().push("windows-build");
                    Ok(())
                },
                |()| {
                    later_steps.borrow_mut().push("publish");
                    Ok(())
                },
            )
            .expect_err("every required test failure must stop the formal release");

            assert_eq!(io::ErrorKind::Other, error.kind());
            assert!(later_steps.into_inner().is_empty());
        }
    }

    #[test]
    fn non_release_configurations_are_rejected() {
        for configuration in ["Debug", "debug", "XiaomiDebug", "xiaomidebug", "Profile"] {
            let error = require_formal_release_configuration(configuration)
                .expect_err("non-release configuration must be rejected");
            assert_eq!(io::ErrorKind::InvalidInput, error.kind());
        }
        require_formal_release_configuration("Release").unwrap();
        require_formal_release_configuration("release").unwrap();
    }

    #[test]
    fn release_apk_requires_the_exact_versioned_output() {
        let project = TestDirectory::new("exact-release-apk");
        let version = sample_version("9.8.7-release");
        let output_dir = project.path().join("app/build/outputs/apk/release");
        fs::create_dir_all(&output_dir).unwrap();
        write_non_empty(&output_dir.join("app-release.apk"));

        let error = resolve_formal_release_apk(project.path(), &version)
            .expect_err("app-release.apk must never be accepted as a fallback");
        assert_eq!(io::ErrorKind::NotFound, error.kind());

        let expected = formal_release_apk_path(project.path(), &version);
        write_non_empty(&expected);
        assert_eq!(
            expected,
            resolve_formal_release_apk(project.path(), &version).unwrap()
        );
    }

    #[test]
    fn forbidden_packages_are_removed_across_delivery_scopes_case_insensitively() {
        let project = TestDirectory::new("forbidden-delivery-scope");
        let forbidden = [
            project
                .path()
                .join("grid_timer_app_DEBUG_v9.8.7-release.apk"),
            project
                .path()
                .join("app/build/outputs/apk/release/app-release.apk"),
            project
                .path()
                .join("app/build/outputs/bundle/release/app-release.aab"),
            project
                .path()
                .join("old_apks/grid_timer_app_v9.8.7-DeBuG.apk"),
            project.path().join("APK/grid_timer_app_v9.8.7-XIAOMI.apk"),
            project
                .path()
                .join("APK/grid_timer_app_v9.8.7-App-Release.apk"),
            project.path().join("release_artifacts/staging/unknown.apk"),
        ];
        for path in &forbidden {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            write_non_empty(path);
        }
        let allowed_root = project.path().join("grid_timer_app_v9.8.7-release.apk");
        let allowed_archive = project
            .path()
            .join("old_apks/grid_timer_app_v9.8.6-release.apk");
        write_non_empty(&allowed_root);
        write_non_empty(&allowed_archive);

        enforce_forbidden_delivery_package_policy(project.path(), false).unwrap();

        for path in forbidden {
            assert!(
                !path.exists(),
                "forbidden package survived: {}",
                path.display()
            );
        }
        assert!(allowed_root.is_file());
        assert!(allowed_archive.is_file());
        validate_no_forbidden_delivery_packages(project.path()).unwrap();
    }

    #[test]
    fn forbidden_package_in_current_fails_closed_without_deleting_current() {
        let project = TestDirectory::new("forbidden-current-protected");
        let forbidden = project
            .path()
            .join("release_artifacts/current/grid_timer_app_v9.8.7-debug.apk");
        fs::create_dir_all(forbidden.parent().unwrap()).unwrap();
        write_non_empty(&forbidden);

        let error = enforce_forbidden_delivery_package_policy(project.path(), false)
            .expect_err("a corrupt current release must block delivery");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert!(forbidden.is_file());
    }

    #[test]
    fn failed_build_still_removes_forbidden_packages_and_blocks_publication() {
        let project = TestDirectory::new("failed-build-cleanup");
        let forbidden = project
            .path()
            .join("app/build/outputs/apk/debug/app-debug.apk");
        fs::create_dir_all(forbidden.parent().unwrap()).unwrap();

        let error = run_with_forbidden_package_cleanup(project.path(), || -> io::Result<()> {
            write_non_empty(&forbidden);
            Err(io::Error::new(
                io::ErrorKind::Other,
                "injected build failure",
            ))
        })
        .expect_err("build failure must remain a failure after cleanup");

        assert_eq!(io::ErrorKind::Other, error.kind());
        assert!(!forbidden.exists());
    }

    #[test]
    fn current_archiver_rejects_aab_without_retaining_archive_copies() {
        let project = TestDirectory::new("current-aab-rejection");
        let current = project.path().join("release_artifacts/current");
        fs::create_dir_all(&current).unwrap();
        write_non_empty(&current.join("aaa.exe"));
        write_non_empty(&current.join("zzz.aab"));

        let error = prepare_current_release_archives(project.path(), &current)
            .expect_err("AAB must never be copied into an archive");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert_eq!(
            0,
            count_files_with_extension(&project.path().join("old_exes"), "exe")
        );
        assert_eq!(
            0,
            count_files_with_extension(&project.path().join("old_apks"), "aab")
        );
    }

    #[test]
    fn root_archiver_rejects_debug_disguised_as_a_versioned_release() {
        let project = TestDirectory::new("root-debug-archive-rejection");
        let disguised = project.path().join("grid_timer_app_v9.8.7-DeBuG.apk");
        write_non_empty(&disguised);

        let error = archive_existing_root_release_apks(project.path())
            .expect_err("a debug-named APK must never enter old_apks");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert!(disguised.is_file());
        assert_eq!(
            0,
            count_files_with_extension(&project.path().join("old_apks"), "apk")
        );
    }

    #[test]
    fn stable_desktop_entry_path_and_publication_order_are_fixed() {
        let project = TestDirectory::new("stable-entry-order");
        assert_eq!(
            project
                .path()
                .join("release_artifacts/desktop_entry/TenRate_Desktop_Launcher.exe"),
            stable_desktop_entry_path(project.path())
        );

        let events = RefCell::new(Vec::new());
        publish_stable_entry_before_current(
            || {
                events.borrow_mut().push("stable");
                Ok(())
            },
            || {
                events.borrow_mut().push("current");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(vec!["stable", "current"], events.into_inner());

        let current_switched = Cell::new(false);
        let error = publish_stable_entry_before_current(
            || {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "locked entry",
                ))
            },
            || {
                current_switched.set(true);
                Ok(())
            },
        )
        .expect_err("stable entry failure must block current switch");
        assert_eq!(io::ErrorKind::PermissionDenied, error.kind());
        assert!(!current_switched.get());
    }

    #[test]
    fn stable_desktop_entry_replacement_uses_the_fixed_path() {
        let project = TestDirectory::new("stable-entry-replace");
        let source = project.path().join("tenrate_desktop_launcher.exe");
        let destination = stable_desktop_entry_path(project.path());
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&source, b"new-stable-entry").unwrap();
        fs::write(&destination, b"old-stable-entry").unwrap();

        publish_stable_desktop_entry(project.path(), &source).unwrap();

        assert_eq!(
            b"new-stable-entry",
            fs::read(&destination).unwrap().as_slice()
        );
        assert_eq!(
            vec![STABLE_DESKTOP_ENTRY_FILE_NAME.to_string()],
            sorted_entry_names(destination.parent().unwrap())
        );
    }

    #[test]
    fn stable_entry_atomic_replace_failure_preserves_old_destination() {
        let project = TestDirectory::new("stable-entry-atomic-failure");
        let destination = project.path().join(STABLE_DESKTOP_ENTRY_FILE_NAME);
        let staged = project.path().join("staged-entry.exe");
        fs::write(&destination, b"old-stable-entry").unwrap();
        fs::write(&staged, b"new-stable-entry").unwrap();

        let error = commit_staged_file_with_replace(
            &staged,
            &destination,
            "stable desktop entry",
            |destination, replacement, backup| {
                assert!(destination.is_file());
                assert!(replacement.is_file());
                assert!(!backup.exists());
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected ReplaceFileW failure",
                ))
            },
        )
        .expect_err("atomic replacement failure must stop publication");

        assert_eq!(io::ErrorKind::PermissionDenied, error.kind());
        assert_eq!(
            b"old-stable-entry",
            fs::read(&destination).unwrap().as_slice()
        );
        assert_eq!(b"new-stable-entry", fs::read(&staged).unwrap().as_slice());
        assert_eq!(
            vec![
                STABLE_DESKTOP_ENTRY_FILE_NAME.to_string(),
                "staged-entry.exe".to_string(),
            ],
            sorted_entry_names(project.path())
        );
    }

    #[test]
    fn stable_entry_partial_replace_failure_rolls_backup_back() {
        let project = TestDirectory::new("stable-entry-partial-failure");
        let destination = project.path().join(STABLE_DESKTOP_ENTRY_FILE_NAME);
        let staged = project.path().join("staged-entry.exe");
        fs::write(&destination, b"old-stable-entry").unwrap();
        fs::write(&staged, b"new-stable-entry").unwrap();

        let error = commit_staged_file_with_replace(
            &staged,
            &destination,
            "stable desktop entry",
            |destination, replacement, backup| {
                fs::rename(destination, backup)?;
                assert!(!destination.exists());
                assert!(replacement.is_file());
                Err(io::Error::from_raw_os_error(1177))
            },
        )
        .expect_err("partial atomic replacement failure must roll back");

        assert_eq!(Some(1177), error.raw_os_error());
        assert_eq!(
            b"old-stable-entry",
            fs::read(&destination).unwrap().as_slice()
        );
        assert_eq!(b"new-stable-entry", fs::read(&staged).unwrap().as_slice());
        assert_eq!(
            vec![
                STABLE_DESKTOP_ENTRY_FILE_NAME.to_string(),
                "staged-entry.exe".to_string(),
            ],
            sorted_entry_names(project.path())
        );
    }

    #[test]
    fn current_release_accepts_only_exact_versioned_set_and_runtime_dependency() {
        let directory = TestDirectory::new("exact-current");
        let version = sample_version("9.8.7-release");
        write_current_release_set(directory.path(), &version.version_name);

        validate_current_release_directory(directory.path(), &version).unwrap();
    }

    #[test]
    fn current_release_rejects_debug_aab_and_extra_executable_artifacts() {
        for forbidden_name in [
            "grid_timer_app_debug_v9.8.7-release.apk",
            "grid_timer_app_xiaomi_debug_v9.8.7-release.apk",
            "app-release.apk",
            "grid_timer_app_v9.8.7-release.aab",
            "unexpected_helper.exe",
        ] {
            let directory = TestDirectory::new("forbidden-current");
            let version = sample_version("9.8.7-release");
            write_current_release_set(directory.path(), &version.version_name);
            write_non_empty(&directory.path().join(forbidden_name));

            let error = validate_current_release_directory(directory.path(), &version)
                .expect_err("forbidden artifact must fail validation");
            assert_eq!(io::ErrorKind::InvalidData, error.kind());
        }
    }

    #[test]
    fn current_release_rejects_mixed_or_stale_versions() {
        let directory = TestDirectory::new("version-mismatch");
        let expected = sample_version("9.8.7-release");
        write_current_release_set(directory.path(), "9.8.6-release");

        let error = validate_current_release_directory(directory.path(), &expected)
            .expect_err("stale version must fail validation");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert!(error.to_string().contains("current release set mismatch"));
    }

    #[test]
    fn source_release_identity_rejects_version_mismatch_and_unsafe_components() {
        let mismatch = validate_release_identity("9.8.7-release", "9.8.6-release")
            .expect_err("different source versions must fail validation");
        assert_eq!(io::ErrorKind::InvalidData, mismatch.kind());
        assert!(mismatch.to_string().contains("release identity mismatch"));

        let unsafe_version = validate_release_identity("../9.8.7", "../9.8.7")
            .expect_err("path-like version must fail validation");
        assert_eq!(io::ErrorKind::InvalidData, unsafe_version.kind());
    }

    #[test]
    fn staging_failure_preserves_existing_current_release() {
        let project = TestDirectory::new("staging-rollback");
        let current = project.path().join("release_artifacts/current");
        fs::create_dir_all(&current).unwrap();
        let old_version = sample_version("9.8.6-release");
        write_current_release_set(&current, &old_version.version_name);

        let input_dir = project.path().join("build-inputs");
        fs::create_dir_all(&input_dir).unwrap();
        let root_apk = input_dir.join("app.apk");
        write_non_empty(&root_apk);
        let windows_artifacts = test_windows_artifacts(&input_dir);
        let new_version = sample_version("9.8.7-release");

        let error = publish_current_release_artifacts(
            project.path(),
            &new_version,
            &root_apk,
            &windows_artifacts,
        )
        .expect_err("missing cloudflared dependency must stop staging");
        assert_eq!(io::ErrorKind::NotFound, error.kind());
        validate_current_release_directory(&current, &old_version).unwrap();
        assert_eq!(
            vec!["current".to_string()],
            sorted_entry_names(&project.path().join("release_artifacts"))
        );
        assert!(!stable_desktop_entry_path(project.path()).exists());
    }

    #[test]
    fn successful_publication_switches_complete_tree_and_archives_previous_tree() {
        let project = TestDirectory::new("atomic-publication");
        let current = project.path().join("release_artifacts/current");
        fs::create_dir_all(&current).unwrap();
        let old_version = sample_version("9.8.6-release");
        write_current_release_set(&current, &old_version.version_name);

        let input_dir = project.path().join("build-inputs");
        fs::create_dir_all(&input_dir).unwrap();
        let root_apk = input_dir.join("app.apk");
        write_non_empty(&root_apk);
        let windows_artifacts = test_windows_artifacts(&input_dir);
        fs::create_dir_all(project.path().join("tools")).unwrap();
        write_non_empty(&project.path().join(r"tools\cloudflared.exe"));
        let new_version = sample_version("9.8.7-release");

        publish_current_release_artifacts(
            project.path(),
            &new_version,
            &root_apk,
            &windows_artifacts,
        )
        .unwrap();

        validate_current_release_directory(&current, &new_version).unwrap();
        assert_eq!(
            1,
            count_files_with_extension(&project.path().join("old_apks"), "apk")
        );
        assert_eq!(
            4,
            count_files_with_extension(&project.path().join("old_exes"), "exe")
        );
        assert_eq!(
            vec!["current".to_string(), "desktop_entry".to_string()],
            sorted_entry_names(&project.path().join("release_artifacts"))
        );
        assert!(stable_desktop_entry_path(project.path()).is_file());
    }

    #[test]
    fn ordinary_and_verbatim_release_paths_share_one_identity_text() {
        assert_eq!(
            normalize_release_root_text(r"C:\Work\Timer\release_artifacts\"),
            normalize_release_root_text(r"\\?\C:\WORK\Timer\release_artifacts")
        );
        assert_eq!(
            normalize_release_root_text(r"\\server\share\Timer\release_artifacts"),
            normalize_release_root_text(r"\\?\UNC\SERVER\Share\Timer\release_artifacts\")
        );
        assert!(!transaction_journal_size_is_allowed(0));
        assert!(transaction_journal_size_is_allowed(
            MAX_RELEASE_TRANSACTION_JOURNAL_BYTES
        ));
        assert!(!transaction_journal_size_is_allowed(
            MAX_RELEASE_TRANSACTION_JOURNAL_BYTES + 1
        ));
    }

    #[test]
    fn old_current_must_be_an_exact_single_version_set_before_transaction_prepare() {
        let project = TestDirectory::new("transaction-old-current-exact-set");
        let current = project.path().join("release_artifacts/current");
        write_current_release_set(&current, "9.8.6-release");
        write_non_empty(&current.join("unexpected_helper.exe"));
        fs::create_dir_all(project.path().join("release_artifacts/desktop_entry")).unwrap();
        let stable = stable_desktop_entry_path(project.path());
        fs::write(&stable, b"old-stable").unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);

        let error = prepare_release_transaction(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
        )
        .expect_err("an extra old-current file must fail before staging or publication");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert_eq!(b"old-stable", fs::read(stable).unwrap().as_slice());
        assert!(!project
            .path()
            .join("release_artifacts/.release-transaction-v1.json")
            .exists());
        assert_eq!(
            vec!["current".to_string(), "desktop_entry".to_string()],
            sorted_entry_names(&project.path().join("release_artifacts"))
        );
    }

    #[test]
    fn prepare_cleanup_never_deletes_unowned_preoccupied_transaction_paths() {
        let project = TestDirectory::new("transaction-unowned-cleanup");
        let release_root = project.path().join("release_artifacts");
        let desktop = release_root.join("desktop_entry");
        fs::create_dir_all(&desktop).unwrap();
        let occupied = release_root.join(".current-staging-t1p1");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("owner.txt"), b"other").unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);

        let error = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t1p1",
        )
        .expect_err("preoccupied transaction staging must fail closed");

        assert_eq!(io::ErrorKind::AlreadyExists, error.kind());
        assert_eq!(
            b"other",
            fs::read(occupied.join("owner.txt")).unwrap().as_slice()
        );
        assert!(!release_root.join(".transaction-t1p1").exists());
        assert!(!desktop.join(".stable-staging-t1p1.exe").exists());
    }

    #[test]
    fn prepare_rejects_and_preserves_preoccupied_current_backup_slot() {
        let project = TestDirectory::new("transaction-current-backup-preoccupied");
        let release_root = project.path().join("release_artifacts");
        fs::create_dir_all(release_root.join("desktop_entry")).unwrap();
        let occupied = release_root.join(".current-backup-t2p2");
        fs::write(&occupied, b"other-current-backup-owner").unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);

        let error = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t2p2",
        )
        .expect_err("preoccupied current backup must fail before owned staging is created");

        assert_eq!(io::ErrorKind::AlreadyExists, error.kind());
        assert_eq!(
            b"other-current-backup-owner",
            fs::read(&occupied).unwrap().as_slice()
        );
        assert!(!release_root.join(".current-staging-t2p2").exists());
        assert!(!release_root.join(".transaction-t2p2").exists());
        assert!(!release_root
            .join("desktop_entry/.stable-staging-t2p2.exe")
            .exists());
    }

    #[test]
    fn prepare_rejects_and_preserves_preoccupied_stable_backup_slot() {
        let project = TestDirectory::new("transaction-stable-backup-preoccupied");
        let release_root = project.path().join("release_artifacts");
        let desktop = release_root.join("desktop_entry");
        fs::create_dir_all(&desktop).unwrap();
        let occupied = desktop.join(".stable-backup-t3p3.exe");
        fs::write(&occupied, b"other-stable-backup-owner").unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);

        let error = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t3p3",
        )
        .expect_err("preoccupied stable backup must fail before owned staging is created");

        assert_eq!(io::ErrorKind::AlreadyExists, error.kind());
        assert_eq!(
            b"other-stable-backup-owner",
            fs::read(&occupied).unwrap().as_slice()
        );
        assert!(!release_root.join(".current-staging-t3p3").exists());
        assert!(!release_root.join(".transaction-t3p3").exists());
        assert!(!desktop.join(".stable-staging-t3p3.exe").exists());
    }

    #[test]
    fn rollback_preserves_current_backup_raced_in_after_prepare() {
        let project = TestDirectory::new("transaction-current-backup-race");
        let release_root = project.path().join("release_artifacts");
        write_current_release_set(&release_root.join("current"), "9.8.6-release");
        fs::create_dir_all(release_root.join("desktop_entry")).unwrap();
        fs::write(stable_desktop_entry_path(project.path()), b"old-stable").unwrap();
        fs::create_dir(project.path().join("tools")).unwrap();
        fs::write(
            project.path().join("tools/cloudflared.exe"),
            b"new-cloudflared",
        )
        .unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);
        let mut journal = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t5p5",
        )
        .unwrap();
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        atomic_store_transaction_journal(project.path(), &mut journal).unwrap();
        fs::create_dir(&paths.current_backup).unwrap();
        let sentinel = paths.current_backup.join("not-owned.txt");
        fs::write(&sentinel, b"other-owner").unwrap();

        let error = rollback_release_transaction(project.path(), &journal, &paths)
            .expect_err("rollback must not clean a raced-in current backup");

        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert_eq!(b"other-owner", fs::read(sentinel).unwrap().as_slice());
        assert!(paths.journal.exists());
    }

    #[test]
    fn rollback_preserves_stable_backup_raced_in_after_prepare() {
        let project = TestDirectory::new("transaction-stable-backup-race");
        let release_root = project.path().join("release_artifacts");
        write_current_release_set(&release_root.join("current"), "9.8.6-release");
        fs::create_dir_all(release_root.join("desktop_entry")).unwrap();
        fs::write(stable_desktop_entry_path(project.path()), b"old-stable").unwrap();
        fs::create_dir(project.path().join("tools")).unwrap();
        fs::write(
            project.path().join("tools/cloudflared.exe"),
            b"new-cloudflared",
        )
        .unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);
        let mut journal = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t6p6",
        )
        .unwrap();
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        atomic_store_transaction_journal(project.path(), &mut journal).unwrap();
        fs::write(&paths.stable_backup, b"other-owner").unwrap();

        let error = rollback_release_transaction(project.path(), &journal, &paths)
            .expect_err("rollback must not clean a raced-in stable backup");

        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert_eq!(
            b"other-owner",
            fs::read(&paths.stable_backup).unwrap().as_slice()
        );
        assert!(paths.journal.exists());
    }

    #[test]
    fn prepare_rejects_invalid_desktop_entry_parent_before_reading_old_stable() {
        let project = TestDirectory::new("transaction-invalid-desktop-entry-parent");
        let release_root = project.path().join("release_artifacts");
        fs::create_dir_all(&release_root).unwrap();
        let desktop = release_root.join("desktop_entry");
        fs::write(&desktop, b"not-a-real-directory").unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);

        let error = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t4p4",
        )
        .expect_err("desktop_entry parent must be a real directory");

        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert_eq!(
            b"not-a-real-directory",
            fs::read(&desktop).unwrap().as_slice()
        );
        assert!(!release_root.join(".current-staging-t4p4").exists());
        assert!(!release_root.join(".transaction-t4p4").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_reparse_attribute_is_rejected_for_transaction_paths() {
        assert!(windows_file_attributes_are_reparse_point(0x0000_0400));
        assert!(windows_file_attributes_are_reparse_point(0x0000_0410));
        assert!(!windows_file_attributes_are_reparse_point(0x0000_0010));
    }

    #[test]
    fn forbidden_delivery_scope_itself_must_be_a_real_directory() {
        let project = TestDirectory::new("forbidden-scope-not-directory");
        let scope = project.path().join("APK");
        fs::write(&scope, b"not-a-directory").unwrap();
        let mut forbidden = BTreeSet::new();

        let error = collect_forbidden_packages_in_directory(&scope, true, &mut forbidden)
            .expect_err("a non-directory delivery scope must fail closed");

        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert!(forbidden.is_empty());
        assert_eq!(b"not-a-directory", fs::read(scope).unwrap().as_slice());
    }

    #[cfg(windows)]
    #[test]
    fn forbidden_delivery_scope_rejects_directory_reparse_points_when_supported() {
        use std::os::windows::fs::symlink_dir;

        let project = TestDirectory::new("forbidden-scope-reparse");
        let target = project.path().join("outside-scope");
        let scope = project.path().join("APK");
        fs::create_dir(&target).unwrap();
        if symlink_dir(&target, &scope).is_err() {
            return;
        }
        let mut forbidden = BTreeSet::new();
        let error = collect_forbidden_packages_in_directory(&scope, true, &mut forbidden)
            .expect_err("a reparse delivery scope must fail closed");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        fs::remove_dir(&scope).unwrap();
    }

    #[test]
    fn archive_parent_validation_preserves_source_on_failure() {
        let project = TestDirectory::new("transaction-archive-parent-invalid");
        let release_root = project.path().join("release_artifacts");
        fs::create_dir_all(release_root.join("desktop_entry")).unwrap();
        fs::create_dir(project.path().join("tools")).unwrap();
        fs::write(project.path().join("tools/cloudflared.exe"), b"cloudflared").unwrap();
        let old_root_apk = project.path().join("grid_timer_app_v9.8.6-release.apk");
        fs::write(&old_root_apk, b"old-root-apk").unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let release_apk = input.join("new.apk");
        fs::write(&release_apk, b"new-apk").unwrap();
        let artifacts = test_windows_artifacts(&input);
        let journal = prepare_release_transaction_with_id(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
            "t5p5",
        )
        .unwrap();
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        let entry = journal
            .archive_plan
            .iter()
            .find(|entry| entry.source_kind == "project_root")
            .expect("old root APK archive entry");
        let invalid_parent = archive_directory(project.path());
        fs::write(&invalid_parent, b"not-a-real-archive-directory").unwrap();

        let error = finalize_archive_entry(project.path(), &journal, &paths, entry)
            .expect_err("invalid archive parent must fail before copying or deleting source");

        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        assert_eq!(b"old-root-apk", fs::read(&old_root_apk).unwrap().as_slice());
        assert_eq!(
            b"not-a-real-archive-directory",
            fs::read(invalid_parent).unwrap().as_slice()
        );
    }

    #[cfg(windows)]
    #[test]
    fn archive_parent_guard_rejects_directory_reparse_points_when_supported() {
        use std::os::windows::fs::symlink_dir;

        let project = TestDirectory::new("transaction-archive-parent-reparse");
        let target = project.path().join("outside-archive");
        let archive_parent = archive_directory(project.path());
        fs::create_dir(&target).unwrap();
        if symlink_dir(&target, &archive_parent).is_err() {
            return;
        }

        let error = ensure_real_directory(&archive_parent, "release archive destination directory")
            .expect_err("archive parent reparse point must fail closed");
        assert_eq!(io::ErrorKind::InvalidData, error.kind());
        fs::remove_dir(&archive_parent).unwrap();
    }

    #[test]
    fn committed_transaction_archives_only_whitelisted_old_artifacts_idempotently() {
        let project = TestDirectory::new("transaction-commit-archive");
        let release_root = project.path().join("release_artifacts");
        let current = release_root.join("current");
        write_current_release_set(&current, "9.8.6-release");
        fs::create_dir_all(release_root.join("desktop_entry")).unwrap();
        fs::write(stable_desktop_entry_path(project.path()), b"old-stable").unwrap();
        fs::write(
            project.path().join("grid_timer_app_v9.8.7-release.apk"),
            b"old-root-apk",
        )
        .unwrap();
        fs::create_dir(project.path().join("APK")).unwrap();
        fs::write(
            project.path().join("APK/grid_timer_app_v9.8.5-release.apk"),
            b"old-legacy-apk",
        )
        .unwrap();
        let unrelated = project.path().join("r2_calendar");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("grid_timer_app_v1.0.0.apk"), b"untouched").unwrap();
        fs::create_dir(project.path().join("tools")).unwrap();
        fs::write(
            project.path().join("tools/cloudflared.exe"),
            b"new-cloudflared",
        )
        .unwrap();
        let inputs = project.path().join("inputs");
        fs::create_dir(&inputs).unwrap();
        let release_apk = inputs.join("new-release.apk");
        fs::write(&release_apk, b"new-root-apk").unwrap();
        let artifacts = test_windows_artifacts(&inputs);
        let version = sample_version("9.8.7-release");

        let mut journal =
            prepare_release_transaction(project.path(), &version, &release_apk, &artifacts)
                .unwrap();
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        assert!(journal
            .archive_plan
            .iter()
            .any(|entry| entry.source_kind == "stable_backup"));
        assert!(journal
            .archive_plan
            .iter()
            .any(|entry| entry.source_kind == "legacy_apk"));
        assert!(!journal
            .archive_plan
            .iter()
            .any(|entry| entry.source_file_name.contains("r2_calendar")));
        atomic_store_transaction_journal(project.path(), &mut journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::StableSwitching,
        )
        .unwrap();
        switch_stable_for_transaction(&paths, &journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::CurrentSwitching,
        )
        .unwrap();
        switch_current_for_transaction(&paths, &journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::RootPublishing,
        )
        .unwrap();
        publish_root_for_transaction(project.path(), &paths, &journal).unwrap();
        publish_legacy_apk_for_transaction(project.path(), &paths, &journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::Committed,
        )
        .unwrap();
        let root_archive = journal
            .archive_plan
            .iter()
            .find(|entry| entry.source_kind == "project_root")
            .unwrap();
        finalize_archive_entry(project.path(), &journal, &paths, root_archive).unwrap();
        assert!(!paths.root_same_name_backup.exists());
        finalize_committed_transaction_with_verifier(project.path(), &journal, &paths, |_| Ok(()))
            .unwrap();

        verify_release_set_descriptor(&current, &journal.new_current).unwrap();
        verify_file_descriptor(
            &stable_desktop_entry_path(project.path()),
            &journal.stable_new,
        )
        .unwrap();
        verify_file_descriptor(
            &project.path().join(&journal.root_new.file_name),
            &journal.root_new,
        )
        .unwrap();
        verify_file_descriptor(
            &project
                .path()
                .join("APK")
                .join(&journal.legacy_new.file_name),
            &journal.legacy_new,
        )
        .unwrap();
        assert_eq!(
            3,
            count_files_with_extension(&project.path().join("old_apks"), "apk")
        );
        assert_eq!(
            5,
            count_files_with_extension(&project.path().join("old_exes"), "exe")
        );
        assert_eq!(
            b"untouched",
            fs::read(unrelated.join("grid_timer_app_v1.0.0.apk"))
                .unwrap()
                .as_slice()
        );
        assert!(!paths.journal.exists());
        assert!(!paths.current_backup.exists());
        assert!(!paths.stable_backup.exists());
        assert!(!paths.transaction_root.exists());
    }

    #[test]
    fn crash_between_current_renames_rolls_current_and_stable_back() {
        let project = TestDirectory::new("transaction-precommit-rollback");
        let release_root = project.path().join("release_artifacts");
        let current = release_root.join("current");
        write_current_release_set(&current, "9.8.6-release");
        fs::create_dir_all(release_root.join("desktop_entry")).unwrap();
        let stable = stable_desktop_entry_path(project.path());
        fs::write(&stable, b"old-stable").unwrap();
        fs::create_dir(project.path().join("tools")).unwrap();
        fs::write(
            project.path().join("tools/cloudflared.exe"),
            b"new-cloudflared",
        )
        .unwrap();
        let inputs = project.path().join("inputs");
        fs::create_dir(&inputs).unwrap();
        let release_apk = inputs.join("new-release.apk");
        fs::write(&release_apk, b"new-root-apk").unwrap();
        let artifacts = test_windows_artifacts(&inputs);

        let mut journal = prepare_release_transaction(
            project.path(),
            &sample_version("9.8.7-release"),
            &release_apk,
            &artifacts,
        )
        .unwrap();
        let old_current = journal.old_current.clone().unwrap();
        let old_stable = journal.stable_old.clone().unwrap();
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        atomic_store_transaction_journal(project.path(), &mut journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::StableSwitching,
        )
        .unwrap();
        switch_stable_for_transaction(&paths, &journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::CurrentSwitching,
        )
        .unwrap();
        fs::rename(&paths.current, &paths.current_backup).unwrap();
        assert!(!paths.current.exists());
        assert!(paths.current_staging.exists());
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::RollingBack,
        )
        .unwrap();
        rollback_release_transaction(project.path(), &journal, &paths).unwrap();

        verify_release_set_descriptor(&current, &old_current).unwrap();
        verify_file_descriptor(&stable, &old_stable).unwrap();
        assert!(!paths.journal.exists());
        assert!(!paths.current_staging.exists());
        assert!(!paths.current_backup.exists());
        assert!(!paths.stable_backup.exists());
    }

    #[test]
    fn sync_server_runtime_smoke_requires_the_exact_build_and_process_identity() {
        let identity = BuildIdentity {
            git_commit: "1".repeat(40),
            source_worktree_dirty: true,
            source_snapshot_sha256: "b".repeat(64),
            toolchain: test_toolchain_identity(),
        };
        let mut response = SyncClientResult {
            ok: true,
            product_id: PRODUCT.internal_id.to_string(),
            service_role: "sync_server".to_string(),
            sync_protocol_version: PRODUCT.sync_protocol_version,
            mode: "health".to_string(),
            server_process_name: "timer_sync_server.exe".to_string(),
            server_build_id: SYNC_SERVER_BUILD_ID.to_string(),
            server_git_commit: identity.git_commit.clone(),
            server_source_snapshot_sha256: identity.source_snapshot_sha256.clone(),
            server_process_id: 4242,
            ..SyncClientResult::default()
        };
        let valid = serde_json::to_vec(&response).unwrap();
        assert!(validate_sync_server_smoke_response(
            &valid,
            "timer_sync_server.exe",
            4242,
            &identity,
        ));

        response.server_source_snapshot_sha256 = "c".repeat(64);
        let stale = serde_json::to_vec(&response).unwrap();
        assert!(!validate_sync_server_smoke_response(
            &stale,
            "timer_sync_server.exe",
            4242,
            &identity,
        ));
    }

    #[test]
    fn verification_report_is_bound_to_the_current_release_manifest() {
        let project = TestDirectory::new("verification-report-binding");
        let current = project.path().join("release_artifacts/current");
        write_current_release_set(&current, PRODUCT.app_version);
        let inputs = project.path().join("inputs");
        fs::create_dir(&inputs).unwrap();
        let artifacts = test_windows_artifacts(&inputs);
        let version = sample_version(PRODUCT.app_version);

        write_windows_build_verification(project.path(), &version, &artifacts).unwrap();
        validate_windows_build_verification(project.path(), &current, &version).unwrap();

        let report_path = project
            .path()
            .join("release_artifacts/verification/windows_build_verification.json");
        let mut report =
            serde_json::from_slice::<serde_json::Value>(&fs::read(&report_path).unwrap()).unwrap();
        report["sourceSnapshotSha256"] = serde_json::Value::String("c".repeat(64));
        fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        assert!(validate_windows_build_verification(project.path(), &current, &version).is_err());
    }

    #[test]
    fn windows_only_pipeline_verifies_retained_apk_before_windows_work() {
        let calls = RefCell::new(Vec::new());
        run_windows_only_release_pipeline(
            || {
                calls.borrow_mut().push("verify_existing_apk");
                Ok(())
            },
            || {
                calls.borrow_mut().push("windows_tests");
                Ok(())
            },
            || {
                calls.borrow_mut().push("windows_build");
                Ok(42)
            },
            |artifacts| {
                assert_eq!(artifacts, 42);
                calls.borrow_mut().push("publish_windows");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                "verify_existing_apk",
                "windows_tests",
                "windows_build",
                "publish_windows"
            ]
        );
        calls.borrow_mut().clear();
        let error = run_windows_only_release_pipeline(
            || {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "signature rejected",
                ))
            },
            || {
                calls.borrow_mut().push("tests");
                Ok(())
            },
            || {
                calls.borrow_mut().push("build");
                Ok(())
            },
            |_| {
                calls.borrow_mut().push("publish");
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn windows_only_identity_accepts_android_candidate_without_changing_it() {
        let mut android = sample_version(product_identity::ANDROID_APP_VERSION);
        android.version_code = product_identity::ANDROID_VERSION_CODE;
        let windows = windows_release_version(&android).unwrap();
        assert_eq!(windows.version_name, product_identity::WINDOWS_APP_VERSION);
        assert_eq!(android.version_name, product_identity::ANDROID_APP_VERSION);
        assert_eq!(android.version_code, product_identity::ANDROID_VERSION_CODE);
        assert!(validate_source_release_identity(&android).is_err());
        android.version_name = "2.22.49.11".to_string();
        android.version_code += 1;
        let windows = windows_release_version(&android).unwrap();
        assert_eq!(windows.version_name, product_identity::WINDOWS_APP_VERSION);
        assert_eq!(android.version_name, "2.22.49.11");
        assert_eq!(
            android.version_code,
            product_identity::ANDROID_VERSION_CODE + 1
        );
        android.version_name = "../unsafe".to_string();
        assert!(windows_release_version(&android).is_err());
        android.version_name = "2.22.49.11".to_string();
        android.version_code = 0;
        assert!(windows_release_version(&android).is_err());
        for flag in [
            "--archive-old-packages",
            "--keep-previous-root-packages",
            "--gradle-user-home",
            "--expected-version-code",
        ] {
            assert!(reject_android_mutation_flags(&[flag.to_string()]).is_err());
        }
        assert!(reject_android_mutation_flags(&[
            "--windows-only".to_string(),
            "--validate-only".to_string()
        ])
        .is_ok());
    }

    fn windows_only_fixture(
        label: &str,
    ) -> (
        TestDirectory,
        RetainedAndroidRelease,
        WindowsReleaseArtifacts,
    ) {
        let project = TestDirectory::new(label);
        let current = project.path().join("release_artifacts/current");
        write_current_release_set(&current, product_identity::ANDROID_APP_VERSION);
        fs::create_dir_all(project.path().join("release_artifacts/desktop_entry")).unwrap();
        fs::write(stable_desktop_entry_path(project.path()), b"old-stable").unwrap();
        fs::create_dir(project.path().join("APK")).unwrap();
        fs::create_dir(project.path().join("tools")).unwrap();
        fs::write(
            project.path().join("tools/cloudflared.exe"),
            b"new-cloudflared",
        )
        .unwrap();
        let name = versioned_artifact_name("tenfold", product_identity::ANDROID_APP_VERSION, "apk");
        let modified = std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for directory in [
            current.clone(),
            project.path().to_path_buf(),
            project.path().join("APK"),
        ] {
            let path = directory.join(&name);
            if !path.exists() {
                fs::copy(current.join(&name), &path).unwrap();
            }
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
        }
        fs::create_dir(project.path().join("old_apks")).unwrap();
        fs::write(
            project.path().join("old_apks/prior_archive.apk"),
            b"old-archive-untouched",
        )
        .unwrap();
        let input = project.path().join("inputs");
        fs::create_dir(&input).unwrap();
        let artifacts = test_windows_artifacts(&input);
        let retained =
            RetainedAndroidRelease::capture(project.path(), product_identity::ANDROID_APP_VERSION)
                .unwrap();
        (project, retained, artifacts)
    }

    fn prepare_windows_only_fixture(
        project: &TestDirectory,
        retained: &RetainedAndroidRelease,
        artifacts: &WindowsReleaseArtifacts,
        txid: &str,
    ) -> ReleaseTransactionJournal {
        prepare_release_transaction_with_scope(
            project.path(),
            &sample_version(PRODUCT.app_version),
            retained.current_apk(),
            artifacts,
            txid,
            true,
        )
        .unwrap()
    }

    #[test]
    fn windows_only_preserves_independent_android_release_provenance() {
        let (project, retained, artifacts) = windows_only_fixture("android-provenance");
        let current = project.path().join("release_artifacts/current");
        let path = current.join(product_identity::RELEASE_MANIFEST_FILE_NAME);
        let mut manifest = read_release_manifest_file(&path).unwrap();
        let provenance = AndroidReleaseProvenance {
            version: product_identity::ANDROID_APP_VERSION.to_string(),
            version_code: product_identity::ANDROID_VERSION_CODE,
            android_only: true,
            verification: format!(
                "release_artifacts/verification/v{}",
                product_identity::ANDROID_APP_VERSION
            ),
            sha256: retained.descriptor.sha256.clone(),
            source_snapshot_sha256: Some("b".repeat(64)),
            created_at: "2026-09-09T12:17:24.0114523Z".to_string(),
        };
        manifest.windows_only = true;
        manifest.passed_checks = release_passed_checks(true);
        manifest.android_release = Some(provenance.clone());
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        RetainedAndroidRelease::capture(project.path(), product_identity::ANDROID_APP_VERSION)
            .unwrap();
        let journal = prepare_windows_only_fixture(&project, &retained, &artifacts, "t15p15");
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        let staged = read_release_manifest_file(
            &paths
                .current_staging
                .join(product_identity::RELEASE_MANIFEST_FILE_NAME),
        )
        .unwrap();
        assert_eq!(staged.android_release, Some(provenance));
        retained.verify_unchanged().unwrap();

        let original = serde_json::to_value(&manifest).unwrap();
        for (field, invalid) in [
            ("version", serde_json::json!("2.22.36-android-timer")),
            ("sha256", serde_json::json!("0".repeat(64))),
            ("androidOnly", serde_json::json!(false)),
            ("versionCode", serde_json::json!(0)),
            ("verification", serde_json::json!("../untrusted")),
            ("createdAt", serde_json::json!("")),
            ("sourceSnapshotSha256", serde_json::json!("invalid")),
            ("unknownOverride", serde_json::json!(true)),
        ] {
            let mut invalid_manifest = original.clone();
            invalid_manifest["androidRelease"][field] = invalid;
            fs::write(&path, serde_json::to_vec(&invalid_manifest).unwrap()).unwrap();
            assert!(derive_release_set_descriptor(&current).is_err(), "{field}");
        }
    }

    #[test]
    fn windows_only_commit_preserves_all_apk_bytes_mtimes_and_archives_only_windows() {
        let (project, retained, artifacts) = windows_only_fixture("windows-only-commit");
        let mut journal = prepare_windows_only_fixture(&project, &retained, &artifacts, "t11p11");
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        assert!(journal.windows_only);
        assert_eq!(
            release_apk_version(&journal.new_current).unwrap(),
            product_identity::ANDROID_APP_VERSION
        );
        assert!(!paths.root_staging.exists());
        assert!(!paths.legacy_staging.exists());
        assert!(journal
            .archive_plan
            .iter()
            .all(|entry| entry.destination_kind == "old_exes"));
        retained.verify_unchanged().unwrap();
        atomic_store_transaction_journal(project.path(), &mut journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::StableSwitching,
        )
        .unwrap();
        switch_stable_for_transaction(&paths, &journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::CurrentSwitching,
        )
        .unwrap();
        switch_current_for_transaction(&paths, &journal).unwrap();
        publish_root_for_transaction(project.path(), &paths, &journal).unwrap();
        publish_legacy_apk_for_transaction(project.path(), &paths, &journal).unwrap();
        retained.verify_unchanged().unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::Committed,
        )
        .unwrap();
        let signatures = Cell::new(0);
        finalize_committed_transaction_with_verifier(project.path(), &journal, &paths, |_| {
            signatures.set(signatures.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(signatures.get(), 2);
        retained.verify_unchanged().unwrap();
        finalize_committed_transaction_with_verifier(project.path(), &journal, &paths, |_| Ok(()))
            .unwrap();
        retained.verify_unchanged().unwrap();
        assert_eq!(
            count_files_with_extension(&project.path().join("old_apks"), "apk"),
            1
        );
        assert_eq!(
            fs::read(project.path().join("old_apks/prior_archive.apk")).unwrap(),
            b"old-archive-untouched"
        );
        assert_eq!(
            count_files_with_extension(&project.path().join("old_exes"), "exe"),
            5
        );
        assert!(!paths.current_backup.exists());
        assert!(!paths.journal.exists());
        write_windows_build_verification(
            project.path(),
            &sample_version(PRODUCT.app_version),
            &artifacts,
        )
        .unwrap();
        validate_windows_only_current_release(
            project.path(),
            &sample_version(PRODUCT.app_version),
            product_identity::ANDROID_APP_VERSION,
        )
        .unwrap();
        let manifest = read_release_manifest_file(
            &paths
                .current
                .join(product_identity::RELEASE_MANIFEST_FILE_NAME),
        )
        .unwrap();
        assert!(manifest.windows_only);
        assert!(!manifest
            .passed_checks
            .iter()
            .any(|check| check == "android_release_build"));
        assert!(manifest
            .passed_checks
            .iter()
            .any(|check| check == "retained_android_signature_verification"));
    }

    #[test]
    fn windows_only_crash_between_current_renames_restores_windows_without_touching_android() {
        let (project, retained, artifacts) = windows_only_fixture("windows-only-rollback");
        let mut journal = prepare_windows_only_fixture(&project, &retained, &artifacts, "t12p12");
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        atomic_store_transaction_journal(project.path(), &mut journal).unwrap();
        switch_stable_for_transaction(&paths, &journal).unwrap();
        set_transaction_phase(
            project.path(),
            &mut journal,
            ReleaseTransactionPhase::CurrentSwitching,
        )
        .unwrap();
        fs::rename(&paths.current, &paths.current_backup).unwrap();
        let recovered = load_transaction_journal(project.path()).unwrap().unwrap();
        assert!(recovered.windows_only);
        rollback_release_transaction(project.path(), &recovered, &paths).unwrap();
        retained.verify_unchanged().unwrap();
        assert_eq!(
            fs::read(stable_desktop_entry_path(project.path())).unwrap(),
            b"old-stable"
        );
        assert!(!paths.root_same_name_backup.exists());
        assert!(!paths.legacy_same_name_backup.exists());
        assert!(!project.path().join("old_exes").exists());
    }

    #[test]
    fn windows_only_rejects_changed_android_replica_and_modified_time() {
        let (project, retained, _) = windows_only_fixture("windows-only-retained-tampering");
        let path = &retained.locations[1].0;
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(retained.locations[1].1 + Duration::from_secs(1)),
            )
            .unwrap();
        assert!(retained.verify_unchanged().is_err());
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(retained.locations[1].1))
            .unwrap();
        retained.verify_unchanged().unwrap();
        fs::write(&retained.locations[2].0, b"different-apk").unwrap();
        assert!(RetainedAndroidRelease::capture(
            project.path(),
            product_identity::ANDROID_APP_VERSION
        )
        .is_err());
    }

    #[test]
    fn windows_only_retains_verified_published_apk_when_android_source_has_advanced() {
        let project = TestDirectory::new("windows-retains-published-android");
        let current = project.path().join("release_artifacts/current");
        let published = "2.22.1-published";
        assert_ne!(published, product_identity::ANDROID_APP_VERSION);
        write_current_release_set(&current, published);
        fs::create_dir(project.path().join("APK")).unwrap();
        let name = versioned_artifact_name("tenfold", published, "apk");
        for directory in [project.path().to_path_buf(), project.path().join("APK")] {
            fs::copy(current.join(&name), directory.join(&name)).unwrap();
        }
        let (retained, version) =
            RetainedAndroidRelease::capture_published(project.path()).unwrap();
        assert_eq!(version, published);
        retained.verify_unchanged().unwrap();
        assert!(RetainedAndroidRelease::capture(
            project.path(),
            product_identity::ANDROID_APP_VERSION
        )
        .is_err());
        fs::write(project.path().join("APK").join(&name), b"tampered-apk").unwrap();
        assert!(RetainedAndroidRelease::capture_published(project.path()).is_err());
        assert!(retained.verify_unchanged().is_err());
    }

    #[test]
    fn mixed_versions_require_windows_only_manifest_and_unchanged_journal_apk() {
        let (project, retained, artifacts) = windows_only_fixture("windows-only-scope-proof");
        let mut journal = prepare_windows_only_fixture(&project, &retained, &artifacts, "t13p13");
        let paths = ReleaseTransactionPaths::derive(project.path(), &journal).unwrap();
        let manifest_path = paths
            .current_staging
            .join(product_identity::RELEASE_MANIFEST_FILE_NAME);
        let mut manifest = read_release_manifest_file(&manifest_path).unwrap();
        manifest.windows_only = false;
        manifest.passed_checks = release_passed_checks(false);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(derive_release_set_descriptor(&paths.current_staging).is_err());
        fs::remove_file(&manifest_path).unwrap();
        assert!(derive_release_set_descriptor(&paths.current_staging).is_err());
        journal.windows_only = false;
        journal.journal_checksum = compute_journal_checksum(&journal).unwrap();
        assert!(validate_transaction_journal(project.path(), &journal).is_err());
        journal.windows_only = true;
        journal.new_current.files[0].sha256 = "e".repeat(64);
        journal.journal_checksum = compute_journal_checksum(&journal).unwrap();
        assert!(validate_transaction_journal(project.path(), &journal).is_err());
    }

    #[test]
    fn legacy_journal_encoding_omits_windows_scope_and_unknown_fields_remain_rejected() {
        let (project, retained, artifacts) = windows_only_fixture("windows-only-journal-codec");
        let mut journal = prepare_windows_only_fixture(&project, &retained, &artifacts, "t14p14");
        journal.windows_only = false;
        let encoded = serde_json::to_value(&journal).unwrap();
        assert!(encoded.get("windows_only").is_none());
        let restored: ReleaseTransactionJournal = serde_json::from_value(encoded.clone()).unwrap();
        assert!(!restored.windows_only);
        assert_eq!(
            compute_journal_checksum(&journal).unwrap(),
            compute_journal_checksum(&restored).unwrap()
        );
        let mut unexpected = encoded;
        unexpected["allow_arbitrary_apk"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReleaseTransactionJournal>(unexpected).is_err());
    }

    fn sample_version(version_name: &str) -> ProjectVersionInfo {
        ProjectVersionInfo {
            application_id: "com.example.gridtimer".to_string(),
            debug_suffix: ".debug".to_string(),
            version_name: version_name.to_string(),
            version_code: 1,
        }
    }

    fn write_current_release_set(directory: &Path, version_name: &str) {
        fs::create_dir_all(directory).unwrap();
        for name in expected_current_release_names(version_name) {
            if name == product_identity::RELEASE_MANIFEST_FILE_NAME {
                continue;
            }
            write_non_empty(&directory.join(name));
        }
        let tools = directory.join("tools");
        fs::create_dir_all(&tools).unwrap();
        write_non_empty(&tools.join("cloudflared.exe"));
        let payload = derive_release_set_descriptor(directory).unwrap();
        write_release_manifest(
            directory,
            &payload,
            &BuildIdentity {
                git_commit: "0".repeat(40),
                source_worktree_dirty: true,
                source_snapshot_sha256: "a".repeat(64),
                toolchain: test_toolchain_identity(),
            },
        )
        .unwrap();
    }

    fn test_windows_artifacts(directory: &Path) -> WindowsReleaseArtifacts {
        let current = RELEASE_BINARY_NAMES
            .iter()
            .map(|(binary_name, public_base_name)| {
                let source = directory.join(format!("{binary_name}.exe"));
                write_non_empty(&source);
                WindowsReleaseArtifact {
                    public_base_name,
                    source,
                }
            })
            .collect();
        let stable_desktop_entry =
            directory.join(format!("{STABLE_DESKTOP_ENTRY_BINARY_NAME}.exe"));
        write_non_empty(&stable_desktop_entry);
        WindowsReleaseArtifacts {
            current,
            stable_desktop_entry,
            build_identity: BuildIdentity {
                git_commit: "0".repeat(40),
                source_worktree_dirty: true,
                source_snapshot_sha256: "a".repeat(64),
                toolchain: test_toolchain_identity(),
            },
        }
    }

    fn test_toolchain_identity() -> ReleaseToolchainIdentity {
        ReleaseToolchainIdentity {
            rustc: "rustc 1.95.0 (test)".to_string(),
            cargo: "cargo 1.95.0 (test)".to_string(),
            host: WINDOWS_MSVC_TARGET.to_string(),
            target: WINDOWS_MSVC_TARGET.to_string(),
            linker: r"C:\test\rust-lld.exe".to_string(),
        }
    }

    fn write_non_empty(path: &Path) {
        fs::write(path, b"test-artifact").unwrap();
    }

    fn sorted_entry_names(directory: &Path) -> Vec<String> {
        let mut names = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn count_files_with_extension(directory: &Path, extension: &str) -> usize {
        let Ok(entries) = fs::read_dir(directory) else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.path().is_file()
                    && entry
                        .path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
            .count()
    }
}
