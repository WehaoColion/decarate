// v2.22.48 - Move backup actions to the risk header while preserving the merge confirmation.
// v2.22.35 - Portable finance backups with bounded import and reviewed shared merge.

const FINANCE_IMPORT_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct DesktopTransferState {
    pending_finance: Option<FinanceImportPreview>,
    last_finance_export: Option<PathBuf>,
    last_note_export: Option<PathBuf>,
}

struct FinanceImportPreview {
    source_path: PathBuf,
    workspace_path: PathBuf,
    profile_json: String,
    current_profile: Value,
    base_version: u64,
    summary: FinanceImportSummary,
}

#[derive(Debug, Default, PartialEq)]
struct FinanceImportSummary {
    current_days: usize,
    current_months: usize,
    backup_days: usize,
    backup_months: usize,
    added_days: usize,
    added_months: usize,
    added_rows: usize,
    changed_fields: usize,
}

fn finance_period_count(profile: &Value, key: &str) -> usize {
    profile
        .get(key)
        .and_then(Value::as_object)
        .map_or(0, |map| map.len())
}

fn finance_stored_row_count(profile: &Value) -> usize {
    [
        ("dailyLedgers", ["incomes", "expenses"]),
        ("monthlySnapshots", ["assets", "liabilities"]),
    ]
    .into_iter()
    .map(|(period, fields)| {
        profile
            .get(period)
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|map| map.values())
            .map(|record| {
                fields
                    .iter()
                    .map(|field| {
                        record
                            .get(*field)
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len)
                    })
                    .sum::<usize>()
            })
            .sum::<usize>()
    })
    .sum()
}

fn validate_finance_import(raw: &str) -> Result<String, String> {
    if raw.len() > FINANCE_IMPORT_MAX_BYTES {
        return Err("风控备份超过 8 MB，未读取".into());
    }
    let root: Value = serde_json::from_str(raw).map_err(|_| "备份格式无效或文件已损坏")?;
    if (root.get("schemaVersion").is_some() || root.get("appVersionName").is_some())
        && root.get("financeProfile").is_none()
    {
        return Err("备份缺少 financeProfile 风控内容".into());
    }
    let mut pending = vec![(&root, 1_usize)];
    let mut containers = 0_usize;
    while let Some((value, depth)) = pending.pop() {
        if value.is_array() || value.is_object() {
            containers += 1;
            if depth > 64 || containers > 100_000 {
                return Err("备份层级或记录结构超出安全上限".into());
            }
            match value {
                Value::Array(items) => pending.extend(items.iter().map(|item| (item, depth + 1))),
                Value::Object(items) => {
                    pending.extend(items.values().map(|item| (item, depth + 1)))
                }
                _ => {}
            }
        }
    }
    let profile = root.get("financeProfile").unwrap_or(&root);
    for (collection, monthly) in [("dailyLedgers", false), ("monthlySnapshots", true)] {
        if let Some(records) = profile.get(collection).and_then(Value::as_object) {
            for key in records.keys() {
                let valid = if monthly {
                    gridtimer_native::finance_month_snapshot_or_default_json("{}", key).is_some()
                } else {
                    gridtimer_native::finance_day_ledger_or_default_json("{}", key).is_some()
                };
                if !valid {
                    return Err(format!("备份包含无效账期：{key}"));
                }
            }
        }
    }
    if !profile.is_object()
        || finance_period_count(profile, "dailyLedgers") > 10_000
        || finance_period_count(profile, "monthlySnapshots") > 1_200
        || finance_stored_row_count(profile) > 50_000
        || profile
            .pointer("/settings/expenseCategories")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows.len() > 128)
    {
        return Err("备份内的账期、账目或分类数量超出安全上限".into());
    }
    // Android uses this same payload schema and profile sanitation.
    gridtimer_native::decode_finance_backup_profile_json(raw)
        .ok_or_else(|| "不支持的风控备份版本或数据格式".into())
}

fn read_finance_import(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| error.to_string())?
        .take((FINANCE_IMPORT_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > FINANCE_IMPORT_MAX_BYTES {
        return Err("风控备份超过 8 MB，未读取".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "备份不是有效的 UTF-8 文本")?;
    validate_finance_import(text.strip_prefix('\u{feff}').unwrap_or(text))
}

fn finance_import_preview(
    state_json: &str,
    profile_json: &str,
    now: i64,
) -> Result<(Value, FinanceImportSummary), String> {
    let current: Value = serde_json::from_str(state_json).map_err(|_| "当前工作区无法读取")?;
    let imported: Value = serde_json::from_str(profile_json).map_err(|_| "备份无法读取")?;
    let merged = app_data::merge_finance_profile_app_data_json(state_json, profile_json, now)
        .ok_or("风控备份无法与当前工作区安全合并")?;
    let merged: Value = serde_json::from_str(&merged).map_err(|_| "合并结果无法读取")?;
    let current = current
        .get("financeProfile")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = merged.get("financeProfile").ok_or("合并结果缺少风控数据")?;
    let changed_fields = [
        "activeIncomeMonthly",
        "assetIncomeMonthly",
        "livingExpenseMonthly",
        "liabilityPaymentMonthly",
        "cashReserve",
        "productiveAssetValue",
        "liabilityBalance",
        "acquisitionFocus",
        "liabilityFocus",
        "settings",
    ]
    .into_iter()
    .filter(|field| current.get(*field) != result.get(*field))
    .count();
    let summary = FinanceImportSummary {
        current_days: finance_period_count(&current, "dailyLedgers"),
        current_months: finance_period_count(&current, "monthlySnapshots"),
        backup_days: finance_period_count(&imported, "dailyLedgers"),
        backup_months: finance_period_count(&imported, "monthlySnapshots"),
        added_days: finance_period_count(result, "dailyLedgers")
            .saturating_sub(finance_period_count(&current, "dailyLedgers")),
        added_months: finance_period_count(result, "monthlySnapshots")
            .saturating_sub(finance_period_count(&current, "monthlySnapshots")),
        added_rows: finance_stored_row_count(result)
            .saturating_sub(finance_stored_row_count(&current)),
        changed_fields,
    };
    Ok((current, summary))
}

fn write_transfer_text_verified(path: &Path, text: &str) -> Result<(), String> {
    atomic_replace_text_no_backup(path, text).map_err(|error| error.to_string())?;
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    let mut len = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        len += count as u64;
        digest.update(&buffer[..count]);
    }
    if len != text.len() as u64 || digest.finalize() != Sha256::digest(text.as_bytes()) {
        return Err("导出文件写入后校验失败".into());
    }
    Ok(())
}

impl TimerWindowsClient {
    fn export_finance_backup(&mut self) {
        if let Err(error) = self.flush_all_pending_saves() {
            self.status = format!("请先完成本机保存：{error}");
            return;
        }
        let now = now_millis();
        let profile = match serde_json::to_string(&self.data.finance_profile) {
            Ok(profile) => profile,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let Some(backup) =
            gridtimer_native::encode_finance_backup_json(&profile, WINDOWS_CLIENT_VERSION, now)
        else {
            self.status = "风控数据无法导出".into();
            return;
        };
        let name = format!("finance_backup_v{WINDOWS_CLIENT_VERSION}_{now}.json");
        match desktop_transfer_file_dialog(true, "导出风控备份", &name, "json", "风控备份")
        {
            Ok(Some(path)) => match write_transfer_text_verified(&path, &backup) {
                Ok(()) => {
                    self.status = format!("风控备份已导出：{}", path.display());
                    self.transfers.last_finance_export = Some(path);
                }
                Err(error) => self.status = format!("导出失败：{error}"),
            },
            Ok(None) => {}
            Err(error) => self.status = error.to_string(),
        }
    }

    fn choose_finance_import(&mut self) {
        if self.workspace_edit_locked() {
            return;
        }
        if let Err(error) = self.flush_all_pending_saves() {
            self.status = format!("请先完成本机保存：{error}");
            return;
        }
        let path = match desktop_transfer_file_dialog(false, "选择风控备份", "", "json", "风控备份")
        {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let result = read_finance_import(&path).and_then(|profile_json| {
            let (current_profile, summary) =
                finance_import_preview(&self.state_json, &profile_json, now_millis())?;
            Ok(FinanceImportPreview {
                source_path: path,
                workspace_path: self.state_path.clone(),
                profile_json,
                current_profile,
                base_version: self.data_version,
                summary,
            })
        });
        match result {
            Ok(preview) => self.transfers.pending_finance = Some(preview),
            Err(error) => self.status = format!("未导入：{error}"),
        }
    }

    fn confirm_finance_import(&mut self) {
        let Some(mut preview) = self.transfers.pending_finance.take() else {
            return;
        };
        if self.workspace_edit_locked() || preview.workspace_path != self.state_path {
            self.status = "工作区已变化，请重新选择备份".into();
            return;
        }
        if let Err(error) = self.flush_all_pending_saves() {
            self.status = format!("请先完成本机保存：{error}");
            self.transfers.pending_finance = Some(preview);
            return;
        }
        let refreshed =
            match finance_import_preview(&self.state_json, &preview.profile_json, now_millis()) {
                Ok(value) => value,
                Err(error) => {
                    self.status = error;
                    return;
                }
            };
        if refreshed.0 != preview.current_profile {
            preview.current_profile = refreshed.0;
            preview.summary = refreshed.1;
            preview.base_version = self.data_version;
            self.transfers.pending_finance = Some(preview);
            self.status = "本机风控数据刚有更新，请核对新的合并预览后确认".into();
            return;
        }
        let now = now_millis();
        let backup_path = self
            .state_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("finance_backups")
            .join(format!(
                "finance_pre_restore_{now}_{}.json",
                random_desktop_identifier("snapshot")
            ));
        let backup = gridtimer_native::encode_finance_backup_json(
            &preview.current_profile.to_string(),
            WINDOWS_CLIENT_VERSION,
            now,
        );
        let result = backup
            .ok_or_else(|| "恢复前快照无法编码".to_string())
            .and_then(|backup| write_transfer_text_verified(&backup_path, &backup));
        if let Err(error) = result {
            self.status = format!("未恢复，恢复前快照保存失败：{error}");
            self.transfers.pending_finance = Some(preview);
            return;
        }
        let next = app_data::merge_finance_profile_app_data_json(
            &self.state_json,
            &preview.profile_json,
            now,
        );
        if self.replace_state(next, "风控备份已安全合并") {
            self.status = format!("风控备份已安全合并；恢复前快照：{}", backup_path.display());
            self.transfers.last_finance_export = Some(backup_path);
        } else {
            self.transfers.pending_finance = Some(preview);
        }
    }

    fn ui_finance_backup_actions(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            if ui.button("导出风控备份").clicked() {
                self.export_finance_backup();
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    !self.workspace_edit_locked(),
                    egui::Button::new("导入风控备份"),
                )
                .clicked()
            {
                self.choose_finance_import();
                ui.close_menu();
            }
            if let Some(path) = self.transfers.last_finance_export.clone() {
                if ui.button("打开备份文件").clicked() {
                    if let Err(error) = open_local_file(&path) {
                        self.status = error.to_string();
                    }
                    ui.close_menu();
                }
            }
        });
    }

    fn ui_finance_backup_tools(&mut self, ui: &mut egui::Ui) {
        if self.desktop_ui.legal_risk.open {
            return;
        }
        let Some(mut preview) = self.transfers.pending_finance.take() else {
            return;
        };
        if preview.workspace_path != self.state_path {
            return;
        }
        if preview.base_version != self.data_version {
            match finance_import_preview(&self.state_json, &preview.profile_json, now_millis()) {
                Ok((current, summary)) => {
                    preview.current_profile = current;
                    preview.summary = summary;
                    preview.base_version = self.data_version;
                }
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }
        let mut confirm = false;
        let mut open = true;
        let mut cancel = false;
        egui::Window::new("确认合并风控备份")
            .id(egui::Id::new("finance_backup_confirmation"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .max_width((ui.ctx().screen_rect().width() - 40.0).max(240.0))
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "文件：{}",
                    preview
                        .source_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                let s = &preview.summary;
                ui.label(format!(
                    "当前：{} 个日账，{} 个月快照。备份：{} 个日账，{} 个月快照。",
                    s.current_days, s.current_months, s.backup_days, s.backup_months
                ));
                ui.label(format!(
                    "将补入 {} 个日期、{} 个月份和 {} 条记录，补齐 {} 项空缺参数或设置。",
                    s.added_days, s.added_months, s.added_rows, s.changed_fields
                ));
                ui.label(
                    "同 ID 记录保留当前内容。计时记录和便签保持原样。恢复前会先保存当前风控快照。",
                );
                ui.horizontal_wrapped(|ui| {
                    confirm = ui
                        .add_enabled(!self.workspace_edit_locked(), egui::Button::new("确认合并"))
                        .clicked();
                    cancel = ui.button("取消").clicked();
                });
            });
        if open && !cancel {
            self.transfers.pending_finance = Some(preview);
            if confirm {
                self.confirm_finance_import();
            }
        }
    }
}
