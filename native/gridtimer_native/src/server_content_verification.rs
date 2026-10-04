// v0.0.1 - Reuse successful pure content checks with bounded, process-local fingerprints.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_ENTRIES: usize = 2048;

#[derive(Default)]
struct Proofs(VecDeque<[u8; 32]>);

impl Proofs {
    fn remember(&mut self, key: [u8; 32]) {
        if self.0.contains(&key) {
            return;
        }
        while self.0.len() >= MAX_ENTRIES {
            self.0.pop_front();
        }
        self.0.push_back(key);
    }
}

fn verified(key: [u8; 32], check: impl FnOnce() -> StoreResult<()>) -> StoreResult<()> {
    // Only fixed-size fingerprints survive a call. No plaintext, policy,
    // database handle or on-disk authority is retained. Never hold the lock
    // while validating: checks may nest and concurrent misses are harmless.
    static PROOFS: OnceLock<Mutex<Proofs>> = OnceLock::new();
    let proofs = PROOFS.get_or_init(Default::default);
    if proofs.lock().is_ok_and(|proofs| proofs.0.contains(&key)) {
        return Ok(());
    }
    #[cfg(test)]
    VALIDATIONS.with(|count| count.set(count.get() + 1));
    check()?;
    if let Ok(mut proofs) = proofs.lock() {
        proofs.remember(key);
    }
    Ok(())
}

struct HashWriter(Sha256);
impl io::Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_fingerprint(value: &impl Serialize) -> StoreResult<[u8; 32]> {
    let mut output = HashWriter(Sha256::new());
    serde_json::to_writer(&mut output, value)?;
    Ok(output.0.finalize().into())
}

fn snapshot_fingerprint(stored: &SnapshotContentRow) -> StoreResult<[u8; 32]> {
    let metadata = json_fingerprint(&(
        "server-compressed-snapshot-v1",
        &stored.sha256,
        &stored.compression,
        stored.uncompressed_size_bytes,
        stored.compressed_size_bytes,
    ))?;
    let mut hash = Sha256::new();
    hash.update(metadata);
    // Hash the actual compressed bytes even when their declared digest and
    // length match a previously verified record.
    hash.update(Sha256::digest(&stored.content));
    Ok(hash.finalize().into())
}

pub(super) fn verify_structure(raw: &str) -> StoreResult<()> {
    let mut hash = Sha256::new();
    hash.update(b"server-json-object-proof-v1\0");
    hash.update(raw.as_bytes());
    verified(hash.finalize().into(), || {
        validate_app_data_json_structure(raw).map(|_| ())
    })
}

pub(super) fn verify_compressed_schema(stored: &SnapshotContentRow) -> StoreResult<()> {
    let mut hash = Sha256::new();
    hash.update(b"server-compressed-schema-proof-v1\0");
    hash.update(APP_DATA_SCHEMA_VERSION.to_le_bytes());
    hash.update(snapshot_fingerprint(stored)?);
    verified(hash.finalize().into(), || {
        let raw = decode_snapshot_content(stored)?;
        validate_app_data_json(&raw)
    })
}

pub(super) fn verify_private_history(
    policy: &DesktopPrivacyPolicy,
    stored: SnapshotContentRow,
) -> StoreResult<()> {
    let mut hash = Sha256::new();
    hash.update(b"server-private-history-proof-v1\0");
    hash.update(json_fingerprint(&(policy, &stored.user_id))?);
    hash.update(snapshot_fingerprint(&stored)?);
    verified(hash.finalize().into(), || {
        let snapshot = decode_snapshot_history(stored)?;
        if note_privacy::project(policy, &snapshot.app_data_json)?.is_some() {
            return Err(StoreError::Integrity(
                "server note privacy: history snapshot violates privacy barrier".into(),
            ));
        }
        Ok(())
    })
}

#[cfg(test)]
thread_local! { static VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
#[path = "server_content_verification_tests.rs"]
mod tests;
