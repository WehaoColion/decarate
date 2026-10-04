//! Final Android legal-analysis UI transform. Run after every older source transform.
//! Only the explicit confirmation handler may start a model request.

const GRID_PATH: &str = "com/ofairyo/gridtimer/ui/GridTimerScreen.kt";
const LEGAL_PATH: &str = "com/ofairyo/gridtimer/ui/LegalRiskScreen.kt";
const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/LegalSendReadyTest.kt";

fn replace_once(source: &mut String, label: &str, before: &str, after: &str) -> Result<(), String> {
    let count = source.matches(before).count();
    if count != 1 {
        return Err(format!("legal workflow {label}: expected one anchor, found {count}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path == TEST_PATH {
        return Ok(format!("{source}{TESTS}"));
    }
    if path != GRID_PATH && path != LEGAL_PATH {
        return Ok(source.to_string());
    }
    let mut source = source.to_string();
    for import in ["androidx.compose.ui.platform.testTag", "androidx.compose.foundation.rememberScrollState", "androidx.compose.foundation.verticalScroll"] {
        let statement = format!("import {import}\n");
        if !source.contains(&statement) {
            replace_once(&mut source, "workflow_import", "package com.ofairyo.gridtimer.ui\n",
                &format!("package com.ofairyo.gridtimer.ui\n{statement}"))?;
        }
    }
    let replacements = if path == GRID_PATH { GRID_REPLACEMENTS } else { LEGAL_REPLACEMENTS };
    for &(label, before, after) in replacements {
        replace_once(&mut source, label, before, after)?;
    }
    if path == LEGAL_PATH {
        source.push_str(HELPERS);
    }
    Ok(source)
}

const GRID_REPLACEMENTS: &[(&str, &str, &str)] = &[
    ("entry_button", r####"                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextButton(onClick = { showLegalRisk = true }) { Text("开始 AI 分析") }
                            TextButton(onClick = onOpenAiSettings) { Text("AI 设置") }
                        }"####, r####"                        androidx.compose.material3.Button(
                            onClick = { showLegalRisk = true },
                            modifier = Modifier.fillMaxWidth().testTag("legal_open_analysis")
                        ) { Text("开始 AI 分析") }
                        TextButton(onClick = onOpenAiSettings) { Text("AI 设置") }"####),
    ("entry_hint", r####"${syncSession.aiModel} · 发送前查看范围并确认"####, r####"${syncSession.aiModel} · 点击上方按钮，整理资料后确认发送"####),
    ("full_screen_route", r####"    if (showLegalRisk) {
        LegalRiskScreen("####, r####"    if (showLegalRisk) {
        // Isolate the workflow from the finance pager and floating home navigation.
        androidx.compose.ui.window.Dialog(
            onDismissRequest = { showLegalRisk = false },
            properties = androidx.compose.ui.window.DialogProperties(
                usePlatformDefaultWidth = false,
                securePolicy = androidx.compose.ui.window.SecureFlagPolicy.SecureOn
            )
        ) {
            Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        LegalRiskScreen("####),
    ("full_screen_route_end", r####"            modifier = modifier
        )
    } else if (asPage) {"####, r####"            modifier = Modifier
        )
            }
        }
    } else if (asPage) {"####),
];

const LEGAL_REPLACEMENTS: &[(&str, &str, &str)] = &[
    ("consent_state", r####"    var preview by remember(workspaceKey, accountIdentity) { mutableStateOf<LegalPreview?>(null) }"####, r####"    var preview by remember(workspaceKey, accountIdentity) { mutableStateOf<LegalPreview?>(null) }
    var pendingConsent by remember(workspaceKey, accountIdentity) { mutableStateOf<LegalSendConsent?>(null) }"####),
    ("remove_buried_prepare", r####"            if (current == null && !running) {
                item {
                    Button(onClick = ::prepare, enabled = !preparing, modifier = Modifier.fillMaxWidth()) {
                        if (preparing) CircularProgressIndicator(modifier = Modifier.height(18.dp))
                        else Text("准备扫描")
                    }
                }
            }
"####, r####""####),
    ("remove_buried_send", r####"                item {
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        Button(onClick = ::send, enabled = sendEnabled, modifier = Modifier.weight(1f)) {
                            Text("发送并分析")
                        }
                        OutlinedButton(onClick = {
                            // Re-check in the action as a click may precede recomposition.
                            if (requests.canDiscardPreview() && preview?.scanId == current.scanId) {
                                requests.invalidate()
                                NativeOptimizerBridge.closeLegalScan(current.scanId)
                                preview = null
                                showContent = false
                                openedEvidence = null
                                evidenceLoading = false
                                message = "已撤销本次预览。"
                            }
                        }, enabled = requests.canDiscardPreview()) { Text("撤销") }
                    }
                    if (!sendEnabled && syncSession.aiApiKey.isBlank()) {
                        Text("请先在“我的”中配置 AI 密钥。")
                    }
                }
"####, r####""####),
    ("remove_buried_running", r####"            if (running) {
                item {
                    LegalSection {
                        CircularProgressIndicator()
                        Text(if (cancelling) "正在停止后续请求…" else "正在分析。可取消后续请求。")
                        OutlinedButton(onClick = ::cancelCurrentScan, enabled = !cancelling) {
                            Text("取消分析")
                        }
                    }
                }
            }
"####, r####""####),
    ("pin_actions", r####"        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxWidth(),"####, r####"        // Outside LazyColumn: actions cannot be buried by notes, omissions or old reports.
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp)
        ) {
            val action = legalActionState(
                latestAccountIdentity == accountIdentity && viewModel.currentWorkspaceKey() == workspaceKey,
                preparing, running, cancelling, current != null,
                current?.manifest?.optInt("evidenceCount") ?: 0,
                AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel).testable,
                recipient != null, current?.capturedData == latestAppData
            )
            Button(
                modifier = Modifier.fillMaxWidth().testTag("legal_primary_action"),
                enabled = action.action != LegalPrimaryAction.WAIT,
                onClick = {
                    when (action.action) {
                        LegalPrimaryAction.PREPARE -> prepare()
                        LegalPrimaryAction.SETTINGS -> onOpenAiSettings()
                        LegalPrimaryAction.REOPEN -> onDismiss()
                        LegalPrimaryAction.REVIEW_SEND -> current?.let {
                            pendingConsent = LegalSendConsent(it.scanId, syncSession.aiBaseUrl,
                                syncSession.aiModel, syncSession.aiApiKey)
                        }
                        LegalPrimaryAction.WAIT -> Unit
                    }
                }
            ) { Text(action.label) }
            Text(action.hint, style = MaterialTheme.typography.bodySmall)
            if (message.isNotBlank()) Text(message, style = MaterialTheme.typography.bodySmall,
                maxLines = 3, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            if (running) {
                OutlinedButton(onClick = ::cancelCurrentScan, enabled = !cancelling,
                    modifier = Modifier.testTag("legal_cancel_analysis")) { Text("取消分析") }
            } else if (current != null) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { showContent = true }) { Text("查看发送范围和内容") }
                    TextButton(onClick = {
                        if (requests.canDiscardPreview() && preview?.scanId == current.scanId) {
                            requests.invalidate()
                            NativeOptimizerBridge.closeLegalScan(current.scanId)
                            preview = null
                            pendingConsent = null
                            showContent = false
                            openedEvidence = null
                            evidenceLoading = false
                            message = "已撤销预览，可解锁笔记或重新整理资料。"
                        }
                    }, enabled = requests.canDiscardPreview()) { Text("撤销预览") }
                }
            }
        }
        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxWidth(),"####),
    ("confirm_send", r####"    if (showContent && current != null) {"####, r####"    val consent = pendingConsent
    if (consent != null && current != null) {
        AlertDialog(
            onDismissRequest = { pendingConsent = null },
            title = { Text("确认发送并开始分析") },
            text = {
                Column(modifier = Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("接收方：${legalRecipient(consent.baseUrl) ?: "地址无效"}")
                    Text("模型：${consent.model}")
                    Text("资料：${current.manifest.optInt("evidenceCount")} 条；约 ${formatLegalBytes(current.manifest.optLong("uploadBytes"))}")
                    Text("快照时间：${formatLegalTime(current.manifest.optLong("capturedAtEpochMillis"))}")
                    Text("确认后才会向上方服务发送本次预览的资料。可能包含个人信息，请先核对范围；服务方按其政策处理输入。")
                    if (syncSession.loggedIn) Text("最终报告会同步到同一账户的配对设备，同步服务及其备份会保存报告；扫描原文和临时解锁笔记不会作为报告同步。")
                    if (!sendEnabled) Text("当前不能发送，请取消后核对资料、AI 配置及正在进行的请求。")
                    TextButton(onClick = { pendingConsent = null; showContent = true }) { Text("先查看发送内容") }
                }
            },
            confirmButton = {
                TextButton(modifier = Modifier.testTag("legal_confirm_send"), enabled = sendEnabled,
                    onClick = {
                        if (consent.consume(current.scanId, syncSession.aiBaseUrl,
                                syncSession.aiModel, syncSession.aiApiKey, sendEnabled)) {
                            pendingConsent = null
                            send()
                        } else {
                            pendingConsent = null
                            message = "资料或 AI 配置已变化，请重新核对发送范围。"
                        }
                    }) { Text("确认发送并分析") }
            },
            dismissButton = { TextButton(onClick = { pendingConsent = null }) { Text("暂不发送") } }
        )
    }

    if (showContent && current != null) {"####),
    ("prepare_workspace_feedback", r####"        if (preparing || running || cancelling || viewModel.currentWorkspaceKey() != workspaceKey) return"####, r####"        if (viewModel.currentWorkspaceKey() != workspaceKey) {
            message = "工作区已变化，请关闭此页后重新进入。"
            return
        }
        if (preparing || running || cancelling) return"####),
    ("send_blocker_feedback", r####"        )) return
        val currentScan = current ?: return"####, r####"        )) {
            pendingConsent = null
            message = "当前不能发送，请核对资料是否更新、AI 配置及工作区，并等待正在进行的请求结束。"
            return
        }
        val currentScan = current ?: return"####),
    ("show_analysis_failure", r####"                message = if (complete) "分析已完成，报告已保存在本机。" else "分析未完成，已保存现有线索和未覆盖项。""####, r####"                val failure = JSONObject(loaded.first).optJSONArray("errors")?.optString(0).orEmpty()
                message = if (complete) "分析已完成，报告已保存在本机。"
                    else "分析未完成：${failure.ifBlank { "请查看下方报告中的未覆盖项。" }}""####),
    ("clear_stale_consent", r####"    val selectedReport = reports.firstOrNull { it.id == selectedReportId }"####, r####"    LaunchedEffect(current?.scanId) {
        if (pendingConsent?.scanId != current?.scanId) pendingConsent = null
    }
    val selectedReport = reports.firstOrNull { it.id == selectedReportId }"####),
];

const HELPERS: &str = r####"
internal enum class LegalPrimaryAction { PREPARE, REVIEW_SEND, SETTINGS, WAIT, REOPEN }

internal data class LegalActionState(val action: LegalPrimaryAction, val label: String, val hint: String)

/** The same policy drives the visible action and the click handler. No action auto-sends. */
internal fun legalActionState(
    identityCurrent: Boolean,
    preparing: Boolean,
    running: Boolean,
    cancelling: Boolean,
    hasPreview: Boolean,
    evidenceCount: Int,
    configured: Boolean,
    recipientValid: Boolean,
    snapshotCurrent: Boolean
): LegalActionState = when {
    !identityCurrent -> LegalActionState(LegalPrimaryAction.REOPEN, "关闭分析页", "工作区已变化，请关闭此页后重新进入。")
    cancelling -> LegalActionState(LegalPrimaryAction.WAIT, "正在取消分析", "等待当前请求结束，不会继续发送后续批次。")
    running -> LegalActionState(LegalPrimaryAction.WAIT, "AI 正在分析", "结果返回后会保存报告；可取消后续请求。")
    preparing -> LegalActionState(LegalPrimaryAction.WAIT, "正在整理本机资料", "此阶段不向模型发送资料。")
    !hasPreview -> LegalActionState(LegalPrimaryAction.PREPARE, "整理资料并继续", "先在本机整理任务、笔记、知识页和账目，再确认是否发送。")
    !snapshotCurrent -> LegalActionState(LegalPrimaryAction.PREPARE, "重新整理资料", "资料已经更新，旧预览不能发送。")
    evidenceCount <= 0 -> LegalActionState(LegalPrimaryAction.PREPARE, "重新整理资料", "没有可分析的资料。请核对下方未覆盖项，录入资料或解锁笔记后重试。")
    !configured || !recipientValid -> LegalActionState(LegalPrimaryAction.SETTINGS, "配置 AI 后继续", "密钥、模型或接口地址不完整，请检查 AI 设置。")
    else -> LegalActionState(LegalPrimaryAction.REVIEW_SEND, "发送并开始分析", "已有 $evidenceCount 条可读资料，点击后核对接收方并确认发送。")
}

/** Consent belongs to exactly one scan and one provider configuration; never log its key. */
internal class LegalSendConsent(
    val scanId: String,
    val baseUrl: String,
    val model: String,
    private val apiKey: String
) {
    private var consumed = false

    @Synchronized fun consume(scanId: String, baseUrl: String, model: String, apiKey: String, ready: Boolean): Boolean {
        if (consumed || !ready || this.scanId != scanId || this.baseUrl != baseUrl ||
            this.model != model || this.apiKey != apiKey) return false
        consumed = true
        return true
    }
}
"####;

const TESTS: &str = r####"

class LegalAnalysisWorkflowTest {
    private fun state(
        identity: Boolean = true, preparing: Boolean = false, running: Boolean = false,
        cancelling: Boolean = false, preview: Boolean = false, count: Int = 1,
        configured: Boolean = true, recipient: Boolean = true, snapshot: Boolean = true
    ) = legalActionState(identity, preparing, running, cancelling, preview, count, configured, recipient, snapshot)

    @Test fun entryAlwaysOffersLocalPreparationEvenWithoutAiConfiguration() {
        assertTrue(state().action == LegalPrimaryAction.PREPARE)
        assertTrue(state(configured = false, recipient = false).action == LegalPrimaryAction.PREPARE)
    }
    @Test fun readablePreparedInputOffersARealSendAction() {
        val ready = state(preview = true)
        assertTrue(ready.action == LegalPrimaryAction.REVIEW_SEND)
        assertTrue(ready.label.contains("发送"))
        assertTrue(ready.hint.contains("1 条"))
    }
    @Test fun emptyPreviewCannotSendOrPretendToBeSafe() {
        for (count in listOf(0, -1)) {
            val empty = state(preview = true, count = count)
            assertTrue(empty.action == LegalPrimaryAction.PREPARE)
            assertTrue(empty.hint.contains("没有可分析"))
        }
    }
    @Test fun invalidConfigurationHasAnActionableSettingsButton() {
        assertTrue(state(preview = true, configured = false).action == LegalPrimaryAction.SETTINGS)
        assertTrue(state(preview = true, recipient = false).action == LegalPrimaryAction.SETTINGS)
    }
    @Test fun changedSnapshotRequiresPreparationInsteadOfSilentDisabledSend() {
        assertTrue(state(preview = true, snapshot = false).action == LegalPrimaryAction.PREPARE)
        assertTrue(state(preview = true, snapshot = false).hint.contains("更新"))
    }
    @Test fun changedWorkspaceCannotPrepareOrSend() {
        assertTrue(state(identity = false).action == LegalPrimaryAction.REOPEN)
        assertTrue(state(identity = false, preview = true).action == LegalPrimaryAction.REOPEN)
    }
    @Test fun preparationAnalysisAndCancellationHaveDistinctBusyFeedback() {
        val states = listOf(state(preparing = true), state(running = true), state(cancelling = true))
        assertTrue(states.all { it.action == LegalPrimaryAction.WAIT })
        assertTrue(states.map { it.label }.distinct().size == 3)
    }
    @Test fun confirmingAnUnchangedReadyPreviewConsumesConsentExactlyOnce() {
        val consent = LegalSendConsent("s1", "https://example.test/v1", "model", "test-key")
        assertTrue(consent.consume("s1", "https://example.test/v1", "model", "test-key", true))
        assertFalse(consent.consume("s1", "https://example.test/v1", "model", "test-key", true))
    }
    @Test fun changedScanEndpointModelOrKeyCannotReuseConsent() {
        for (change in listOf(0, 1, 2, 3)) {
            val fields = mutableListOf("s1", "https://example.test/v1", "model", "test-key")
            val consent = LegalSendConsent(fields[0], fields[1], fields[2], fields[3])
            fields[change] += "-changed"
            assertFalse(consent.consume(fields[0], fields[1], fields[2], fields[3], true))
        }
    }
    @Test fun busyOrStalePreviewCannotBeConfirmed() {
        val consent = LegalSendConsent("s1", "https://example.test/v1", "model", "test-key")
        assertFalse(consent.consume("s1", "https://example.test/v1", "model", "test-key", false))
    }
    @Test fun openingAndPreparingDoNotCallTransportAndConfirmationCallsItOnce() {
        var calls = 0
        assertTrue(state().action == LegalPrimaryAction.PREPARE)
        assertTrue(state(preparing = true).action == LegalPrimaryAction.WAIT)
        assertTrue(state(preview = true).action == LegalPrimaryAction.REVIEW_SEND)
        val consent = LegalSendConsent("s1", "https://example.test/v1", "model", "test-key")
        assertTrue(calls == 0)
        repeat(2) {
            if (consent.consume("s1", "https://example.test/v1", "model", "test-key", true)) calls++
        }
        assertTrue(calls == 1)
    }
    @Test fun endingFailedOrCancelledWorkOffersRetry() {
        assertTrue(state(running = true).action == LegalPrimaryAction.WAIT)
        assertTrue(state(cancelling = true).action == LegalPrimaryAction.WAIT)
        assertTrue(state().action == LegalPrimaryAction.PREPARE)
    }
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_screen_has_pinned_actions_and_only_one_confirmed_send() {
        let rendered = render(LEGAL_PATH, crate::legal_risk_ui_source::CONTENTS).unwrap();
        let action = rendered.find("testTag(\"legal_primary_action\")").unwrap();
        let list = rendered.find("        LazyColumn(\n            modifier = Modifier.weight(1f)").unwrap();
        assert!(action < list);
        assert!(!rendered.contains("Button(onClick = ::send"));
        assert!(!rendered.contains("Button(onClick = ::prepare"));
        assert_eq!(rendered.matches("                            send()\n").count(), 1);
        assert_eq!(rendered.matches("NativeOptimizerBridge.runLegalScan(").count(), 1);
        assert!(rendered.contains("if (consent.consume("));
        assert!(rendered.contains("if (requests.finish(request))"));
    }

    #[test]
    fn entry_is_full_width_and_separate_from_the_home_navigation() {
        let mut fixture = String::from("package com.ofairyo.gridtimer.ui\n");
        for &(_, before, _) in GRID_REPLACEMENTS {
            fixture.push_str(before);
            fixture.push('\n');
        }
        let rendered = render(GRID_PATH, &fixture).unwrap();
        assert!(rendered.contains("testTag(\"legal_open_analysis\")"));
        assert!(rendered.contains("usePlatformDefaultWidth = false"));
        assert!(rendered.contains("SecureFlagPolicy.SecureOn"));
        assert!(!rendered.contains("发送前查看范围并确认"));
        assert!(!rendered.contains("runLegalScan"));
    }

    #[test]
    fn template_drift_fails_generation_instead_of_silently_losing_an_action() {
        assert!(render(GRID_PATH, "package com.ofairyo.gridtimer.ui\n").is_err());
        let duplicate = format!("{}{}", crate::legal_risk_ui_source::CONTENTS, crate::legal_risk_ui_source::CONTENTS);
        assert!(render(LEGAL_PATH, &duplicate).is_err());
    }

    #[test]
    fn tests_are_emitted_and_unrelated_sources_are_unchanged() {
        let tests = render(TEST_PATH, crate::legal_risk_ui_source::TEST_CONTENTS).unwrap();
        assert!(tests.contains("class LegalAnalysisWorkflowTest"));
        assert!(tests.contains("class LegalSendReadyTest"));
        assert_eq!(render("Other.kt", "untouched").unwrap(), "untouched");
        let generator = include_str!("../bin/gridtimer_sourcegen.rs");
        assert!(generator.contains("android_legal_workflow::render(relative_path, contents)"));
    }
}
