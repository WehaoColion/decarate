//! HTTP handlers for optional account-scoped legal report exchange.
use super::*;
use crate::server_store::{
    LegalReportManifestItem, LegalReportUploadOutcome, MAX_LEGAL_REPORT_CHUNK_BYTES,
};
use base64::Engine as _;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegalRequest {
    #[serde(default)]
    acknowledged_generation: i64,
    #[serde(default)]
    workspace_id: String,
    #[serde(default)]
    workspace_proof: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    report_id: String,
    #[serde(default)]
    created_at_epoch_millis: i64,
    #[serde(default)]
    source_workspace_id: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    total_bytes: i64,
    #[serde(default)]
    offset_bytes: usize,
    #[serde(default)]
    max_bytes: usize,
    #[serde(default)]
    chunk_base64: String,
}

pub(super) fn handle(
    route: &str,
    request: &HttpRequest,
    store: &SqliteServerStore,
) -> (u16, SyncClientResult) {
    let Some(raw_token) = bearer_token(request) else {
        return (401, error_result("Missing sync token."));
    };
    let authenticated = match authenticate_token_request(store, raw_token, now_millis()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let body: LegalRequest = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid legal report request.")),
    };
    if body.workspace_id.is_empty() || body.workspace_proof.is_empty() {
        return (
            403,
            error_result("Legal report workspace proof is required."),
        );
    }
    let user_id = authenticated.user_id;
    let token_id = authenticated.token_id;
    let token_identifier = authenticated.token_identifier;
    let generation = body.acknowledged_generation;
    let workspace_id = body.workspace_id.as_str();
    let proof = body.workspace_proof.as_str();
    let outcome = match route {
        "/v1/legal-reports/manifest" => store
            .legal_report_manifest(&user_id, token_id, generation, workspace_id, proof)
            .map(|(reports, tombstones)| SyncClientResult {
                ok: true,
                message: "Legal report manifest loaded.".into(),
                user_id: user_id.clone(),
                current_generation: generation,
                mode: "legal_manifest".into(),
                legal_reports: reports,
                legal_tombstones: tombstones,
                ..SyncClientResult::default()
            }),
        "/v1/legal-reports/upload" => {
            let metadata = LegalReportManifestItem {
                report_id: body.report_id.clone(),
                created_at_epoch_millis: body.created_at_epoch_millis,
                sha256: body.sha256.clone(),
                size_bytes: body.total_bytes,
                source_workspace_id: body.source_workspace_id.clone(),
            };
            let upload = match body.phase.as_str() {
                "chunk" => {
                    if body.chunk_base64.len() > (MAX_LEGAL_REPORT_CHUNK_BYTES * 4 / 3 + 8) {
                        return (413, error_result("Legal report chunk is too large."));
                    }
                    let chunk = match BASE64_STANDARD.decode(body.chunk_base64.as_bytes()) {
                        Ok(value) => value,
                        Err(_) => {
                            return (400, error_result("Legal report chunkBase64 is invalid."))
                        }
                    };
                    store.legal_report_upload_chunk(
                        &user_id,
                        token_id,
                        generation,
                        workspace_id,
                        proof,
                        &metadata,
                        body.offset_bytes,
                        &chunk,
                        now_millis(),
                    )
                }
                "commit" => store.legal_report_commit_upload(
                    &user_id,
                    token_id,
                    generation,
                    workspace_id,
                    proof,
                    &metadata,
                    now_millis(),
                ),
                _ => return (400, error_result("Legal report upload phase is invalid.")),
            };
            upload.map(|value| match value {
                LegalReportUploadOutcome::ChunkAccepted { received_bytes } => SyncClientResult {
                    ok: true,
                    message: "Legal report chunk accepted.".into(),
                    user_id: user_id.clone(),
                    current_generation: generation,
                    mode: "legal_upload_chunk".into(),
                    legal_report_received_bytes: received_bytes,
                    ..SyncClientResult::default()
                },
                LegalReportUploadOutcome::Committed
                | LegalReportUploadOutcome::AlreadyCommitted => SyncClientResult {
                    ok: true,
                    message: "Legal report committed.".into(),
                    user_id: user_id.clone(),
                    current_generation: generation,
                    mode: "legal_upload_committed".into(),
                    ..SyncClientResult::default()
                },
                LegalReportUploadOutcome::Tombstoned => SyncClientResult {
                    ok: false,
                    message: "Legal report was already deleted.".into(),
                    user_id: user_id.clone(),
                    current_generation: generation,
                    mode: "legal_tombstoned".into(),
                    ..SyncClientResult::default()
                },
            })
        }
        "/v1/legal-reports/download" => store
            .legal_report_download_chunk(
                &user_id,
                token_id,
                generation,
                workspace_id,
                proof,
                &body.report_id,
                body.offset_bytes,
                body.max_bytes,
            )
            .and_then(|maybe| maybe.ok_or_else(|| StoreError::NotFound("legal report".into())))
            .map(|chunk| SyncClientResult {
                ok: true,
                message: "Legal report chunk downloaded.".into(),
                user_id: user_id.clone(),
                current_generation: generation,
                mode: "legal_download".into(),
                legal_report_chunk_base64: BASE64_STANDARD.encode(chunk.content),
                legal_report_total_bytes: chunk.total_bytes,
                legal_report_sha256: chunk.sha256,
                legal_report_created_at_epoch_millis: chunk.created_at_epoch_millis,
                legal_report_source_workspace_id: chunk.source_workspace_id,
                ..SyncClientResult::default()
            }),
        "/v1/legal-reports/delete" => store
            .legal_report_delete(
                &user_id,
                token_id,
                generation,
                workspace_id,
                proof,
                &body.report_id,
                now_millis(),
            )
            .map(|tombstone| SyncClientResult {
                ok: true,
                message: "Legal report deletion recorded.".into(),
                user_id: user_id.clone(),
                current_generation: generation,
                mode: "legal_delete".into(),
                legal_tombstones: vec![tombstone],
                ..SyncClientResult::default()
            }),
        _ => return (404, error_result("Endpoint not found.")),
    };
    match outcome {
        Ok(mut result) => {
            let identity = match store.server_account_identity(&user_id) {
                Ok(value) => value,
                Err(error) => {
                    return store_failure("Could not bind legal report account identity", error)
                }
            };
            result.user_id = user_id.clone();
            result.token_id = token_identifier;
            result.server_instance_id = identity.server_instance_id;
            result.account_namespace = identity.account_namespace;
            if result.mode == "legal_tombstoned" {
                (409, result)
            } else {
                (200, result)
            }
        }
        Err(StoreError::NotFound(_)) => (404, error_result("Legal report was not found.")),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation, ..
        })
        | Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
            media_restore_required_response(store, &user_id, token_id, actual_generation)
        }
        Err(StoreError::ServerGenerationRollback {
            server_generation, ..
        }) => server_generation_rollback_response(&user_id, server_generation),
        Err(StoreError::Integrity(message)) if message.contains("workspace proof") => {
            (403, error_result(&message))
        }
        Err(StoreError::Integrity(message))
            if message.contains("capacity exceeded")
                || message.contains("pending upload limit") =>
        {
            (507, error_result(&message))
        }
        Err(StoreError::Integrity(message)) if message.contains("too large") => {
            (413, error_result(&message))
        }
        Err(StoreError::Integrity(message)) => (400, error_result(&message)),
        Err(error) => store_failure("Could not exchange legal report", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_report_route_binds_authenticated_account_identity() {
        let directory = std::env::temp_dir().join(format!(
            "gridtimer_legal_route_{}_{}",
            std::process::id(),
            now_millis(),
        ));
        fs::create_dir_all(&directory).unwrap();
        let store = SqliteServerStore::open(directory.join("server.sqlite3"), None).unwrap();
        let now = now_millis();
        store
            .create_user(NewStoredUser {
                id: "legal-route-user".into(),
                email: "legal-route@example.test".into(),
                password_salt: "salt".into(),
                password_hash: "hash".into(),
                password_scheme: "legacy_sha256".into(),
                created_at_epoch_millis: now,
                updated_at_epoch_millis: now,
                app_data_json: "{}".into(),
                account_revision: 0,
            })
            .unwrap();
        let token = store
            .issue_token(
                "legal-route-user",
                "route-token",
                "Windows",
                now,
                now + 60 * 60 * 1000,
            )
            .unwrap();
        let receipt = store
            .ensure_restore_receipt("legal-route-user", token.id)
            .unwrap();
        store
            .acknowledge_restore_generation("legal-route-user", token.id, 0, &receipt.receipt)
            .unwrap();
        let workspace = "a".repeat(64);
        let proof = store
            .workspace_capability_proof("legal-route-user", &workspace, 0)
            .unwrap();
        let identity = store.server_account_identity("legal-route-user").unwrap();
        let base = json!({"acknowledgedGeneration":0,"workspaceId":workspace,
                          "workspaceProof":proof});
        let request = |route: &str, body: Value| HttpRequest {
            method: "POST".into(),
            path: route.into(),
            headers: vec![("authorization".into(), "Bearer route-token".into())],
            body: body.to_string(),
        };
        let assert_bound = |result: &SyncClientResult| {
            assert_eq!(result.user_id, "legal-route-user");
            assert_eq!(result.token_id, token.token_id);
            assert_eq!(result.server_instance_id, identity.server_instance_id);
            assert_eq!(result.account_namespace, identity.account_namespace);
            assert_eq!(result.current_generation, 0);
        };
        let route_manifest = "/v1/legal-reports/manifest";
        let (status, manifest) = handle(
            route_manifest,
            &request(route_manifest, base.clone()),
            &store,
        );
        assert_eq!(status, 200);
        assert_bound(&manifest);

        let report = json!({"workspaceId":"source-local","capturedAtEpochMillis":now,
            "completed":true,"findings":[],"errors":[],"manifest":{
                "workspaceId":"source-local","capturedAtEpochMillis":now,"coverage":{},
                "omissions":[],"evidenceCount":0,"uploadBytes":0,"estimatedCalls":0}})
        .to_string();
        let hash = format!("{:x}", Sha256::digest(report.as_bytes()));
        let mut upload = base.clone();
        upload["reportId"] = json!("route-report");
        upload["createdAtEpochMillis"] = json!(now + 1);
        upload["sourceWorkspaceId"] = json!("source-local");
        upload["sha256"] = json!(hash);
        upload["totalBytes"] = json!(report.len());
        upload["phase"] = json!("chunk");
        upload["offsetBytes"] = json!(0);
        upload["chunkBase64"] = json!(BASE64_STANDARD.encode(report.as_bytes()));
        let route_upload = "/v1/legal-reports/upload";
        let (status, chunk) = handle(route_upload, &request(route_upload, upload.clone()), &store);
        assert_eq!(status, 200);
        assert_bound(&chunk);
        upload["phase"] = json!("commit");
        let (status, committed) = handle(route_upload, &request(route_upload, upload), &store);
        assert_eq!(status, 200);
        assert_bound(&committed);

        let mut download = base.clone();
        download["reportId"] = json!("route-report");
        download["offsetBytes"] = json!(0);
        download["maxBytes"] = json!(MAX_LEGAL_REPORT_CHUNK_BYTES);
        let route_download = "/v1/legal-reports/download";
        let (status, downloaded) =
            handle(route_download, &request(route_download, download), &store);
        assert_eq!(status, 200);
        assert_bound(&downloaded);
        assert_eq!(downloaded.legal_report_source_workspace_id, "source-local");

        let mut deletion = base;
        deletion["reportId"] = json!("route-report");
        let route_delete = "/v1/legal-reports/delete";
        let (status, deleted) = handle(route_delete, &request(route_delete, deletion), &store);
        assert_eq!(status, 200);
        assert_bound(&deleted);
        drop(store);
        let _ = fs::remove_dir_all(directory);
    }
}
