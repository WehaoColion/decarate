// v0.0.6 - Bind transport receipts to compact identities and recheck them at commit.
// v0.0.5 - Distinguish retained transport receipts from live editor authority.
// v0.0.4 - Bind receipts to exact retained ciphertexts, including conflict records.
// v0.0.3 - Accept stronger retained content proofs without downgrading metadata.
// v0.0.2 - Bind imported declarations to the active account and parse each snapshot once.
// v0.0.1 - Carry authenticated session references without exposing session tokens to persistence.
use super::*;
use crate::sealed_media_references::SealedMediaReferences;

/// Created from a live encryption session or authenticated transport receipt. It contains
/// attachment identities and an envelope binding, never passwords or note text.
#[derive(Clone)]
pub struct DesktopSealedMediaDeclaration {
    note_id: String,
    references: SealedMediaReferences,
    allow_retained_snapshot: bool,
    authority: DeclarationAuthority,
}

#[derive(Clone)]
enum DeclarationAuthority {
    SessionScope(String),
    TransportOwner(String),
}

impl DesktopSealedMediaDeclaration {
    /// Import only a transport-verified response for the currently active account
    /// and restore state. A note edited during the request gets no stale binding.
    pub fn from_verified_replies(
        receipts: &[crate::private_media_protocol::VerifiedPrivateMediaReply],
        snapshot: &str,
        user: &str,
        token: &str,
        server: &str,
        namespace: &str,
        generation: i64,
    ) -> DesktopStateStoreResult<Vec<Self>> {
        Self::validate_receipt_scope(receipts, user, token, server, namespace, generation)?;
        let owner = desktop_state_owner(server, namespace, user)?;
        let snapshot: Value = serde_json::from_str(snapshot)?;
        let index = crate::private_media_retained::RetainedSealedNotes::from_snapshot(&snapshot)
            .map_err(integrity)?;
        let mut declarations = Vec::new();
        for entry in receipts.iter().flat_map(|receipt| receipt.entries()) {
            let Some(note) = index.get(&entry.note_id, &entry.envelope_sha256) else {
                continue;
            };
            if let Some(value) = &entry.declaration {
                let references: SealedMediaReferences = serde_json::from_value(value.clone())?;
                if !references.valid_for(&entry.note_id, &note["encryption"]) {
                    return Err(integrity(
                        "private reference receipt has an invalid envelope binding",
                    ));
                }
                declarations.push(Self {
                    note_id: entry.note_id.clone(),
                    references,
                    allow_retained_snapshot: false,
                    authority: DeclarationAuthority::TransportOwner(owner.clone()),
                });
            }
        }
        Ok(declarations)
    }

    fn validate_receipt_scope(
        receipts: &[crate::private_media_protocol::VerifiedPrivateMediaReply],
        user: &str,
        token: &str,
        server: &str,
        namespace: &str,
        generation: i64,
    ) -> DesktopStateStoreResult<()> {
        if user.trim().is_empty()
            || token.trim().is_empty()
            || server.is_empty()
            || namespace.is_empty()
        {
            return Err(integrity(
                "private reference receipt has no complete account identity",
            ));
        }
        desktop_state_owner(server, namespace, user)?;
        for receipt in receipts {
            if receipt.user_id != user
                || receipt.token_id != sync_core::token_identifier(token)
                || receipt.server_instance_id != server
                || receipt.account_namespace != namespace
                || receipt.generation() != generation
            {
                return Err(integrity(
                    "private reference receipt no longer belongs to this workspace",
                ));
            }
        }
        Ok(())
    }

    /// Validate the active peer before skipping receipts that are already durable.
    /// No historical JSON is needed for this read-only idempotency decision.
    pub fn receipts_are_retained(
        receipts: &[crate::private_media_protocol::VerifiedPrivateMediaReply],
        policy: &DesktopPrivacyPolicy,
        user: &str,
        token: &str,
        server: &str,
        namespace: &str,
        generation: i64,
    ) -> DesktopStateStoreResult<bool> {
        Self::validate_receipt_scope(receipts, user, token, server, namespace, generation)?;
        for entry in receipts.iter().flat_map(|receipt| receipt.entries()) {
            let Some(value) = &entry.declaration else {
                continue;
            };
            let references: SealedMediaReferences = serde_json::from_value(value.clone())?;
            if !references.valid_metadata() || references.envelope_sha256() != entry.envelope_sha256
            {
                return Err(integrity(
                    "private reference receipt has invalid retained metadata",
                ));
            }
            if !policy
                .sealed_declaration_by_fingerprint(&entry.envelope_sha256)
                .is_some_and(|known| known.covers(&references))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn is_retained_by(&self, policy: &DesktopPrivacyPolicy) -> bool {
        policy
            .sealed_declaration_by_fingerprint(self.references.envelope_sha256())
            .is_some_and(|known| known.covers(&self.references))
    }

    /// The store rechecks every retained ciphertext in its owner transaction
    /// before allowing these authenticated receipts to update private metadata.
    pub fn from_verified_retained_replies(
        receipts: &[crate::private_media_protocol::VerifiedPrivateMediaReply],
        references: &crate::desktop_private_media_index::DesktopPrivateMediaReferences,
        user: &str,
        token: &str,
        server: &str,
        namespace: &str,
        generation: i64,
    ) -> DesktopStateStoreResult<Vec<Self>> {
        Self::validate_receipt_scope(receipts, user, token, server, namespace, generation)?;
        let owner = desktop_state_owner(server, namespace, user)?;
        let mut declarations = Vec::new();
        for entry in receipts.iter().flat_map(|receipt| receipt.entries()) {
            if !references.contains(&entry.note_id, &entry.envelope_sha256) {
                continue;
            }
            let Some(value) = &entry.declaration else {
                continue;
            };
            let declaration: SealedMediaReferences = serde_json::from_value(value.clone())?;
            if !declaration.valid_metadata()
                || declaration.envelope_sha256() != entry.envelope_sha256
            {
                return Err(integrity(
                    "private reference receipt has an invalid envelope binding",
                ));
            }
            declarations.push(Self {
                note_id: entry.note_id.clone(),
                references: declaration,
                allow_retained_snapshot: true,
                authority: DeclarationAuthority::TransportOwner(owner.clone()),
            });
        }
        Ok(declarations)
    }

    pub(super) fn permits_retained_snapshot(&self) -> bool {
        self.allow_retained_snapshot
    }

    pub fn from_session(sealed_note: &str, session_token: &str) -> Option<Self> {
        let (raw, scope) =
            crate::note_crypto::session_media_references_with_scope(sealed_note, session_token)?;
        // Unscoped sessions remain compatible with the note API, but cannot
        // authorize writes to any desktop account or workspace.
        if !scope.starts_with("desktop-note-session-v1:") {
            return None;
        }
        let note: Value = serde_json::from_str(sealed_note).ok()?;
        Some(Self {
            note_id: note.get("id")?.as_str()?.to_owned(),
            references: serde_json::from_str(&raw).ok()?,
            allow_retained_snapshot: false,
            authority: DeclarationAuthority::SessionScope(scope),
        })
    }

    pub fn from_snapshot_session(
        snapshot: &str,
        note_id: &str,
        session_token: &str,
    ) -> Option<Self> {
        if note_id.is_empty() || session_token.is_empty() {
            return None;
        }
        let snapshot: Value = serde_json::from_str(snapshot).ok()?;
        let index =
            crate::private_media_retained::RetainedSealedNotes::from_snapshot(&snapshot).ok()?;
        let declaration = index
            .records()
            .filter(|(_, note)| note["id"] == note_id)
            .find_map(|(_, note)| Self::from_session(&note.to_string(), session_token));
        declaration
    }

    pub fn note_id(&self) -> &str {
        &self.note_id
    }

    pub fn metadata_json(&self) -> String {
        serde_json::to_string(&self.references).expect("reference metadata is serializable")
    }

    pub(super) fn apply_to_index(
        &self,
        owner: &str,
        session_scope: &str,
        policy: &DesktopPrivacyPolicy,
        index: &crate::desktop_private_media_index::DesktopPrivateMediaReferences,
    ) -> DesktopStateStoreResult<DesktopPrivacyPolicy> {
        self.validate_authority(owner, session_scope)?;
        let next =
            policy.including_indexed_sealed_media(index, &self.note_id, self.references.clone())?;
        if !next
            .sealed_declaration_by_fingerprint(self.references.envelope_sha256())
            .is_some_and(|retained| retained.covers(&self.references))
        {
            return Err(integrity(
                "sealed reference declaration exceeded the retained metadata capacity",
            ));
        }
        Ok(next)
    }

    pub(super) fn validate_authority(
        &self,
        owner: &str,
        session_scope: &str,
    ) -> DesktopStateStoreResult<()> {
        let matches = match &self.authority {
            DeclarationAuthority::SessionScope(expected) => expected == session_scope,
            DeclarationAuthority::TransportOwner(expected) => expected == owner,
        };
        if !matches {
            return Err(integrity(
                "private reference declaration belongs to another account or workspace",
            ));
        }
        Ok(())
    }
}
