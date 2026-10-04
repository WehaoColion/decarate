// v0.0.4 - Commit reference metadata without reprojecting unchanged account history.
// v0.0.3 - Hash only requested note envelopes and reuse verified historical decoding.
// v0.0.2 - Accept first declarations from account-bound verified history and conflicts.
// v0.0.1 - Exchange declarations under the same lock as token and restore checks.
use super::*;
use crate::private_media_protocol::{
    PrivateMediaReply, PrivateMediaReplyEntry, PrivateMediaRequest,
};
use crate::private_media_retained::RetainedSealedNotes;
use crate::sealed_media_references::SealedMediaReferences;

fn include_retained_declarations(
    index: &RetainedSealedNotes<'_>,
    current_snapshot: bool,
    declarations: &[Option<SealedMediaReferences>],
    entries: &mut [PrivateMediaReplyEntry],
    policy: &mut crate::desktop_state_store::DesktopPrivacyPolicy,
) -> StoreResult<()> {
    for (entry, declaration) in entries.iter_mut().zip(declarations) {
        if entry.current_head_matches || entry.retained_note_matches {
            continue;
        }
        let Some(note) = index.get(&entry.note_id, &entry.envelope_sha256) else {
            continue;
        };
        entry.current_head_matches =
            current_snapshot && index.is_current(&entry.note_id, &entry.envelope_sha256);
        entry.retained_note_matches = !entry.current_head_matches;
        if let Some(declaration) = declaration {
            *policy = policy
                .including_sealed_media(note, declaration.clone())
                .map_err(|error| StoreError::Integrity(error.to_string()))?;
            entry.accepted = true;
        }
    }
    Ok(())
}

impl SqliteServerStore {
    pub(crate) fn exchange_private_media_references(
        &self,
        user: &str,
        token_id: i64,
        request: &PrivateMediaRequest,
        now: i64,
    ) -> StoreResult<PrivateMediaReply> {
        #[cfg(test)]
        let _profile = ProfileGuard::start();
        request.validate().map_err(StoreError::Integrity)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active = transaction.query_row(
            "SELECT COUNT(*) FROM tokens WHERE id=?1 AND user_id=?2 AND activation_state=1 AND revoked_at_epoch_millis IS NULL AND expires_at_epoch_millis>?3",
            params![token_id, user, now], |row| row.get::<_, i64>(0))?;
        if active != 1 {
            return Err(StoreError::Integrity(
                "Private attachment session is no longer active.".into(),
            ));
        }
        acknowledge_restore_barrier_in_transaction(
            &transaction,
            user,
            Some((
                token_id,
                request.acknowledged_generation,
                request.restore_receipt.as_str(),
                false,
            )),
        )?;
        #[cfg(test)]
        profile_step("bind_request");
        let current = read_account_in_transaction(&transaction, user)?
            .ok_or_else(|| StoreError::NotFound("private attachment account".into()))?;
        let snapshot: serde_json::Value = serde_json::from_str(&current.app_data_json)?;
        let requested_ids = request
            .queries
            .iter()
            .map(|query| query.note_id.as_str())
            .collect();
        let index = RetainedSealedNotes::for_note_ids(&snapshot, &requested_ids)
            .map_err(StoreError::Integrity)?;
        #[cfg(test)]
        profile_step("read_current_index");
        let previous = note_privacy::read_policy(&transaction, user)?;
        #[cfg(test)]
        profile_step("read_policy");
        let mut policy = previous.clone();
        let declarations = request
            .queries
            .iter()
            .map(|query| query.parsed_declaration().map_err(StoreError::Integrity))
            .collect::<StoreResult<Vec<_>>>()?;
        let mut entries = request
            .queries
            .iter()
            .map(|query| PrivateMediaReplyEntry {
                note_id: query.note_id.clone(),
                envelope_sha256: query.envelope_sha256.clone(),
                current_head_matches: false,
                retained_note_matches: false,
                accepted: false,
                declaration: None,
            })
            .collect::<Vec<_>>();
        include_retained_declarations(&index, true, &declarations, &mut entries, &mut policy)?;
        // Stream one verified historical document at a time. Already retained
        // declarations and read-only queries never require an archive scan.
        #[cfg(test)]
        profile_step("bind_current_declarations");
        let mut before_revision = i64::MAX;
        while entries
            .iter()
            .zip(&declarations)
            .any(|(entry, declaration)| {
                !entry.accepted
                    && declaration.as_ref().is_some_and(|submitted| {
                        policy
                            .sealed_declaration_by_fingerprint(&entry.envelope_sha256)
                            .is_none_or(|known| !known.covers(submitted))
                    })
            })
        {
            let stored = transaction.query_row(
                "SELECT h.user_id,h.revision,h.created_at_epoch_millis,c.sha256,c.compression,c.uncompressed_size_bytes,c.compressed_size_bytes,c.content
                 FROM account_snapshot_history h JOIN snapshot_contents c ON c.sha256=h.content_sha256
                 WHERE h.user_id=?1 AND h.revision<?2 ORDER BY h.revision DESC LIMIT 1",
                params![user, before_revision], snapshot_content_row).optional()?;
            let Some(stored) = stored else {
                break;
            };
            // Decoding already verifies compressed content, JSON structure and
            // the retained snapshot digest. Do not repeat that full byte pass.
            let history = decode_snapshot_history(stored)?;
            if history.user_id != user {
                return Err(StoreError::Integrity(
                    "Retained note belongs to another account.".into(),
                ));
            }
            before_revision = history.revision;
            let retained: serde_json::Value = serde_json::from_str(&history.app_data_json)?;
            let index = RetainedSealedNotes::for_note_ids(&retained, &requested_ids)
                .map_err(StoreError::Integrity)?;
            include_retained_declarations(&index, false, &declarations, &mut entries, &mut policy)?;
        }
        #[cfg(test)]
        profile_step("scan_history");
        let committed = if policy != previous {
            note_privacy::record_reference_metadata(&transaction, user, policy)?
        } else {
            policy
        };
        #[cfg(test)]
        profile_step("persist_reference_metadata");
        #[cfg(test)]
        profile_step("read_committed_policy");
        for entry in &mut entries {
            entry.declaration = committed
                .sealed_declaration_by_fingerprint(&entry.envelope_sha256)
                .map(serde_json::to_value)
                .transpose()?;
        }
        let reply = PrivateMediaReply {
            format_version: 1,
            request_id: request.request_id.clone(),
            generation: request.acknowledged_generation,
            entries,
        };
        // Includes capacity eviction and contradictory-declaration checks before
        // acknowledging any write. A rejected reply rolls back the restore ack too.
        reply.validate_for(request).map_err(StoreError::Integrity)?;
        #[cfg(test)]
        profile_step("validate_reply");
        if committed != previous {
            privacy_journal::stage(&transaction, user, &committed)?;
        }
        #[cfg(test)]
        profile_step("stage_journal");
        privacy_journal::commit(transaction)?;
        #[cfg(test)]
        profile_step("commit_journal");
        Ok(reply)
    }
}

#[cfg(test)]
thread_local! {
    static PROFILE: std::cell::RefCell<(Option<std::time::Instant>, Vec<(&'static str, f64)>)> = const {
        std::cell::RefCell::new((None, Vec::new()))
    };
}

#[cfg(test)]
pub(super) fn profile_step(label: &'static str) {
    PROFILE.with(|profile| {
        let mut profile = profile.borrow_mut();
        if let Some(last) = profile.0.replace(std::time::Instant::now()) {
            profile
                .1
                .push((label, last.elapsed().as_secs_f64() * 1000.0));
        } else {
            profile.0 = None;
        }
    });
}

#[cfg(test)]
struct ProfileGuard;
#[cfg(test)]
impl ProfileGuard {
    fn start() -> Self {
        PROFILE
            .with(|profile| *profile.borrow_mut() = (Some(std::time::Instant::now()), Vec::new()));
        Self
    }
}
#[cfg(test)]
impl Drop for ProfileGuard {
    fn drop(&mut self) {
        profile_step("return");
        PROFILE.with(|profile| profile.borrow_mut().0 = None);
    }
}

#[cfg(test)]
impl SqliteServerStore {
    pub(crate) fn take_private_media_phase_timings() -> Vec<(&'static str, f64)> {
        PROFILE.with(|profile| std::mem::take(&mut profile.borrow_mut().1))
    }
}
