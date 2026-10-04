use base64::Engine as _;
use gridtimer_native::legal_scan::LegalReport;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

const MAX_LEGAL_REPORT_BYTES: usize = 32 * 1024 * 1024;
const LEGAL_REPORT_LIMIT: usize = 5;
static LEGAL_REPORT_MUTATION_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();

fn lock_legal_report_mutation() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    LEGAL_REPORT_MUTATION_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| "法律报告存储锁不可用".into())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopLegalReportMeta {
    id: String,
    created_at_epoch_millis: i64,
    sha256: String,
    completed: bool,
    finding_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopLegalReportIndex {
    format_version: u32,
    scope_fingerprint: String,
    reports: Vec<DesktopLegalReportMeta>,
    tombstones: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopLegalEncryptedFile {
    format_version: u32,
    nonce_base64: String,
    ciphertext_base64: String,
}

struct DesktopLegalReportStore {
    root: PathBuf,
    scope_fingerprint: String,
    key: [u8; 32],
}

impl DesktopLegalReportStore {
    fn open(state_path: &Path, scope_identity: &str) -> Result<Self, String> {
        let _guard = lock_legal_report_mutation()?;
        let parent = state_path.parent().ok_or("法律报告工作区路径无效")?;
        let scope_fingerprint = format!("{:x}", Sha256::digest(scope_identity.as_bytes()));
        let root = parent.join("legal_reports_v1");
        fs::create_dir_all(&root).map_err(|e| format!("无法创建法律报告目录：{e}"))?;
        let key_path = root.join("key.dpapi");
        let key = if key_path.exists() {
            let raw = fs::read(&key_path).map_err(|e| format!("无法读取法律报告密钥：{e}"))?;
            let protected = raw
                .strip_prefix(DPAPI_SECRET_HEADER)
                .ok_or("法律报告密钥没有 Windows 用户保护")?;
            let plain = unprotect_secret_bytes(protected)
                .map_err(|e| format!("无法解锁法律报告密钥：{e}"))?;
            plain.try_into().map_err(|_| "法律报告密钥长度无效")?
        } else {
            let mut fresh = [0_u8; 32];
            OsRng.fill_bytes(&mut fresh);
            let protected =
                protect_secret_bytes(&fresh).map_err(|e| format!("无法保护法律报告密钥：{e}"))?;
            atomic_replace_bytes_no_backup(&key_path, &protected)
                .map_err(|e| format!("无法保存法律报告密钥：{e}"))?;
            fresh
        };
        // Every operation authenticates the index before using it. Defer that
        // read to the operation so a sync can reconcile one coherent snapshot.
        Ok(Self {
            root,
            scope_fingerprint,
            key,
        })
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.secure")
    }

    fn report_path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_legal_report_id(id) {
            return Err("报告编号无效".into());
        }
        Ok(self.root.join(format!("{id}.secure")))
    }

    fn encrypt(&self, purpose: &str, raw: &[u8]) -> Result<Vec<u8>, String> {
        let key = LessSafeKey::new(
            UnboundKey::new(&AES_256_GCM, &self.key).map_err(|_| "法律报告加密密钥无效")?,
        );
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut content = raw.to_vec();
        let aad = format!("legal_reports_v1\0{}\0{purpose}", self.scope_fingerprint);
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad.as_bytes()),
            &mut content,
        )
        .map_err(|_| "法律报告加密失败")?;
        serde_json::to_vec(&DesktopLegalEncryptedFile {
            format_version: 1,
            nonce_base64: base64::engine::general_purpose::STANDARD.encode(nonce),
            ciphertext_base64: base64::engine::general_purpose::STANDARD.encode(content),
        })
        .map_err(|e| e.to_string())
    }

    fn decrypt(&self, purpose: &str, raw: &[u8]) -> Result<Vec<u8>, String> {
        let envelope: DesktopLegalEncryptedFile =
            serde_json::from_slice(raw).map_err(|e| format!("加密报告文件无效：{e}"))?;
        if envelope.format_version != 1 {
            return Err("加密报告文件版本不支持".into());
        }
        let nonce: [u8; 12] = base64::engine::general_purpose::STANDARD
            .decode(envelope.nonce_base64)
            .map_err(|_| "报告随机数无效")?
            .try_into()
            .map_err(|_| "报告随机数长度无效")?;
        let mut content = base64::engine::general_purpose::STANDARD
            .decode(envelope.ciphertext_base64)
            .map_err(|_| "报告密文无效")?;
        let key = LessSafeKey::new(
            UnboundKey::new(&AES_256_GCM, &self.key).map_err(|_| "法律报告加密密钥无效")?,
        );
        let aad = format!("legal_reports_v1\0{}\0{purpose}", self.scope_fingerprint);
        let plain = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad.as_bytes()),
                &mut content,
            )
            .map_err(|_| "报告无法解密，可能属于其他工作区")?;
        Ok(plain.to_vec())
    }

    fn load_index(&self) -> Result<DesktopLegalReportIndex, String> {
        #[cfg(test)]
        legal_store_probe::record(|counts| counts.index_reads += 1);
        let path = self.index_path();
        if !path.exists() {
            return Ok(DesktopLegalReportIndex {
                format_version: 1,
                scope_fingerprint: self.scope_fingerprint.clone(),
                reports: Vec::new(),
                tombstones: Vec::new(),
            });
        }
        let raw = fs::read(path).map_err(|e| format!("无法读取报告清单：{e}"))?;
        let plain = self.decrypt("index", &raw)?;
        let index: DesktopLegalReportIndex =
            serde_json::from_slice(&plain).map_err(|e| format!("报告清单损坏：{e}"))?;
        if index.format_version != 1
            || index.scope_fingerprint != self.scope_fingerprint
            || index.reports.len() > LEGAL_REPORT_LIMIT
            || index.reports.iter().any(|item| {
                !valid_legal_report_id(&item.id)
                    || item
                        .size_bytes
                        .is_some_and(|size| size < 2 || size > MAX_LEGAL_REPORT_BYTES as u64)
                    || item
                        .source_workspace_id
                        .as_ref()
                        .is_some_and(|id| id.trim().is_empty())
            })
            || index.tombstones.iter().any(|id| !valid_legal_report_id(id))
        {
            return Err("法律报告清单归属或结构无效".into());
        }
        Ok(index)
    }

    fn save_index(&self, index: &DesktopLegalReportIndex) -> Result<(), String> {
        #[cfg(test)]
        legal_store_probe::record(|counts| counts.index_writes += 1);
        let raw = serde_json::to_vec(index).map_err(|e| e.to_string())?;
        let encrypted = self.encrypt("index", &raw)?;
        atomic_replace_bytes_no_backup(&self.index_path(), &encrypted)
            .map_err(|e| format!("无法保存报告清单：{e}"))?;
        self.load_index()?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<DesktopLegalReportMeta>, String> {
        Ok(self.load_index()?.reports)
    }

    fn tombstones(&self) -> Result<Vec<String>, String> {
        Ok(self.load_index()?.tombstones)
    }

    /// Hydrate older metadata once and merge remote deletes in one durable
    /// write. The authenticated result is the sync's immutable comparison view;
    /// upload authorization still rereads live tombstones before every chunk.
    fn sync_snapshot(&self, deleted_ids: &[String]) -> Result<DesktopLegalReportIndex, String> {
        let _guard = lock_legal_report_mutation()?;
        let mut index = self.load_index()?;
        let mut changed = merge_legal_tombstones(&mut index, deleted_ids)?;
        let mut hydration_error = None;
        for item in &mut index.reports {
            if item.source_workspace_id.is_none() || item.size_bytes.is_none() {
                match self.validated_raw(item) {
                    Ok((raw, report)) => {
                        item.source_workspace_id = Some(report.workspace_id);
                        item.size_bytes = Some(raw.len() as u64);
                        changed = true;
                    }
                    Err(error) => {
                        hydration_error.get_or_insert(error);
                    }
                }
            }
        }
        if changed {
            self.save_index(&index)?;
            for id in deleted_ids {
                let _ = fs::remove_file(self.report_path(id)?);
            }
        }
        // An unrelated damaged legacy body must not roll back a server delete.
        // Preserve the authenticated tombstones before reporting hydration failure.
        if let Some(error) = hydration_error {
            return Err(error);
        }
        Ok(index)
    }

    /// A cheap cache check keeps normal syncs from decrypting every report.
    /// A missing/truncated envelope is recoverable from its verified remote body;
    /// same-length corruption is still rejected when the report is opened.
    fn report_file_available(&self, meta: &DesktopLegalReportMeta) -> Result<bool, String> {
        let file = match fs::metadata(self.report_path(&meta.id)?) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("无法检查报告文件：{error}")),
        };
        if !file.is_file() || file.len() == 0 {
            return Ok(false);
        }
        let Some(size) = meta.size_bytes else {
            return Ok(true);
        };
        // The writer emits a fixed JSON envelope, a 12-byte nonce, and an
        // AES-GCM ciphertext containing a 16-byte tag. No body read is needed.
        let envelope_bytes = serde_json::to_vec(&DesktopLegalEncryptedFile {
            format_version: 1,
            nonce_base64: base64::engine::general_purpose::STANDARD.encode([0_u8; 12]),
            ciphertext_base64: String::new(),
        })
        .map_err(|error| error.to_string())?
        .len() as u64;
        let ciphertext_bytes = (size + 16).div_ceil(3) * 4;
        Ok(file.len() == envelope_bytes + ciphertext_bytes)
    }

    fn apply_tombstones(&self, deleted_ids: &[String]) -> Result<(), String> {
        if deleted_ids.is_empty() {
            return Ok(());
        }
        let _guard = lock_legal_report_mutation()?;
        let mut index = self.load_index()?;
        if merge_legal_tombstones(&mut index, deleted_ids)? {
            self.save_index(&index)?;
        }
        for id in deleted_ids {
            let _ = fs::remove_file(self.report_path(id)?);
        }
        Ok(())
    }

    fn save(&self, id: &str, created_at: i64, raw: &[u8]) -> Result<(), String> {
        let _guard = lock_legal_report_mutation()?;
        if !valid_legal_report_id(id) {
            return Err("报告编号无效".into());
        }
        if raw.len() < 2 || raw.len() > MAX_LEGAL_REPORT_BYTES {
            return Err("报告大小超出上限".into());
        }
        let report: LegalReport =
            serde_json::from_slice(raw).map_err(|e| format!("报告结构无效：{e}"))?;
        gridtimer_native::legal_sources::validate_report_sources(&report)?;
        if report.workspace_id.trim().is_empty()
            || report.workspace_id != report.manifest.workspace_id
            || report.captured_at_epoch_millis != report.manifest.captured_at_epoch_millis
            || created_at < report.captured_at_epoch_millis
        {
            return Err("报告来源或时间不一致".into());
        }
        let sha256 = format!("{:x}", Sha256::digest(raw));
        let mut index = self.load_index()?;
        if index.tombstones.iter().any(|old| old == id) {
            return Err("已删除的报告不能恢复".into());
        }
        let existing = index.reports.iter().find(|item| item.id == id);
        if let Some(existing) = existing {
            if existing.sha256 != sha256
                || existing.created_at_epoch_millis != created_at
                || existing
                    .source_workspace_id
                    .as_ref()
                    .is_some_and(|source| source != &report.workspace_id)
                || existing
                    .size_bytes
                    .is_some_and(|size| size != raw.len() as u64)
            {
                return Err("相同报告编号对应不同内容".into());
            }
            if self.report_file_available(existing)? {
                return Ok(());
            }
        }
        let encrypted = self.encrypt(id, raw)?;
        let path = self.report_path(id)?;
        atomic_replace_bytes_no_backup(&path, &encrypted)
            .map_err(|e| format!("无法保存法律报告：{e}"))?;
        if self.decrypt(id, &fs::read(&path).map_err(|e| e.to_string())?)? != raw {
            return Err("法律报告写入后校验失败".into());
        }
        if existing.is_some() {
            // Repair the missing cache body without replacing its identity,
            // creation time, or retention position. Tombstones were checked
            // under this same mutation lock before writing the replacement.
            return Ok(());
        }
        index.reports.push(DesktopLegalReportMeta {
            id: id.to_string(),
            created_at_epoch_millis: created_at,
            sha256,
            completed: report.completed,
            finding_count: report.findings.len(),
            source_workspace_id: Some(report.workspace_id),
            size_bytes: Some(raw.len() as u64),
        });
        index.reports.sort_by(|left, right| {
            right
                .created_at_epoch_millis
                .cmp(&left.created_at_epoch_millis)
                .then_with(|| right.id.cmp(&left.id))
        });
        let removed = index
            .reports
            .split_off(index.reports.len().min(LEGAL_REPORT_LIMIT));
        index
            .tombstones
            .extend(removed.iter().map(|item| item.id.clone()));
        self.save_index(&index)?;
        for item in removed {
            let _ = fs::remove_file(self.report_path(&item.id)?);
        }
        Ok(())
    }

    fn read(&self, id: &str) -> Result<LegalReport, String> {
        let index = self.load_index()?;
        let meta = index
            .reports
            .iter()
            .find(|item| item.id == id)
            .ok_or("报告不存在")?;
        self.validated_raw(meta).map(|(_, report)| report)
    }

    fn raw(&self, id: &str) -> Result<Vec<u8>, String> {
        let index = self.load_index()?;
        let meta = index
            .reports
            .iter()
            .find(|item| item.id == id)
            .ok_or("报告不存在")?;
        self.validated_raw(meta).map(|(raw, _)| raw)
    }

    fn validated_raw(
        &self,
        meta: &DesktopLegalReportMeta,
    ) -> Result<(Vec<u8>, LegalReport), String> {
        #[cfg(test)]
        legal_store_probe::record(|counts| counts.body_reads += 1);
        let raw =
            fs::read(self.report_path(&meta.id)?).map_err(|e| format!("无法读取报告：{e}"))?;
        let plain = self.decrypt(&meta.id, &raw)?;
        if plain.len() < 2 || plain.len() > MAX_LEGAL_REPORT_BYTES {
            return Err("报告大小超出上限".into());
        }
        if format!("{:x}", Sha256::digest(&plain)) != meta.sha256 {
            return Err("报告哈希不匹配".into());
        }
        let report: LegalReport =
            serde_json::from_slice(&plain).map_err(|e| format!("报告结构无效：{e}"))?;
        gridtimer_native::legal_sources::validate_report_sources(&report)?;
        if report.workspace_id.trim().is_empty()
            || report.workspace_id != report.manifest.workspace_id
            || report.captured_at_epoch_millis != report.manifest.captured_at_epoch_millis
            || meta.created_at_epoch_millis < report.captured_at_epoch_millis
            || meta
                .source_workspace_id
                .as_ref()
                .is_some_and(|id| id != &report.workspace_id)
            || meta
                .size_bytes
                .is_some_and(|size| size != plain.len() as u64)
        {
            return Err("报告来源或时间不一致".into());
        }
        Ok((plain, report))
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.apply_tombstones(&[id.to_string()])
    }
}

#[cfg(test)]
mod legal_store_probe {
    use std::cell::Cell;
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Counts {
        pub index_reads: usize,
        pub index_writes: usize,
        pub body_reads: usize,
    }
    thread_local! { static COUNTS: Cell<Counts> = Cell::new(Counts::default()); }
    pub fn record(change: impl FnOnce(&mut Counts)) {
        COUNTS.with(|cell| {
            let mut counts = cell.get();
            change(&mut counts);
            cell.set(counts);
        });
    }
    pub fn take() -> Counts {
        COUNTS.with(|cell| cell.replace(Counts::default()))
    }
}

fn merge_legal_tombstones(
    index: &mut DesktopLegalReportIndex,
    ids: &[String],
) -> Result<bool, String> {
    if ids.iter().any(|id| !valid_legal_report_id(id)) {
        return Err("报告编号无效".into());
    }
    let deleted = ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let before = index.reports.len();
    index
        .reports
        .retain(|item| !deleted.contains(item.id.as_str()));
    let mut changed = before != index.reports.len();
    let mut known = index
        .tombstones
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    for id in ids {
        if known.insert(id.clone()) {
            index.tombstones.push(id.clone());
            changed = true;
        }
    }
    Ok(changed)
}

fn valid_legal_report_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn new_legal_report_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes(bytes[0..4].try_into().unwrap()),
        u16::from_be_bytes(bytes[4..6].try_into().unwrap()),
        u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
        u16::from_be_bytes(bytes[8..10].try_into().unwrap()),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    )
}

#[cfg(test)]
mod desktop_legal_report_store_tests {
    use super::*;

    #[test]
    fn deleted_report_stays_deleted_and_other_account_has_no_access() {
        let root =
            std::env::temp_dir().join(format!("tenfold_legal_store_{}", new_legal_report_id()));
        let first_path = root.join("account_a").join("timer_state.json");
        let second_path = root.join("account_b").join("timer_state.json");
        let first = DesktopLegalReportStore::open(&first_path, "account_a").unwrap();
        let id = new_legal_report_id();
        let report = LegalReport {
            workspace_id: "source_a".into(),
            captured_at_epoch_millis: 1,
            completed: true,
            findings: Vec::new(),
            manifest: gridtimer_native::legal_scan::LegalScanManifest {
                workspace_id: "source_a".into(),
                captured_at_epoch_millis: 1,
                coverage: Default::default(),
                omissions: Vec::new(),
                evidence_count: 0,
                upload_bytes: 0,
                estimated_calls: 0,
            },
            errors: Vec::new(),
        };
        let raw = serde_json::to_vec(&report).unwrap();
        first.save(&id, 2, &raw).unwrap();
        assert_eq!(first.list().unwrap().len(), 1);
        let second = DesktopLegalReportStore::open(&second_path, "account_b").unwrap();
        assert!(second.list().unwrap().is_empty());
        assert!(second.read(&id).is_err());
        second.save("legacy_report_1", 2, &raw).unwrap();
        assert!(second.read("legacy_report_1").is_ok());
        first.delete(&id).unwrap();
        let reopened = DesktopLegalReportStore::open(&first_path, "account_a").unwrap();
        assert!(reopened.save(&id, 2, &raw).is_err());
        assert!(reopened.tombstones().unwrap().contains(&id));
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
