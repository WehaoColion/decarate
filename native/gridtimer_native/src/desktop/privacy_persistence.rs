// v1.1.0.3 Windows - Reuse exact commit evidence for ordinary mirror rotation.
// v0.0.2 - Share bounded workspace-only mirror enumeration with media retention.
#[cfg(test)]
fn write_proven_desktop_mirror(
    root: &Path,
    store: &DesktopStateStore,
    writer: &gridtimer_native::desktop_state_store::DesktopMirrorWriter,
    owner: &str,
    state_path: &Path,
    target: &Path,
    raw: &str,
    source: Option<(&Path, &str)>,
    required: bool,
) -> io::Result<()> {
    write_proven_desktop_mirror_with_commit(
        root, store, writer, owner, state_path, target, raw, source, required, None,
    )
}

fn write_proven_desktop_mirror_with_commit(
    root: &Path,
    store: &DesktopStateStore,
    writer: &gridtimer_native::desktop_state_store::DesktopMirrorWriter,
    owner: &str,
    state_path: &Path,
    target: &Path,
    raw: &str,
    source: Option<(&Path, &str)>,
    required: bool,
    mut committed: Option<&mut gridtimer_native::desktop_state_store::DesktopCommittedSnapshot>,
) -> io::Result<()> {
    let base = state_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("managed mirror has no filename"))?;
    let target_key = format!("{:x}", Sha256::digest(target.to_string_lossy().as_bytes()));
    let temporary = (0..100u32)
        .map(|attempt| {
            state_path.with_file_name(format!(
                "{base}.tmp-mirror-{}-{}-{}-{attempt}",
                std::process::id(),
                now_millis(),
                &target_key[..8],
            ))
        })
        .find(|path| !path.exists())
        .ok_or_else(|| io::Error::other("cannot reserve a managed mirror temporary file"))?;
    let proven = {
        #[cfg(test)]
        let _stage = desktop_persistence::TimerSaveStageGuard::begin(
            desktop_persistence::TimerSavePhase::MirrorPrepare,
        );
        match committed.as_deref_mut() {
            Some(receipt) => {
                writer.prepare_committed(owner, target, &temporary, raw, source, receipt)
            }
            None => writer.prepare(owner, target, &temporary, raw, source),
        }
    }
    .map_err(desktop_state_store_io_error)?;
    if required && !proven {
        return Err(io::Error::other(
            "the mirror has no verified owner-bound source",
        ));
    }
    if proven {
        #[cfg(test)]
        let _stage = desktop_persistence::TimerSaveStageGuard::begin(
            desktop_persistence::TimerSavePhase::MirrorEvidence,
        );
        // No managed rename can precede the independent evidence checkpoint.
        if let Some(receipt) = committed.as_deref() {
            verify_and_refresh_desktop_state_evidence_value(root, receipt.journal_evidence())?;
        } else {
            verify_and_refresh_desktop_state_evidence(root, store)?;
        }
    }
    #[cfg(test)]
    let _stage = desktop_persistence::TimerSaveStageGuard::begin(
        desktop_persistence::TimerSavePhase::MirrorFile,
    );
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(raw.as_bytes())?;
    file.sync_all()?;
    drop(file);
    // A failed rename keeps the fully written, proven temporary recovery copy.
    atomic_replace_path(&temporary, target)
}

fn save_proven_desktop_state_mirror(
    root: &Path,
    store: &DesktopStateStore,
    owner: &str,
    state_path: &Path,
    raw: &str,
    preserve_as_invalid: bool,
) -> io::Result<()> {
    save_proven_desktop_state_mirror_with_commit(
        root,
        store,
        owner,
        state_path,
        raw,
        preserve_as_invalid,
        None,
    )
}

fn save_proven_desktop_state_mirror_with_commit(
    root: &Path,
    store: &DesktopStateStore,
    owner: &str,
    state_path: &Path,
    raw: &str,
    preserve_as_invalid: bool,
    mut committed: Option<&mut gridtimer_native::desktop_state_store::DesktopCommittedSnapshot>,
) -> io::Result<()> {
    store
        .prepare_managed_recovery_directory(state_path)
        .map_err(desktop_state_store_io_error)?;
    let writer = store
        .lock_recovery_mirror_writer()
        .map_err(desktop_state_store_io_error)?;
    match store.read_managed_recovery_file(state_path) {
        Ok(existing) if existing == raw => {
            // Adopt exact already-verified bytes into independent provenance.
            // This also upgrades pre-provenance mirrors without trusting names.
            if store
                .recovery_mirror_is_verified(owner, state_path, raw)
                .map_err(desktop_state_store_io_error)?
            {
                return Ok(());
            }
        }
        Ok(existing) => {
            let known = committed
                .as_deref()
                .is_some_and(|receipt| receipt.contains_exact_snapshot(owner, &existing));
            let backup = if preserve_as_invalid
                || (!known && app_data::sanitize_app_data_json(&existing, now_millis()).is_none())
            {
                let name = state_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| io::Error::other("invalid mirror name"))?;
                (0..100u32)
                    .map(|attempt| {
                        state_path
                            .with_file_name(format!("{name}.invalid-{}-{attempt}", now_millis()))
                    })
                    .find(|path| !path.exists())
                    .ok_or_else(|| io::Error::other("cannot preserve previous mirror"))?
            } else {
                backup_path(state_path)
            };
            write_proven_desktop_mirror_with_commit(
                root,
                store,
                &writer,
                owner,
                state_path,
                &backup,
                &existing,
                Some((state_path, &existing)),
                false,
                committed.as_deref_mut(),
            )?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            // Preserve non-UTF8 legacy bytes without inventing any provenance.
            preserve_invalid_primary(state_path)?;
        }
        Err(error) => return Err(error),
    }
    write_proven_desktop_mirror_with_commit(
        root,
        store,
        &writer,
        owner,
        state_path,
        state_path,
        raw,
        None,
        true,
        committed.as_deref_mut(),
    )
}

fn redact_desktop_privacy_mirrors(
    state_path: &Path,
    policy: &gridtimer_native::desktop_state_store::DesktopPrivacyPolicy,
    write: bool,
) -> io::Result<()> {
    if policy.is_empty() {
        return Ok(());
    }
    visit_desktop_recovery_mirrors(state_path, |path, raw| {
        if let Some(redacted) = policy
            .redact_json(raw)
            .map_err(desktop_state_store_io_error)?
        {
            if write {
                atomic_replace_text_no_backup(path, &redacted)?;
            }
        }
        Ok(())
    })
}

fn redact_proven_desktop_privacy_mirrors(
    root: &Path,
    store: &DesktopStateStore,
    owner: &str,
    state_path: &Path,
    policy: &gridtimer_native::desktop_state_store::DesktopPrivacyPolicy,
) -> io::Result<()> {
    redact_proven_desktop_privacy_mirrors_with_commit(root, store, owner, state_path, policy, None)
}

fn redact_proven_desktop_privacy_mirrors_with_commit(
    root: &Path,
    store: &DesktopStateStore,
    owner: &str,
    state_path: &Path,
    policy: &gridtimer_native::desktop_state_store::DesktopPrivacyPolicy,
    mut committed: Option<&mut gridtimer_native::desktop_state_store::DesktopCommittedSnapshot>,
) -> io::Result<()> {
    if policy.is_empty() {
        return Ok(());
    }
    let writer = store
        .lock_recovery_mirror_writer()
        .map_err(desktop_state_store_io_error)?;
    visit_desktop_recovery_mirrors(state_path, |path, raw| {
        if let Some(redacted) = policy
            .redact_json(raw)
            .map_err(desktop_state_store_io_error)?
        {
            write_proven_desktop_mirror_with_commit(
                root,
                store,
                &writer,
                owner,
                state_path,
                path,
                &redacted,
                Some((path, raw)),
                false,
                committed.as_deref_mut(),
            )?;
        }
        Ok(())
    })
}

fn visit_desktop_recovery_mirrors(
    state_path: &Path,
    mut visit: impl FnMut(&Path, &str) -> io::Result<()>,
) -> io::Result<()> {
    visit_desktop_recovery_mirror_files(state_path, |path, raw| visit(path, &raw?))
}

fn include_desktop_private_recovery_mirrors(
    store: &DesktopStateStore,
    owner: &str,
    state_path: &Path,
    references: &mut gridtimer_native::desktop_private_media_index::DesktopPrivateMediaReferences,
) -> io::Result<()> {
    let visited = visit_desktop_recovery_mirror_files(state_path, |path, raw| {
        match raw {
            Ok(raw) => {
                store
                    .include_verified_private_media_recovery_file(owner, path, &raw, references)
                    .map_err(desktop_state_store_io_error)?;
            }
            Err(_) => references.mark_unverified_recovery_file(path),
        }
        Ok(())
    });
    if let Err(error) = visited {
        // Filesystem enumeration failures keep unobserved mirrors pending. A
        // database validation failure is propagated so no partial receipt is used.
        references.mark_unverified_recovery_source();
        return Err(error);
    }
    Ok(())
}

fn visit_desktop_recovery_mirror_files(
    state_path: &Path,
    mut visit: impl FnMut(&Path, io::Result<String>) -> io::Result<()>,
) -> io::Result<()> {
    let parent = state_path
        .parent()
        .ok_or_else(|| io::Error::other("workspace mirror has no directory"))?;
    if !parent.exists() {
        return Ok(());
    }
    let name = state_path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| io::Error::other("invalid workspace mirror filename"))?;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            continue;
        };
        if filename != name
            && filename != format!("{name}.bak")
            && !filename.starts_with(&format!("{name}.invalid-"))
            && !filename.starts_with(&format!("{name}.tmp-"))
        {
            continue;
        }
        let raw = (|| {
            let metadata = entry.path().symlink_metadata()?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(io::Error::other(
                    "managed privacy copy is not a regular file",
                ));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(io::Error::other("managed privacy copy is a reparse point"));
                }
            }
            if metadata.len() > 32 * 1024 * 1024 {
                return Err(io::Error::other(
                    "managed privacy copy exceeds the document limit",
                ));
            }
            let mut raw = String::new();
            fs::File::open(entry.path())?
                .take(32 * 1024 * 1024 + 1)
                .read_to_string(&mut raw)?;
            if raw.len() > 32 * 1024 * 1024 {
                return Err(io::Error::other(
                    "managed recovery copy grew beyond the document limit",
                ));
            }
            Ok(raw)
        })();
        visit(&entry.path(), raw)?;
    }
    Ok(())
}
