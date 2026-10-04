use super::*;
use gridtimer_native::sync_core::{
    LegalReportManifestItem, LegalReportTombstone, SyncClientResult,
};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    root: PathBuf,
    store: DesktopLegalReportStore,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("tenfold_legal_sync_{}", new_legal_report_id()));
        let store =
            DesktopLegalReportStore::open(&root.join("timer_state.json"), "synthetic_scope")
                .unwrap();
        Self { root, store }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self.root.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn report_bytes(extra_bytes: usize) -> Vec<u8> {
    serde_json::to_vec(&LegalReport {
        workspace_id: "synthetic_source".into(),
        captured_at_epoch_millis: 1,
        completed: true,
        findings: Vec::new(),
        manifest: gridtimer_native::legal_scan::LegalScanManifest {
            workspace_id: "synthetic_source".into(),
            captured_at_epoch_millis: 1,
            coverage: Default::default(),
            omissions: Vec::new(),
            evidence_count: 0,
            upload_bytes: 0,
            estimated_calls: 0,
        },
        errors: if extra_bytes == 0 {
            Vec::new()
        } else {
            vec!["x".repeat(extra_bytes)]
        },
    })
    .unwrap()
}

fn metadata(id: &str, raw: &[u8]) -> LegalReportManifestItem {
    LegalReportManifestItem {
        report_id: id.into(),
        created_at_epoch_millis: 2,
        sha256: format!("{:x}", Sha256::digest(raw)),
        size_bytes: raw.len() as i64,
        source_workspace_id: "synthetic_source".into(),
    }
}

#[derive(Default)]
struct FakeServer {
    reports: BTreeMap<String, (LegalReportManifestItem, Vec<u8>)>,
    tombstones: BTreeSet<String>,
    uploads: BTreeMap<String, Vec<u8>>,
    calls: Vec<String>,
    lose_next_commit_reply: bool,
}

impl FakeServer {
    fn reply(&self, mode: &str) -> SyncClientResult {
        SyncClientResult {
            ok: true,
            mode: mode.into(),
            user_id: "synthetic_user".into(),
            current_generation: 3,
            ..Default::default()
        }
    }

    fn request(&mut self, operation: &str, body: &Value) -> Result<SyncClientResult, String> {
        assert_eq!(body["workspaceId"], "synthetic_workspace");
        assert_eq!(body["workspaceProof"], "synthetic_proof");
        assert_eq!(body["acknowledgedGeneration"], 3);
        self.calls.push(format!(
            "{operation}:{}",
            body["phase"].as_str().unwrap_or("")
        ));
        let id = body["reportId"].as_str().unwrap_or("");
        if operation == "manifest" {
            let mut reply = self.reply("legal_manifest");
            reply.legal_reports = self
                .reports
                .values()
                .map(|(meta, _)| meta.clone())
                .collect();
            reply.legal_tombstones = self
                .tombstones
                .iter()
                .map(|id| LegalReportTombstone {
                    report_id: id.clone(),
                    deleted_at_epoch_millis: 4,
                })
                .collect();
            return Ok(reply);
        }
        if operation == "delete" {
            self.reports.remove(id);
            self.uploads.remove(id);
            self.tombstones.insert(id.into());
            let mut reply = self.reply("legal_delete");
            reply.legal_tombstones.push(LegalReportTombstone {
                report_id: id.into(),
                deleted_at_epoch_millis: 4,
            });
            return Ok(reply);
        }
        if self.tombstones.contains(id) {
            let mut reply = self.reply("legal_tombstoned");
            reply.ok = false;
            return Ok(reply);
        }
        if operation == "download" {
            let (meta, raw) = self.reports.get(id).ok_or("report absent")?;
            let start = body["offsetBytes"].as_u64().unwrap() as usize;
            let end = (start + body["maxBytes"].as_u64().unwrap() as usize).min(raw.len());
            let mut reply = self.reply("legal_download");
            reply.legal_report_total_bytes = raw.len();
            reply.legal_report_sha256 = meta.sha256.clone();
            reply.legal_report_created_at_epoch_millis = meta.created_at_epoch_millis;
            reply.legal_report_source_workspace_id = meta.source_workspace_id.clone();
            reply.legal_report_chunk_base64 =
                base64::engine::general_purpose::STANDARD.encode(&raw[start..end]);
            return Ok(reply);
        }
        assert_eq!(operation, "upload");
        if self.reports.contains_key(id) {
            return Ok(self.reply("legal_upload_committed"));
        }
        if body["phase"] == "chunk" {
            let chunk = base64::engine::general_purpose::STANDARD
                .decode(body["chunkBase64"].as_str().unwrap())
                .unwrap();
            let offset = body["offsetBytes"].as_u64().unwrap() as usize;
            let staged = self.uploads.entry(id.into()).or_default();
            if offset == staged.len() {
                staged.extend_from_slice(&chunk);
            } else {
                assert_eq!(&staged[offset..offset + chunk.len()], &chunk);
            }
            let received = staged.len();
            let mut reply = self.reply("legal_upload_chunk");
            reply.legal_report_received_bytes = received;
            return Ok(reply);
        }
        assert_eq!(body["phase"], "commit");
        let raw = self.uploads.remove(id).unwrap();
        assert_eq!(body["totalBytes"].as_u64().unwrap(), raw.len() as u64);
        assert_eq!(body["sha256"], format!("{:x}", Sha256::digest(&raw)));
        self.reports.insert(id.into(), (metadata(id, &raw), raw));
        if self.lose_next_commit_reply {
            self.lose_next_commit_reply = false;
            return Err("synthetic response lost after commit".into());
        }
        Ok(self.reply("legal_upload_committed"))
    }
}

fn sync(store: &DesktopLegalReportStore, server: &mut FakeServer) -> Result<(), String> {
    sync_desktop_legal_reports_with_transport(
        store,
        "synthetic_workspace",
        "synthetic_proof",
        3,
        || true,
        |op, body| server.request(op, body),
    )
}

#[test]
fn legal_sync_unchanged_nonempty_and_empty_use_one_manifest_without_body_reads_or_writes() {
    let fixture = Fixture::new();
    let mut server = FakeServer::default();
    sync(&fixture.store, &mut server).unwrap();
    assert_eq!(server.calls, ["manifest:"]);
    let raw = report_bytes(8_192);
    fixture.store.save("same_report", 2, &raw).unwrap();
    fixture.store.delete("already_deleted").unwrap();
    server
        .reports
        .insert("same_report".into(), (metadata("same_report", &raw), raw));
    server.tombstones.insert("already_deleted".into());
    server.calls.clear();
    legal_store_probe::take();
    sync(&fixture.store, &mut server).unwrap();
    let work = legal_store_probe::take();
    assert_eq!(server.calls, ["manifest:"]);
    assert_eq!(work.index_reads, 1);
    assert_eq!(work.index_writes, 0);
    assert_eq!(work.body_reads, 0);
    println!(
        "LEGAL_NOOP_WORK {}",
        json!({"manifestRequests":server.calls.len(),"indexReads":work.index_reads,"indexWrites":work.index_writes,"bodyReads":work.body_reads})
    );
}

#[test]
fn legal_sync_legacy_metadata_is_verified_and_hydrated_only_once() {
    let fixture = Fixture::new();
    let raw = report_bytes(128);
    fixture.store.save("legacy", 2, &raw).unwrap();
    let mut index = fixture.store.load_index().unwrap();
    index.reports[0].source_workspace_id = None;
    index.reports[0].size_bytes = None;
    fixture.store.save_index(&index).unwrap();
    let mut server = FakeServer::default();
    server
        .reports
        .insert("legacy".into(), (metadata("legacy", &raw), raw));
    legal_store_probe::take();
    sync(&fixture.store, &mut server).unwrap();
    let first = legal_store_probe::take();
    assert_eq!(first.body_reads, 1);
    assert_eq!(first.index_writes, 1);
    sync(&fixture.store, &mut server).unwrap();
    let second = legal_store_probe::take();
    assert_eq!(second.body_reads, 0);
    assert_eq!(second.index_writes, 0);
    assert_eq!(
        fixture.store.list().unwrap()[0]
            .source_workspace_id
            .as_deref(),
        Some("synthetic_source")
    );
}

#[test]
fn legal_sync_merges_remote_tombstones_once_and_never_uploads_deleted_reports() {
    let fixture = Fixture::new();
    let raw = report_bytes(0);
    fixture.store.save("gone_a", 2, &raw).unwrap();
    fixture.store.save("gone_b", 2, &raw).unwrap();
    let mut server = FakeServer::default();
    server.tombstones.extend(["gone_a".into(), "gone_b".into()]);
    legal_store_probe::take();
    sync(&fixture.store, &mut server).unwrap();
    assert_eq!(legal_store_probe::take().index_writes, 1);
    assert_eq!(server.calls, ["manifest:"]);
    assert!(fixture.store.list().unwrap().is_empty());
    legal_store_probe::take();
    sync(&fixture.store, &mut server).unwrap();
    assert_eq!(legal_store_probe::take().index_writes, 0);
    assert!(fixture.store.save("gone_a", 2, &raw).is_err());
}

#[test]
fn legal_sync_persists_remote_delete_despite_unreadable_legacy_body() {
    for missing in [true, false] {
        let fixture = Fixture::new();
        let raw = report_bytes(0);
        fixture.store.save("deleted", 2, &raw).unwrap();
        fixture.store.save("broken_legacy", 2, &raw).unwrap();
        let mut index = fixture.store.load_index().unwrap();
        let legacy = index
            .reports
            .iter_mut()
            .find(|item| item.id == "broken_legacy")
            .unwrap();
        legacy.source_workspace_id = None;
        legacy.size_bytes = None;
        fixture.store.save_index(&index).unwrap();
        let path = fixture.store.report_path("broken_legacy").unwrap();
        if missing {
            fs::remove_file(path).unwrap();
        } else {
            fs::write(path, b"damaged envelope").unwrap();
        }
        let mut server = FakeServer::default();
        server.tombstones.insert("deleted".into());
        legal_store_probe::take();
        assert!(sync(&fixture.store, &mut server).is_err());
        assert_eq!(legal_store_probe::take().index_writes, 1);
        assert_eq!(server.calls, ["manifest:"]);
        assert!(fixture
            .store
            .tombstones()
            .unwrap()
            .contains(&"deleted".into()));
        assert!(!fixture.store.report_path("deleted").unwrap().exists());
        assert!(fixture.store.read("deleted").is_err());
        assert!(fixture.store.save("deleted", 2, &raw).is_err());
        legal_store_probe::take();
        assert!(sync(&fixture.store, &mut server).is_err());
        assert_eq!(legal_store_probe::take().index_writes, 0);
    }
}

#[test]
fn legal_sync_repairs_missing_or_truncated_matched_body_without_replacing_metadata() {
    for missing in [true, false] {
        let fixture = Fixture::new();
        let raw = report_bytes(128);
        fixture.store.save("repair", 2, &raw).unwrap();
        let index_before = fs::read(fixture.store.index_path()).unwrap();
        let path = fixture.store.report_path("repair").unwrap();
        if missing {
            fs::remove_file(&path).unwrap();
        } else {
            fs::write(&path, b"truncated").unwrap();
        }
        let mut server = FakeServer::default();
        server
            .reports
            .insert("repair".into(), (metadata("repair", &raw), raw.clone()));
        sync(&fixture.store, &mut server).unwrap();
        assert_eq!(server.calls, ["manifest:", "download:"]);
        assert_eq!(fixture.store.raw("repair").unwrap(), raw);
        assert_eq!(fs::read(fixture.store.index_path()).unwrap(), index_before);
        server.calls.clear();
        legal_store_probe::take();
        sync(&fixture.store, &mut server).unwrap();
        let work = legal_store_probe::take();
        assert_eq!(server.calls, ["manifest:"]);
        assert_eq!(work.body_reads, 0);
        assert_eq!(work.index_writes, 0);
    }
}

#[test]
fn legal_sync_body_repair_rejects_bad_hash_source_and_concurrent_delete() {
    for failure in ["hash", "source", "delete"] {
        let fixture = Fixture::new();
        let raw = report_bytes(128);
        fixture.store.save("repair", 2, &raw).unwrap();
        let path = fixture.store.report_path("repair").unwrap();
        fs::remove_file(&path).unwrap();
        let mut server = FakeServer::default();
        server
            .reports
            .insert("repair".into(), (metadata("repair", &raw), raw.clone()));
        let result = sync_desktop_legal_reports_with_transport(
            &fixture.store,
            "synthetic_workspace",
            "synthetic_proof",
            3,
            || true,
            |op, body| {
                let mut reply = server.request(op, body)?;
                if op == "download" {
                    match failure {
                        "hash" => {
                            reply.legal_report_chunk_base64 =
                                base64::engine::general_purpose::STANDARD
                                    .encode(vec![b'x'; raw.len()]);
                        }
                        "source" => reply.legal_report_source_workspace_id = "other".into(),
                        _ => fixture.store.delete("repair").unwrap(),
                    }
                }
                Ok(reply)
            },
        );
        assert!(result.is_err(), "{failure}");
        assert!(!path.exists(), "{failure}");
        if failure == "delete" {
            assert!(fixture.store.save("repair", 2, &raw).is_err());
            server.calls.clear();
            sync(&fixture.store, &mut server).unwrap();
            assert_eq!(server.calls, ["manifest:", "delete:", "manifest:"]);
            assert!(fixture.store.list().unwrap().is_empty());
        } else {
            sync(&fixture.store, &mut server).unwrap();
            assert_eq!(fixture.store.raw("repair").unwrap(), raw);
        }
    }
}

#[test]
fn legal_sync_rejects_identity_and_metadata_changes_before_body_upload() {
    for field in ["source", "size", "hash", "generation", "mode", "empty_user"] {
        let fixture = Fixture::new();
        let raw = report_bytes(0);
        fixture.store.save("same", 2, &raw).unwrap();
        let mut server = FakeServer::default();
        server
            .reports
            .insert("same".into(), (metadata("same", &raw), raw));
        let result = sync_desktop_legal_reports_with_transport(
            &fixture.store,
            "synthetic_workspace",
            "synthetic_proof",
            3,
            || true,
            |op, body| {
                let mut reply = server.request(op, body)?;
                match field {
                    "source" => reply.legal_reports[0].source_workspace_id = "other".into(),
                    "size" => reply.legal_reports[0].size_bytes += 1,
                    "hash" => reply.legal_reports[0].sha256 = "0".repeat(64),
                    "generation" => reply.current_generation += 1,
                    "mode" => reply.mode = "wrong".into(),
                    _ => reply.user_id.clear(),
                }
                Ok(reply)
            },
        );
        assert!(result.is_err(), "{field}");
        assert_eq!(server.calls, ["manifest:"]);
    }
}

#[test]
fn legal_sync_upload_download_retry_and_live_delete_preserve_commit_boundaries() {
    let fixture = Fixture::new();
    let raw = report_bytes(DESKTOP_LEGAL_REPORT_CHUNK_BYTES + 32);
    fixture.store.save("large", 2, &raw).unwrap();
    let mut server = FakeServer {
        lose_next_commit_reply: true,
        ..Default::default()
    };
    assert!(sync(&fixture.store, &mut server).is_err());
    assert_eq!(
        server.calls,
        ["manifest:", "upload:chunk", "upload:chunk", "upload:commit"]
    );
    server.calls.clear();
    sync(&fixture.store, &mut server).unwrap();
    assert_eq!(server.calls, ["manifest:"]);
    let recipient = Fixture::new();
    server.calls.clear();
    sync(&recipient.store, &mut server).unwrap();
    assert_eq!(server.calls, ["manifest:", "download:", "download:"]);
    assert_eq!(recipient.store.raw("large").unwrap(), raw);
    recipient.store.delete("large").unwrap();
    server.calls.clear();
    sync(&recipient.store, &mut server).unwrap();
    assert_eq!(server.calls, ["manifest:", "delete:", "manifest:"]);
    sync(&fixture.store, &mut server).unwrap();
    assert!(fixture.store.list().unwrap().is_empty());

    fixture.store.save("delete_mid_upload", 2, &raw).unwrap();
    server.calls.clear();
    let result = sync_desktop_legal_reports_with_transport(
        &fixture.store,
        "synthetic_workspace",
        "synthetic_proof",
        3,
        || true,
        |op, body| {
            let reply = server.request(op, body)?;
            if op == "upload" {
                fixture.store.delete("delete_mid_upload").unwrap();
            }
            Ok(reply)
        },
    );
    assert!(result.is_err());
    assert_eq!(server.calls, ["manifest:", "upload:chunk"]);
    assert!(!server.reports.contains_key("delete_mid_upload"));
}

#[test]
fn legal_sync_cancel_prevents_first_request_and_next_chunk_and_late_result_write() {
    let fixture = Fixture::new();
    let mut server = FakeServer::default();
    assert!(sync_desktop_legal_reports_with_transport(
        &fixture.store,
        "synthetic_workspace",
        "synthetic_proof",
        3,
        || false,
        |op, body| server.request(op, body)
    )
    .is_err());
    assert!(server.calls.is_empty());
    let raw = report_bytes(DESKTOP_LEGAL_REPORT_CHUNK_BYTES + 32);
    for upload in [true, false] {
        let fixture = Fixture::new();
        let mut server = FakeServer::default();
        if upload {
            fixture.store.save("cancelled", 2, &raw).unwrap();
        } else {
            server.reports.insert(
                "cancelled".into(),
                (metadata("cancelled", &raw), raw.clone()),
            );
        }
        let active = std::cell::Cell::new(true);
        let result = sync_desktop_legal_reports_with_transport(
            &fixture.store,
            "synthetic_workspace",
            "synthetic_proof",
            3,
            || active.get(),
            |op, body| {
                let reply = server.request(op, body)?;
                if op != "manifest" {
                    active.set(false);
                }
                Ok(reply)
            },
        );
        assert!(result.is_err());
        assert_eq!(server.calls.len(), 2);
        if !upload {
            assert!(fixture.store.list().unwrap().is_empty());
        }
        assert!(!server.reports.contains_key("cancelled") || !upload);
    }
}

/// Real Windows DPAPI, AES-GCM and production loopback HTTP; all content and
/// credentials are synthetic. The server never opens the user's databases.
#[test]
#[ignore = "Explicit isolated native HTTP/DPAPI legal report exercise"]
fn legal_sync_native_nonempty_roundtrip_benchmark() {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_worker = stop.clone();
    let cancel_download = Arc::new(AtomicBool::new(false));
    let cancel_download_worker = cancel_download.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_worker = cancelled.clone();
    let worker = thread::spawn(move || {
        let mut server = FakeServer {
            lose_next_commit_reply: true,
            ..Default::default()
        };
        while !stop_worker.load(AtomicOrdering::Acquire) {
            let (stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("synthetic listener: {error}"),
            };
            // Windows accepts inherit the listener's nonblocking mode. The
            // listener polls shutdown; each synthetic HTTP exchange is bounded
            // blocking I/O, matching the real production server contract.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let operation = first
                .split_whitespace()
                .nth(1)
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            assert!(length < 8 * 1024 * 1024);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let reply = match server.request(&operation, &serde_json::from_slice(&body).unwrap()) {
                Ok(reply) => reply,
                Err(_) => continue, // Drop one synthetic commit response after durable acceptance.
            };
            if operation == "download" && cancel_download_worker.swap(false, AtomicOrdering::AcqRel)
            {
                cancelled_worker.store(true, AtomicOrdering::Release);
            }
            let raw = serde_json::to_vec(&reply).unwrap();
            let mut stream = reader.into_inner();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", raw.len()).unwrap();
            stream.write_all(&raw).unwrap();
        }
        server.calls.len()
    });
    let started = Instant::now();
    let result = std::panic::catch_unwind(|| {
        let first = Fixture::new();
        let second = Fixture::new();
        let raw = report_bytes(DESKTOP_LEGAL_REPORT_CHUNK_BYTES + 32);
        let url = format!("http://{address}");
        for round in 0..8 {
            let id = format!("native_{round}");
            first.store.save(&id, 2, &raw).unwrap();
            let upload = sync_desktop_legal_reports_while(
                &first.store,
                &url,
                "synthetic_token",
                "synthetic_workspace",
                "synthetic_proof",
                3,
                || true,
            );
            if round == 0 {
                assert!(
                    upload.is_err(),
                    "first response is intentionally lost after commit"
                );
                sync_desktop_legal_reports_while(
                    &first.store,
                    &url,
                    "synthetic_token",
                    "synthetic_workspace",
                    "synthetic_proof",
                    3,
                    || true,
                )
                .unwrap();
                cancel_download.store(true, AtomicOrdering::Release);
                assert!(sync_desktop_legal_reports_while(
                    &second.store,
                    &url,
                    "synthetic_token",
                    "synthetic_workspace",
                    "synthetic_proof",
                    3,
                    || !cancelled.load(AtomicOrdering::Acquire)
                )
                .is_err());
                assert!(second.store.list().unwrap().is_empty());
                cancelled.store(false, AtomicOrdering::Release);
            } else {
                upload.unwrap();
            }
            sync_desktop_legal_reports_while(
                &second.store,
                &url,
                "synthetic_token",
                "synthetic_workspace",
                "synthetic_proof",
                3,
                || true,
            )
            .unwrap();
            assert_eq!(second.store.raw(&id).unwrap(), raw);
            second.store.delete(&id).unwrap();
            for store in [&second.store, &first.store] {
                sync_desktop_legal_reports_while(
                    store,
                    &url,
                    "synthetic_token",
                    "synthetic_workspace",
                    "synthetic_proof",
                    3,
                    || true,
                )
                .unwrap();
            }
            assert!(first.store.list().unwrap().is_empty());
        }
    });
    stop.store(true, AtomicOrdering::Release);
    let requests = worker.join().unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
    println!(
        "LEGAL_NATIVE_PERFORMANCE {}",
        json!({"rounds":8,"requests":requests,"elapsedMillis":started.elapsed().as_millis(),"syntheticOnly":true,"productionHttp":true,"nativeEncryption":cfg!(windows),"ambiguousCommitRetry":true,"cancelledDownloadRetried":true})
    );
}
