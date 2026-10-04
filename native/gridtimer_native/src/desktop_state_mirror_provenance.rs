//! Owner-bound, bounded evidence for managed recovery files. Prepared evidence
//! is useful only after a reader independently verifies the actual file bytes.
use super::*;
use std::io::Read;

pub(super) const TABLE: &str = "desktop_state_mirror_provenance";
const MAX_SOURCES: usize = 32;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Serialize)]
struct Proof {
    owner: String,
    source: String,
    raw_sha256: String,
    size: i64,
    format: i64,
    origin: String,
    journal_id: String,
    workspace: String,
    sequence: i64,
    binding: String,
}

/// A non-blocking, journal-specific OS lock serializes proof preparation and
/// mirror renames. The OS releases it even if a writer process crashes.
pub struct DesktopMirrorWriter {
    store: DesktopStateStore,
    _lock: fs::File,
}

pub(super) fn create_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    connection.execute_batch("CREATE TABLE desktop_state_mirror_provenance (
        owner TEXT NOT NULL, source TEXT NOT NULL CHECK(length(source)<=512),
        raw_sha256 TEXT NOT NULL CHECK(length(raw_sha256)=64),
        size INTEGER NOT NULL CHECK(size>=0 AND size<=33554432),
        format INTEGER NOT NULL CHECK(format>=0), origin TEXT NOT NULL CHECK(length(origin)=64),
        journal_id TEXT NOT NULL CHECK(length(journal_id)=64),
        workspace TEXT NOT NULL CHECK(length(workspace)=64),
        sequence INTEGER NOT NULL CHECK(sequence>0), binding TEXT NOT NULL CHECK(length(binding)=64),
        PRIMARY KEY(owner,source,raw_sha256),
        FOREIGN KEY(owner) REFERENCES desktop_state_owners(owner) ON UPDATE RESTRICT ON DELETE RESTRICT
    ) STRICT;")?;
    Ok(())
}

pub(super) fn verify_schema(connection: &Connection) -> DesktopStateStoreResult<()> {
    verify_table_column_layout(
        connection,
        TABLE,
        &[
            "owner",
            "source",
            "raw_sha256",
            "size",
            "format",
            "origin",
            "journal_id",
            "workspace",
            "sequence",
            "binding",
        ],
    )
}

fn database_root(connection: &Connection) -> DesktopStateStoreResult<PathBuf> {
    let path: String = connection.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'",
        [],
        |row| row.get(0),
    )?;
    let parent = Path::new(&path)
        .parent()
        .ok_or_else(|| integrity("mirror journal has no directory"))?;
    Ok(fs::canonicalize(parent)?)
}

fn regular_metadata(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("mirror path is a symbolic link"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("mirror path is a reparse point"));
        }
    }
    Ok(metadata)
}

fn source_id(root: &Path, path: &Path) -> DesktopStateStoreResult<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| integrity("mirror path is outside its journal workspace"))?;
    let mut cursor = root.to_path_buf();
    let mut parts = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(integrity("mirror path has an unsafe component"));
        };
        let part = part
            .to_str()
            .ok_or_else(|| integrity("mirror path is not UTF-8"))?;
        if part.is_empty() || part.contains(['/', '\\', ':']) {
            return Err(integrity("invalid mirror path component"));
        }
        cursor.push(part);
        match regular_metadata(&cursor) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        parts.push(part);
    }
    let name = parts
        .last()
        .ok_or_else(|| integrity("mirror filename is missing"))?;
    let allowed = ["timer_state.json", "timer_state_guest.json"]
        .iter()
        .any(|base| {
            *name == *base
                || *name == format!("{base}.bak")
                || name.starts_with(&format!("{base}.tmp-"))
                || name.starts_with(&format!("{base}.invalid-"))
        });
    if !allowed || parts.len() > 4 {
        return Err(integrity("path is not a managed desktop recovery source"));
    }
    let source = parts.join("/");
    if source.len() > 512 {
        return Err(integrity("mirror source identity exceeds its limit"));
    }
    Ok(source)
}

fn normalized_target(root: &Path, path: &Path) -> DesktopStateStoreResult<PathBuf> {
    // Resolve only the parent after checking each existing lexical component.
    // This preserves no-follow checks rather than canonicalizing a link away.
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    for ancestor in absolute.ancestors() {
        match regular_metadata(ancestor) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let parent = absolute
        .parent()
        .ok_or_else(|| integrity("mirror target has no parent"))?;
    let resolved = fs::canonicalize(parent)?.join(absolute.file_name().unwrap());
    source_id(&fs::canonicalize(root)?, &resolved)?;
    Ok(resolved)
}

fn binding(proof: &Proof) -> String {
    let bytes = serde_json::to_vec(&(
        "gridtimer-mirror-proof-v1",
        &proof.owner,
        &proof.source,
        &proof.raw_sha256,
        proof.size,
        proof.format,
        &proof.origin,
        &proof.journal_id,
        &proof.workspace,
        proof.sequence,
    ))
    .expect("mirror identity is serializable");
    sha256_hex(&bytes)
}

fn proofs(connection: &Connection, owner: &str) -> DesktopStateStoreResult<Vec<Proof>> {
    let mut statement = connection.prepare("SELECT owner,source,raw_sha256,size,format,origin,journal_id,workspace,sequence,binding FROM desktop_state_mirror_provenance WHERE owner=?1 LIMIT 65")?;
    let rows = statement.query_map(params![owner], |row| {
        Ok(Proof {
            owner: row.get(0)?,
            source: row.get(1)?,
            raw_sha256: row.get(2)?,
            size: row.get(3)?,
            format: row.get(4)?,
            origin: row.get(5)?,
            journal_id: row.get(6)?,
            workspace: row.get(7)?,
            sequence: row.get(8)?,
            binding: row.get(9)?,
        })
    })?;
    let result = rows.collect::<Result<Vec<_>, _>>()?;
    if result.len() > MAX_SOURCES * 2 {
        return Err(integrity("mirror proof count exceeds its bound"));
    }
    let evidence = read_journal_evidence(connection)?;
    let root = database_root(connection)?;
    let workspace = sha256_hex(desktop_note_session_scope(&root, owner)?.as_bytes());
    for proof in &result {
        if proof.owner != owner
            || proof.journal_id != evidence.journal_id
            || proof.workspace != workspace
            || proof.sequence <= 0
            || proof.sequence > evidence.commit_sequence
            || proof.binding != binding(proof)
            || proof.size < 0
            || proof.size > MAX_BYTES as i64
            || source_id(&root, &root.join(&proof.source))? != proof.source
        {
            return Err(integrity("managed mirror provenance did not verify"));
        }
    }
    Ok(result)
}

pub(super) fn read_source(root: &Path, source: &str) -> io::Result<String> {
    let path = root.join(source);
    source_id(root, &path).map_err(|error| io::Error::other(error.to_string()))?;
    let metadata = regular_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(io::Error::other("managed mirror exceeds its file bound"));
    }
    let mut raw = String::new();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if opened.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("opened mirror is a reparse point"));
        }
    }
    if !opened.is_file() {
        return Err(io::Error::other("opened mirror is not a regular file"));
    }
    file.take(MAX_BYTES as u64 + 1).read_to_string(&mut raw)?;
    if raw.len() > MAX_BYTES {
        return Err(io::Error::other(
            "managed mirror grew beyond its file bound",
        ));
    }
    Ok(raw)
}

fn exact(proof: &Proof, raw: &str) -> bool {
    proof.size == raw.len() as i64 && proof.raw_sha256 == sha256_hex(raw.as_bytes())
}

pub(super) fn verified_source(
    connection: &Connection,
    owner: &str,
    path: &Path,
    raw: &str,
) -> DesktopStateStoreResult<bool> {
    if raw.len() > MAX_BYTES || !owner_registry_initialized(connection, owner)? {
        return Ok(false);
    }
    let root = database_root(connection)?;
    let source = source_id(&root, &normalized_target(&root, path)?)?;
    Ok(proofs(connection, owner)?
        .iter()
        .any(|proof| proof.source == source && exact(proof, raw)))
}

pub(super) fn include_sources(
    connection: &Connection,
    owner: &str,
    view: &mut crate::desktop_private_media_index::DesktopPrivateMediaReferences,
) -> DesktopStateStoreResult<()> {
    let root = database_root(connection)?;
    let entries = proofs(connection, owner)?;
    let policy = privacy::read_privacy_policy(connection, owner)?.0;
    let sources: std::collections::BTreeSet<_> =
        entries.iter().map(|proof| proof.source.as_str()).collect();
    for source in sources {
        match read_source(&root, source) {
            Ok(raw)
                if entries
                    .iter()
                    .any(|proof| proof.source == source && exact(proof, &raw)) =>
            {
                let redacted = policy.redact_json(&raw)?;
                if view
                    .include_retained_snapshot(redacted.as_deref().unwrap_or(&raw))
                    .is_err()
                {
                    view.mark_unverified_recovery_file(&root.join(source));
                }
            }
            // Prepared proofs alone never claim that bytes exist.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => view.mark_unverified_recovery_file(&root.join(source)),
        }
    }
    Ok(())
}

impl DesktopStateStore {
    pub fn prepare_managed_recovery_directory(&self, path: &Path) -> DesktopStateStoreResult<()> {
        let lexical_root = self
            .database_path()
            .parent()
            .ok_or_else(|| integrity("mirror journal has no root"))?;
        let root = fs::canonicalize(lexical_root)?;
        let relative = path
            .strip_prefix(lexical_root)
            .or_else(|_| path.strip_prefix(&root))
            .map_err(|_| integrity("mirror directory is outside its workspace"))?;
        let resolved = root.join(relative);
        source_id(&root, &resolved)?;
        fs::create_dir_all(
            resolved
                .parent()
                .ok_or_else(|| integrity("mirror has no parent"))?,
        )?;
        source_id(&root, &resolved)?;
        Ok(())
    }

    pub fn read_managed_recovery_file(&self, path: &Path) -> io::Result<String> {
        let root = self
            .database_path()
            .parent()
            .ok_or_else(|| io::Error::other("mirror journal has no root"))?;
        let normalized = normalized_target(root, path).map_err(|error| match error {
            DesktopStateStoreError::Io(error) => error,
            other => io::Error::other(other.to_string()),
        })?;
        let root = fs::canonicalize(root)?;
        let source =
            source_id(&root, &normalized).map_err(|error| io::Error::other(error.to_string()))?;
        read_source(&root, &source)
    }

    pub fn recovery_mirror_is_verified(
        &self,
        owner: &str,
        path: &Path,
        raw: &str,
    ) -> DesktopStateStoreResult<bool> {
        match self.read_managed_recovery_file(path) {
            Ok(actual) if actual == raw => {}
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        let mut connection = self.open_connection(false)?;
        verify_required_schema(&connection)?;
        let transaction = connection.transaction()?;
        verified_source(&transaction, owner, path, raw)
    }

    pub fn lock_recovery_mirror_writer(&self) -> DesktopStateStoreResult<DesktopMirrorWriter> {
        let root = self
            .database_path()
            .parent()
            .ok_or_else(|| integrity("mirror journal has no root"))?;
        let lock_path = root.join("desktop_state_mirrors.lock");
        match regular_metadata(&lock_path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(integrity("mirror writer lock is not a regular file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000);
        }
        let file = options.open(&lock_path)?;
        regular_metadata(&lock_path)?;
        file.try_lock()
            .map_err(|error| integrity(format!("desktop mirror writer is busy: {error}")))?;
        Ok(DesktopMirrorWriter {
            store: Self {
                database_path: self.database_path.clone(),
            },
            _lock: file,
        })
    }
}

impl DesktopMirrorWriter {
    /// Prove target and temporary bytes before either is written. A source
    /// argument permits only verified rotation or deterministic privacy redaction.
    /// False means preserved legacy bytes remain unknown, not newly authorized.
    pub fn prepare(
        &self,
        owner: &str,
        target: &Path,
        temporary: &Path,
        raw: &str,
        source: Option<(&Path, &str)>,
    ) -> DesktopStateStoreResult<bool> {
        self.prepare_inner(owner, target, temporary, raw, source, None)
    }

    /// Reuse the exact semantic result of this save while still reading the
    /// current row and comparing every byte and metadata field in this reserved
    /// transaction. A concurrent write, rollback or replacement invalidates the
    /// receipt, even if filesystem timestamps happen to be unchanged.
    pub fn prepare_committed(
        &self,
        owner: &str,
        target: &Path,
        temporary: &Path,
        raw: &str,
        source: Option<(&Path, &str)>,
        committed: &mut DesktopCommittedSnapshot,
    ) -> DesktopStateStoreResult<bool> {
        self.prepare_inner(owner, target, temporary, raw, source, Some(committed))
    }

    fn prepare_inner(
        &self,
        owner: &str,
        target: &Path,
        temporary: &Path,
        raw: &str,
        source: Option<(&Path, &str)>,
        mut committed: Option<&mut DesktopCommittedSnapshot>,
    ) -> DesktopStateStoreResult<bool> {
        validate_owner(owner)?;
        if raw.len() > MAX_BYTES {
            return Err(integrity("mirror exceeds its byte bound"));
        }
        let mut connection = self.store.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        verify_required_schema(&transaction)?;
        let verified = if let Some(receipt) = committed.as_deref() {
            if fs::canonicalize(self.store.database_path())? != receipt.database_path
                || receipt.snapshot.owner != owner
                || read_journal_evidence(&transaction)? != receipt.evidence
            {
                return Err(integrity(
                    "committed mirror receipt no longer matches this journal",
                ));
            }
            let exact = std::iter::once(&receipt.snapshot)
                .chain(receipt.parent.iter())
                .find(|snapshot| snapshot.owner == owner && snapshot.app_data_json == raw);
            match exact {
                Some(snapshot) => {
                    if !snapshot_matches_row(&transaction, snapshot)? {
                        return Err(integrity(
                            "committed mirror snapshot changed before publication",
                        ));
                    }
                    Some((
                        snapshot.schema_version,
                        snapshot.raw_sha256.clone(),
                        snapshot.envelope_sha256.clone(),
                    ))
                }
                None => None,
            }
        } else {
            None
        };
        let root = database_root(&transaction)?;
        let lexical_root = self.store.database_path().parent().unwrap();
        let ids = [target, temporary].map(|path| {
            let normalized = normalized_target(lexical_root, path)?;
            source_id(&root, &normalized)
        });
        let ids = [
            ids[0]
                .as_ref()
                .map_err(|e| integrity(e.to_string()))?
                .clone(),
            ids[1]
                .as_ref()
                .map_err(|e| integrity(e.to_string()))?
                .clone(),
        ];
        if !owner_registry_initialized(&transaction, owner)? {
            return Ok(false);
        }
        let mut entries = proofs(&transaction, owner)?;
        let hash = verified
            .as_ref()
            .map(|(_, hash, _)| hash.clone())
            .unwrap_or_else(|| sha256_hex(raw.as_bytes()));
        let origin = if let Some((_, _, origin)) = verified.as_ref() {
            Some(origin.clone())
        } else {
            if let Some(snapshot) = read_snapshot_by_raw_digest(&transaction, owner, &hash, 0)? {
                if snapshot.app_data_json != raw {
                    return Err(integrity("mirror snapshot bytes did not match"));
                }
                Some(snapshot.envelope_sha256)
            } else if let Some((path, original)) = source {
                let path = normalized_target(lexical_root, path)?;
                let id = source_id(&root, &path)?;
                let prior = entries
                    .iter()
                    .find(|entry| entry.source == id && exact(entry, original));
                let snapshot = read_snapshot_by_raw_digest(
                    &transaction,
                    owner,
                    &sha256_hex(original.as_bytes()),
                    0,
                )?;
                let origin = prior.map(|proof| proof.origin.clone()).or_else(|| {
                    snapshot
                        .filter(|s| s.app_data_json == original)
                        .map(|s| s.envelope_sha256)
                });
                let policy = privacy::read_privacy_policy(&transaction, owner)?.0;
                let redacted = policy.redact_json(original)?;
                if raw != original && raw != redacted.as_deref().unwrap_or(original) {
                    return Ok(false);
                }
                origin
            } else {
                None
            }
        };
        let Some(origin) = origin else {
            return Ok(false);
        };
        let format = match verified {
            Some((format, _, _)) => format,
            None => analyze_app_data_json(raw, 0)?.schema_version,
        };
        // Only stale evidence for touched sources is normally collected. At the
        // capacity boundary, collect absent/stale files without deleting bytes.
        let mut sources: std::collections::BTreeSet<_> =
            entries.iter().map(|p| p.source.clone()).collect();
        sources.extend(ids.iter().cloned());
        let collect_all = sources.len() > MAX_SOURCES;
        {
            enum SourceObservation {
                Read { size: i64, hash: Option<String> },
                Missing,
                Unreadable,
            }
            // This observation is only for pruning old proofs in this prepare.
            // It never authorizes the new bytes or survives a mirror rename.
            // Recovery readers independently reopen and verify actual content.
            let mut observations = BTreeMap::new();
            for entry in &entries {
                if !collect_all && !ids.contains(&entry.source) {
                    continue;
                }
                let observed = observations
                    .entry(entry.source.as_str())
                    .or_insert_with(|| match read_source(&root, &entry.source) {
                        Ok(current) => {
                            let size = current.len() as i64;
                            let hash = entries
                                .iter()
                                .any(|proof| proof.source == entry.source && proof.size == size)
                                .then(|| sha256_hex(current.as_bytes()));
                            SourceObservation::Read { size, hash }
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            SourceObservation::Missing
                        }
                        Err(_) => SourceObservation::Unreadable,
                    });
                let present = match observed {
                    SourceObservation::Read { size, hash } => {
                        entry.size == *size && hash.as_deref() == Some(entry.raw_sha256.as_str())
                    }
                    SourceObservation::Missing => false,
                    SourceObservation::Unreadable => true,
                };
                if !present {
                    transaction.execute("DELETE FROM desktop_state_mirror_provenance WHERE owner=?1 AND source=?2 AND raw_sha256=?3", params![owner,entry.source,entry.raw_sha256])?;
                }
            }
        }
        entries = proofs(&transaction, owner)?;
        sources = entries.iter().map(|p| p.source.clone()).collect();
        sources.extend(ids.iter().cloned());
        if sources.len() > MAX_SOURCES {
            return Err(integrity(
                "managed mirror provenance capacity is full; existing evidence was retained",
            ));
        }
        let before = read_journal_evidence(&transaction)?;
        let next = before
            .commit_sequence
            .checked_add(1)
            .ok_or_else(|| integrity("mirror evidence sequence exhausted"))?;
        let workspace = sha256_hex(desktop_note_session_scope(&root, owner)?.as_bytes());
        for source in ids {
            let count = entries.iter().filter(|p| p.source == source).count();
            if count >= 2
                && !entries
                    .iter()
                    .any(|p| p.source == source && p.raw_sha256 == hash)
            {
                return Err(integrity(
                    "mirror source still has two uninspected generations",
                ));
            }
            let mut proof = Proof {
                owner: owner.into(),
                source,
                raw_sha256: hash.clone(),
                size: raw.len() as i64,
                format,
                origin: origin.clone(),
                journal_id: before.journal_id.clone(),
                workspace: workspace.clone(),
                sequence: next,
                binding: String::new(),
            };
            proof.binding = binding(&proof);
            transaction.execute("INSERT INTO desktop_state_mirror_provenance(owner,source,raw_sha256,size,format,origin,journal_id,workspace,sequence,binding) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(owner,source,raw_sha256) DO UPDATE SET size=excluded.size,format=excluded.format,origin=excluded.origin,journal_id=excluded.journal_id,workspace=excluded.workspace,sequence=excluded.sequence,binding=excluded.binding",
                params![proof.owner,proof.source,proof.raw_sha256,proof.size,proof.format,proof.origin,proof.journal_id,proof.workspace,proof.sequence,proof.binding])?;
        }
        advance_metadata_commit_sequence(&transaction, &before)?;
        let after = read_journal_evidence(&transaction)?;
        transaction.commit()?;
        if let Some(receipt) = committed.as_deref_mut() {
            receipt.evidence = after;
        }
        Ok(true)
    }
}
