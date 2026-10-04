// v1.1.0.3 Windows - Keep command-line resolution errors noninteractive.
// v0.0.1 - Recover synchronization when a retained supervisor mutex has no live owner.
// v1.0.1 - Preserve independent Android provenance with an optional validated source fingerprint.
// v1.0.1 - Verify independent Windows version reset with retained Android artifacts.
// v1.0.10 - Recover background sync without launching duplicate processes or pinning a version.
// v1.0.10 - Accept the formal tenfold APK name without weakening release identity.
// v1.0.9 - Reject mixed Windows executable versions in legacy release directories.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CLIENT_PREFIX: &str = "grid_timer_windows_client_v";
const SYNC_LAUNCHER_PREFIX: &str = "grid_timer_sync_launcher_v";
const SYNC_SERVER_PREFIX: &str = "grid_timer_sync_server_v";
const CLIENT_SUFFIX: &str = ".exe";
const RELEASE_TRANSACTION_FILE_NAME: &str = ".release-transaction-v1.json";
const RELEASE_TRANSACTION_MAGIC: &str = "TenRateReleaseTransaction";
const RELEASE_TRANSACTION_SCHEMA_VERSION: u32 = 1;
const STABLE_DESKTOP_ENTRY_FILE_NAME: &str = "TenRate_Desktop_Launcher.exe";
const RELEASE_MUTEX_PREFIX: &str = r"Local\TenRate.ReleaseArtifacts.";
const MAX_TRANSACTION_JOURNAL_BYTES: u64 = 1024 * 1024;
const RELEASE_MANIFEST_FILE_NAME: &str = "release_manifest.json";
const RELEASE_MUTEX_WAIT_MILLIS: u32 = 30_000;
const WAIT_OBJECT_0_RESULT: u32 = 0;
const WAIT_ABANDONED_RESULT: u32 = 0x80;
const WAIT_TIMEOUT_RESULT: u32 = 0x102;
const WAIT_FAILED_RESULT: u32 = 0xffff_ffff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MutexWaitDisposition {
    Acquired,
    AcquiredAbandoned,
    TimedOut,
    Failed,
    Unexpected(u32),
}

#[derive(Clone, Debug)]
struct ReleaseLayout {
    root: PathBuf,
    current: PathBuf,
    journal: PathBuf,
    project_identity: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    stable_new: FileDescriptor,
    stable_old: Option<FileDescriptor>,
    root_new: FileDescriptor,
    legacy_new: FileDescriptor,
    archive_plan: Vec<ArchivePlanEntry>,
    journal_checksum: String,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseSetDescriptor {
    version: String,
    files: Vec<FileDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileDescriptor {
    role: String,
    file_name: String,
    size: u64,
    sha256: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseManifest {
    schema_version: u32,
    product_id: String,
    display_name: String,
    version: String,
    #[serde(default)]
    windows_only: bool,
    sync_protocol_version: i64,
    git_commit: String,
    source_worktree_dirty: bool,
    source_snapshot_sha256: String,
    toolchain: ManifestToolchain,
    passed_checks: Vec<String>,
    created_at_epoch_millis: i64,
    files: Vec<FileDescriptor>,
    #[serde(default)]
    android_release: Option<AndroidReleaseProvenance>,
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestToolchain {
    rustc: String,
    cargo: String,
    host: String,
    target: String,
    linker: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchTarget {
    Client,
    SyncSupervisor,
}

impl LaunchTarget {
    fn prefix(self) -> &'static str {
        match self {
            Self::Client => CLIENT_PREFIX,
            Self::SyncSupervisor => SYNC_LAUNCHER_PREFIX,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Client => "Windows 客户端",
            Self::SyncSupervisor => "同步监督程序",
        }
    }
}

fn main() -> ExitCode {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let target = requested_target(&arguments);
    let resolving = arguments
        .iter()
        .any(|argument| argument == "--resolve-only");
    match run(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = format!(
                "十倍率未能启动。\n\n{}\n\n请确认 release_artifacts\\current 中存在正式{}。",
                error,
                target.description(),
            );
            append_log(&format!("ERROR {error}"));
            if target == LaunchTarget::Client && !resolving {
                show_error(&message);
            }
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<OsString>) -> io::Result<()> {
    let executable = env::current_exe()?;
    let layout = canonical_release_layout(&executable)?;
    let (target_kind, resolve_only) = parse_arguments(&arguments)?;
    let ensure_sync = arguments.iter().any(|argument| argument == "--ensure-sync") && !resolve_only;
    with_sync_recovery(ensure_sync, sync_supervisor_running, || {
        let _release_guard = acquire_release_mutex(&layout.root)?;
        recover_current_if_required(&layout)?;
        if resolve_only {
            let target = resolve_with_retry(&layout.current, target_kind)?;
            write_resolved_target(&target)?;
            return Ok(());
        }
        let target = launch_with_retry(&layout.current, target_kind)?;
        append_log(&format!("LAUNCH {}", target.display()));
        Ok(())
    })
}

fn with_sync_recovery(
    ensure_sync: bool,
    running: impl FnOnce() -> io::Result<bool>,
    launch: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    if ensure_sync && running()? {
        return Ok(());
    }
    launch()
}

#[cfg(target_os = "windows")]
fn sync_supervisor_running() -> io::Result<bool> {
    sync_supervisor_running_for_name(gridtimer_native::product_identity::SYNC_SUPERVISOR_MUTEX_NAME)
}

#[cfg(target_os = "windows")]
fn sync_supervisor_running_for_name(mutex_name: &str) -> io::Result<bool> {
    gridtimer_native::runtime::named_mutex::held_by_other_thread(mutex_name)
}

#[cfg(not(target_os = "windows"))]
fn sync_supervisor_running() -> io::Result<bool> {
    Ok(false)
}

fn requested_target(arguments: &[OsString]) -> LaunchTarget {
    if arguments
        .iter()
        .any(|argument| argument == "--sync-supervisor" || argument == "--ensure-sync")
    {
        LaunchTarget::SyncSupervisor
    } else {
        LaunchTarget::Client
    }
}

fn parse_arguments(arguments: &[OsString]) -> io::Result<(LaunchTarget, bool)> {
    let mut target = LaunchTarget::Client;
    let mut resolve_only = false;
    for argument in arguments {
        if argument == "--sync-supervisor" || argument == "--ensure-sync" {
            target = LaunchTarget::SyncSupervisor;
        } else if argument == "--resolve-only" {
            resolve_only = true;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("无法识别的稳定启动参数：{}", argument.to_string_lossy()),
            ));
        }
    }
    Ok((target, resolve_only))
}

fn write_resolved_target(target: &Path) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    let result = stdout
        .write_all(target.as_os_str().as_encoded_bytes())
        .and_then(|()| stdout.write_all(b"\n"));
    match result {
        Ok(()) => Ok(()),
        Err(error) if is_detached_output_error(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn is_detached_output_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::BrokenPipe || error.raw_os_error() == Some(232)
}

fn formal_current_directory(executable: &Path) -> io::Result<PathBuf> {
    Ok(release_directory_from_executable(executable)?.join("current"))
}

fn release_directory_from_executable(executable: &Path) -> io::Result<PathBuf> {
    let entry_directory = executable.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "稳定启动器没有可识别的父目录")
    })?;
    if !file_name_eq(entry_directory, "desktop_entry") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "稳定启动器必须位于 release_artifacts\\desktop_entry，实际位置：{}",
                entry_directory.display()
            ),
        ));
    }
    let release_directory = entry_directory.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "稳定启动器缺少 release_artifacts 父目录",
        )
    })?;
    if !file_name_eq(release_directory, "release_artifacts") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "稳定启动器必须位于 release_artifacts\\desktop_entry，实际位置：{}",
                entry_directory.display()
            ),
        ));
    }
    Ok(release_directory.to_path_buf())
}

fn canonical_release_layout(executable: &Path) -> io::Result<ReleaseLayout> {
    let lexical_root = release_directory_from_executable(executable)?;
    let lexical_metadata = fs::symlink_metadata(&lexical_root)?;
    if !metadata_is_real_directory(&lexical_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "release_artifacts 必须是非 reparse 的真实目录：{}",
                lexical_root.display()
            ),
        ));
    }
    let root = fs::canonicalize(&lexical_root).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "无法规范化正式 release_artifacts 目录 {}: {error}",
                lexical_root.display()
            ),
        )
    })?;
    let metadata = fs::symlink_metadata(&root)?;
    if !metadata_is_real_directory(&metadata) || !file_name_eq(&root, "release_artifacts") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("release_artifacts 必须是同名真实目录：{}", root.display()),
        ));
    }
    let project_identity = release_root_identity(&root);
    Ok(ReleaseLayout {
        current: root.join("current"),
        journal: root.join(RELEASE_TRANSACTION_FILE_NAME),
        root,
        project_identity,
    })
}

fn normalized_release_root(root: &Path) -> String {
    normalize_release_root_text(&root.to_string_lossy())
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
    while normalized.ends_with('\\') && !normalized.ends_with(":\\") {
        normalized.pop();
    }
    normalized
}

fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        windows_file_attributes_are_reparse_point(metadata.file_attributes())
    }
    #[cfg(not(target_os = "windows"))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(target_os = "windows")]
fn windows_file_attributes_are_reparse_point(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn metadata_is_real_directory(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_dir() && !metadata_is_reparse_point(metadata)
}

fn metadata_is_real_file(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file() && !metadata_is_reparse_point(metadata)
}

fn release_root_identity(root: &Path) -> String {
    sha256_bytes(normalized_release_root(root).as_bytes())
}

fn release_mutex_name(root: &Path) -> String {
    format!("{RELEASE_MUTEX_PREFIX}{}", release_root_identity(root))
}

fn classify_mutex_wait_result(result: u32) -> MutexWaitDisposition {
    match result {
        WAIT_OBJECT_0_RESULT => MutexWaitDisposition::Acquired,
        WAIT_ABANDONED_RESULT => MutexWaitDisposition::AcquiredAbandoned,
        WAIT_TIMEOUT_RESULT => MutexWaitDisposition::TimedOut,
        WAIT_FAILED_RESULT => MutexWaitDisposition::Failed,
        other => MutexWaitDisposition::Unexpected(other),
    }
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
struct ReleaseMutexGuard(*mut core::ffi::c_void);

#[cfg(target_os = "windows")]
impl Drop for ReleaseMutexGuard {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn ReleaseMutex(mutex: *mut core::ffi::c_void) -> i32;
            fn CloseHandle(object: *mut core::ffi::c_void) -> i32;
        }
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn acquire_release_mutex(release_root: &Path) -> io::Result<ReleaseMutexGuard> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attributes: *mut core::ffi::c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut core::ffi::c_void;
        fn WaitForSingleObject(handle: *mut core::ffi::c_void, milliseconds: u32) -> u32;
        fn CloseHandle(object: *mut core::ffi::c_void) -> i32;
    }

    let name = std::ffi::OsStr::new(&release_mutex_name(release_root))
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if mutex.is_null() {
        return Err(io::Error::last_os_error());
    }
    let wait_result = unsafe { WaitForSingleObject(mutex, RELEASE_MUTEX_WAIT_MILLIS) };
    match classify_mutex_wait_result(wait_result) {
        MutexWaitDisposition::Acquired | MutexWaitDisposition::AcquiredAbandoned => {
            Ok(ReleaseMutexGuard(mutex))
        }
        disposition => {
            let wait_error = if disposition == MutexWaitDisposition::Failed {
                io::Error::last_os_error()
            } else if disposition == MutexWaitDisposition::TimedOut {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("等待正式发布互斥锁超过 {} 毫秒", RELEASE_MUTEX_WAIT_MILLIS),
                )
            } else {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("等待正式发布互斥锁失败，结果码 {wait_result:#x}"),
                )
            };
            unsafe {
                let _ = CloseHandle(mutex);
            }
            Err(wait_error)
        }
    }
}

#[cfg(not(target_os = "windows"))]
struct ReleaseMutexGuard;

#[cfg(not(target_os = "windows"))]
fn acquire_release_mutex(_release_root: &Path) -> io::Result<ReleaseMutexGuard> {
    Ok(ReleaseMutexGuard)
}

fn recover_current_if_required(layout: &ReleaseLayout) -> io::Result<()> {
    let Some(journal) = read_and_validate_transaction_journal(layout)? else {
        return validate_canonical_current_path_type_if_present(&layout.current);
    };
    let state = classify_canonical_current(layout, &journal)?;
    if journal.phase == ReleaseTransactionPhase::Committed {
        return match state {
            CanonicalCurrentState::New => Ok(()),
            CanonicalCurrentState::Missing => {
                let staging = layout.root.join(&journal.current_staging);
                validate_release_directory(&staging, &journal.new_current)?;
                restore_directory_atomically(&staging, &layout.current, &journal.new_current)
            }
            CanonicalCurrentState::Old => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Committed 事务的 canonical current 仍是旧版本",
            )),
        };
    }

    let Some(old_descriptor) = &journal.old_current else {
        return match state {
            CanonicalCurrentState::Missing => Ok(()),
            CanonicalCurrentState::New => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "首次发布尚未 Committed，拒绝启动未提交的新 current",
            )),
            CanonicalCurrentState::Old => unreachable!("old state requires an old descriptor"),
        };
    };
    match state {
        CanonicalCurrentState::Old => Ok(()),
        CanonicalCurrentState::Missing => {
            let backup = layout.root.join(&journal.current_backup);
            validate_release_directory(&backup, old_descriptor)?;
            restore_directory_atomically(&backup, &layout.current, old_descriptor)
        }
        CanonicalCurrentState::New => {
            let backup = layout.root.join(&journal.current_backup);
            validate_release_directory(&backup, old_descriptor)?;
            let staging = layout.root.join(&journal.current_staging);
            match fs::symlink_metadata(&staging) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "回滚未提交 current 时 staging 已存在：{}",
                            staging.display()
                        ),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(&layout.current, &staging)?;
            if let Err(error) = fs::rename(&backup, &layout.current) {
                let rollback = fs::rename(&staging, &layout.current);
                return match rollback {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("恢复旧 current 失败 ({error})；恢复新 current 也失败 ({rollback_error})"),
                    )),
                };
            }
            validate_release_directory(&layout.current, old_descriptor)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalCurrentState {
    Missing,
    Old,
    New,
}

fn validate_canonical_current_path_type_if_present(current: &Path) -> io::Result<()> {
    match fs::symlink_metadata(current) {
        Ok(metadata) if metadata_is_real_directory(&metadata) => {
            validate_current_manifest_if_present(current)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("正式发布槽必须是真实目录：{}", current.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_current_manifest_if_present(current: &Path) -> io::Result<()> {
    let manifest_path = current.join(RELEASE_MANIFEST_FILE_NAME);
    match fs::symlink_metadata(&manifest_path) {
        Ok(_) => {
            let manifest = read_release_manifest(&manifest_path)?;
            let mut files = manifest.files.clone();
            files.push(FileDescriptor {
                role: "release_manifest".to_string(),
                file_name: RELEASE_MANIFEST_FILE_NAME.to_string(),
                size: fs::metadata(&manifest_path)?.len(),
                sha256: sha256_file(&manifest_path)?,
            });
            validate_release_directory(
                current,
                &ReleaseSetDescriptor {
                    version: manifest.version,
                    files,
                },
            )
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut apk_versions = BTreeSet::new();
            let mut windows_versions = BTreeSet::new();
            for entry in fs::read_dir(current)? {
                let name = entry?.file_name().to_string_lossy().to_ascii_lowercase();
                if let Some(version) = name
                    .strip_prefix("grid_timer_app_v")
                    .or_else(|| name.strip_prefix("tenfold_v"))
                    .and_then(|value| value.strip_suffix(".apk"))
                {
                    apk_versions.insert(version.to_string());
                }
                for prefix in [CLIENT_PREFIX, SYNC_LAUNCHER_PREFIX, SYNC_SERVER_PREFIX] {
                    if let Some(version) = name
                        .strip_prefix(prefix)
                        .and_then(|value| value.strip_suffix(CLIENT_SUFFIX))
                    {
                        windows_versions.insert(version.to_string());
                    }
                }
            }
            if windows_versions.len() > 1 {
                return Err(invalid_journal("Windows 客户端与同步组件版本不一致"));
            }
            if !apk_versions.is_empty() && apk_versions != windows_versions {
                return Err(invalid_journal("不同平台版本缺少 Windows-only 发布清单"));
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn classify_canonical_current(
    layout: &ReleaseLayout,
    journal: &ReleaseTransactionJournal,
) -> io::Result<CanonicalCurrentState> {
    match fs::symlink_metadata(&layout.current) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CanonicalCurrentState::Missing)
        }
        Err(error) => return Err(error),
        Ok(metadata) if !metadata_is_real_directory(&metadata) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("正式发布槽必须是真实目录：{}", layout.current.display()),
            ))
        }
        Ok(_) => {}
    }
    let old_matches = || {
        journal
            .old_current
            .as_ref()
            .is_some_and(|old| validate_release_directory(&layout.current, old).is_ok())
    };
    let new_matches = || validate_release_directory(&layout.current, &journal.new_current).is_ok();
    if journal.phase == ReleaseTransactionPhase::Committed {
        if new_matches() {
            return Ok(CanonicalCurrentState::New);
        }
        if old_matches() {
            return Ok(CanonicalCurrentState::Old);
        }
    } else {
        if old_matches() {
            return Ok(CanonicalCurrentState::Old);
        }
        if new_matches() {
            return Ok(CanonicalCurrentState::New);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "canonical current 与事务的新旧 descriptor 均不匹配",
    ))
}

fn restore_directory_atomically(
    source: &Path,
    current: &Path,
    descriptor: &ReleaseSetDescriptor,
) -> io::Result<()> {
    match fs::symlink_metadata(current) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("恢复前正式发布槽已出现：{}", current.display()),
            ))
        }
        Err(error) => return Err(error),
    }
    fs::rename(source, current).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "无法原子恢复 {} 到 {}: {error}",
                source.display(),
                current.display()
            ),
        )
    })?;
    validate_release_directory(current, descriptor)
}

fn read_and_validate_transaction_journal(
    layout: &ReleaseLayout,
) -> io::Result<Option<ReleaseTransactionJournal>> {
    let metadata = match fs::symlink_metadata(&layout.journal) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata_is_real_file(&metadata) || !transaction_journal_size_is_allowed(metadata.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "发布事务日志必须是大小受限的真实文件：{}",
                layout.journal.display()
            ),
        ));
    }
    let bytes = fs::read(&layout.journal)?;
    let journal: ReleaseTransactionJournal = serde_json::from_slice(&bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("发布事务日志 JSON 无效：{error}"),
        )
    })?;
    validate_transaction_journal(&journal, &layout.project_identity)?;
    Ok(Some(journal))
}

fn transaction_journal_size_is_allowed(size: u64) -> bool {
    size > 0 && size <= MAX_TRANSACTION_JOURNAL_BYTES
}

fn validate_transaction_journal(
    journal: &ReleaseTransactionJournal,
    expected_project_identity: &str,
) -> io::Result<()> {
    if journal.magic != RELEASE_TRANSACTION_MAGIC
        || journal.schema_version != RELEASE_TRANSACTION_SCHEMA_VERSION
    {
        return Err(invalid_journal("magic 或 schema_version 不匹配"));
    }
    if !valid_transaction_id(&journal.transaction_id) {
        return Err(invalid_journal("transaction_id 格式无效"));
    }
    if journal.project_identity != expected_project_identity {
        return Err(invalid_journal("project_identity 与规范发布目录不匹配"));
    }
    validate_sha256(&journal.journal_checksum, "journal_checksum")?;
    let mut checksum_input = journal.clone();
    checksum_input.journal_checksum.clear();
    let canonical_bytes = serde_json::to_vec(&checksum_input).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("无法规范化发布事务日志：{error}"),
        )
    })?;
    if sha256_bytes(&canonical_bytes) != journal.journal_checksum {
        return Err(invalid_journal("journal_checksum 校验失败"));
    }

    let txid = &journal.transaction_id;
    validate_exact_basename(
        &journal.current_staging,
        &format!(".current-staging-{txid}"),
    )?;
    validate_exact_basename(&journal.current_backup, &format!(".current-backup-{txid}"))?;
    validate_exact_basename(
        &journal.stable_staging,
        &format!(".stable-staging-{txid}.exe"),
    )?;
    validate_exact_basename(
        &journal.stable_backup,
        &format!(".stable-backup-{txid}.exe"),
    )?;
    validate_exact_basename(&journal.transaction_root, &format!(".transaction-{txid}"))?;

    if !valid_release_version(&journal.new_version) {
        return Err(invalid_journal("new_version 无效"));
    }
    validate_release_set_descriptor_with_scope(
        &journal.new_current,
        &journal.new_version,
        journal.windows_only,
    )?;
    match (&journal.old_version, &journal.old_current) {
        (Some(version), Some(descriptor)) if valid_release_version(version) => {
            validate_release_set_descriptor_with_scope(descriptor, version, true)?;
        }
        (None, None) => {}
        _ => return Err(invalid_journal("old_version 与 old_current 不一致")),
    }
    validate_file_descriptor(&journal.stable_new)?;
    if journal.stable_new.role != "stable"
        || journal.stable_new.file_name != STABLE_DESKTOP_ENTRY_FILE_NAME
    {
        return Err(invalid_journal("stable_new 身份无效"));
    }
    if let Some(stable_old) = &journal.stable_old {
        validate_file_descriptor(stable_old)?;
        if stable_old.role != "stable" || stable_old.file_name != STABLE_DESKTOP_ENTRY_FILE_NAME {
            return Err(invalid_journal("stable_old 身份无效"));
        }
    }
    validate_file_descriptor(&journal.root_new)?;
    let apk = descriptor_apk(&journal.new_current)?;
    if journal.windows_only {
        let old = journal
            .old_current
            .as_ref()
            .ok_or_else(|| invalid_journal("Windows-only 事务缺少原发布集"))?;
        if descriptor_apk(old)? != apk || journal.new_current.files.len() != 6 {
            return Err(invalid_journal(
                "Windows-only 事务未保留原 APK 身份或发布清单",
            ));
        }
    }
    if journal.root_new.role != "root_apk"
        || journal.root_new.file_name != apk.file_name
        || (journal.windows_only
            && (journal.root_new.size != apk.size || journal.root_new.sha256 != apk.sha256))
    {
        return Err(invalid_journal("root_new 身份无效"));
    }
    validate_file_descriptor(&journal.legacy_new)?;
    if journal.legacy_new.role != "legacy_apk"
        || journal.legacy_new.file_name != journal.root_new.file_name
        || journal.legacy_new.size != journal.root_new.size
        || journal.legacy_new.sha256 != journal.root_new.sha256
    {
        return Err(invalid_journal("legacy_new 身份无效"));
    }
    let mut archive_destinations = BTreeSet::new();
    for entry in &journal.archive_plan {
        validate_archive_plan_entry(entry, &journal.transaction_id)?;
        if journal.windows_only && entry.destination_kind != "old_exes" {
            return Err(invalid_journal("Windows-only 事务不能归档 APK"));
        }
        if !archive_destinations.insert((
            entry.destination_kind.clone(),
            entry.destination_file_name.clone(),
        )) {
            return Err(invalid_journal("archive_plan 包含重复目标"));
        }
    }
    Ok(())
}

fn valid_transaction_id(value: &str) -> bool {
    let Some(after_t) = value.strip_prefix('t') else {
        return false;
    };
    let Some((millis, pid)) = after_t.split_once('p') else {
        return false;
    };
    !millis.is_empty()
        && !pid.is_empty()
        && millis.bytes().all(|byte| byte.is_ascii_digit())
        && pid.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_exact_basename(actual: &str, expected: &str) -> io::Result<()> {
    validate_basename(actual)?;
    if actual != expected {
        return Err(invalid_journal(&format!(
            "事务路径必须为 {expected}，实际为 {actual}"
        )));
    }
    Ok(())
}

fn validate_basename(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', ':', '\0'])
        || Path::new(value).file_name().and_then(|name| name.to_str()) != Some(value)
    {
        return Err(invalid_journal(&format!("不是安全的 basename：{value:?}")));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> io::Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid_journal(&format!("{label} 不是小写 SHA-256")));
    }
    Ok(())
}

fn validate_file_descriptor(descriptor: &FileDescriptor) -> io::Result<()> {
    if descriptor.role.is_empty()
        || !descriptor
            .role
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        || descriptor.size == 0
    {
        return Err(invalid_journal("文件 descriptor 的 role 或 size 无效"));
    }
    validate_basename(&descriptor.file_name)?;
    validate_sha256(&descriptor.sha256, "descriptor sha256")
}

fn validate_release_set_descriptor(
    descriptor: &ReleaseSetDescriptor,
    expected_version: &str,
) -> io::Result<()> {
    validate_release_set_descriptor_with_scope(descriptor, expected_version, false)
}

fn descriptor_apk(descriptor: &ReleaseSetDescriptor) -> io::Result<&FileDescriptor> {
    descriptor
        .files
        .iter()
        .find(|file| file.role == "apk")
        .ok_or_else(|| invalid_journal("发布集缺少 APK 身份"))
}

fn descriptor_apk_version(descriptor: &ReleaseSetDescriptor) -> io::Result<&str> {
    let name = &descriptor_apk(descriptor)?.file_name;
    name.strip_prefix("grid_timer_app_v")
        .or_else(|| name.strip_prefix("tenfold_v"))
        .and_then(|value| value.strip_suffix(".apk"))
        .filter(|value| valid_release_version(value))
        .ok_or_else(|| invalid_journal("APK 版本或文件名无效"))
}

fn validate_release_set_descriptor_with_scope(
    descriptor: &ReleaseSetDescriptor,
    expected_version: &str,
    windows_only: bool,
) -> io::Result<()> {
    const ROLES: [&str; 5] = [
        "apk",
        "sync_server",
        "sync_launcher",
        "windows_client",
        "cloudflared",
    ];
    if descriptor.version != expected_version || !matches!(descriptor.files.len(), 5 | 6) {
        return Err(invalid_journal("发布集 descriptor 的版本或文件数无效"));
    }
    let mixed_platforms = descriptor_apk_version(descriptor)? != expected_version;
    if mixed_platforms && (!windows_only || descriptor.files.len() != 6) {
        return Err(invalid_journal(
            "不同平台版本必须有明确的 Windows-only 发布清单",
        ));
    }
    let mut roles = ROLES.to_vec();
    if descriptor.files.len() == 6 {
        roles.push("release_manifest");
    }
    for (index, role) in roles.into_iter().enumerate() {
        let matches = descriptor
            .files
            .iter()
            .filter(|file| file.role == role)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(invalid_journal(&format!(
                "发布集 role {role} 必须恰好出现一次"
            )));
        }
        let file = matches[0];
        if descriptor.files[index].role != role {
            return Err(invalid_journal("发布集 descriptor 顺序无效"));
        }
        validate_file_descriptor(file)?;
        let version = if role == "apk" && windows_only {
            descriptor_apk_version(descriptor)?
        } else {
            expected_version
        };
        let expected_name = expected_release_file_name(role, version);
        if file.file_name != expected_name
            && !(role == "apk" && file.file_name == format!("tenfold_v{version}.apk"))
        {
            return Err(invalid_journal(&format!(
                "role {role} 的文件名必须为 {expected_name}"
            )));
        }
    }
    Ok(())
}

fn expected_release_file_name(role: &str, version: &str) -> String {
    match role {
        "apk" => format!("grid_timer_app_v{version}.apk"),
        "sync_server" => format!("grid_timer_sync_server_v{version}.exe"),
        "sync_launcher" => format!("grid_timer_sync_launcher_v{version}.exe"),
        "windows_client" => format!("grid_timer_windows_client_v{version}.exe"),
        "cloudflared" => "cloudflared.exe".to_string(),
        "release_manifest" => RELEASE_MANIFEST_FILE_NAME.to_string(),
        _ => unreachable!("release role was checked before filename construction"),
    }
}

fn validate_archive_plan_entry(entry: &ArchivePlanEntry, transaction_id: &str) -> io::Result<()> {
    validate_basename(&entry.source_file_name)?;
    validate_basename(&entry.destination_file_name)?;
    validate_sha256(&entry.sha256, "archive sha256")?;
    if entry.size == 0
        || !matches!(
            entry.source_kind.as_str(),
            "current_backup" | "stable_backup" | "project_root" | "legacy_apk"
        )
        || !matches!(entry.destination_kind.as_str(), "old_apks" | "old_exes")
    {
        return Err(invalid_journal("archive_plan kind 或 size 无效"));
    }
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
    if !valid_role
        || entry.destination_kind != expected_destination_kind
        || entry.destination_file_name
            != deterministic_archive_name(&entry.source_file_name, source_tag, transaction_id)?
    {
        return Err(invalid_journal("archive_plan source_role 无效"));
    }
    Ok(())
}

fn deterministic_archive_name(
    file_name: &str,
    source_tag: &str,
    transaction_id: &str,
) -> io::Result<String> {
    validate_basename(file_name)?;
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid_journal("archive source 缺少文件 stem"))?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let destination = format!("{stem}_{source_tag}_{transaction_id}{extension}");
    validate_basename(&destination)?;
    Ok(destination)
}

fn invalid_journal(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("发布事务日志无效：{message}"),
    )
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_release_directory(
    directory: &Path,
    descriptor: &ReleaseSetDescriptor,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata_is_real_directory(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("恢复候选必须是真实目录：{}", directory.display()),
        ));
    }
    let manifest = if descriptor
        .files
        .iter()
        .any(|file| file.role == "release_manifest")
    {
        Some(read_release_manifest(
            &directory.join(RELEASE_MANIFEST_FILE_NAME),
        )?)
    } else {
        None
    };
    validate_release_set_descriptor_with_scope(
        descriptor,
        &descriptor.version,
        manifest.as_ref().is_some_and(|value| value.windows_only),
    )?;
    if let Some(manifest) = &manifest {
        validate_release_manifest(manifest, descriptor)?;
    }

    let mut expected_root_names = descriptor
        .files
        .iter()
        .filter(|file| file.role != "cloudflared")
        .map(|file| file.file_name.clone())
        .collect::<Vec<_>>();
    expected_root_names.push("tools".to_string());
    expected_root_names.sort();
    let mut actual_root_names = real_directory_entry_names(directory)?;
    actual_root_names.sort();
    if actual_root_names != expected_root_names {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("恢复候选根目录不是精确发布集：{}", directory.display()),
        ));
    }

    let tools = directory.join("tools");
    let tools_metadata = fs::symlink_metadata(&tools)?;
    if !metadata_is_real_directory(&tools_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "恢复候选 tools 必须是真实目录",
        ));
    }
    let mut tool_names = real_directory_entry_names(&tools)?;
    tool_names.sort();
    if tool_names != vec!["cloudflared.exe".to_string()] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "恢复候选 tools 目录不是精确发布集",
        ));
    }

    for file in &descriptor.files {
        let path = if file.role == "cloudflared" {
            tools.join(&file.file_name)
        } else {
            directory.join(&file.file_name)
        };
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata_is_real_file(&metadata)
            || metadata.len() != file.size
            || sha256_file(&path)? != file.sha256
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("恢复候选文件与 descriptor 不匹配：{}", path.display()),
            ));
        }
        if file.file_name.ends_with(".exe") {
            validate_pe_executable(&path, metadata.len(), LaunchTarget::Client)?;
        } else if file.role == "apk" {
            validate_apk_file(&path, metadata.len())?;
        }
    }
    Ok(())
}

fn read_release_manifest(path: &Path) -> io::Result<ReleaseManifest> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata_is_real_file(&metadata) || !transaction_journal_size_is_allowed(metadata.len()) {
        return Err(invalid_journal("发布清单不是有界真实文件"));
    }
    let bytes = fs::read(path)?;
    if !transaction_journal_size_is_allowed(bytes.len() as u64) {
        return Err(invalid_journal("发布清单读取时大小改变"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| invalid_journal(&format!("发布清单 JSON 无效：{error}")))
}

fn validate_release_manifest(
    manifest: &ReleaseManifest,
    descriptor: &ReleaseSetDescriptor,
) -> io::Result<()> {
    if let Some(android) = &manifest.android_release {
        let apk = descriptor
            .files
            .iter()
            .find(|file| file.role == "apk")
            .ok_or_else(|| invalid_journal("发布清单缺少安卓文件"))?;
        if !manifest.windows_only
            || !android.android_only
            || android.version != descriptor_apk_version(descriptor)?
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
            return Err(invalid_journal("安卓发布记录与保留安装包不一致"));
        }
    }
    let mut checks = vec![
        "cargo_fmt",
        "core_library_tests",
        "desktop_media_tests",
        "windows_client_tests",
        "sync_launcher_tests",
        "packager_tests",
        "stable_desktop_entry_tests",
    ];
    if manifest.windows_only {
        checks.extend([
            "retained_android_signature_verification",
            "retained_android_bytes_and_modified_time",
        ]);
    } else {
        checks.extend([
            "android_unit_tests",
            "android_lint",
            "android_release_build",
            "android_signature_verification",
        ]);
    }
    checks.extend([
        "windows_executable_integrity_chain",
        "windows_server_runtime_smoke",
    ]);
    let payload = descriptor
        .files
        .iter()
        .filter(|file| file.role != "release_manifest")
        .cloned()
        .collect::<Vec<_>>();
    validate_sha256(&manifest.source_snapshot_sha256, "source snapshot")?;
    let valid_git = manifest.git_commit == "unknown"
        || (matches!(manifest.git_commit.len(), 40 | 64)
            && manifest
                .git_commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()));
    let _source_worktree_dirty = manifest.source_worktree_dirty;
    if manifest.schema_version != 1
        || manifest.product_id != "gridtimer"
        || manifest.display_name != "十倍率"
        || manifest.version != descriptor.version
        || manifest.sync_protocol_version != 1
        || !valid_git
        || manifest.created_at_epoch_millis <= 0
        || manifest.files != payload
        || manifest.passed_checks != checks
        || !manifest.toolchain.rustc.starts_with("rustc ")
        || !manifest.toolchain.cargo.starts_with("cargo ")
        || manifest.toolchain.host != "x86_64-pc-windows-msvc"
        || manifest.toolchain.target != "x86_64-pc-windows-msvc"
        || manifest.toolchain.linker.trim().is_empty()
        || manifest.toolchain.linker.chars().any(char::is_control)
        || (!manifest.windows_only && descriptor_apk_version(descriptor)? != descriptor.version)
    {
        return Err(invalid_journal(
            "发布清单与平台范围、源代码身份或文件描述符不一致",
        ));
    }
    Ok(())
}

fn real_directory_entry_names(directory: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "发布目录包含非 Unicode 名称")
        })?;
        names.push(name);
    }
    Ok(names)
}

fn validate_apk_file(path: &Path, length: u64) -> io::Result<()> {
    if length < 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "APK 文件过短"));
    }
    let mut signature = [0_u8; 4];
    File::open(path)?.read_exact(&mut signature)?;
    if signature != *b"PK\x03\x04" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("APK 不是有效 ZIP 容器：{}", path.display()),
        ));
    }
    Ok(())
}

fn resolve_with_retry(current_directory: &Path, target_kind: LaunchTarget) -> io::Result<PathBuf> {
    let mut last_error = None;
    for attempt in 0..6 {
        match find_unique_formal_executable(current_directory, target_kind) {
            Ok(Some(candidate)) => return Ok(candidate),
            Ok(None) => last_error = Some(missing_target_error(current_directory, target_kind)),
            Err(error) if is_transient_release_switch_error(&error) => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
        wait_before_retry(attempt);
    }
    Err(last_error.unwrap_or_else(|| missing_target_error(current_directory, target_kind)))
}

fn launch_with_retry(current_directory: &Path, target_kind: LaunchTarget) -> io::Result<PathBuf> {
    let mut last_error = None;
    for attempt in 0..6 {
        let target = match find_unique_formal_executable(current_directory, target_kind) {
            Ok(Some(candidate)) => candidate,
            Ok(None) => {
                last_error = Some(missing_target_error(current_directory, target_kind));
                wait_before_retry(attempt);
                continue;
            }
            Err(error) if is_transient_release_switch_error(&error) => {
                last_error = Some(error);
                wait_before_retry(attempt);
                continue;
            }
            Err(error) => return Err(error),
        };

        match Command::new(&target).current_dir(current_directory).spawn() {
            Ok(_) => return Ok(target),
            Err(error) if is_transient_release_switch_error(&error) => {
                last_error = Some(error);
                wait_before_retry(attempt);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| missing_target_error(current_directory, target_kind)))
}

fn missing_target_error(current_directory: &Path, target_kind: LaunchTarget) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "找不到正式{}。已检查：{}",
            target_kind.description(),
            current_directory.display()
        ),
    )
}

fn wait_before_retry(attempt: usize) {
    if attempt < 5 {
        thread::sleep(Duration::from_millis((75_u64 << attempt).min(600)));
    }
}

fn is_transient_release_switch_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || matches!(error.raw_os_error(), Some(32 | 33))
}

fn find_unique_formal_executable(
    directory: &Path,
    target_kind: LaunchTarget,
) -> io::Result<Option<PathBuf>> {
    let directory_metadata = match directory.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata_is_real_directory(&directory_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "正式发布槽必须是真实目录，不能是文件或链接：{}",
                directory.display()
            ),
        ));
    }

    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let mut client: Option<PathBuf> = None;
    for entry in entries {
        let entry = entry?;
        if !is_formal_executable_name(&entry.file_name(), target_kind) {
            continue;
        }
        let metadata = entry.path().symlink_metadata()?;
        if !metadata_is_real_file(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "正式{}必须是普通文件：{}",
                    target_kind.description(),
                    entry.path().display()
                ),
            ));
        }
        validate_pe_executable(&entry.path(), metadata.len(), target_kind)?;
        if let Some(existing) = &client {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "正式发布槽内存在多个{}：{}；{}",
                    target_kind.description(),
                    existing.display(),
                    entry.path().display()
                ),
            ));
        }
        client = Some(entry.path());
    }
    Ok(client)
}

fn is_formal_executable_name(name: &OsString, target_kind: LaunchTarget) -> bool {
    let name = name.to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let prefix = target_kind.prefix();
    if !lower.starts_with(prefix) || !lower.ends_with(CLIENT_SUFFIX) {
        return false;
    }
    let version_text = &name[prefix.len()..name.len() - CLIENT_SUFFIX.len()];
    valid_release_version(version_text)
}

fn valid_release_version(version: &str) -> bool {
    let mut parts = version.split('-');
    let Some(numeric) = parts.next() else {
        return false;
    };
    if numeric.is_empty()
        || numeric
            .split('.')
            .any(|component| component.is_empty() || !component.chars().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    for qualifier in parts {
        if qualifier.is_empty()
            || !qualifier
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return false;
        }
    }
    true
}

fn validate_pe_executable(path: &Path, length: u64, target_kind: LaunchTarget) -> io::Result<()> {
    if length < 68 {
        return Err(invalid_executable_error(path, target_kind));
    }
    let mut file = File::open(path)?;
    let mut dos_header = [0_u8; 64];
    file.read_exact(&mut dos_header)?;
    if &dos_header[..2] != b"MZ" {
        return Err(invalid_executable_error(path, target_kind));
    }
    let pe_offset = u32::from_le_bytes(
        dos_header[60..64]
            .try_into()
            .expect("PE offset slice has fixed width"),
    ) as u64;
    if pe_offset > length.saturating_sub(4) {
        return Err(invalid_executable_error(path, target_kind));
    }
    file.seek(SeekFrom::Start(pe_offset))?;
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature)?;
    if &signature != b"PE\0\0" {
        return Err(invalid_executable_error(path, target_kind));
    }
    Ok(())
}

fn invalid_executable_error(path: &Path, target_kind: LaunchTarget) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "正式{}不是有效的 Windows PE 文件：{}",
            target_kind.description(),
            path.display()
        ),
    )
}

fn append_log(message: &str) {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA") else {
        return;
    };
    let directory = PathBuf::from(local_app_data).join("TenRate");
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("desktop_launcher.log"))
    else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let safe_message = message.replace(['\r', '\n'], " ");
    let _ = writeln!(file, "{timestamp} {safe_message}");
}

#[cfg(target_os = "windows")]
fn show_error(message: &str) {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            window: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }

    let text = std::ffi::OsStr::new(message)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let caption = std::ffi::OsStr::new("十倍率")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            0x0000_0010,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn show_error(message: &str) {
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    mod mutex_fixture {
        include!("../runtime/mutex_test_support.rs");
    }
    #[test]
    #[cfg(windows)]
    fn mutex_unowned_object_does_not_skip_sync_recovery() {
        let name = mutex_fixture::unique_name("entry-unowned");
        let _object = mutex_fixture::unowned(&name);
        assert!(
            !sync_supervisor_running_for_name(&name).unwrap(),
            "a retained unowned mutex is not a live supervisor"
        );
    }

    #[test]
    #[cfg(windows)]
    fn mutex_live_owner_suppresses_duplicate_and_orderly_exit_allows_recovery() {
        let name = mutex_fixture::unique_name("entry-live");
        let owner = mutex_fixture::owned(&name);
        let _retained = mutex_fixture::unowned(&name);
        let other_name = name.clone();
        let launched = std::thread::spawn(move || {
            let mut launched = false;
            with_sync_recovery(
                true,
                || sync_supervisor_running_for_name(&other_name),
                || {
                    launched = true;
                    Ok(())
                },
            )
            .unwrap();
            launched
        })
        .join()
        .unwrap();
        assert!(!launched, "live supervisor must not be duplicated");
        drop(owner);
        let mut launched = false;
        with_sync_recovery(
            true,
            || sync_supervisor_running_for_name(&name),
            || {
                launched = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(
            launched,
            "retained object must not suppress recovery after orderly exit"
        );
    }

    #[test]
    #[cfg(windows)]
    fn mutex_liveness_after_owner_process_exit() {
        if mutex_fixture::owner_child_if_requested() {
            return;
        }
        let fixture = mutex_fixture::abandoned("entry-crash");
        let mut launched = false;
        with_sync_recovery(
            true,
            || sync_supervisor_running_for_name(&fixture.name),
            || {
                launched = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(
            launched,
            "exited owner must not be treated as a running supervisor"
        );
        assert!(
            !sync_supervisor_running_for_name(&fixture.name).unwrap(),
            "liveness probe must release its temporary ownership"
        );
    }

    #[test]
    #[cfg(windows)]
    fn mutex_probe_failure_does_not_authorize_a_duplicate_start() {
        let name = mutex_fixture::unique_name("entry-wrong-object");
        let _wrong = mutex_fixture::wrong_object(&name);
        let mut launched = false;
        assert!(with_sync_recovery(
            true,
            || sync_supervisor_running_for_name(&name),
            || {
                launched = true;
                Ok(())
            }
        )
        .is_err());
        assert!(!launched);
    }
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn parses_versioned_client_names_only() {
        assert!(is_formal_executable_name(
            &OsString::from("grid_timer_windows_client_v2.22.19-windows-knowledge.exe"),
            LaunchTarget::Client,
        ));
        assert!(is_formal_executable_name(
            &OsString::from("grid_timer_sync_launcher_v2.22.19-windows-knowledge.exe"),
            LaunchTarget::SyncSupervisor,
        ));
        assert!(!is_formal_executable_name(
            &OsString::from("grid_timer_sync_server_v9.0.exe"),
            LaunchTarget::SyncSupervisor,
        ));
        assert!(!is_formal_executable_name(
            &OsString::from("grid_timer_windows_client_v2.22.apk"),
            LaunchTarget::Client,
        ));
    }

    #[test]
    fn parses_only_the_supported_stable_entry_modes() {
        assert_eq!(
            (LaunchTarget::Client, false),
            parse_arguments(&[]).expect("default client mode")
        );
        assert_eq!(
            (LaunchTarget::SyncSupervisor, true),
            parse_arguments(&[
                OsString::from("--resolve-only"),
                OsString::from("--sync-supervisor"),
            ])
            .expect("sync resolve mode")
        );
        assert_eq!(
            io::ErrorKind::InvalidInput,
            parse_arguments(&[OsString::from("--unknown")])
                .expect_err("reject unknown arguments")
                .kind()
        );
    }

    #[test]
    fn sync_recovery_skips_live_supervisors_and_relaunches_after_exit() {
        use std::cell::Cell;
        for (ensure, live, expected) in [(true, true, 0), (true, false, 1), (false, true, 1)] {
            let calls = Cell::new(0);
            with_sync_recovery(
                ensure,
                || Ok(live),
                || {
                    calls.set(calls.get() + 1);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(expected, calls.get());
        }
        let calls = Cell::new(0);
        assert!(with_sync_recovery(
            true,
            || Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            || {
                calls.set(1);
                Ok(())
            }
        )
        .is_err());
        assert_eq!(
            0,
            calls.get(),
            "a mutex access failure must not spawn an unverified duplicate"
        );
        assert_eq!(
            (LaunchTarget::SyncSupervisor, false),
            parse_arguments(&[OsString::from("--ensure-sync")]).unwrap()
        );
        assert_eq!(
            LaunchTarget::SyncSupervisor,
            requested_target(&[OsString::from("--ensure-sync")])
        );
    }

    #[test]
    fn rejects_malformed_release_versions() {
        for version in ["", ".2", "2.", "2..1", "v2.1", "2.1-", "2.1-bad_name"] {
            assert!(!valid_release_version(version), "accepted {version:?}");
        }
        for version in ["2", "2.22.19", "2.22.19-windows-knowledge", "10.0-RC1"] {
            assert!(valid_release_version(version), "rejected {version:?}");
        }
    }

    #[test]
    fn accepts_one_valid_formal_client_and_ignores_unrelated_files() {
        let root = unique_test_directory();
        fs::create_dir_all(&root).expect("create test directory");
        let newest = root.join("grid_timer_windows_client_v2.22.19-current.exe");
        write_minimal_pe(&newest);
        fs::write(root.join("grid_timer_sync_server_v99.0.exe"), b"unrelated")
            .expect("create unrelated executable");

        let selected = find_unique_formal_executable(&root, LaunchTarget::Client)
            .expect("resolve current directory")
            .expect("find a client");
        assert_eq!(selected, newest);

        let temp_root = env::temp_dir();
        assert!(root.starts_with(&temp_root));
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn sync_mode_selects_only_the_unique_formal_sync_launcher() {
        let root = unique_test_directory();
        fs::create_dir_all(&root).expect("create test directory");
        let sync_launcher = root.join("grid_timer_sync_launcher_v2.22.20-windows-stability.exe");
        write_minimal_pe(&sync_launcher);
        write_minimal_pe(&root.join("grid_timer_windows_client_v2.22.20-windows-stability.exe"));

        let selected = find_unique_formal_executable(&root, LaunchTarget::SyncSupervisor)
            .expect("resolve current directory")
            .expect("find the sync supervisor");
        assert_eq!(selected, sync_launcher);
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn detached_gui_stdout_is_not_reported_as_a_launcher_failure() {
        assert!(is_detached_output_error(&io::Error::from_raw_os_error(232)));
        assert!(is_detached_output_error(&io::Error::new(
            io::ErrorKind::BrokenPipe,
            "closed"
        )));
        assert!(!is_detached_output_error(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "denied"
        )));
    }

    #[test]
    fn rejects_multiple_or_invalid_formal_clients() {
        let multiple = unique_test_directory();
        fs::create_dir_all(&multiple).expect("create multiple directory");
        write_minimal_pe(&multiple.join("grid_timer_windows_client_v2.22.19-a.exe"));
        write_minimal_pe(&multiple.join("grid_timer_windows_client_v2.22.20-b.exe"));
        assert_eq!(
            find_unique_formal_executable(&multiple, LaunchTarget::Client)
                .expect_err("reject multiple clients")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let invalid = unique_test_directory();
        fs::create_dir_all(&invalid).expect("create invalid directory");
        fs::write(
            invalid.join("grid_timer_windows_client_v2.22.19-invalid.exe"),
            b"not a PE",
        )
        .expect("create invalid client");
        assert_eq!(
            find_unique_formal_executable(&invalid, LaunchTarget::Client)
                .expect_err("reject invalid client")
                .kind(),
            io::ErrorKind::InvalidData
        );

        let temp_root = env::temp_dir();
        for directory in [multiple, invalid] {
            assert!(directory.starts_with(&temp_root));
            fs::remove_dir_all(directory).expect("remove test directory");
        }
    }

    #[test]
    fn binds_only_to_the_sibling_formal_current_slot() {
        let launcher = PathBuf::from(
            r"C:\work\timer\release_artifacts\desktop_entry\TenRate_Desktop_Launcher.exe",
        );
        assert_eq!(
            formal_current_directory(&launcher).expect("resolve formal slot"),
            PathBuf::from(r"C:\work\timer\release_artifacts\current")
        );

        let shadow = PathBuf::from(r"C:\work\timer\shadow\TenRate_Desktop_Launcher.exe");
        assert_eq!(
            formal_current_directory(&shadow)
                .expect_err("reject shadow slot")
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn retries_while_the_formal_slot_is_being_published() {
        let root = unique_test_directory();
        let current = root.join("release_artifacts").join("current");
        let published = current.join("grid_timer_windows_client_v3.0.0-published.exe");
        let publisher_current = current.clone();
        let publisher_target = published.clone();
        let publisher = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            fs::create_dir_all(&publisher_current).expect("publish current directory");
            write_minimal_pe(&publisher_target);
        });

        let resolved =
            resolve_with_retry(&current, LaunchTarget::Client).expect("wait for formal slot");
        publisher.join().expect("publisher thread");
        assert_eq!(resolved, published);

        let temp_root = env::temp_dir();
        assert!(root.starts_with(&temp_root));
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn mutex_name_is_derived_only_from_the_normalized_release_root() {
        let upper = PathBuf::from(r"C:\Work\Timer\release_artifacts\");
        let lower = PathBuf::from(r"c:\work\timer\release_artifacts");
        assert_eq!(release_mutex_name(&upper), release_mutex_name(&lower));
        let name = release_mutex_name(&upper);
        assert!(name.starts_with(RELEASE_MUTEX_PREFIX));
        assert_eq!(RELEASE_MUTEX_PREFIX.len() + 64, name.len());
    }

    #[test]
    fn verbatim_and_ordinary_release_paths_have_the_same_identity() {
        assert_eq!(
            normalize_release_root_text(r"C:\Work\Timer\release_artifacts\"),
            normalize_release_root_text(r"\\?\C:\WORK\Timer\release_artifacts")
        );
        assert_eq!(
            normalize_release_root_text(r"\\server\share\Timer\release_artifacts"),
            normalize_release_root_text(r"\\?\UNC\SERVER\Share\Timer\release_artifacts\")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_reparse_attribute_is_always_fail_closed() {
        assert!(windows_file_attributes_are_reparse_point(0x0000_0400));
        assert!(windows_file_attributes_are_reparse_point(0x0000_0410));
        assert!(!windows_file_attributes_are_reparse_point(0x0000_0010));
    }

    #[test]
    fn mutex_wait_results_are_exhaustively_classified_for_bounded_waiting() {
        assert_eq!(
            MutexWaitDisposition::Acquired,
            classify_mutex_wait_result(WAIT_OBJECT_0_RESULT)
        );
        assert_eq!(
            MutexWaitDisposition::AcquiredAbandoned,
            classify_mutex_wait_result(WAIT_ABANDONED_RESULT)
        );
        assert_eq!(
            MutexWaitDisposition::TimedOut,
            classify_mutex_wait_result(WAIT_TIMEOUT_RESULT)
        );
        assert_eq!(
            MutexWaitDisposition::Failed,
            classify_mutex_wait_result(WAIT_FAILED_RESULT)
        );
        assert_eq!(
            MutexWaitDisposition::Unexpected(7),
            classify_mutex_wait_result(7)
        );
        assert_eq!(30_000, RELEASE_MUTEX_WAIT_MILLIS);
    }

    #[test]
    fn valid_journal_recovers_complete_staging_before_complete_backup() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t123456789p42";
        let staging = root.join(format!(".current-staging-{txid}"));
        let backup = root.join(format!(".current-backup-{txid}"));
        let new_descriptor = write_release_set(&staging, "2.22.20");
        let old_descriptor = write_release_set(&backup, "2.22.19");
        let mut journal = test_journal(&layout, txid, new_descriptor, Some(old_descriptor));
        journal.phase = ReleaseTransactionPhase::Committed;
        journal.journal_checksum = journal_checksum(&journal);
        write_journal(&layout.journal, &journal);

        recover_current_if_required(&layout).expect("recover staging");

        assert!(layout.current.is_dir());
        assert!(!staging.exists());
        assert!(backup.is_dir());
        assert!(layout
            .current
            .join("grid_timer_windows_client_v2.22.20.exe")
            .is_file());
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn root_publishing_phase_rolls_uncommitted_current_back_before_launch() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t222333p44";
        let new_descriptor = write_release_set(&layout.current, "7.0.0");
        let backup = root.join(format!(".current-backup-{txid}"));
        let old_descriptor = write_release_set(&backup, "6.0.0");
        let mut journal = test_journal(&layout, txid, new_descriptor, Some(old_descriptor.clone()));
        journal.phase = ReleaseTransactionPhase::RootPublishing;
        journal.journal_checksum = journal_checksum(&journal);
        write_journal(&layout.journal, &journal);

        recover_current_if_required(&layout).expect("roll back uncommitted current");

        validate_release_directory(&layout.current, &old_descriptor).unwrap();
        assert!(root.join(format!(".current-staging-{txid}")).is_dir());
        assert!(!backup.exists());
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn same_version_current_uses_phase_aware_descriptor_priority() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t222444p55";
        let descriptor = write_release_set(&layout.current, "7.0.0");
        let mut journal = test_journal(&layout, txid, descriptor.clone(), Some(descriptor.clone()));

        assert_eq!(
            CanonicalCurrentState::Old,
            classify_canonical_current(&layout, &journal)
                .expect("pre-Commit must prefer the old descriptor")
        );

        journal.phase = ReleaseTransactionPhase::Committed;
        journal.journal_checksum = journal_checksum(&journal);
        write_journal(&layout.journal, &journal);
        assert_eq!(
            CanonicalCurrentState::New,
            classify_canonical_current(&layout, &journal)
                .expect("Committed must prefer the new descriptor")
        );
        recover_current_if_required(&layout)
            .expect("same-version committed current must remain launchable");
        validate_release_directory(&layout.current, &descriptor).unwrap();
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn transaction_journal_size_limit_matches_packager_contract() {
        assert_eq!(1024 * 1024, MAX_TRANSACTION_JOURNAL_BYTES);
        assert!(!transaction_journal_size_is_allowed(0));
        assert!(transaction_journal_size_is_allowed(
            MAX_TRANSACTION_JOURNAL_BYTES
        ));
        assert!(!transaction_journal_size_is_allowed(
            MAX_TRANSACTION_JOURNAL_BYTES + 1
        ));
    }

    #[test]
    fn invalid_staging_falls_back_to_the_complete_backup() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t987654321p7";
        let staging = root.join(format!(".current-staging-{txid}"));
        let backup = root.join(format!(".current-backup-{txid}"));
        let new_descriptor = write_release_set(&staging, "3.0.0");
        fs::write(staging.join("unexpected.txt"), b"shadow").expect("corrupt staging exact set");
        let old_descriptor = write_release_set(&backup, "2.22.20");
        let journal = test_journal(&layout, txid, new_descriptor, Some(old_descriptor));
        write_journal(&layout.journal, &journal);

        recover_current_if_required(&layout).expect("recover backup");

        assert!(layout
            .current
            .join("grid_timer_windows_client_v2.22.20.exe")
            .is_file());
        assert!(staging.is_dir());
        assert!(!backup.exists());
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn missing_journal_ignores_shadow_release_directories() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        write_release_set(&root.join(".current-staging-t1p1"), "9.9.9");
        write_release_set(&root.join("random-shadow"), "8.8.8");

        recover_current_if_required(&layout).expect("ignore unjournaled shadows");

        assert!(!layout.current.exists());
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn existing_invalid_current_is_never_bypassed_by_a_valid_backup() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        fs::write(&layout.current, b"not a directory").expect("create invalid current");
        let txid = "t111p222";
        let staging = root.join(format!(".current-staging-{txid}"));
        let backup = root.join(format!(".current-backup-{txid}"));
        let new_descriptor = write_release_set(&staging, "4.0.0");
        let old_descriptor = write_release_set(&backup, "3.0.0");
        let journal = test_journal(&layout, txid, new_descriptor, Some(old_descriptor));
        write_journal(&layout.journal, &journal);

        assert_eq!(
            io::ErrorKind::InvalidData,
            recover_current_if_required(&layout)
                .expect_err("invalid canonical current must fail closed")
                .kind()
        );
        assert!(layout.current.is_file());
        assert!(backup.is_dir());
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn journal_checksum_and_basename_are_fail_closed() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t333p444";
        let staging = root.join(format!(".current-staging-{txid}"));
        let descriptor = write_release_set(&staging, "5.0.0");
        let mut journal = test_journal(&layout, txid, descriptor, None);
        journal.current_staging = "..\\outside".to_string();
        journal.journal_checksum = journal_checksum(&journal);
        write_journal(&layout.journal, &journal);
        assert_eq!(
            io::ErrorKind::InvalidData,
            read_and_validate_transaction_journal(&layout)
                .expect_err("reject non-basename")
                .kind()
        );

        journal.current_staging = format!(".current-staging-{txid}");
        journal.journal_checksum = "0".repeat(64);
        write_journal(&layout.journal, &journal);
        assert_eq!(
            io::ErrorKind::InvalidData,
            read_and_validate_transaction_journal(&layout)
                .expect_err("reject checksum mismatch")
                .kind()
        );
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn journal_rejects_unknown_json_fields_before_recovery() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t555p666";
        let descriptor = write_release_set(&root.join(format!(".current-staging-{txid}")), "6.0.0");
        let journal = test_journal(&layout, txid, descriptor, None);
        let mut json = serde_json::to_value(journal).expect("serialize journal value");
        json.as_object_mut().expect("journal object").insert(
            "shadow_directory".to_string(),
            serde_json::json!("current-copy"),
        );
        fs::write(&layout.journal, serde_json::to_vec(&json).unwrap()).expect("write journal");

        assert_eq!(
            io::ErrorKind::InvalidData,
            read_and_validate_transaction_journal(&layout)
                .expect_err("deny unknown fields")
                .kind()
        );
        assert!(!layout.current.exists());
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    #[test]
    fn journal_rejects_duplicate_archive_destinations() {
        let root = unique_test_directory().join("release_artifacts");
        fs::create_dir_all(&root).expect("create release root");
        let layout = test_release_layout(&root);
        let txid = "t777p888";
        let descriptor = write_release_set(&root.join(format!(".current-staging-{txid}")), "7.0.0");
        let mut journal = test_journal(&layout, txid, descriptor, None);
        let entry = ArchivePlanEntry {
            source_kind: "stable_backup".to_string(),
            source_role: "stable".to_string(),
            source_file_name: STABLE_DESKTOP_ENTRY_FILE_NAME.to_string(),
            destination_kind: "old_exes".to_string(),
            destination_file_name: deterministic_archive_name(
                STABLE_DESKTOP_ENTRY_FILE_NAME,
                "stable",
                txid,
            )
            .expect("archive name"),
            size: 1,
            sha256: sha256_bytes(b"x"),
        };
        journal.archive_plan = vec![entry.clone(), entry];
        journal.journal_checksum = journal_checksum(&journal);

        assert_eq!(
            io::ErrorKind::InvalidData,
            validate_transaction_journal(&journal, &layout.project_identity)
                .expect_err("duplicate archive destinations must fail closed")
                .kind()
        );
        fs::remove_dir_all(root.parent().unwrap()).expect("remove test root");
    }

    fn append_test_manifest(
        directory: &Path,
        descriptor: &mut ReleaseSetDescriptor,
        windows_only: bool,
    ) {
        let mut checks = vec![
            "cargo_fmt",
            "core_library_tests",
            "desktop_media_tests",
            "windows_client_tests",
            "sync_launcher_tests",
            "packager_tests",
            "stable_desktop_entry_tests",
        ];
        if windows_only {
            checks.extend([
                "retained_android_signature_verification",
                "retained_android_bytes_and_modified_time",
            ]);
        } else {
            checks.extend([
                "android_unit_tests",
                "android_lint",
                "android_release_build",
                "android_signature_verification",
            ]);
        }
        checks.extend([
            "windows_executable_integrity_chain",
            "windows_server_runtime_smoke",
        ]);
        let manifest = serde_json::json!({
            "schemaVersion": 1, "productId": "gridtimer", "displayName": "十倍率", "version": descriptor.version,
            "windowsOnly": windows_only, "syncProtocolVersion": 1, "gitCommit": "0".repeat(40),
            "sourceWorktreeDirty": true, "sourceSnapshotSha256": "a".repeat(64),
            "toolchain": {"rustc":"rustc 1.95.0", "cargo":"cargo 1.95.0", "host":"x86_64-pc-windows-msvc", "target":"x86_64-pc-windows-msvc", "linker":"rust-lld.exe"},
            "passedChecks": checks, "createdAtEpochMillis": 1_700_000_000_000_i64, "files": descriptor.files,
        });
        let path = directory.join(RELEASE_MANIFEST_FILE_NAME);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        descriptor.files.push(FileDescriptor {
            role: "release_manifest".to_string(),
            file_name: RELEASE_MANIFEST_FILE_NAME.to_string(),
            size: fs::metadata(&path).unwrap().len(),
            sha256: sha256_file(&path).unwrap(),
        });
    }

    fn windows_only_test_release(
        directory: &Path,
        android: &str,
        windows: &str,
    ) -> ReleaseSetDescriptor {
        let mut descriptor = write_release_set(directory, windows);
        let old_name = descriptor.files[0].file_name.clone();
        descriptor.files[0].file_name = expected_release_file_name("apk", android);
        fs::rename(
            directory.join(old_name),
            directory.join(&descriptor.files[0].file_name),
        )
        .unwrap();
        append_test_manifest(directory, &mut descriptor, true);
        descriptor
    }

    fn mark_test_journal_windows_only(journal: &mut ReleaseTransactionJournal) {
        let apk = descriptor_apk(&journal.new_current).unwrap().clone();
        journal.windows_only = true;
        journal.root_new.file_name = apk.file_name.clone();
        journal.root_new.size = apk.size;
        journal.root_new.sha256 = apk.sha256.clone();
        journal.legacy_new.file_name = apk.file_name;
        journal.legacy_new.size = apk.size;
        journal.legacy_new.sha256 = apk.sha256;
        journal.journal_checksum = journal_checksum(journal);
    }

    #[test]
    fn windows_independent_version_reset_recovers_current_and_preserves_android() {
        let root = unique_test_directory();
        let layout = test_release_layout(&root);
        let txid = "t101p101";
        let backup = root.join(format!(".current-backup-{txid}"));
        let staging = root.join(format!(".current-staging-{txid}"));
        let old = windows_only_test_release(&backup, "2.22.49", "2.22.59-archive-privacy");
        let new = windows_only_test_release(&staging, "2.22.49", "1.0.1");
        let apk = descriptor_apk(&new).unwrap().clone();
        let apk_modified = fs::metadata(staging.join(&apk.file_name))
            .unwrap()
            .modified()
            .unwrap();
        let mut journal = test_journal(&layout, txid, new, Some(old));
        mark_test_journal_windows_only(&mut journal);
        journal.phase = ReleaseTransactionPhase::Committed;
        journal.journal_checksum = journal_checksum(&journal);
        write_journal(&layout.journal, &journal);
        recover_current_if_required(&layout).unwrap();
        validate_current_manifest_if_present(&layout.current).unwrap();
        let client = resolve_with_retry(&layout.current, LaunchTarget::Client).unwrap();
        let supervisor = resolve_with_retry(&layout.current, LaunchTarget::SyncSupervisor).unwrap();
        assert_eq!(
            client.file_name().unwrap(),
            "grid_timer_windows_client_v1.0.1.exe"
        );
        assert_eq!(
            supervisor.file_name().unwrap(),
            "grid_timer_sync_launcher_v1.0.1.exe"
        );
        assert_eq!(
            sha256_file(&layout.current.join(&apk.file_name)).unwrap(),
            apk.sha256
        );
        assert_eq!(
            fs::metadata(layout.current.join(&apk.file_name))
                .unwrap()
                .modified()
                .unwrap(),
            apk_modified
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn windows_only_committed_manifest_and_journal_preserve_the_old_apk_identity() {
        let root = unique_test_directory();
        let layout = test_release_layout(&root);
        let old = write_release_set(
            &root.join(".current-backup-t21p21"),
            "2.22.34-sync-recovery",
        );
        let new = windows_only_test_release(
            &layout.current,
            "2.22.34-sync-recovery",
            "2.22.35-windows-usability",
        );
        let mut journal = test_journal(&layout, "t21p21", new, Some(old));
        mark_test_journal_windows_only(&mut journal);
        journal.phase = ReleaseTransactionPhase::Committed;
        journal.journal_checksum = journal_checksum(&journal);
        write_journal(&layout.journal, &journal);
        validate_transaction_journal(&journal, &layout.project_identity).unwrap();
        recover_current_if_required(&layout).unwrap();
        validate_current_manifest_if_present(&layout.current).unwrap();
        journal.windows_only = false;
        journal.journal_checksum = journal_checksum(&journal);
        assert!(validate_transaction_journal(&journal, &layout.project_identity).is_err());
        mark_test_journal_windows_only(&mut journal);
        journal.new_current.files[0].sha256 = "e".repeat(64);
        journal.journal_checksum = journal_checksum(&journal);
        assert!(validate_transaction_journal(&journal, &layout.project_identity).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn windows_only_crash_recovery_accepts_legacy_backup_and_retains_android_mtime() {
        let root = unique_test_directory();
        let layout = test_release_layout(&root);
        let backup = root.join(".current-backup-t22p22");
        let old = write_release_set(&backup, "2.22.34-sync-recovery");
        let apk_name = old.files[0].file_name.clone();
        let apk_modified = fs::metadata(backup.join(&apk_name))
            .unwrap()
            .modified()
            .unwrap();
        let new = windows_only_test_release(
            &root.join(".current-staging-t22p22"),
            "2.22.34-sync-recovery",
            "2.22.35-windows-usability",
        );
        let mut journal = test_journal(&layout, "t22p22", new, Some(old));
        mark_test_journal_windows_only(&mut journal);
        write_journal(&layout.journal, &journal);
        recover_current_if_required(&layout).unwrap();
        assert_eq!(
            fs::metadata(layout.current.join(&apk_name))
                .unwrap()
                .modified()
                .unwrap(),
            apk_modified
        );
        assert_eq!(
            fs::read(layout.current.join(apk_name)).unwrap(),
            b"PK\x03\x04"
        );
        assert!(!backup.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stable_entry_accepts_independent_android_update_and_rejects_false_provenance() {
        let root = unique_test_directory();
        let directory = root.join("current");
        let descriptor =
            windows_only_test_release(&directory, "2.22.37-note-save", "2.22.35-windows-usability");
        let path = directory.join(RELEASE_MANIFEST_FILE_NAME);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let apk = descriptor
            .files
            .iter()
            .find(|file| file.role == "apk")
            .unwrap();
        manifest["androidRelease"] = serde_json::json!({
            "version": "2.22.37-note-save",
            "versionCode": 22237,
            "androidOnly": true,
            "verification": "release_artifacts/verification/v2.22.37-note-save",
            "sha256": apk.sha256,
            "sourceSnapshotSha256": "b".repeat(64),
            "createdAt": "2026-09-09T12:17:24.0114523Z"
        });
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        validate_current_manifest_if_present(&directory).unwrap();
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
            let mut invalid_manifest = manifest.clone();
            invalid_manifest["androidRelease"][field] = invalid;
            fs::write(&path, serde_json::to_vec(&invalid_manifest).unwrap()).unwrap();
            assert!(
                validate_current_manifest_if_present(&directory).is_err(),
                "{field}"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_release_rejects_mixed_windows_components_including_uppercase_names() {
        let directory = unique_test_directory();
        write_release_set(&directory, "2.22.35");
        validate_current_manifest_if_present(&directory).unwrap();
        for prefix in [SYNC_LAUNCHER_PREFIX, SYNC_SERVER_PREFIX] {
            let original = directory.join(format!("{prefix}2.22.35.exe"));
            let mismatched = directory.join(format!("{prefix}2.22.34.exe").to_ascii_uppercase());
            fs::rename(&original, &mismatched).unwrap();
            assert_eq!(
                io::ErrorKind::InvalidData,
                validate_current_manifest_if_present(&directory)
                    .unwrap_err()
                    .kind()
            );
            fs::rename(&mismatched, &original).unwrap();
            validate_current_manifest_if_present(&directory).unwrap();
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stable_entry_rejects_mixed_versions_without_explicit_manifest_and_unknown_manifest_fields() {
        let root = unique_test_directory();
        let directory = root.join("current");
        let descriptor = windows_only_test_release(
            &directory,
            "2.22.34-sync-recovery",
            "2.22.35-windows-usability",
        );
        validate_release_directory(&directory, &descriptor).unwrap();
        let path = directory.join(RELEASE_MANIFEST_FILE_NAME);
        let original = fs::read(&path).unwrap();
        let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
        manifest["allowUnknownArtifacts"] = serde_json::json!(true);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(validate_current_manifest_if_present(&directory).is_err());
        manifest
            .as_object_mut()
            .unwrap()
            .remove("allowUnknownArtifacts");
        manifest["windowsOnly"] = serde_json::json!(false);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(validate_current_manifest_if_present(&directory).is_err());
        fs::remove_file(path).unwrap();
        assert!(validate_current_manifest_if_present(&directory).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stable_entry_accepts_full_release_manifest_as_sixth_file_and_rejects_payload_tampering() {
        let root = unique_test_directory();
        let current = root.join("current");
        let mut descriptor = write_release_set(&current, "2.22.34-sync-recovery");
        append_test_manifest(&current, &mut descriptor, false);
        validate_release_set_descriptor(&descriptor, &descriptor.version).unwrap();
        validate_current_manifest_if_present(&current).unwrap();
        fs::write(
            current.join(&descriptor.files[0].file_name),
            b"PK\x03\x04tampered",
        )
        .unwrap();
        assert!(validate_current_manifest_if_present(&current).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_journal_scope_is_omitted_and_unknown_journal_fields_fail_closed() {
        let root = unique_test_directory();
        let layout = test_release_layout(&root);
        let new = write_release_set(&layout.current, "2.22.34-sync-recovery");
        let journal = test_journal(&layout, "t23p23", new, None);
        let mut value = serde_json::to_value(&journal).unwrap();
        assert!(value.get("windows_only").is_none());
        let restored: ReleaseTransactionJournal = serde_json::from_value(value.clone()).unwrap();
        assert!(!restored.windows_only);
        assert_eq!(journal_checksum(&journal), journal_checksum(&restored));
        value["trust_apk"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ReleaseTransactionJournal>(value).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    fn test_release_layout(root: &Path) -> ReleaseLayout {
        ReleaseLayout {
            root: root.to_path_buf(),
            current: root.join("current"),
            journal: root.join(RELEASE_TRANSACTION_FILE_NAME),
            project_identity: release_root_identity(root),
        }
    }

    fn test_journal(
        layout: &ReleaseLayout,
        txid: &str,
        new_current: ReleaseSetDescriptor,
        old_current: Option<ReleaseSetDescriptor>,
    ) -> ReleaseTransactionJournal {
        let new_version = new_current.version.clone();
        let old_version = old_current
            .as_ref()
            .map(|descriptor| descriptor.version.clone());
        let placeholder = FileDescriptor {
            role: "stable".to_string(),
            file_name: "TenRate_Desktop_Launcher.exe".to_string(),
            size: 1,
            sha256: sha256_bytes(b"x"),
        };
        let mut journal = ReleaseTransactionJournal {
            magic: RELEASE_TRANSACTION_MAGIC.to_string(),
            schema_version: RELEASE_TRANSACTION_SCHEMA_VERSION,
            transaction_id: txid.to_string(),
            project_identity: layout.project_identity.clone(),
            phase: ReleaseTransactionPhase::CurrentSwitching,
            new_version: new_version.clone(),
            windows_only: false,
            old_version,
            current_staging: format!(".current-staging-{txid}"),
            current_backup: format!(".current-backup-{txid}"),
            stable_staging: format!(".stable-staging-{txid}.exe"),
            stable_backup: format!(".stable-backup-{txid}.exe"),
            transaction_root: format!(".transaction-{txid}"),
            new_current,
            old_current,
            stable_new: placeholder,
            stable_old: None,
            root_new: FileDescriptor {
                role: "root_apk".to_string(),
                file_name: expected_release_file_name("apk", &new_version),
                size: 1,
                sha256: sha256_bytes(b"x"),
            },
            legacy_new: FileDescriptor {
                role: "legacy_apk".to_string(),
                file_name: expected_release_file_name("apk", &new_version),
                size: 1,
                sha256: sha256_bytes(b"x"),
            },
            archive_plan: Vec::new(),
            journal_checksum: String::new(),
        };
        journal.journal_checksum = journal_checksum(&journal);
        journal
    }

    fn journal_checksum(journal: &ReleaseTransactionJournal) -> String {
        let mut input = journal.clone();
        input.journal_checksum.clear();
        sha256_bytes(&serde_json::to_vec(&input).expect("serialize journal"))
    }

    fn write_journal(path: &Path, journal: &ReleaseTransactionJournal) {
        fs::write(
            path,
            serde_json::to_vec(journal).expect("serialize journal"),
        )
        .expect("write journal");
    }

    #[test]
    fn formal_apk_names_preserve_platform_version_and_filename_guards() {
        let version = "2.22.40-sync-unicode";
        let mut descriptor = ReleaseSetDescriptor {
            version: version.to_string(),
            files: [
                "apk",
                "sync_server",
                "sync_launcher",
                "windows_client",
                "cloudflared",
                "release_manifest",
            ]
            .into_iter()
            .map(|role| FileDescriptor {
                role: role.to_string(),
                file_name: expected_release_file_name(role, version),
                size: 4,
                sha256: "a".repeat(64),
            })
            .collect(),
        };
        for prefix in ["grid_timer_app_v", "tenfold_v"] {
            descriptor.files[0].file_name = format!("{prefix}{version}.apk");
            validate_release_set_descriptor_with_scope(&descriptor, version, false).unwrap();
            descriptor.files[0].file_name = format!("{prefix}2.22.43-pause-layout.apk");
            validate_release_set_descriptor_with_scope(&descriptor, version, true).unwrap();
            assert!(
                validate_release_set_descriptor_with_scope(&descriptor, version, false).is_err()
            );
        }
        for name in [
            "unknown_v2.22.43.apk",
            "tenfold_v../2.22.43.apk",
            "tenfold_v2.22.43.apk.exe",
            "tenfold_v.apk",
        ] {
            descriptor.files[0].file_name = name.to_string();
            assert!(
                validate_release_set_descriptor_with_scope(&descriptor, version, true).is_err()
            );
        }
    }

    fn write_release_set(directory: &Path, version: &str) -> ReleaseSetDescriptor {
        fs::create_dir_all(directory.join("tools")).expect("create release directories");
        let files = [
            ("apk", format!("grid_timer_app_v{version}.apk"), false),
            (
                "sync_server",
                format!("grid_timer_sync_server_v{version}.exe"),
                true,
            ),
            (
                "sync_launcher",
                format!("grid_timer_sync_launcher_v{version}.exe"),
                true,
            ),
            (
                "windows_client",
                format!("grid_timer_windows_client_v{version}.exe"),
                true,
            ),
            ("cloudflared", "cloudflared.exe".to_string(), true),
        ];
        let mut descriptors = Vec::new();
        for (role, file_name, pe) in files {
            let path = if role == "cloudflared" {
                directory.join("tools").join(&file_name)
            } else {
                directory.join(&file_name)
            };
            if pe {
                write_minimal_pe(&path);
            } else {
                fs::write(&path, b"PK\x03\x04").expect("write APK");
            }
            let metadata = fs::metadata(&path).expect("read artifact metadata");
            descriptors.push(FileDescriptor {
                role: role.to_string(),
                file_name,
                size: metadata.len(),
                sha256: sha256_file(&path).expect("hash artifact"),
            });
        }
        ReleaseSetDescriptor {
            version: version.to_string(),
            files: descriptors,
        }
    }

    fn write_minimal_pe(path: &Path) {
        let mut bytes = vec![0_u8; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        fs::write(path, bytes).expect("write minimal PE");
    }

    fn unique_test_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        unique_test_directory_with_nonce(nonce)
    }

    fn unique_test_directory_with_nonce(nonce: u128) -> PathBuf {
        // Windows wall-clock reads can coincide across parallel tests. Reserve
        // each directory atomically so one test can never delete another's files.
        static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
        loop {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let directory = env::temp_dir().join(format!(
                "tenrate_desktop_launcher_test_{}_{}_{}",
                std::process::id(),
                nonce,
                sequence
            ));
            match fs::create_dir(&directory) {
                Ok(()) => return directory,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("reserve test directory {}: {error}", directory.display()),
            }
        }
    }

    #[test]
    fn parallel_tests_reserve_distinct_directories_with_identical_clock_reads() {
        let workers: Vec<_> = (0..16)
            .map(|worker| {
                thread::spawn(move || {
                    let directory = unique_test_directory_with_nonce(0);
                    fs::write(directory.join("owner"), worker.to_string()).expect("write owner");
                    (worker, directory)
                })
            })
            .collect();
        let directories: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().expect("directory reservation worker"))
            .collect();
        let unique: BTreeSet<_> = directories.iter().map(|(_, directory)| directory).collect();
        assert_eq!(unique.len(), directories.len());
        let temporary_root = env::temp_dir();
        for (worker, directory) in directories {
            assert!(directory.is_absolute() && directory.starts_with(&temporary_root));
            assert_eq!(
                fs::read_to_string(directory.join("owner")).expect("read owner"),
                worker.to_string()
            );
            fs::remove_dir_all(directory).expect("remove reserved test directory");
        }
    }
}
