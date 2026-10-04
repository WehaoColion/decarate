// v0.0.1 - Redact imported legacy files with authenticated, resumable ownership records.
use super::*;
use crate::desktop_state_store::DesktopPrivacyPolicy;
use std::collections::BTreeMap;

const MAX_PROOF_BYTES: u64 = 128 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Origin {
    raw_sha256: String,
    source: String,
    backup: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    sha256: String,
    raw_sha256: String,
    size: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    version: u32,
    target: String,
    server: String,
    file: String,
    encrypted: bool,
    origin: Origin,
    generation: String,
    current: Content,
    previous: Option<Content>,
    pending_stage: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedProof {
    proof: Proof,
    mac: String,
}

fn failure(message: impl fmt::Display) -> StoreError {
    StoreError::Integrity(format!("legacy backup privacy: {message}"))
}

fn single_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && Path::new(name).file_name().and_then(|name| name.to_str()) == Some(name)
}

fn normalized_name(name: &str) -> String {
    if cfg!(windows) {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

pub(super) fn ordinary(path: &Path) -> StoreResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            let reparse = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x400 != 0
            };
            #[cfg(not(windows))]
            let reparse = metadata.file_type().is_symlink();
            if !metadata.is_file() || reparse {
                return Err(failure("owned path is not an ordinary file"));
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn local_name(path: &str, parent: &Path) -> Option<String> {
    let path = absolute_path(Path::new(path)).ok()?;
    if fs::canonicalize(path.parent()?).ok()? != parent {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    single_name(name).then(|| name.to_owned())
}

fn encrypted_name(name: &str) -> Option<bool> {
    match Path::new(name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => Some(false),
        "gtlbak" => Some(true),
        _ => None,
    }
}

fn proof_prefix(target: &str) -> String {
    format!(".legacy_privacy_{}_", &target[..16])
}
fn proof_name(target: &str, file: &str) -> String {
    format!(
        "{}{}.json",
        proof_prefix(target),
        &sha256_hex(normalized_name(file).as_bytes())[..24]
    )
}
fn stage_prefix(proof: &Proof) -> String {
    format!(
        ".legacy_stage_{}_",
        &sha256_hex(format!("{}\0{}", proof.target, normalized_name(&proof.file)).as_bytes())[..24]
    )
}
fn valid_content(value: &Content) -> bool {
    value.size > 0
        && valid_lowercase_opaque_identifier(&value.sha256)
        && valid_lowercase_opaque_identifier(&value.raw_sha256)
}

fn mac(proof: &Proof, key: &[u8]) -> StoreResult<String> {
    let mut message = b"legacy-privacy-ownership-v1\0".to_vec();
    message.extend_from_slice(&serde_json::to_vec(proof)?);
    Ok(hmac_sha256_hex(key, &message))
}

fn read_proof(path: &Path, target: &str, server: &str, key: &[u8]) -> StoreResult<Proof> {
    if !ordinary(path)? || fs::metadata(path)?.len() > MAX_PROOF_BYTES {
        return Err(failure("ownership record is missing or too large"));
    }
    let signed: SignedProof = serde_json::from_slice(&fs::read(path)?)?;
    let value = signed.proof;
    if !constant_time_bytes_eq(signed.mac.as_bytes(), mac(&value, key)?.as_bytes()) {
        return Err(failure("ownership authentication failed"));
    }
    let stage_valid = value.pending_stage.as_ref().is_none_or(|name| {
        single_name(name)
            && name
                .strip_prefix(&stage_prefix(&value))
                .and_then(|s| s.strip_suffix(".tmp"))
                .is_some_and(|s| {
                    s.len() == 32
                        && s.bytes()
                            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                })
    });
    if value.version != 1
        || value.target != target
        || value.server != server
        || !single_name(&value.file)
        || encrypted_name(&value.file) != Some(value.encrypted)
        || !single_name(&value.origin.source)
        || !single_name(&value.origin.backup)
        || ![&value.origin.source, &value.origin.backup]
            .iter()
            .any(|name| normalized_name(name) == normalized_name(&value.file))
        || !valid_lowercase_opaque_identifier(&value.origin.raw_sha256)
        || !valid_lowercase_opaque_identifier(&value.generation)
        || !valid_content(&value.current)
        || value.previous.as_ref().is_some_and(|v| !valid_content(v))
        || !stage_valid
        || path.file_name().and_then(|s| s.to_str())
            != Some(proof_name(target, &value.file).as_str())
    {
        return Err(failure("ownership record has an invalid binding"));
    }
    Ok(value)
}

pub(super) fn publish(source: &Path, destination: &Path) -> StoreResult<()> {
    ordinary(source)?;
    ordinary(destination)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        }
        let source = fs::canonicalize(source)?;
        let destination = fs::canonicalize(
            destination
                .parent()
                .ok_or_else(|| failure("destination has no parent"))?,
        )?
        .join(
            destination
                .file_name()
                .ok_or_else(|| failure("destination has no name"))?,
        );
        if source.parent() != destination.parent() {
            return Err(failure("publication left its directory"));
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)?;
        fs::File::open(
            destination
                .parent()
                .ok_or_else(|| failure("destination has no parent"))?,
        )?
        .sync_all()?;
    }
    Ok(())
}

fn write_proof(parent: &Path, value: &Proof, key: &[u8]) -> StoreResult<()> {
    let path = parent.join(proof_name(&value.target, &value.file));
    ordinary(&path)?;
    let temporary = parent.join(format!(
        ".legacy_metadata_{}.tmp",
        &random_opaque_identifier()[..32]
    ));
    let bytes = serde_json::to_vec(&SignedProof {
        proof: value.clone(),
        mac: mac(value, key)?,
    })?;
    let result = (|| -> StoreResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        publish(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn raw_bytes(bytes: Vec<u8>, encrypted: bool) -> StoreResult<Vec<u8>> {
    if encrypted {
        unprotect_legacy_backup(&bytes)
    } else {
        Ok(bytes)
    }
}

fn project(
    raw: &[u8],
    policies: &HashMap<String, DesktopPrivacyPolicy>,
) -> StoreResult<Option<Vec<u8>>> {
    let legacy: LegacyServerStore = serde_json::from_slice(raw)?;
    validate_legacy_store(&legacy)?;
    let mut replacements = Vec::new();
    for (index, user) in legacy.users.iter().enumerate() {
        if let Some(policy) = policies.get(&user.id) {
            if let Some(value) = note_privacy::project(policy, &user.app_data_json)? {
                replacements.push((index, value));
            }
        }
    }
    drop(legacy);
    if replacements.is_empty() {
        return Ok(None);
    }
    let mut value: serde_json::Value = serde_json::from_slice(raw)?;
    for (index, redacted) in replacements {
        value["users"][index]["appDataJson"] = serde_json::Value::String(redacted);
    }
    Ok(Some(serde_json::to_vec(&value)?))
}

fn retain_consumed_import(
    connection: &Connection,
    proof: &Proof,
    raw_hash: &str,
) -> StoreResult<()> {
    // Do not mark a restored database which predates this import as consumed:
    // its ordinary importer must still be allowed to restore missing accounts.
    connection.execute("INSERT OR IGNORE INTO legacy_imports(content_sha256,source_path,backup_path,imported_at_epoch_millis,users_imported,tokens_imported)
        SELECT ?1,source_path,backup_path,imported_at_epoch_millis,users_imported,tokens_imported FROM legacy_imports
        WHERE content_sha256 IN (?2,?3,?4) LIMIT 1", params![raw_hash, proof.origin.raw_sha256, proof.current.raw_sha256, proof.previous.as_ref().map(|v| v.raw_sha256.as_str()).unwrap_or("")])?;
    Ok(())
}

fn clean_file(
    connection: &Connection,
    parent: &Path,
    file_name: &str,
    origins: &[Origin],
    mut proof: Option<Proof>,
    target: &str,
    server: &str,
    key: &[u8],
    generation: &str,
    policies: &HashMap<String, DesktopPrivacyPolicy>,
) -> StoreResult<bool> {
    let path = parent.join(file_name);
    if !ordinary(&path)? {
        if let Some(stage) = proof.as_ref().and_then(|p| p.pending_stage.as_ref()) {
            if ordinary(&parent.join(stage))? {
                return Err(failure("source disappeared while a rewrite is unfinished"));
            }
        }
        return Ok(false);
    }
    let size = fs::metadata(&path)?.len();
    let digest = sha256_file(&path)?;
    if let Some(value) = &mut proof {
        if !std::iter::once(&value.current)
            .chain(value.previous.as_ref())
            .any(|item| item.sha256 == digest && item.size == size)
        {
            return Err(failure("legacy file changed outside its recorded rewrite"));
        }
        let needs_completion = value.pending_stage.is_some() || value.previous.is_some();
        if let Some(stage) = value.pending_stage.take() {
            let stage = parent.join(stage);
            if ordinary(&stage)? {
                fs::remove_file(stage)?;
            }
        }
        if value.current.sha256 == digest && value.generation == generation {
            retain_consumed_import(connection, value, &value.current.raw_sha256)?;
            value.previous = None;
            if needs_completion {
                write_proof(parent, value, key)?;
            }
            return Ok(false);
        }
    }
    let encrypted =
        encrypted_name(file_name).ok_or_else(|| failure("unsupported legacy file type"))?;
    let bytes = fs::read(&path)?;
    if sha256_hex(&bytes) != digest {
        return Err(failure("legacy file changed while reading"));
    }
    let raw = raw_bytes(bytes, encrypted)?;
    let raw_hash = sha256_hex(&raw);
    let original = Content {
        sha256: digest,
        raw_sha256: raw_hash.clone(),
        size,
    };
    let origin = if let Some(value) = &proof {
        if !std::iter::once(&value.current)
            .chain(value.previous.as_ref())
            .any(|item| item.sha256 == original.sha256 && item.raw_sha256 == raw_hash)
        {
            return Err(failure("legacy payload disagrees with ownership record"));
        }
        value.origin.clone()
    } else {
        let Some(origin) = origins.iter().find(|origin| origin.raw_sha256 == raw_hash) else {
            return Ok(false);
        };
        origin.clone()
    };
    let Some(redacted) = project(&raw, policies)? else {
        if let Some(value) = &mut proof {
            value.current = original;
            value.previous = None;
            value.generation = generation.into();
            retain_consumed_import(connection, value, &raw_hash)?;
            write_proof(parent, value, key)?;
        }
        return Ok(false);
    };
    drop(raw);
    let raw_hash = sha256_hex(&redacted);
    let replacement = if encrypted {
        let protected = protect_legacy_backup(&redacted)?;
        drop(redacted);
        protected
    } else {
        redacted
    };
    let mut value = Proof {
        version: 1,
        target: target.into(),
        server: server.into(),
        file: file_name.into(),
        encrypted,
        origin,
        generation: generation.into(),
        current: Content {
            sha256: sha256_hex(&replacement),
            raw_sha256: raw_hash.clone(),
            size: replacement.len() as u64,
        },
        previous: Some(original),
        pending_stage: None,
    };
    let stage = format!(
        "{}{}.tmp",
        stage_prefix(&value),
        &random_opaque_identifier()[..32]
    );
    value.pending_stage = Some(stage.clone());
    write_proof(parent, &value, key)?;
    interruption(0)?;
    let staged = parent.join(stage);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    output.write_all(&replacement)?;
    output.sync_all()?;
    drop(output);
    drop(replacement);
    interruption(1)?;
    retain_consumed_import(connection, &value, &raw_hash)?;
    interruption(2)?;
    if !ordinary(&path)?
        || fs::metadata(&path)?.len() != size
        || sha256_file(&path)? != value.previous.as_ref().unwrap().sha256
    {
        return Err(failure("legacy source changed before publication"));
    }
    if !ordinary(&staged)? || sha256_file(&staged)? != value.current.sha256 {
        return Err(failure("staged legacy file changed"));
    }
    publish(&staged, &path)?;
    interruption(3)?;
    if sha256_file(&path)? != value.current.sha256 {
        return Err(failure("published legacy file failed verification"));
    }
    value.previous = None;
    value.pending_stage = None;
    write_proof(parent, &value, key)?;
    Ok(true)
}

impl SqliteServerStore {
    pub(crate) fn clean_legacy_privacy_files(&self) -> StoreResult<usize> {
        let Some(generation) = self.backup_privacy_generation()? else {
            return Ok(0);
        };
        let parent = fs::canonicalize(
            self.database_path
                .parent()
                .ok_or_else(|| failure("database has no parent"))?,
        )?;
        let target = privacy_journal::target_fingerprint(&self.database_path)?;
        let lock_path = parent.join(format!("{}.lock", proof_prefix(&target)));
        ordinary(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => StoreError::Io(io::Error::new(
                io::ErrorKind::WouldBlock,
                "legacy privacy maintenance is busy",
            )),
            fs::TryLockError::Error(error) => StoreError::Io(error),
        })?;
        let connection = self.open_connection(false)?;
        let (server,secret): (String,String) = connection.query_row("SELECT server_instance_id,workspace_capability_secret FROM server_identity WHERE singleton=1",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
        let key = decode_lowercase_hex_32(&secret)?;
        let policies: HashMap<_, _> =
            privacy_journal::recovery_policies(&self.database_path, &server)?
                .into_iter()
                .collect();
        let mut files: BTreeMap<String, (String, Vec<Origin>, Option<Proof>)> = BTreeMap::new();
        let mut statement = connection
            .prepare("SELECT content_sha256,source_path,backup_path FROM legacy_imports")?;
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (raw_sha256, source, backup) = row?;
            let (Some(source), Some(backup)) =
                (local_name(&source, &parent), local_name(&backup, &parent))
            else {
                continue;
            };
            let origin = Origin {
                raw_sha256,
                source,
                backup,
            };
            for name in [&origin.source, &origin.backup] {
                if encrypted_name(name).is_none() {
                    continue;
                }
                files
                    .entry(normalized_name(name))
                    .or_insert_with(|| (name.clone(), Vec::new(), None))
                    .1
                    .push(origin.clone());
            }
        }
        drop(statement);
        for entry in fs::read_dir(&parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with(&proof_prefix(&target)) || !name.ends_with(".json") {
                continue;
            }
            let proof = read_proof(&entry.path(), &target, &server, &key)?;
            let item = files
                .entry(normalized_name(&proof.file))
                .or_insert_with(|| (proof.file.clone(), Vec::new(), None));
            item.0 = proof.file.clone();
            item.2 = Some(proof);
        }
        let mut changed = 0;
        for (_, (file, origins, proof)) in files {
            changed += usize::from(clean_file(
                &connection,
                &parent,
                &file,
                &origins,
                proof,
                &target,
                &server,
                &key,
                &generation,
                &policies,
            )?);
        }
        Ok(changed)
    }
}

#[cfg(test)]
thread_local! { pub(crate) static INTERRUPT: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) }; }
fn interruption(point: u8) -> StoreResult<()> {
    #[cfg(test)]
    if INTERRUPT.with(|value| value.get() == Some(point)) {
        return Err(failure("injected legacy cleanup interruption"));
    }
    let _ = point;
    Ok(())
}
