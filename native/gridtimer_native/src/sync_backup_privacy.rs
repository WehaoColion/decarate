// v0.0.6 - Include format-15-to-16 recovery copies in privacy maintenance.
// v0.0.5 - Include schema 10/11 archives and earlier upgrade destinations in maintenance.
// v0.0.4 - Redact schema 12/13 archives with authenticated recovery provenance.
// v0.0.3 - Include authenticated legacy JSON sources and migration containers.
// v0.0.2 - Share maintenance identity across Windows filename case aliases.
// v0.0.1 - Rewrite owned recovery backups with resumable manifest publication.
use super::*;
use crate::server_store::VerifiedBackupReport;

const PENDING_SUFFIX: &str = ".privacy_pending_v1.json";
const CERT_SUFFIX: &str = ".privacy_verified_v1.json";
const MAX_METADATA_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Manifest {
    Startup(Option<StartupBackupState>),
    Runtime(RuntimeBackupManifest),
    PreSchema,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    version: u32,
    target: String,
    server: String,
    file: String,
    temporary: String,
    timestamp: i64,
    original_sha256: String,
    original_size: u64,
    generation: String,
    replacement: Option<(String, u64)>,
    manifest: Manifest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Certificate {
    version: u32,
    file: String,
    target: String,
    server: String,
    generation: String,
    sha256: String,
    size: u64,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("managed backup privacy: {message}"),
    )
}

fn ordinary(path: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = metadata.file_type().is_symlink();
    if !metadata.is_file() || reparse {
        return Err(invalid("path is not an ordinary file"));
    }
    Ok(true)
}

fn side_path(path: &Path, suffix: &str) -> PathBuf {
    if suffix == PENDING_SUFFIX || suffix == CERT_SUFFIX || suffix == ".privacy_maintenance.lock" {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let name = if cfg!(windows) {
            name.to_lowercase()
        } else {
            name.into_owned()
        };
        let digest = hex_bytes(&Sha256::digest(name.as_bytes()));
        return path.with_file_name(format!(".privacy_backup_{}{suffix}", &digest[..24]));
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn file_name(path: &Path) -> io::Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| invalid("file name is not UTF-8"))
}

fn single_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && name != "."
        && name != ".."
        && Path::new(name).file_name().and_then(|name| name.to_str()) == Some(name)
}

fn read_metadata<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    if !ordinary(path)? {
        return Ok(None);
    }
    if fs::metadata(path)?.len() > MAX_METADATA_BYTES {
        return Err(invalid("metadata is too large"));
    }
    serde_json::from_slice(&fs::read(path)?)
        .map(Some)
        .map_err(|_| invalid("metadata is invalid"))
}

#[cfg(windows)]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)?;
    fs::File::open(destination.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

fn write_metadata(path: &Path, value: &impl Serialize) -> io::Result<()> {
    ordinary(path)?;
    let temporary = side_path(path, &format!(".{}.tmp", random_token(8)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&serde_json::to_vec(value).map_err(io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        publish(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn maintenance_lock(database: &Path) -> io::Result<fs::File> {
    let path = side_path(database, ".privacy_maintenance.lock");
    ordinary(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "backup privacy maintenance is busy",
        ),
        fs::TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

fn verified(path: &Path, timestamp: i64) -> io::Result<VerifiedBackupReport> {
    if !ordinary(path)? {
        return Err(invalid("backup is missing"));
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        if fs::symlink_metadata(sync_sqlite_sidecar_path(path, suffix)).is_ok() {
            return Err(invalid("backup has an unexpected SQLite sidecar"));
        }
    }
    SqliteServerStore::verify_managed_privacy_backup(path, timestamp).map_err(store_io_error)
}

fn matches(report: &VerifiedBackupReport, sha: &str, size: u64, server: &str) -> bool {
    report.sha256 == sha && report.size_bytes == size && report.server_instance_id == server
}

fn pre_schema_timestamp(database: &Path, name: &str) -> Option<i64> {
    let stem = database.file_stem()?.to_str()?;
    let normalized;
    let normalized_stem;
    let (stem, name) = if cfg!(windows) {
        normalized = name.to_lowercase();
        normalized_stem = stem.to_lowercase();
        (normalized_stem.as_str(), normalized.as_str())
    } else {
        (stem, name)
    };
    let tail = name.strip_prefix(&format!("{stem}_pre_schema_v"))?;
    let (from, tail) = tail.split_once("_to_v")?;
    let (to, tail) = tail.split_once('_')?;
    let from: u32 = from.parse().ok()?;
    let to: u32 = to.parse().ok()?;
    if !(10..=16).contains(&from) || !(11..=17).contains(&to) || from >= to {
        return None;
    }
    let tail = tail.strip_suffix(".sqlite3")?;
    let mut parts = tail.split('_');
    let timestamp = parts.next()?;
    let sequence = parts.next();
    if timestamp.is_empty()
        || !timestamp.bytes().all(|c| c.is_ascii_digit())
        || sequence.is_some_and(|s| s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()))
        || parts.next().is_some()
    {
        return None;
    }
    timestamp.parse().ok()
}

fn valid_pending(database: &Path, directory: &Path, pending: &Pending) -> bool {
    let token = pending
        .temporary
        .strip_prefix(".privacy_copy_")
        .and_then(|s| s.strip_suffix(".sqlite3"));
    let name = std::ffi::OsStr::new(&pending.file);
    let kind = match &pending.manifest {
        Manifest::Startup(state) => {
            directory == database.parent().unwrap_or_else(|| Path::new("."))
                && startup_backup_timestamp(name) == Some(pending.timestamp)
                && state.as_ref().is_none_or(|state| {
                    state.backup_file_name == pending.file
                        && state.backup_sha256 == pending.original_sha256
                        && state.backup_size_bytes == pending.original_size
                        && state.created_at_epoch_millis == pending.timestamp
                        && valid_runtime_backup_binding(&state.source_sha256)
                        && ((state.state_version == 0
                            && state.server_instance_id.is_empty()
                            && state.target_store_fingerprint.is_empty())
                            || (state.state_version == STARTUP_BACKUP_STATE_VERSION
                                && state.server_instance_id == pending.server
                                && state.target_store_fingerprint == pending.target))
                })
        }
        Manifest::Runtime(manifest) => {
            runtime_backup_identity_and_timestamp(name)
                == Some((pending.server.clone(), pending.timestamp))
                && manifest.backup_file_name == pending.file
                && manifest.target_store_fingerprint == pending.target
                && manifest.server_instance_id == pending.server
                && manifest.backup_sha256 == pending.original_sha256
                && manifest.backup_size_bytes == pending.original_size
                && manifest.created_at_epoch_millis == pending.timestamp
                && manifest.manifest_version == RUNTIME_BACKUP_MANIFEST_VERSION
        }
        Manifest::PreSchema => {
            directory == database.parent().unwrap_or_else(|| Path::new("."))
                && pre_schema_timestamp(database, &pending.file) == Some(pending.timestamp)
        }
    };
    pending.version == 1
        && single_name(&pending.file)
        && single_name(&pending.temporary)
        && token.is_some_and(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        && valid_runtime_backup_binding(&pending.server)
        && valid_runtime_backup_binding(&pending.target)
        && valid_runtime_backup_binding(&pending.generation)
        && valid_runtime_backup_binding(&pending.original_sha256)
        && pending.original_size > 0
        && pending.timestamp >= 0
        && kind
        && pending
            .replacement
            .as_ref()
            .is_none_or(|(sha, size)| valid_runtime_backup_binding(sha) && *size > 0)
}

fn remove_temporary(path: &Path) -> io::Result<()> {
    for candidate in std::iter::once(path.to_path_buf())
        .chain(["-wal", "-shm", "-journal"].map(|suffix| sync_sqlite_sidecar_path(path, suffix)))
    {
        if ordinary(&candidate)? {
            fs::remove_file(candidate)?;
        }
    }
    Ok(())
}

fn finish_manifest(
    database: &Path,
    path: &Path,
    pending: &Pending,
    report: &VerifiedBackupReport,
) -> io::Result<()> {
    match &pending.manifest {
        Manifest::Runtime(original) => {
            let mut updated = original.clone();
            updated.backup_sha256 = report.sha256.clone();
            updated.backup_size_bytes = report.size_bytes;
            let destination = runtime_backup_manifest_path(path);
            if let Some(current) = read_metadata::<RuntimeBackupManifest>(&destination)? {
                let value = serde_json::to_value(current).map_err(io::Error::other)?;
                if value != serde_json::to_value(original).map_err(io::Error::other)?
                    && value != serde_json::to_value(&updated).map_err(io::Error::other)?
                {
                    return Err(invalid("runtime manifest changed during cleanup"));
                }
            }
            write_metadata(&destination, &updated)?;
        }
        Manifest::Startup(original) => {
            if original.is_none() {
                return Ok(());
            }
            let destination = startup_backup_state_path(database);
            let current = read_metadata::<StartupBackupState>(&destination)?;
            // A newer startup copy may have become the selected backup. Its
            // state must not be replaced by an older cleanup transaction.
            if current
                .as_ref()
                .is_some_and(|value| value.backup_file_name != pending.file)
            {
                return Ok(());
            }
            if let Some(original) = original {
                let mut updated = original.clone();
                updated.state_version = STARTUP_BACKUP_STATE_VERSION;
                updated.server_instance_id = pending.server.clone();
                updated.target_store_fingerprint = pending.target.clone();
                updated.backup_sha256 = report.sha256.clone();
                updated.backup_size_bytes = report.size_bytes;
                if let Some(current) = current {
                    let value = serde_json::to_value(current).map_err(io::Error::other)?;
                    if value != serde_json::to_value(original).map_err(io::Error::other)?
                        && value != serde_json::to_value(&updated).map_err(io::Error::other)?
                    {
                        return Err(invalid("startup state changed during cleanup"));
                    }
                }
                write_metadata(&destination, &updated)?;
            }
        }
        Manifest::PreSchema => {}
    }
    Ok(())
}

fn resume_one(database: &Path, directory: &Path, pending: &Pending) -> io::Result<()> {
    if !valid_pending(database, directory, pending) {
        return Err(invalid("pending rewrite failed identity validation"));
    }
    let path = directory.join(&pending.file);
    let temporary = directory.join(&pending.temporary);
    let marker = side_path(&path, PENDING_SUFFIX);
    let mut current = verified(&path, pending.timestamp)?;
    let Some((new_sha, new_size)) = &pending.replacement else {
        if !matches(
            &current,
            &pending.original_sha256,
            pending.original_size,
            &pending.server,
        ) {
            return Err(invalid("unfinished source changed before cleanup"));
        }
        remove_temporary(&temporary)?;
        fs::remove_file(marker)?;
        return Ok(());
    };
    if !matches(&current, new_sha, *new_size, &pending.server) {
        if !matches(
            &current,
            &pending.original_sha256,
            pending.original_size,
            &pending.server,
        ) {
            return Err(invalid("backup changed before replacement"));
        }
        let staged = verified(&temporary, pending.timestamp)?;
        if !matches(&staged, new_sha, *new_size, &pending.server) {
            return Err(invalid("staged privacy backup changed"));
        }
        publish(&temporary, &path)?;
        interruption(2)?;
        current = verified(&path, pending.timestamp)?;
        if !matches(&current, new_sha, *new_size, &pending.server) {
            return Err(invalid("published privacy backup failed verification"));
        }
    }
    finish_manifest(database, &path, pending, &current)?;
    interruption(3)?;
    write_metadata(
        &side_path(&path, CERT_SUFFIX),
        &Certificate {
            version: 1,
            file: pending.file.clone(),
            target: pending.target.clone(),
            server: pending.server.clone(),
            generation: pending.generation.clone(),
            sha256: current.sha256,
            size: current.size_bytes,
        },
    )?;
    remove_temporary(&temporary)?;
    fs::remove_file(marker)?;
    Ok(())
}

fn directories(database: &Path, runtime: &Path) -> Vec<PathBuf> {
    let parent = database
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    if parent == runtime {
        vec![parent]
    } else {
        vec![parent, runtime.to_path_buf()]
    }
}

fn resume_locked(database: &Path, runtime: &Path) -> io::Result<()> {
    let target = runtime_backup_target_fingerprint(database)?;
    for directory in directories(database, runtime) {
        if !directory.exists() {
            continue;
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(_) = name
                .to_str()
                .and_then(|name| name.strip_suffix(PENDING_SUFFIX))
            else {
                continue;
            };
            let pending: Pending = read_metadata(&entry.path())?
                .ok_or_else(|| invalid("pending rewrite disappeared"))?;
            if pending.target != target {
                continue;
            }
            if entry.path() != side_path(&directory.join(&pending.file), PENDING_SUFFIX) {
                return Err(invalid("pending rewrite name disagrees with its contents"));
            }
            resume_one(database, &directory, &pending)?;
        }
    }
    Ok(())
}

pub(super) fn resume(database: &Path, runtime: &Path) -> io::Result<()> {
    if !database.parent().unwrap_or_else(|| Path::new(".")).exists() {
        return Ok(());
    }
    let _lock = maintenance_lock(database)?;
    resume_locked(database, runtime)
}

fn rewrite(
    database: &Path,
    report: VerifiedBackupReport,
    manifest: Manifest,
    target: &str,
    generation: &str,
) -> io::Result<bool> {
    let path = &report.destination;
    let certificate_path = side_path(path, CERT_SUFFIX);
    ordinary(&certificate_path)?;
    // Completion certificates are disposable caches; a damaged certificate
    // must trigger verified cleanup rather than block a healthy service.
    if let Ok(Some(cert)) = read_metadata::<Certificate>(&certificate_path) {
        if cert.version == 1
            && cert.file == file_name(path)?
            && cert.target == target
            && cert.server == report.server_instance_id
            && cert.generation == generation
            && cert.sha256 == report.sha256
            && cert.size == report.size_bytes
        {
            return Ok(false);
        }
    }
    let mut pending = Pending {
        version: 1,
        target: target.into(),
        server: report.server_instance_id.clone(),
        file: file_name(path)?,
        temporary: format!(".privacy_copy_{}.sqlite3", random_token(16)),
        timestamp: report.created_at_epoch_millis,
        original_sha256: report.sha256,
        original_size: report.size_bytes,
        generation: generation.into(),
        replacement: None,
        manifest,
    };
    let marker = side_path(path, PENDING_SUFFIX);
    write_metadata(&marker, &pending)?;
    let temporary = path.with_file_name(&pending.temporary);
    let mut source = fs::File::open(path)?;
    let mut copy = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    io::copy(&mut source, &mut copy)?;
    copy.sync_all()?;
    drop(copy);
    drop(source);
    interruption(0)?;
    let copied = verified(&temporary, pending.timestamp)?;
    if !matches(
        &copied,
        &pending.original_sha256,
        pending.original_size,
        &pending.server,
    ) {
        return Err(invalid("backup changed while preparing cleanup"));
    }
    let clean = SqliteServerStore::apply_external_privacy_to_backup_copy(
        &temporary,
        database,
        pending.timestamp,
    )
    .map_err(|error| match error {
        StoreError::Io(error) => error,
        other => store_io_error(other),
    })?;
    if matches!(pending.manifest, Manifest::PreSchema) {
        SqliteServerStore::record_preschema_privacy_rewrite(
            database,
            path,
            &temporary,
            pending.timestamp,
        )
        .map_err(store_io_error)?;
    }
    pending.replacement = Some((clean.sha256, clean.size_bytes));
    write_metadata(&marker, &pending)?;
    interruption(1)?;
    resume_one(
        database,
        path.parent().unwrap_or_else(|| Path::new(".")),
        &pending,
    )?;
    Ok(true)
}

pub(super) fn clean(store: &SqliteServerStore, runtime: &Path) -> io::Result<usize> {
    let database = store.database_path();
    let _lock = maintenance_lock(database)?;
    resume_locked(database, runtime)?;
    let Some(generation) = store.backup_privacy_generation().map_err(store_io_error)? else {
        return Ok(0);
    };
    let server = store.server_instance_id().map_err(store_io_error)?;
    let target = runtime_backup_target_fingerprint(database)?;
    let selected = load_valid_startup_backup_state(database)?;
    let parent = database.parent().unwrap_or_else(|| Path::new("."));
    let mut count = store.clean_legacy_privacy_files().map_err(store_io_error)?;
    for directory in directories(database, runtime) {
        if !directory.exists() {
            continue;
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with(".privacy_backup_") && name.ends_with(CERT_SUFFIX) {
                let Ok(Some(cert)) = read_metadata::<Certificate>(&entry.path()) else {
                    continue;
                };
                if cert.version == 1
                    && cert.target == target
                    && cert.server == server
                    && single_name(&cert.file)
                    && valid_runtime_backup_binding(&cert.sha256)
                    && valid_runtime_backup_binding(&cert.generation)
                    && entry.path() == side_path(&directory.join(&cert.file), CERT_SUFFIX)
                    && fs::symlink_metadata(directory.join(&cert.file))
                        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
                {
                    fs::remove_file(entry.path())?;
                }
                continue;
            }
            let startup = (directory == parent)
                .then(|| startup_backup_timestamp(std::ffi::OsStr::new(name)))
                .flatten();
            let migration = (directory == parent)
                .then(|| pre_schema_timestamp(database, name))
                .flatten();
            let runtime_identity = (directory == runtime)
                .then(|| runtime_backup_identity_and_timestamp(std::ffi::OsStr::new(name)))
                .flatten();
            let timestamp = startup
                .or(migration)
                .or_else(|| runtime_identity.as_ref().map(|(_, time)| *time));
            let Some(timestamp) = timestamp else {
                continue;
            };
            let path = entry.path();
            let report = match verified(&path, timestamp) {
                Ok(report) => report,
                Err(_) => continue,
            };
            if report.server_instance_id != server {
                continue;
            }
            let manifest = if startup.is_some() {
                Manifest::Startup(
                    if selected
                        .as_ref()
                        .is_some_and(|(_, selected)| *selected == path)
                    {
                        read_metadata(&startup_backup_state_path(database))?
                    } else {
                        None
                    },
                )
            } else if migration.is_some() {
                Manifest::PreSchema
            } else {
                if runtime_identity
                    .as_ref()
                    .is_none_or(|(id, _)| *id != server)
                {
                    continue;
                }
                let Some(manifest) = load_valid_runtime_backup_manifest(&path, &report, timestamp)?
                else {
                    continue;
                };
                if manifest.target_store_fingerprint != target {
                    continue;
                }
                Manifest::Runtime(manifest)
            };
            count += usize::from(rewrite(database, report, manifest, &target, &generation)?);
        }
    }
    Ok(count)
}

#[cfg(test)]
thread_local! { pub(super) static INTERRUPT: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(super) fn rewrite_preschema_for_test(
    store: &SqliteServerStore,
    path: &Path,
    timestamp: i64,
) -> io::Result<bool> {
    let database = store.database_path();
    let generation = store
        .backup_privacy_generation()
        .map_err(store_io_error)?
        .unwrap();
    rewrite(
        database,
        verified(path, timestamp)?,
        Manifest::PreSchema,
        &runtime_backup_target_fingerprint(database)?,
        &generation,
    )
}
fn interruption(point: u8) -> io::Result<()> {
    #[cfg(test)]
    if INTERRUPT.with(|value| value.get() == Some(point)) {
        return Err(io::Error::other("injected backup cleanup interruption"));
    }
    let _ = point;
    Ok(())
}
