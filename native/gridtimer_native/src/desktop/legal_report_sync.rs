// Account-authorized exchange of completed legal reports only. Source scan
// snapshots and AI request batches are never read by this transport.

const DESKTOP_LEGAL_REPORT_CHUNK_BYTES: usize = 4 * 1024 * 1024;

fn sync_desktop_legal_reports(
    store: &DesktopLegalReportStore,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    workspace_proof: &str,
    generation: i64,
) -> Result<(), String> {
    sync_desktop_legal_reports_while(
        store,
        server_url,
        token,
        workspace_id,
        workspace_proof,
        generation,
        || true,
    )
}

/// Callers may invalidate the captured session between chunks. The server
/// still checks the token, workspace proof and generation on every request.
fn sync_desktop_legal_reports_while<F: Fn() -> bool>(
    store: &DesktopLegalReportStore,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    workspace_proof: &str,
    generation: i64,
    active: F,
) -> Result<(), String> {
    if server_url.trim().is_empty()
        || token.trim().is_empty()
        || workspace_id.trim().is_empty()
        || workspace_proof.trim().is_empty()
        || generation < 0
    {
        return Err("尚未取得已验证的账户工作区，报告留在本机待同步".into());
    }
    sync_desktop_legal_reports_with_transport(
        store,
        workspace_id,
        workspace_proof,
        generation,
        active,
        |operation, body| {
            let raw = gridtimer_native::sync_core::legal_reports_request_json(
                server_url,
                token,
                operation,
                &body.to_string(),
            );
            serde_json::from_str(&raw).map_err(|_| "法律报告同步响应无效".into())
        },
    )
}

fn sync_desktop_legal_reports_with_transport<F, T>(
    store: &DesktopLegalReportStore,
    workspace_id: &str,
    workspace_proof: &str,
    generation: i64,
    active: F,
    mut transport: T,
) -> Result<(), String>
where
    F: Fn() -> bool,
    T: FnMut(
        &str,
        &serde_json::Value,
    ) -> Result<gridtimer_native::sync_core::SyncClientResult, String>,
{
    // Keep request storage owned by its caller and make mutation explicit.
    // The optimized Windows build otherwise reused a consumed empty Value
    // across the two manifest calls; the deletion/retry regression covers it.
    let mut request = |operation: &str,
                       body: &mut serde_json::Value|
     -> Result<gridtimer_native::sync_core::SyncClientResult, String> {
        if !active() {
            return Err("账户或工作区已切换，报告同步已停止".into());
        }
        let Some(object) = body.as_object_mut() else {
            return Err("法律报告请求结构无效".into());
        };
        object.insert(
            "acknowledgedGeneration".into(),
            serde_json::json!(generation),
        );
        object.insert("workspaceId".into(), serde_json::json!(workspace_id));
        object.insert("workspaceProof".into(), serde_json::json!(workspace_proof));
        let result = transport(operation, body);
        if !active() {
            return Err("账户或工作区已切换，报告同步已停止".into());
        }
        result
    };
    let accepted = |reply: &gridtimer_native::sync_core::SyncClientResult,
                    expected_mode: &str,
                    user_id: &str|
     -> Result<(), String> {
        if !reply.ok {
            return Err(format!("法律报告同步失败：{}", reply.message));
        }
        if reply.mode != expected_mode
            || reply.current_generation != generation
            || reply.user_id.trim().is_empty()
            || (!user_id.is_empty() && reply.user_id != user_id)
        {
            return Err("法律报告同步响应与当前账户或工作区不一致".into());
        }
        Ok(())
    };
    let tombstoned = |reply: &gridtimer_native::sync_core::SyncClientResult, user_id: &str| {
        reply.mode == "legal_tombstoned"
            && !reply.ok
            && reply.current_generation == generation
            && reply.user_id == user_id
    };

    let mut manifest = request("manifest", &mut serde_json::json!({}))?;
    accepted(&manifest, "legal_manifest", "")?;
    let user_id = manifest.user_id.clone();
    let mut local_snapshot = store.sync_snapshot(
        &manifest
            .legal_tombstones
            .iter()
            .map(|item| item.report_id.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut remote_mutated = false;
    let server_deleted = manifest
        .legal_tombstones
        .iter()
        .map(|item| item.report_id.as_str())
        .collect::<std::collections::HashSet<_>>();

    // A local delete must win over an offline device's late upload. Send all
    // local tombstones before any report body leaves the machine.
    for id in &local_snapshot.tombstones {
        if server_deleted.contains(id.as_str()) {
            continue;
        }
        let reply = request("delete", &mut serde_json::json!({ "reportId": id }))?;
        accepted(&reply, "legal_delete", &user_id)?;
        if !reply
            .legal_tombstones
            .iter()
            .any(|item| &item.report_id == id)
        {
            return Err("服务端未确认法律报告删除".into());
        }
        remote_mutated = true;
    }

    let remote_by_id = manifest
        .legal_reports
        .iter()
        .map(|item| (item.report_id.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>();
    let mut local = local_snapshot.reports.clone();
    local.sort_by(|left, right| {
        left.created_at_epoch_millis
            .cmp(&right.created_at_epoch_millis)
            .then_with(|| left.id.cmp(&right.id))
    });
    for item in local {
        if !active() {
            return Err("账户或工作区已切换，报告同步已停止".into());
        }
        if let Some(remote) = remote_by_id.get(item.id.as_str()) {
            legal_report_metadata_matches(&item, remote)?;
            continue;
        }
        let raw = store.raw(&item.id)?;
        let metadata = serde_json::json!({
            "reportId": item.id.clone(),
            "createdAtEpochMillis": item.created_at_epoch_millis,
            "sourceWorkspaceId": item.source_workspace_id,
            "sha256": item.sha256,
            "totalBytes": raw.len(),
        });
        let mut already_committed = false;
        for (offset, chunk) in raw.chunks(DESKTOP_LEGAL_REPORT_CHUNK_BYTES).enumerate() {
            if store
                .tombstones()?
                .iter()
                .any(|deleted| deleted == &item.id)
            {
                return Err("报告已在本机删除，停止后续分块".into());
            }
            let offset_bytes = offset * DESKTOP_LEGAL_REPORT_CHUNK_BYTES;
            let mut body = metadata.clone();
            let object = body.as_object_mut().expect("metadata is an object");
            object.insert("phase".into(), serde_json::json!("chunk"));
            object.insert("offsetBytes".into(), serde_json::json!(offset_bytes));
            object.insert(
                "chunkBase64".into(),
                serde_json::json!(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    chunk
                )),
            );
            let reply = request("upload", &mut body)?;
            remote_mutated = true;
            if tombstoned(&reply, &user_id) {
                store.delete(&item.id)?;
                already_committed = true;
                break;
            }
            if reply.mode == "legal_upload_committed" {
                accepted(&reply, "legal_upload_committed", &user_id)?;
                already_committed = true;
                break;
            }
            accepted(&reply, "legal_upload_chunk", &user_id)?;
            if reply.legal_report_received_bytes < offset_bytes + chunk.len()
                || reply.legal_report_received_bytes > raw.len()
            {
                return Err("服务端法律报告分块确认位置无效".into());
            }
        }
        if !already_committed {
            if store
                .tombstones()?
                .iter()
                .any(|deleted| deleted == &item.id)
            {
                return Err("报告已在本机删除，停止提交".into());
            }
            let mut body = metadata;
            body.as_object_mut()
                .expect("metadata is an object")
                .insert("phase".into(), serde_json::json!("commit"));
            let reply = request("upload", &mut body)?;
            if tombstoned(&reply, &user_id) {
                store.delete(&item.id)?;
            } else {
                accepted(&reply, "legal_upload_committed", &user_id)?;
            }
        }
    }

    // Only a server mutation invalidates the initial comparison view. Uploads
    // may evict older reports, so refresh once after all mutations and apply
    // the authoritative tombstones before any downloads.
    if remote_mutated {
        manifest = request("manifest", &mut serde_json::json!({}))?;
        accepted(&manifest, "legal_manifest", &user_id)?;
        local_snapshot = store.sync_snapshot(
            &manifest
                .legal_tombstones
                .iter()
                .map(|item| item.report_id.clone())
                .collect::<Vec<_>>(),
        )?;
    }
    let local_by_id = local_snapshot
        .reports
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect::<std::collections::HashMap<_, _>>();
    for item in manifest.legal_reports {
        if !active() {
            return Err("账户或工作区已切换，报告同步已停止".into());
        }
        if let Some(existing) = local_by_id.get(&item.report_id) {
            legal_report_metadata_matches(existing, &item)?;
            if store.report_file_available(existing)? {
                continue;
            }
        }
        let total = usize::try_from(item.size_bytes).map_err(|_| "服务端法律报告大小无效")?;
        if total == 0 || total > 32 * 1024 * 1024 {
            return Err("服务端法律报告超过大小上限".into());
        }
        let mut raw = Vec::with_capacity(total);
        while raw.len() < total {
            let offset = raw.len();
            let reply = request(
                "download",
                &mut serde_json::json!({
                    "reportId": item.report_id.clone(),
                    "offsetBytes": offset,
                    "maxBytes": DESKTOP_LEGAL_REPORT_CHUNK_BYTES.min(total - offset),
                }),
            )?;
            accepted(&reply, "legal_download", &user_id)?;
            if reply.legal_report_total_bytes != total
                || reply.legal_report_sha256 != item.sha256
                || reply.legal_report_created_at_epoch_millis != item.created_at_epoch_millis
                || reply.legal_report_source_workspace_id != item.source_workspace_id
            {
                return Err("服务端法律报告分块元数据发生变化".into());
            }
            let chunk = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                reply.legal_report_chunk_base64.as_bytes(),
            )
            .map_err(|_| "服务端法律报告分块编码无效")?;
            if chunk.is_empty()
                || chunk.len() > DESKTOP_LEGAL_REPORT_CHUNK_BYTES
                || chunk.len() > total - offset
            {
                return Err("服务端法律报告分块边界无效".into());
            }
            raw.extend_from_slice(&chunk);
        }
        if format!("{:x}", Sha256::digest(&raw)) != item.sha256 {
            return Err("下载的法律报告哈希不匹配".into());
        }
        if !active() {
            return Err("账户或工作区已切换，报告同步已停止".into());
        }
        let report: LegalReport =
            serde_json::from_slice(&raw).map_err(|_| "下载的法律报告结构无效".to_string())?;
        if report.workspace_id != item.source_workspace_id {
            return Err("下载的法律报告来源与清单不一致".into());
        }
        store.save(&item.report_id, item.created_at_epoch_millis, &raw)?;
    }
    Ok(())
}

fn legal_report_metadata_matches(
    local: &DesktopLegalReportMeta,
    remote: &gridtimer_native::sync_core::LegalReportManifestItem,
) -> Result<(), String> {
    if local.sha256 != remote.sha256
        || local.created_at_epoch_millis != remote.created_at_epoch_millis
        || local.size_bytes.and_then(|size| i64::try_from(size).ok()) != Some(remote.size_bytes)
        || local.source_workspace_id.as_deref() != Some(remote.source_workspace_id.as_str())
    {
        return Err("相同法律报告编号在两端对应不同内容，已停止同步".into());
    }
    Ok(())
}

#[cfg(test)]
mod desktop_legal_report_sync_tests {
    include!("legal_report_sync_tests.rs");
}
