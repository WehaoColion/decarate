// v0.0.1 - Reuse successful immutable archive checks only after hashing every byte again.
use super::*;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_ENTRIES: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Domain {
    Recovery,
    PreSchema,
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    sha256: String,
    size: u64,
}

struct Entry {
    domain: Domain,
    fingerprint: Fingerprint,
    server: String,
}

static CACHE: OnceLock<Mutex<VecDeque<Entry>>> = OnceLock::new();

// This cache holds no account payload, file handle, connection, or persisted
// authority. A fresh process must validate each distinct image in full again.
fn cache() -> &'static Mutex<VecDeque<Entry>> {
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn eligible(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let indirect = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let indirect = metadata.file_type().is_symlink();
    if indirect || !metadata.is_file() {
        return Ok(false);
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        match fs::symlink_metadata(sqlite_sidecar_path(path, suffix)) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

fn fingerprint(path: &Path) -> StoreResult<Option<Fingerprint>> {
    if !eligible(path)? {
        return Ok(None);
    }
    let before = fs::metadata(path)?;
    let sha256 = sha256_file(path)?;
    let after = fs::metadata(path)?;
    if !eligible(path)? || before.len() != after.len() || before.modified()? != after.modified()? {
        return Ok(None);
    }
    Ok(Some(Fingerprint {
        sha256,
        size: after.len(),
    }))
}

pub(super) fn verify(
    path: &Path,
    timestamp: i64,
    domain: Domain,
    validate: impl FnOnce() -> StoreResult<VerifiedBackupReport>,
) -> StoreResult<VerifiedBackupReport> {
    // Failure to fingerprint is a cache miss, never a validation success.
    let before = fingerprint(path).ok().flatten();
    if let Some(before) = before.as_ref() {
        if let Ok(mut entries) = cache().lock() {
            if let Some(index) = entries
                .iter()
                .position(|entry| entry.domain == domain && entry.fingerprint == *before)
            {
                let entry = entries.remove(index).expect("index came from this queue");
                let server = entry.server.clone();
                entries.push_back(entry);
                return Ok(VerifiedBackupReport {
                    destination: path.into(),
                    size_bytes: before.size,
                    sha256: before.sha256.clone(),
                    created_at_epoch_millis: timestamp,
                    server_instance_id: server,
                });
            }
        }
    }
    #[cfg(test)]
    VALIDATIONS.with(|count| count.set(count.get() + 1));
    let report = validate()?;
    if let Some(before) = before {
        // The validator hashes the image again after all SQL checks. Never
        // remember a scan that crossed an image change or gained a sidecar.
        if report.sha256 == before.sha256
            && report.size_bytes == before.size
            && eligible(path).unwrap_or(false)
        {
            if let Ok(mut entries) = cache().lock() {
                entries.retain(|entry| !(entry.domain == domain && entry.fingerprint == before));
                while entries.len() >= MAX_ENTRIES {
                    entries.pop_front();
                }
                entries.push_back(Entry {
                    domain,
                    fingerprint: before,
                    server: report.server_instance_id.clone(),
                });
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
thread_local! { pub(super) static VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
