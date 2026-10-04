//! Account-bound legal report exchange. Raw scan snapshots never enter this store.
use super::*;
use crate::legal_scan::LegalReport;
use crate::sync_core::{LegalReportManifestItem, LegalReportTombstone};
use serde_json::Value;

pub const MAX_LEGAL_REPORT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_LEGAL_REPORT_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_REPORTS_PER_WORKSPACE: i64 = 5;
const MAX_PENDING_UPLOADS_PER_WORKSPACE: i64 = 5;
const MAX_LEGAL_REPORT_STORAGE_BYTES_GLOBAL: i64 = 512 * 1024 * 1024;
const STAGED_UPLOAD_RETENTION_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Clone, Debug)]
pub enum LegalReportUploadOutcome {
    ChunkAccepted { received_bytes: usize },
    Committed,
    AlreadyCommitted,
    Tombstoned,
}

#[derive(Clone, Debug)]
pub struct LegalReportDownloadChunk {
    pub content: Vec<u8>,
    pub total_bytes: usize,
    pub sha256: String,
    pub created_at_epoch_millis: i64,
    pub source_workspace_id: String,
}

pub(super) fn schema_sql() -> &'static str {
    "CREATE TABLE IF NOT EXISTS legal_reports (
         user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
         workspace_id TEXT NOT NULL,
         report_id TEXT NOT NULL,
         created_at_epoch_millis INTEGER NOT NULL CHECK(created_at_epoch_millis > 0),
         source_workspace_id TEXT NOT NULL,
         sha256 TEXT NOT NULL,
         size_bytes INTEGER NOT NULL CHECK(size_bytes > 0 AND size_bytes <= 33554432),
         report_json BLOB NOT NULL,
         PRIMARY KEY(user_id, workspace_id, report_id)
     );
     CREATE INDEX IF NOT EXISTS legal_reports_recent_index ON legal_reports(
         user_id, workspace_id, created_at_epoch_millis DESC, report_id DESC
     );
     CREATE TABLE IF NOT EXISTS legal_report_uploads (
         user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
         workspace_id TEXT NOT NULL,
         report_id TEXT NOT NULL,
         created_at_epoch_millis INTEGER NOT NULL CHECK(created_at_epoch_millis > 0),
         source_workspace_id TEXT NOT NULL,
         sha256 TEXT NOT NULL,
         total_bytes INTEGER NOT NULL CHECK(total_bytes > 0 AND total_bytes <= 33554432),
         content BLOB NOT NULL,
         updated_at_epoch_millis INTEGER NOT NULL CHECK(updated_at_epoch_millis > 0),
         PRIMARY KEY(user_id, workspace_id, report_id)
     );
     CREATE TABLE IF NOT EXISTS legal_report_tombstones (
         user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
         workspace_id TEXT NOT NULL,
         report_id TEXT NOT NULL,
         deleted_at_epoch_millis INTEGER NOT NULL CHECK(deleted_at_epoch_millis > 0),
         PRIMARY KEY(user_id, workspace_id, report_id)
     );"
}

pub(super) fn verify_schema(connection: &Connection) -> StoreResult<()> {
    for table in [
        "legal_reports",
        "legal_report_uploads",
        "legal_report_tombstones",
    ] {
        let exists = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(StoreError::Integrity(format!(
                "required legal report table missing: {table}"
            )));
        }
    }
    let mut statement = connection.prepare(
        "SELECT user_id, workspace_id, report_id, sha256, size_bytes, report_json
         FROM legal_reports",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Vec<u8>>(5)?,
        ))
    })?;
    for row in rows {
        let (_, workspace, report_id, hash, size, content) = row?;
        if !valid_report_id(&report_id)
            || !valid_lowercase_opaque_identifier(&workspace)
            || size != content.len() as i64
            || sha256_hex(&content) != hash
        {
            return Err(StoreError::Integrity(
                "legal report content or identity is corrupt".into(),
            ));
        }
    }
    let overlapping = connection.query_row(
        "SELECT 1 FROM legal_reports r JOIN legal_report_tombstones t
         ON t.user_id=r.user_id AND t.workspace_id=r.workspace_id AND t.report_id=r.report_id LIMIT 1",
        [], |_| Ok(()),
    ).optional()?.is_some();
    let staged_deleted = connection.query_row(
        "SELECT 1 FROM legal_report_uploads u JOIN legal_report_tombstones t
         ON t.user_id=u.user_id AND t.workspace_id=u.workspace_id AND t.report_id=u.report_id LIMIT 1",
        [], |_| Ok(()),
    ).optional()?.is_some();
    let too_many_reports = connection
        .query_row(
            "SELECT 1 FROM legal_reports GROUP BY user_id,workspace_id HAVING COUNT(*) > 5 LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    let too_many_uploads = connection.query_row(
        "SELECT 1 FROM legal_report_uploads GROUP BY user_id,workspace_id HAVING COUNT(*) > 5 LIMIT 1",
        [], |_| Ok(()),
    ).optional()?.is_some();
    if overlapping || staged_deleted || too_many_reports || too_many_uploads {
        return Err(StoreError::Integrity(
            "legal report retention or tombstone state is corrupt".into(),
        ));
    }
    Ok(())
}

fn valid_report_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_source_workspace(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn legal_scope(
    transaction: &Transaction<'_>,
    user_id: &str,
    token_id: i64,
    generation: i64,
    client_workspace_id: &str,
    proof: &str,
) -> StoreResult<String> {
    if !valid_lowercase_opaque_identifier(client_workspace_id)
        || !valid_lowercase_opaque_identifier(proof)
    {
        return Err(StoreError::Integrity(
            "legal report workspace proof is invalid".into(),
        ));
    }
    let barrier = read_restore_barrier(transaction, user_id, token_id)?;
    if generation > barrier.current_generation {
        return Err(StoreError::ServerGenerationRollback {
            client_generation: generation,
            server_generation: barrier.current_generation,
        });
    }
    if generation < barrier.current_generation {
        return Err(StoreError::RestoreGenerationConflict {
            expected_generation: generation,
            actual_generation: barrier.current_generation,
        });
    }
    if !barrier.token_restore_acknowledged || barrier.token_last_seen_generation != generation {
        return Err(StoreError::RestoreReceiptRequired {
            actual_generation: barrier.current_generation,
        });
    }
    let (server_instance_id, secret, namespace, material_generation) =
        read_workspace_capability_material(transaction, user_id)?;
    if material_generation != generation {
        return Err(StoreError::RestoreGenerationConflict {
            expected_generation: generation,
            actual_generation: material_generation,
        });
    }
    let expected = workspace_capability_hmac(
        &secret,
        &server_instance_id,
        user_id,
        &namespace,
        client_workspace_id,
        generation,
    )?;
    if !constant_time_bytes_eq(expected.as_bytes(), proof.as_bytes()) {
        return Err(StoreError::Integrity(
            "legal report workspace proof is invalid".into(),
        ));
    }
    Ok(namespace)
}

fn validate_metadata(item: &LegalReportManifestItem) -> StoreResult<()> {
    if !valid_report_id(&item.report_id)
        || !valid_source_workspace(&item.source_workspace_id)
        || !valid_sha256_hex(&item.sha256)
        || item.sha256.bytes().any(|byte| byte.is_ascii_uppercase())
        || item.created_at_epoch_millis <= 0
        || item.size_bytes <= 0
        || item.size_bytes as u64 > MAX_LEGAL_REPORT_BYTES as u64
    {
        return Err(StoreError::Integrity(
            "legal report metadata is invalid".into(),
        ));
    }
    Ok(())
}

impl SqliteServerStore {
    pub fn legal_report_manifest(
        &self,
        user_id: &str,
        token_id: i64,
        generation: i64,
        client_workspace_id: &str,
        proof: &str,
    ) -> StoreResult<(Vec<LegalReportManifestItem>, Vec<LegalReportTombstone>)> {
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let workspace = legal_scope(
            &transaction,
            user_id,
            token_id,
            generation,
            client_workspace_id,
            proof,
        )?;
        let mut reports_stmt = transaction.prepare(
            "SELECT report_id, created_at_epoch_millis, sha256, size_bytes, source_workspace_id
             FROM legal_reports WHERE user_id = ?1 AND workspace_id = ?2
             ORDER BY created_at_epoch_millis DESC, report_id DESC",
        )?;
        let reports = reports_stmt
            .query_map(params![user_id, workspace], |row| {
                Ok(LegalReportManifestItem {
                    report_id: row.get(0)?,
                    created_at_epoch_millis: row.get(1)?,
                    sha256: row.get(2)?,
                    size_bytes: row.get(3)?,
                    source_workspace_id: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(reports_stmt);
        let mut tombstones_stmt = transaction.prepare(
            "SELECT report_id, deleted_at_epoch_millis FROM legal_report_tombstones
             WHERE user_id = ?1 AND workspace_id = ?2 ORDER BY deleted_at_epoch_millis, report_id",
        )?;
        let tombstones = tombstones_stmt
            .query_map(params![user_id, workspace], |row| {
                Ok(LegalReportTombstone {
                    report_id: row.get(0)?,
                    deleted_at_epoch_millis: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((reports, tombstones))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn legal_report_upload_chunk(
        &self,
        user_id: &str,
        token_id: i64,
        generation: i64,
        client_workspace_id: &str,
        proof: &str,
        item: &LegalReportManifestItem,
        offset: usize,
        chunk: &[u8],
        now: i64,
    ) -> StoreResult<LegalReportUploadOutcome> {
        validate_metadata(item)?;
        if chunk.is_empty()
            || chunk.len() > MAX_LEGAL_REPORT_CHUNK_BYTES
            || offset
                .checked_add(chunk.len())
                .is_none_or(|end| end > item.size_bytes as usize)
        {
            return Err(StoreError::Integrity(
                "legal report chunk bounds are invalid".into(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let workspace = legal_scope(
            &transaction,
            user_id,
            token_id,
            generation,
            client_workspace_id,
            proof,
        )?;
        transaction.execute(
            "DELETE FROM legal_report_uploads WHERE updated_at_epoch_millis < ?1",
            params![now.saturating_sub(STAGED_UPLOAD_RETENTION_MILLIS)],
        )?;
        if is_tombstoned(&transaction, user_id, &workspace, &item.report_id)? {
            return Ok(LegalReportUploadOutcome::Tombstoned);
        }
        if let Some(existing) = report_metadata(&transaction, user_id, &workspace, &item.report_id)?
        {
            if same_metadata(&existing, item) {
                return Ok(LegalReportUploadOutcome::AlreadyCommitted);
            }
            return Err(StoreError::Integrity(
                "legal report ID is already bound to different content".into(),
            ));
        }
        let staged = transaction
            .query_row(
                "SELECT created_at_epoch_millis, source_workspace_id, sha256, total_bytes, content
             FROM legal_report_uploads WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
                params![user_id, workspace, item.report_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                    ))
                },
            )
            .optional()?;
        if staged.is_none() {
            let pending: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM legal_report_uploads WHERE user_id=?1 AND workspace_id=?2",
                params![user_id, workspace],
                |row| row.get(0),
            )?;
            if pending >= MAX_PENDING_UPLOADS_PER_WORKSPACE {
                return Err(StoreError::Integrity(
                    "legal report pending upload limit reached".into(),
                ));
            }
        }
        let mut content = if let Some((created, source, hash, total, content)) = staged {
            if created != item.created_at_epoch_millis
                || source != item.source_workspace_id
                || hash != item.sha256
                || total != item.size_bytes
            {
                return Err(StoreError::Integrity(
                    "legal report upload metadata changed".into(),
                ));
            }
            content
        } else {
            if offset != 0 {
                return Err(StoreError::Integrity(
                    "legal report upload must begin at offset zero".into(),
                ));
            }
            Vec::new()
        };
        let previous_size = content.len() as i64;
        if offset < content.len() {
            let end = offset + chunk.len();
            if end > content.len() || content[offset..end] != *chunk {
                return Err(StoreError::Integrity(
                    "legal report chunk replay differs".into(),
                ));
            }
        } else if offset == content.len() {
            content.extend_from_slice(chunk);
        } else {
            return Err(StoreError::Integrity(
                "legal report chunk offset skips bytes".into(),
            ));
        }
        let retained: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(size_bytes),0) FROM legal_reports",
            [],
            |row| row.get(0),
        )?;
        let pending: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(LENGTH(content)),0) FROM legal_report_uploads",
            [],
            |row| row.get(0),
        )?;
        if retained
            .saturating_add(pending)
            .saturating_sub(previous_size)
            .saturating_add(content.len() as i64)
            > MAX_LEGAL_REPORT_STORAGE_BYTES_GLOBAL
        {
            return Err(StoreError::Integrity(
                "legal report server capacity exceeded".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO legal_report_uploads(user_id,workspace_id,report_id,created_at_epoch_millis,
                 source_workspace_id,sha256,total_bytes,content,updated_at_epoch_millis)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(user_id,workspace_id,report_id) DO UPDATE SET
                 content=excluded.content, updated_at_epoch_millis=excluded.updated_at_epoch_millis",
            params![user_id, workspace, item.report_id, item.created_at_epoch_millis,
                    item.source_workspace_id, item.sha256, item.size_bytes, content, now.max(1)],
        )?;
        privacy_journal::commit(transaction)?;
        Ok(LegalReportUploadOutcome::ChunkAccepted {
            received_bytes: content.len(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn legal_report_commit_upload(
        &self,
        user_id: &str,
        token_id: i64,
        generation: i64,
        client_workspace_id: &str,
        proof: &str,
        item: &LegalReportManifestItem,
        now: i64,
    ) -> StoreResult<LegalReportUploadOutcome> {
        validate_metadata(item)?;
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let workspace = legal_scope(
            &transaction,
            user_id,
            token_id,
            generation,
            client_workspace_id,
            proof,
        )?;
        if is_tombstoned(&transaction, user_id, &workspace, &item.report_id)? {
            return Ok(LegalReportUploadOutcome::Tombstoned);
        }
        if let Some(existing) = report_metadata(&transaction, user_id, &workspace, &item.report_id)?
        {
            if same_metadata(&existing, item) {
                return Ok(LegalReportUploadOutcome::AlreadyCommitted);
            }
            return Err(StoreError::Integrity(
                "legal report ID is already bound to different content".into(),
            ));
        }
        let content: Vec<u8> = transaction
            .query_row(
                "SELECT content FROM legal_report_uploads WHERE user_id=?1 AND workspace_id=?2
             AND report_id=?3 AND created_at_epoch_millis=?4 AND source_workspace_id=?5
             AND sha256=?6 AND total_bytes=?7",
                params![
                    user_id,
                    workspace,
                    item.report_id,
                    item.created_at_epoch_millis,
                    item.source_workspace_id,
                    item.sha256,
                    item.size_bytes
                ],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound("legal report upload is incomplete".into()))?;
        if content.len() != item.size_bytes as usize || sha256_hex(&content) != item.sha256 {
            return Err(StoreError::Integrity(
                "legal report upload hash or length mismatch".into(),
            ));
        }
        let report: LegalReport = serde_json::from_slice(&content)?;
        strict_report_shape(&content)?;
        crate::legal_sources::validate_report_sources(&report).map_err(|message| {
            StoreError::Integrity(format!(
                "legal report evidence or law is invalid: {message}"
            ))
        })?;
        if report.workspace_id != item.source_workspace_id
            || report.manifest.workspace_id != item.source_workspace_id
            || report.captured_at_epoch_millis <= 0
            || report.captured_at_epoch_millis != report.manifest.captured_at_epoch_millis
            || report.captured_at_epoch_millis > item.created_at_epoch_millis
        {
            return Err(StoreError::Integrity(
                "legal report origin or scan manifest does not match".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO legal_reports(user_id,workspace_id,report_id,created_at_epoch_millis,
                 source_workspace_id,sha256,size_bytes,report_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                user_id,
                workspace,
                item.report_id,
                item.created_at_epoch_millis,
                item.source_workspace_id,
                item.sha256,
                item.size_bytes,
                content
            ],
        )?;
        transaction.execute("DELETE FROM legal_report_uploads WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
            params![user_id, workspace, item.report_id])?;
        let mut prune_stmt = transaction.prepare(
            "SELECT report_id FROM legal_reports WHERE user_id=?1 AND workspace_id=?2
             ORDER BY created_at_epoch_millis DESC, report_id DESC LIMIT -1 OFFSET ?3",
        )?;
        let pruned = prune_stmt
            .query_map(
                params![user_id, workspace, MAX_REPORTS_PER_WORKSPACE],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(prune_stmt);
        for report_id in pruned {
            delete_report_in_transaction(
                &transaction,
                user_id,
                &workspace,
                &report_id,
                now.max(1),
            )?;
        }
        privacy_journal::commit(transaction)?;
        Ok(LegalReportUploadOutcome::Committed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn legal_report_download_chunk(
        &self,
        user_id: &str,
        token_id: i64,
        generation: i64,
        client_workspace_id: &str,
        proof: &str,
        report_id: &str,
        offset: usize,
        max_bytes: usize,
    ) -> StoreResult<Option<LegalReportDownloadChunk>> {
        if !valid_report_id(report_id) || max_bytes == 0 || max_bytes > MAX_LEGAL_REPORT_CHUNK_BYTES
        {
            return Err(StoreError::Integrity(
                "legal report download bounds are invalid".into(),
            ));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let workspace = legal_scope(
            &transaction,
            user_id,
            token_id,
            generation,
            client_workspace_id,
            proof,
        )?;
        let found = transaction
            .query_row(
                "SELECT report_json,sha256,size_bytes,created_at_epoch_millis,source_workspace_id
             FROM legal_reports WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
                params![user_id, workspace, report_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((content, hash, total, created, source)) = found else {
            return Ok(None);
        };
        if content.len() as i64 != total || sha256_hex(&content) != hash {
            return Err(StoreError::Integrity(
                "stored legal report content is corrupt".into(),
            ));
        }
        if offset > content.len() {
            return Err(StoreError::Integrity(
                "legal report offset exceeds size".into(),
            ));
        }
        let end = offset.saturating_add(max_bytes).min(content.len());
        Ok(Some(LegalReportDownloadChunk {
            content: content[offset..end].to_vec(),
            total_bytes: content.len(),
            sha256: hash,
            created_at_epoch_millis: created,
            source_workspace_id: source,
        }))
    }

    pub fn legal_report_delete(
        &self,
        user_id: &str,
        token_id: i64,
        generation: i64,
        client_workspace_id: &str,
        proof: &str,
        report_id: &str,
        now: i64,
    ) -> StoreResult<LegalReportTombstone> {
        if !valid_report_id(report_id) {
            return Err(StoreError::Integrity("legal report ID is invalid".into()));
        }
        let mut connection = self.open_connection(false)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let workspace = legal_scope(
            &transaction,
            user_id,
            token_id,
            generation,
            client_workspace_id,
            proof,
        )?;
        delete_report_in_transaction(&transaction, user_id, &workspace, report_id, now.max(1))?;
        let deleted_at_epoch_millis: i64 = transaction.query_row(
            "SELECT deleted_at_epoch_millis FROM legal_report_tombstones
             WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
            params![user_id, workspace, report_id],
            |row| row.get(0),
        )?;
        privacy_journal::commit(transaction)?;
        Ok(LegalReportTombstone {
            report_id: report_id.to_string(),
            deleted_at_epoch_millis,
        })
    }
}

fn same_metadata(left: &LegalReportManifestItem, right: &LegalReportManifestItem) -> bool {
    left.report_id == right.report_id
        && left.created_at_epoch_millis == right.created_at_epoch_millis
        && left.sha256 == right.sha256
        && left.size_bytes == right.size_bytes
        && left.source_workspace_id == right.source_workspace_id
}

fn strict_report_shape(content: &[u8]) -> StoreResult<()> {
    let root: Value = serde_json::from_slice(content)?;
    let allowed_root = [
        "workspaceId",
        "capturedAtEpochMillis",
        "completed",
        "findings",
        "manifest",
        "errors",
    ];
    let allowed_manifest = [
        "workspaceId",
        "capturedAtEpochMillis",
        "coverage",
        "omissions",
        "evidenceCount",
        "uploadBytes",
        "estimatedCalls",
    ];
    let allowed_finding = [
        "title",
        "area",
        "fact",
        "evidence",
        "eventAtEpochMillis",
        "missingFacts",
        "recommendation",
        "laws",
    ];
    let allowed_evidence = ["evidenceId", "sourcePath", "title", "quote"];
    let allowed_law = [
        "id",
        "title",
        "version",
        "url",
        "checkedOn",
        "articleNumber",
        "articleText",
    ];
    let object = root
        .as_object()
        .ok_or_else(|| StoreError::Integrity("legal report must be an object".into()))?;
    if object
        .keys()
        .any(|key| !allowed_root.contains(&key.as_str()))
    {
        return Err(StoreError::Integrity(
            "legal report contains non-report data".into(),
        ));
    }
    let manifest = object
        .get("manifest")
        .and_then(Value::as_object)
        .ok_or_else(|| StoreError::Integrity("legal report manifest is missing".into()))?;
    if manifest
        .keys()
        .any(|key| !allowed_manifest.contains(&key.as_str()))
    {
        return Err(StoreError::Integrity(
            "legal report manifest contains non-report data".into(),
        ));
    }
    let coverage = manifest
        .get("coverage")
        .and_then(Value::as_object)
        .ok_or_else(|| StoreError::Integrity("legal report coverage is invalid".into()))?;
    for value in coverage.values() {
        let Some(value) = value.as_object() else {
            return Err(StoreError::Integrity(
                "legal report coverage item is invalid".into(),
            ));
        };
        if value.keys().any(|key| {
            !["candidates", "included", "unavailable", "excludedDeleted"].contains(&key.as_str())
        }) {
            return Err(StoreError::Integrity(
                "legal report coverage contains non-report data".into(),
            ));
        }
    }
    let omissions = manifest
        .get("omissions")
        .and_then(Value::as_array)
        .ok_or_else(|| StoreError::Integrity("legal report omissions are invalid".into()))?;
    for omission in omissions {
        let Some(omission) = omission.as_object() else {
            return Err(StoreError::Integrity(
                "legal report omission is invalid".into(),
            ));
        };
        if omission
            .keys()
            .any(|key| !["sourcePath", "reason"].contains(&key.as_str()))
        {
            return Err(StoreError::Integrity(
                "legal report omission contains non-report data".into(),
            ));
        }
    }
    if let Some(findings) = object.get("findings").and_then(Value::as_array) {
        for finding in findings {
            let Some(finding) = finding.as_object() else {
                return Err(StoreError::Integrity(
                    "legal report finding is invalid".into(),
                ));
            };
            if finding
                .keys()
                .any(|key| !allowed_finding.contains(&key.as_str()))
            {
                return Err(StoreError::Integrity(
                    "legal report finding contains non-report data".into(),
                ));
            }
            for (key, allowed) in [
                ("evidence", &allowed_evidence[..]),
                ("laws", &allowed_law[..]),
            ] {
                let Some(items) = finding.get(key).and_then(Value::as_array) else {
                    return Err(StoreError::Integrity(
                        "legal report finding details are invalid".into(),
                    ));
                };
                for item in items {
                    let Some(object) = item.as_object() else {
                        return Err(StoreError::Integrity(
                            "legal report finding detail is invalid".into(),
                        ));
                    };
                    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
                        return Err(StoreError::Integrity(
                            "legal report finding contains non-report data".into(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn report_metadata(
    transaction: &Transaction<'_>,
    user_id: &str,
    workspace: &str,
    report_id: &str,
) -> StoreResult<Option<LegalReportManifestItem>> {
    transaction
        .query_row(
            "SELECT report_id,created_at_epoch_millis,sha256,size_bytes,source_workspace_id
         FROM legal_reports WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
            params![user_id, workspace, report_id],
            |row| {
                Ok(LegalReportManifestItem {
                    report_id: row.get(0)?,
                    created_at_epoch_millis: row.get(1)?,
                    sha256: row.get(2)?,
                    size_bytes: row.get(3)?,
                    source_workspace_id: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
}

fn is_tombstoned(
    transaction: &Transaction<'_>,
    user_id: &str,
    workspace: &str,
    report_id: &str,
) -> StoreResult<bool> {
    Ok(transaction.query_row(
        "SELECT 1 FROM legal_report_tombstones WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
        params![user_id, workspace, report_id], |_| Ok(()),
    ).optional()?.is_some())
}

fn delete_report_in_transaction(
    transaction: &Transaction<'_>,
    user_id: &str,
    workspace: &str,
    report_id: &str,
    now: i64,
) -> StoreResult<()> {
    transaction.execute(
        "DELETE FROM legal_reports WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
        params![user_id, workspace, report_id],
    )?;
    transaction.execute(
        "DELETE FROM legal_report_uploads WHERE user_id=?1 AND workspace_id=?2 AND report_id=?3",
        params![user_id, workspace, report_id],
    )?;
    transaction.execute(
        "INSERT INTO legal_report_tombstones(user_id,workspace_id,report_id,deleted_at_epoch_millis)
         VALUES (?1,?2,?3,?4) ON CONFLICT(user_id,workspace_id,report_id) DO NOTHING",
        params![user_id, workspace, report_id, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestDb(PathBuf);
    impl TestDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gridtimer_legal_report_{name}_{}_{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> (
        TestDb,
        SqliteServerStore,
        i64,
        String,
        String,
        String,
        String,
    ) {
        let directory = TestDb::new("exchange");
        let store = SqliteServerStore::open(directory.0.join("store.sqlite3"), None).unwrap();
        store
            .create_user(NewStoredUser {
                id: "legal-user".into(),
                email: "legal@example.test".into(),
                password_salt: "salt".into(),
                password_hash: "hash".into(),
                password_scheme: "legacy_sha256".into(),
                created_at_epoch_millis: 10,
                updated_at_epoch_millis: 10,
                app_data_json: "{}".into(),
                account_revision: 0,
            })
            .unwrap();
        let token = store
            .issue_token("legal-user", "legal-token", "phone", 20, 10000)
            .unwrap();
        let receipt = store
            .ensure_restore_receipt("legal-user", token.id)
            .unwrap();
        store
            .acknowledge_restore_generation("legal-user", token.id, 0, &receipt.receipt)
            .unwrap();
        let device_a = "a".repeat(64);
        let device_b = "b".repeat(64);
        let proof_a = store
            .workspace_capability_proof("legal-user", &device_a, 0)
            .unwrap();
        let proof_b = store
            .workspace_capability_proof("legal-user", &device_b, 0)
            .unwrap();
        (
            directory, store, token.id, device_a, proof_a, device_b, proof_b,
        )
    }

    fn report(report_id: &str, created: i64) -> (LegalReportManifestItem, Vec<u8>) {
        let content = json!({
            "workspaceId": "source-local", "capturedAtEpochMillis": 50,
            "completed": true, "findings": [], "errors": [],
            "manifest": {"workspaceId":"source-local", "capturedAtEpochMillis":50,
                "coverage":{},"omissions":[],"evidenceCount":0,"uploadBytes":0,
                "estimatedCalls":0}
        })
        .to_string()
        .into_bytes();
        (
            LegalReportManifestItem {
                report_id: report_id.into(),
                created_at_epoch_millis: created,
                sha256: sha256_hex(&content),
                size_bytes: content.len() as i64,
                source_workspace_id: "source-local".into(),
            },
            content,
        )
    }

    #[test]
    fn cross_device_exchange_and_tombstone_blocks_replay() {
        let (_directory, store, token, device_a, proof_a, device_b, proof_b) = fixture();
        let (item, content) = report("r-1", 100);
        let split = content.len() / 2;
        assert!(matches!(
            store
                .legal_report_upload_chunk(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &item,
                    0,
                    &content[..split],
                    101
                )
                .unwrap(),
            LegalReportUploadOutcome::ChunkAccepted { .. }
        ));
        // A byte-for-byte retry does not append the same chunk twice.
        store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &proof_a,
                &item,
                0,
                &content[..split],
                102,
            )
            .unwrap();
        store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &proof_a,
                &item,
                split,
                &content[split..],
                103,
            )
            .unwrap();
        assert!(matches!(
            store
                .legal_report_commit_upload("legal-user", token, 0, &device_a, &proof_a, &item, 104)
                .unwrap(),
            LegalReportUploadOutcome::Committed
        ));
        let (seen, _) = store
            .legal_report_manifest("legal-user", token, 0, &device_b, &proof_b)
            .unwrap();
        assert_eq!(seen.len(), 1);
        let downloaded = store
            .legal_report_download_chunk(
                "legal-user",
                token,
                0,
                &device_b,
                &proof_b,
                "r-1",
                0,
                MAX_LEGAL_REPORT_CHUNK_BYTES,
            )
            .unwrap()
            .unwrap();
        assert_eq!(downloaded.content, content);
        assert_eq!(downloaded.source_workspace_id, "source-local");
        store
            .legal_report_delete("legal-user", token, 0, &device_b, &proof_b, "r-1", 105)
            .unwrap();
        assert!(matches!(
            store
                .legal_report_upload_chunk(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &item,
                    0,
                    &content,
                    106
                )
                .unwrap(),
            LegalReportUploadOutcome::Tombstoned
        ));
        let (seen, tombstones) = store
            .legal_report_manifest("legal-user", token, 0, &device_a, &proof_a)
            .unwrap();
        assert!(seen.is_empty());
        assert_eq!(tombstones[0].report_id, "r-1");
    }

    #[test]
    fn bad_proof_or_content_never_commits() {
        let (_directory, store, token, device_a, proof_a, _, _) = fixture();
        let (item, content) = report("r-2", 100);
        assert!(store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &"f".repeat(64),
                &item,
                0,
                &content,
                101
            )
            .is_err());
        assert!(store
            .legal_report_manifest("legal-user", token, 0, &device_a, &"f".repeat(64))
            .is_err());
        let mut tampered = content.clone();
        tampered[0] ^= 1;
        store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &proof_a,
                &item,
                0,
                &tampered,
                101,
            )
            .unwrap();
        assert!(store
            .legal_report_commit_upload("legal-user", token, 0, &device_a, &proof_a, &item, 102)
            .is_err());
        assert!(store
            .legal_report_manifest("legal-user", token, 0, &device_a, &proof_a)
            .unwrap()
            .0
            .is_empty());
        let (mut extra, content) = report("r-3", 100);
        let mut raw: Value = serde_json::from_slice(&content).unwrap();
        raw["batches"] = json!([{"raw":"forbidden scan payload"}]);
        let raw = raw.to_string().into_bytes();
        extra.sha256 = sha256_hex(&raw);
        extra.size_bytes = raw.len() as i64;
        store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &proof_a,
                &extra,
                0,
                &raw,
                101,
            )
            .unwrap();
        assert!(store
            .legal_report_commit_upload("legal-user", token, 0, &device_a, &proof_a, &extra, 102)
            .is_err());
    }

    #[test]
    fn invalid_evidence_or_unverified_law_cannot_enter_shared_report() {
        let (_directory, store, token, device_a, proof_a, _, _) = fixture();
        let law = crate::legal_sources::verified_laws()
            .into_iter()
            .next()
            .unwrap();
        for (report_id, evidence_id, law_id) in [
            ("bad-evidence", "E999999", law.id.as_str()),
            ("bad-law", "E000001", "invented_law"),
        ] {
            let (mut item, content) = report(report_id, 100);
            let mut raw: Value = serde_json::from_slice(&content).unwrap();
            raw["manifest"]["evidenceCount"] = json!(1);
            let mut law_json = serde_json::to_value(&law).unwrap();
            law_json["id"] = json!(law_id);
            raw["findings"] = json!([{
                "title":"待核查债务","area":"合同债务","fact":"欠款",
                "evidence":[{"evidenceId":evidence_id,"sourcePath":"task:t1",
                    "title":"任务","quote":"欠款"}],
                "eventAtEpochMillis":null,"missingFacts":"合同文本",
                "recommendation":"核对合同","laws":[law_json]
            }]);
            let raw = raw.to_string().into_bytes();
            item.sha256 = sha256_hex(&raw);
            item.size_bytes = raw.len() as i64;
            store
                .legal_report_upload_chunk(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &item,
                    0,
                    &raw,
                    101,
                )
                .unwrap();
            assert!(store
                .legal_report_commit_upload("legal-user", token, 0, &device_a, &proof_a, &item, 102)
                .is_err());
        }
        assert!(store
            .legal_report_manifest("legal-user", token, 0, &device_a, &proof_a)
            .unwrap()
            .0
            .is_empty());
    }

    #[test]
    fn schema_sixteen_migration_keeps_verified_recovery_copy() {
        let directory = TestDb::new("migration");
        let path = directory.0.join("store.sqlite3");
        let store = SqliteServerStore::open(&path, None).unwrap();
        let connection = store.open_connection(false).unwrap();
        connection
            .execute_batch(
                "DROP TABLE legal_report_tombstones;
             DROP TABLE legal_report_uploads;
             DROP TABLE legal_reports;
             DELETE FROM schema_migrations WHERE version=17;
             PRAGMA user_version=16;",
            )
            .unwrap();
        drop(connection);
        drop(store);
        let mut options = ServerStoreOpenOptions::new(&path);
        options.now_epoch_millis = 123456;
        let opened = SqliteServerStore::open_with_options(options).unwrap();
        assert!(opened.schema_migrated);
        let backup = opened.pre_schema_migration_backup.unwrap();
        SqliteServerStore::verify_existing_backup(&backup.destination, 123456).unwrap();
        let current = opened.store.open_connection(false).unwrap();
        assert_eq!(current_schema_version(&current).unwrap(), 17);
        verify_schema(&current).unwrap();
    }

    #[test]
    fn sixth_report_prunes_oldest_and_fences_offline_replay() {
        let (_directory, store, token, device_a, proof_a, _, _) = fixture();
        for index in 0..6 {
            let (item, content) = report(&format!("report-{index}"), 100 + index);
            store
                .legal_report_upload_chunk(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &item,
                    0,
                    &content,
                    200 + index,
                )
                .unwrap();
            store
                .legal_report_commit_upload(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &item,
                    300 + index,
                )
                .unwrap();
        }
        let (reports, tombstones) = store
            .legal_report_manifest("legal-user", token, 0, &device_a, &proof_a)
            .unwrap();
        assert_eq!(reports.len(), 5);
        assert!(reports.iter().all(|item| item.report_id != "report-0"));
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].report_id, "report-0");
        let (old, content) = report("report-0", 100);
        assert!(matches!(
            store
                .legal_report_upload_chunk(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &old,
                    0,
                    &content,
                    500
                )
                .unwrap(),
            LegalReportUploadOutcome::Tombstoned
        ));
    }

    #[test]
    fn abandoned_uploads_expire_so_another_device_can_upload() {
        let (_directory, store, token, device_a, proof_a, _, _) = fixture();
        for index in 0..MAX_PENDING_UPLOADS_PER_WORKSPACE {
            let (item, content) = report(&format!("abandoned-{index}"), 100);
            store
                .legal_report_upload_chunk(
                    "legal-user",
                    token,
                    0,
                    &device_a,
                    &proof_a,
                    &item,
                    0,
                    &content[..1],
                    100,
                )
                .unwrap();
        }
        let (fresh, content) = report("fresh-report", 101);
        assert!(store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &proof_a,
                &fresh,
                0,
                &content[..1],
                101,
            )
            .is_err());
        store
            .legal_report_upload_chunk(
                "legal-user",
                token,
                0,
                &device_a,
                &proof_a,
                &fresh,
                0,
                &content[..1],
                100 + STAGED_UPLOAD_RETENTION_MILLIS + 1,
            )
            .unwrap();
        let connection = store.open_connection(false).unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM legal_report_uploads", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    #[ignore = "requires GRIDTIMER_LEGAL_VERIFY_COPY pointing to an isolated online backup"]
    fn live_v16_copy_migrates_with_verified_backup_and_recovers_after_failure() {
        let source = PathBuf::from(
            std::env::var_os("GRIDTIMER_LEGAL_VERIFY_COPY")
                .expect("set GRIDTIMER_LEGAL_VERIFY_COPY to the isolated live database copy"),
        );
        let source_connection = Connection::open_with_flags(
            &source,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .unwrap();
        assert_eq!(current_schema_version(&source_connection).unwrap(), 16);
        verify_required_schema_at_version(&source_connection, 16).unwrap();
        let source_accounts = account_signatures(&source_connection);
        let source_media: i64 = source_connection
            .query_row("SELECT COUNT(*) FROM note_media", [], |row| row.get(0))
            .unwrap();
        drop(source_connection);
        let directory = TestDb::new("live_copy_migration");
        let migrate_path = directory.0.join("migrate.sqlite3");
        fs::copy(&source, &migrate_path).unwrap();
        let opened =
            SqliteServerStore::open_with_options(ServerStoreOpenOptions::new(&migrate_path))
                .unwrap();
        assert!(opened.schema_migrated);
        let pre_backup = opened
            .pre_schema_migration_backup
            .expect("migration must preserve v16 backup");
        assert_eq!(
            pre_backup.sha256,
            sha256_file(&pre_backup.destination).unwrap()
        );
        SqliteServerStore::verify_existing_backup(
            &pre_backup.destination,
            pre_backup.created_at_epoch_millis,
        )
        .unwrap();
        let migrated = opened.store.open_connection(false).unwrap();
        assert_eq!(current_schema_version(&migrated).unwrap(), 17);
        assert_eq!(account_signatures(&migrated), source_accounts);
        assert_eq!(
            migrated
                .query_row("SELECT COUNT(*) FROM note_media", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            source_media
        );
        drop(migrated);

        let fail_path = directory.0.join("fail.sqlite3");
        fs::copy(&pre_backup.destination, &fail_path).unwrap();
        let fail_connection = Connection::open(&fail_path).unwrap();
        fail_connection
            .execute_batch("CREATE TABLE legal_reports (injected_conflict TEXT)")
            .unwrap();
        drop(fail_connection);
        assert!(
            SqliteServerStore::open_with_options(ServerStoreOpenOptions::new(&fail_path)).is_err()
        );
        let failed =
            Connection::open_with_flags(&fail_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(current_schema_version(&failed).unwrap(), 16);
        drop(failed);

        let recovered_path = directory.0.join("recovered.sqlite3");
        fs::copy(&pre_backup.destination, &recovered_path).unwrap();
        let recovered =
            SqliteServerStore::open_with_options(ServerStoreOpenOptions::new(&recovered_path))
                .unwrap();
        let recovered_connection = recovered.store.open_connection(false).unwrap();
        assert_eq!(current_schema_version(&recovered_connection).unwrap(), 17);
        assert_eq!(account_signatures(&recovered_connection), source_accounts);
    }

    fn account_signatures(connection: &Connection) -> Vec<(String, String, i64)> {
        let mut statement = connection
            .prepare(
                "SELECT user_id, content_sha256, revision FROM account_snapshots ORDER BY user_id",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }
}
