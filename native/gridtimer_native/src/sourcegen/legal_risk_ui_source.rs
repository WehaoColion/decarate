// v2.23.1.1 - Explain account report sync and retain evidence excerpts across devices.
// v2.23 - Generate the Android legal-clue review page from Rust-owned source.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/LegalRiskScreen.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/LegalSendReadyTest.kt";
pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LegalSendReadyTest {
    private fun ready(
        hasPreview: Boolean = true,
        evidenceCount: Int = 1,
        apiConfigured: Boolean = true,
        modelConfigured: Boolean = true,
        recipientValid: Boolean = true,
        identityCurrent: Boolean = true,
        busy: Boolean = false
    ) = legalSendReady(
        hasPreview, evidenceCount, apiConfigured, modelConfigured,
        recipientValid, identityCurrent, busy
    )

    @Test fun emptyScanCannotSend() {
        assertFalse(ready(hasPreview = false))
        assertFalse(ready(evidenceCount = 0))
    }

    @Test fun missingAiConfigurationCannotSend() {
        assertFalse(ready(apiConfigured = false))
        assertFalse(ready(modelConfigured = false))
        assertFalse(ready(recipientValid = false))
    }

    @Test fun staleIdentityOrActiveRequestCannotSend() {
        assertFalse(ready(identityCurrent = false))
        assertFalse(ready(busy = true))
    }

    @Test fun completeCurrentPreviewCanSend() {
        assertTrue(ready())
    }
}
"####;

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.ofairyo.gridtimer.core.NativeOptimizerBridge
import com.ofairyo.gridtimer.data.AppData
import com.ofairyo.gridtimer.data.LegalLocalStore
import com.ofairyo.gridtimer.data.NoteEntry
import com.ofairyo.gridtimer.data.NoteMediaStore
import com.ofairyo.gridtimer.data.StoredLegalReport
import com.ofairyo.gridtimer.data.SyncAccountSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

private val legalInputJson = Json { encodeDefaults = true }

private data class LegalPreview(
    val scanId: String,
    val manifest: JSONObject,
    val evidence: List<JSONObject>,
    val capturedData: AppData
)

internal fun legalSendReady(
    hasPreview: Boolean,
    evidenceCount: Int,
    apiConfigured: Boolean,
    modelConfigured: Boolean,
    recipientValid: Boolean,
    identityCurrent: Boolean,
    busy: Boolean
): Boolean = hasPreview && evidenceCount > 0 && apiConfigured && modelConfigured &&
    recipientValid && identityCurrent && !busy

/** Separate from the finance overview pager. No upload starts before the send button. */
@Composable
fun LegalRiskScreen(
    appData: AppData,
    workspaceKey: String,
    syncSession: SyncAccountSession,
    viewModel: TimerViewModel,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    onOpenEvidence: ((String) -> Unit)? = null,
    onOpenAiSettings: () -> Unit = {}
) {
    EncryptedNoteSecureWindowEffect(true)
    val context = LocalContext.current
    val appContext = context.applicationContext
    val scope = rememberCoroutineScope()
    val accountIdentity = listOf(
        syncSession.loggedIn.toString(), syncSession.serverInstanceId,
        syncSession.accountNamespace, syncSession.userId,
        syncSession.email.trim().lowercase(), syncSession.serverUrl.trim()
    ).joinToString("\u0000")
    val latestAccountIdentity by rememberUpdatedState(accountIdentity)
    val latestAppData by rememberUpdatedState(appData)
    val generation = remember(workspaceKey, accountIdentity) { AtomicInteger(0) }
    val preparingScanId = remember(workspaceKey, accountIdentity) { AtomicReference<String?>(null) }
    val unlocked = remember(workspaceKey, accountIdentity) { mutableStateMapOf<String, NoteEntry>() }
    val reports = remember(workspaceKey, accountIdentity) { mutableStateListOf<StoredLegalReport>() }
    var preview by remember(workspaceKey, accountIdentity) { mutableStateOf<LegalPreview?>(null) }
    var preparingData by remember(workspaceKey, accountIdentity) { mutableStateOf<AppData?>(null) }
    var preparing by remember(workspaceKey, accountIdentity) { mutableStateOf(false) }
    var running by remember(workspaceKey, accountIdentity) { mutableStateOf(false) }
    var cancelling by remember(workspaceKey, accountIdentity) { mutableStateOf(false) }
    var message by remember(workspaceKey, accountIdentity) { mutableStateOf("") }
    var selectedReportId by remember(workspaceKey, accountIdentity) { mutableStateOf<String?>(null) }
    var pendingSyncCount by remember(workspaceKey, accountIdentity) { mutableStateOf(0) }
    var deleteReportId by remember(workspaceKey, accountIdentity) { mutableStateOf<String?>(null) }
    var showContent by remember(workspaceKey, accountIdentity) { mutableStateOf(false) }
    var openedEvidence by remember(workspaceKey, accountIdentity) { mutableStateOf<JSONObject?>(null) }
    var evidenceLoading by remember(workspaceKey, accountIdentity) { mutableStateOf(false) }
    var noteToUnlock by remember(workspaceKey, accountIdentity) { mutableStateOf<NoteEntry?>(null) }
    var password by remember(workspaceKey, accountIdentity) { mutableStateOf("") }
    var unlockingId by remember(workspaceKey, accountIdentity) { mutableStateOf<String?>(null) }

    fun isCurrent(request: Int): Boolean =
        generation.get() == request &&
            latestAccountIdentity == accountIdentity &&
            viewModel.currentWorkspaceKey() == workspaceKey

    fun cancelCurrentScan() {
        preview?.scanId?.let(NativeOptimizerBridge::cancelLegalScan)
        cancelling = running
    }

    DisposableEffect(workspaceKey, accountIdentity) {
        onDispose {
            generation.incrementAndGet()
            preview?.scanId?.let(NativeOptimizerBridge::cancelLegalScan)
            if (!running) preview?.scanId?.let(NativeOptimizerBridge::closeLegalScan)
            preparingScanId.getAndSet(null)?.let(NativeOptimizerBridge::closeLegalScan)
            unlocked.keys.toList().forEach { noteId ->
                viewModel.lockEncryptedNote(noteId, workspaceKey)
            }
            unlocked.clear()
        }
    }

    LaunchedEffect(workspaceKey, accountIdentity, syncSession.lastMessage) {
        runCatching { withContext(Dispatchers.IO) {
            LegalLocalStore.listReports(appContext, workspaceKey) to
                LegalLocalStore.pendingSyncCount(appContext, workspaceKey)
        } }
            .onSuccess { (loaded, pending) -> if (viewModel.currentWorkspaceKey() == workspaceKey &&
                latestAccountIdentity == accountIdentity) {
                reports.clear(); reports.addAll(loaded)
                pendingSyncCount = pending
            } }
            .onFailure { message = "本机报告无法读取：${it.message.orEmpty()}" }
    }

    LaunchedEffect(appData, workspaceKey, accountIdentity) {
        if (preparingData != null && preparingData != appData) {
            generation.incrementAndGet()
            preparingScanId.getAndSet(null)?.let(NativeOptimizerBridge::closeLegalScan)
            preparing = false
            preparingData = null
            message = "资料已更新，请重新准备扫描。"
        }
        val captured = preview?.capturedData
        if (captured != null && captured != appData) {
            generation.incrementAndGet()
            cancelCurrentScan()
            if (!running) preview?.scanId?.let(NativeOptimizerBridge::closeLegalScan)
            preview = null
            message = "资料已更新，请重新准备扫描。"
        }
    }

    fun prepare() {
        if (preparing || running || viewModel.currentWorkspaceKey() != workspaceKey) return
        val request = generation.incrementAndGet()
        val captured = appData
        val unlockedSnapshot = unlocked.toMap()
        preview?.scanId?.let(NativeOptimizerBridge::closeLegalScan)
        preview = null
        message = ""
        preparing = true
        preparingData = captured
        scope.launch {
            try {
                val result = withContext(Dispatchers.IO) {
                    val rawData = legalInputJson.encodeToString(captured)
                    val unlockedJson = unlockedNotesJson(unlockedSnapshot)
                    val attachmentJson = attachmentDescriptorsJson(
                        appContext, workspaceKey, captured, unlockedSnapshot
                    )
                    var scanId: String? = null
                    try {
                        val prepared = NativeOptimizerBridge.prepareLegalScan(
                            rawData,
                            workspaceKey,
                            System.currentTimeMillis(),
                            unlockedJson,
                            attachmentJson
                        ) ?: error("无法建立扫描快照")
                        val envelope = JSONObject(prepared)
                        val preparationError = envelope.optString("error")
                        if (preparationError.isNotBlank()) error(preparationError)
                        scanId = envelope.getString("scanId")
                        preparingScanId.set(scanId)
                        val manifest = envelope.getJSONObject("manifest")
                        val evidenceJson = NativeOptimizerBridge.legalScanEvidenceIndexJson(scanId)
                            ?: error("无法读取发送内容预览")
                        val evidence = JSONArray(evidenceJson).let { array ->
                            (0 until array.length()).map { array.getJSONObject(it) }
                        }
                        if (generation.get() != request) error("扫描已取消")
                        LegalPreview(scanId, manifest, evidence, captured)
                    } catch (error: Throwable) {
                        scanId?.let(NativeOptimizerBridge::closeLegalScan)
                        preparingScanId.compareAndSet(scanId, null)
                        throw error
                    }
                }
                if (!isCurrent(request) || latestAppData != captured) {
                    NativeOptimizerBridge.closeLegalScan(result.scanId)
                    preparingScanId.compareAndSet(result.scanId, null)
                    return@launch
                }
                preview = result
                preparingScanId.compareAndSet(result.scanId, null)
                message = if (result.manifest.optInt("evidenceCount") == 0) {
                    "本次没有可读取的资料。未覆盖项可在下方查看。"
                } else {
                    "请核对接收方和内容，再决定是否发送。"
                }
                // Unlocked note plaintext is already inside the one-shot native scan.
                unlocked.keys.toList().forEach { viewModel.lockEncryptedNote(it, workspaceKey) }
                unlocked.clear()
            } catch (error: Throwable) {
                if (isCurrent(request)) message = "准备失败：${error.message.orEmpty()}"
            } finally {
                if (isCurrent(request)) {
                    preparing = false
                    preparingData = null
                }
            }
        }
    }

    fun send() {
        val current = preview
        val recipient = legalRecipient(syncSession.aiBaseUrl)
        if (!legalSendReady(
            hasPreview = current != null,
            evidenceCount = current?.manifest?.optInt("evidenceCount") ?: 0,
            apiConfigured = AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel).testable,
            modelConfigured = syncSession.aiModel.isNotBlank(),
            recipientValid = recipient != null,
            identityCurrent = latestAccountIdentity == accountIdentity &&
                current?.capturedData == appData && viewModel.currentWorkspaceKey() == workspaceKey,
            busy = preparing || running || cancelling
        )) return
        val currentScan = current ?: return
        val request = generation.get()
        running = true
        cancelling = false
        message = "正在分批分析…"
        scope.launch {
            try {
                // Closing this page cancels future native requests. The incomplete result is
                // still saved under the original workspace once the active request returns.
                val loaded = withContext(NonCancellable + Dispatchers.IO) {
                    val reportJson = runCatching {
                        NativeOptimizerBridge.runLegalScan(
                            currentScan.scanId,
                            syncSession.aiApiKey.trim(),
                            syncSession.aiBaseUrl,
                            syncSession.aiModel.trim()
                        ) ?: error("接口未返回报告，请检查结构化输出或图片能力")
                    }.getOrElse { error ->
                        incompleteLegalReport(workspaceKey, currentScan.manifest, error.message.orEmpty())
                    }
                    val id = LegalLocalStore.saveReport(appContext, workspaceKey, reportJson)
                    Triple(reportJson, id, LegalLocalStore.listReports(appContext, workspaceKey))
                }
                if (!isCurrent(request)) return@launch
                reports.clear(); reports.addAll(loaded.third)
                selectedReportId = loaded.second
                pendingSyncCount = withContext(Dispatchers.IO) {
                    LegalLocalStore.pendingSyncCount(appContext, workspaceKey)
                }
                val complete = JSONObject(loaded.first).optBoolean("completed", false)
                message = if (complete) "分析已完成，报告已保存在本机。" else "分析未完成，已保存现有线索和未覆盖项。"
                if (syncSession.loggedIn) viewModel.syncNow()
                preview = null
            } catch (error: Throwable) {
                if (isCurrent(request)) message = "分析未完成：${error.message.orEmpty()}"
            } finally {
                withContext(NonCancellable + Dispatchers.IO) {
                    NativeOptimizerBridge.closeLegalScan(currentScan.scanId)
                }
                if (isCurrent(request)) {
                    preview = null
                    running = false
                    cancelling = false
                }
            }
        }
    }

    BackHandler {
        if (running) cancelCurrentScan()
        onDismiss()
    }

    val current = preview
    val recipient = legalRecipient(syncSession.aiBaseUrl)
    val sendEnabled = legalSendReady(
        hasPreview = current != null,
        evidenceCount = current?.manifest?.optInt("evidenceCount") ?: 0,
        apiConfigured = AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel).testable,
        modelConfigured = syncSession.aiModel.isNotBlank(),
        recipientValid = recipient != null,
        identityCurrent = latestAccountIdentity == accountIdentity &&
            current?.capturedData == appData && viewModel.currentWorkspaceKey() == workspaceKey,
        busy = preparing || running || cancelling
    )
    val selectedReport = reports.firstOrNull { it.id == selectedReportId }
    val encryptedNotes = remember(appData.notes, workspaceKey) {
        appData.notes.filter { it.encryption != null && it.deletedAtEpochMillis == null }
    }

    Column(modifier = modifier.fillMaxSize()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text("法律风险线索", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            TextButton(onClick = {
                if (running) cancelCurrentScan()
                onDismiss()
            }) { Text("关闭") }
        }
        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
            item {
                LegalSection {
                    Text("按中国大陆法律寻找待核查线索。结论会标出原始记录、缺失事实和法条来源。")
                    Text("资料会先在本机整理；点击发送前可以查看覆盖范围和内容。")
                    val aiConfiguration = AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel)
                    Text(if (aiConfiguration.testable) "AI：${syncSession.aiModel} · ${aiConfiguration.recipientHost}" else "AI 未配置完整，先配置后才能发送分析。", fontWeight = FontWeight.SemiBold)
                    TextButton(onClick = {
                        if (running) cancelCurrentScan()
                        onOpenAiSettings()
                    }, enabled = !running && !preparing) { Text(if (aiConfiguration.testable) "查看 AI 设置" else "配置 DeepSeek / 其他 AI") }
                    Text("使用步骤：准备扫描 → 查看资料与覆盖范围 → 发送并分析 → 查看线索和证据。")
                    if (syncSession.loggedIn) {
                        Text("已登录账户的最终报告会同步到配对设备。同步服务及其备份会保存可由服务读取的报告内容；扫描原文和临时解锁的笔记不会作为报告同步。")
                        if (pendingSyncCount > 0) Text("$pendingSyncCount 项报告或删除记录待同步。")
                    } else Text("访客报告只保存在本机。")
                    if (message.isNotBlank()) Text(message, color = MaterialTheme.colorScheme.primary)
                }
            }
            if (encryptedNotes.isNotEmpty() && current == null && !running) {
                item {
                    LegalSection {
                        Text("加密笔记", fontWeight = FontWeight.SemiBold)
                        Text("需逐条解锁，明文只用于本次扫描。未解锁的笔记会列为未覆盖。")
                    }
                }
                items(encryptedNotes, key = { it.id }) { note ->
                    LegalSection {
                        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                            Text(note.title.ifBlank { "未命名笔记" }, modifier = Modifier.weight(1f))
                            TextButton(onClick = { noteToUnlock = note; password = "" }) {
                                Text(if (unlocked.containsKey(note.id)) "已解锁" else "解锁")
                            }
                        }
                    }
                }
            }
            if (current == null && !running) {
                item {
                    Button(onClick = ::prepare, enabled = !preparing, modifier = Modifier.fillMaxWidth()) {
                        if (preparing) CircularProgressIndicator(modifier = Modifier.height(18.dp))
                        else Text("准备扫描")
                    }
                }
            }
            if (current != null) {
                item {
                    LegalSection {
                        Text("发送前核对", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                        Text("接收域名：${recipient ?: "地址无效"}")
                        Text("模型：${syncSession.aiModel.ifBlank { "未配置" }}")
                        Text("上传量：${formatLegalBytes(current.manifest.optLong("uploadBytes"))}")
                        Text("预计约 ${current.manifest.optInt("estimatedCalls")} 次（图片转写会改变批次数）")
                        Text(if (recipient == "api.openai.com") "本次请求设置 store=false；输入仍按服务方数据政策处理。" else "自定义服务的数据保存规则请向服务提供者核对。")
                        Text("可能包含他人个人信息，请核对发送范围。")
                        TextButton(onClick = { showContent = true }) { Text("查看发送内容") }
                    }
                }
                item { LegalManifestView(current.manifest) }
                item {
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        Button(onClick = ::send, enabled = sendEnabled, modifier = Modifier.weight(1f)) {
                            Text("发送并分析")
                        }
                        OutlinedButton(onClick = {
                            NativeOptimizerBridge.closeLegalScan(current.scanId)
                            preview = null
                            message = "已撤销本次预览。"
                        }) { Text("撤销") }
                    }
                    if (!sendEnabled && syncSession.aiApiKey.isBlank()) {
                        Text("请先在“我的”中配置 AI 密钥。")
                    }
                }
            }
            if (running) {
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
            if (reports.isNotEmpty()) {
                item { Text("法律风险线索报告", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold) }
                items(reports, key = { it.id }) { report ->
                    LegalSection {
                        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                            Text(formatLegalTime(report.createdAtEpochMillis), fontWeight = FontWeight.SemiBold)
                            TextButton(onClick = { deleteReportId = report.id }) { Text("删除") }
                        }
                        val parsed = remember(report.reportJson) { runCatching { JSONObject(report.reportJson) }.getOrNull() }
                        Text(if (parsed?.optBoolean("completed") == true) "已完成" else "未完成")
                        Text("线索 ${parsed?.optJSONArray("findings")?.length() ?: 0} 条")
                        TextButton(onClick = { selectedReportId = if (selectedReportId == report.id) null else report.id }) {
                            Text(if (selectedReportId == report.id) "收起报告" else "查看报告")
                        }
                    }
                }
            }
            if (selectedReport != null) {
                item {
                    val sameSource = runCatching {
                        JSONObject(selectedReport.reportJson).optString("workspaceId") == workspaceKey
                    }.getOrDefault(false)
                    LegalReportView(selectedReport, onOpenEvidence,
                        sourceAvailable = sameSource,
                        canOpenEvidence = { path -> sameSource && legalEvidencePathAvailable(path, appData) })
                }
            }
        }
    }

    if (showContent && current != null) {
        AlertDialog(
            onDismissRequest = { showContent = false; openedEvidence = null },
            title = { Text("本次发送内容") },
            text = {
                LazyColumn(modifier = Modifier.heightIn(max = 480.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    items(current.evidence) { item ->
                        Column {
                            Text("${item.optString("id")} · ${item.optString("title")}", fontWeight = FontWeight.SemiBold)
                            Text(item.optString("category"))
                            if (item.optBoolean("hasImage")) Text("图片将作为视觉输入发送")
                            TextButton(onClick = {
                                val request = generation.get()
                                evidenceLoading = true
                                scope.launch {
                                    val loaded = withContext(Dispatchers.IO) {
                                        NativeOptimizerBridge.legalScanEvidenceJson(current.scanId, item.optString("id"))
                                    }
                                    if (isCurrent(request) && preview?.scanId == current.scanId) {
                                        openedEvidence = loaded?.let { raw -> runCatching { JSONObject(raw) }.getOrNull() }
                                        evidenceLoading = false
                                    }
                                }
                            }) { Text("查看内容") }
                        }
                    }
                    if (evidenceLoading) item { CircularProgressIndicator() }
                    openedEvidence?.let { opened ->
                        item {
                            Text("${opened.optString("id")} · ${opened.optString("title")}", fontWeight = FontWeight.SemiBold)
                            Text(opened.optString("text").ifBlank {
                                if (opened.optBoolean("hasImage")) "图片将作为视觉输入发送" else "没有可显示的文字"
                            })
                            val sourcePath = opened.optString("sourcePath")
                            if (opened.optBoolean("hasImage") && sourcePath.isNotBlank() && onOpenEvidence != null) {
                                TextButton(onClick = { onOpenEvidence(sourcePath) }) { Text("查看原附件") }
                            }
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { showContent = false; openedEvidence = null }) { Text("关闭") } }
        )
    }

    val pendingNote = noteToUnlock
    if (pendingNote != null) {
        AlertDialog(
            onDismissRequest = { if (unlockingId == null) { noteToUnlock = null; password = "" } },
            title = { Text("解锁 ${pendingNote.title.ifBlank { "笔记" }}") },
            text = {
                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it },
                    label = { Text("密码") },
                    visualTransformation = PasswordVisualTransformation(),
                    singleLine = true
                )
            },
            confirmButton = {
                TextButton(
                    enabled = password.isNotBlank() && unlockingId == null,
                    onClick = {
                        val request = generation.get()
                        val secret = password
                        password = ""
                        unlockingId = pendingNote.id
                        viewModel.unlockEncryptedNote(pendingNote, secret, workspaceKey) { note, failure ->
                            if (isCurrent(request) && note?.id == pendingNote.id) {
                                unlocked[note.id] = note
                                noteToUnlock = null
                                message = "已解锁：${note.title.ifBlank { "笔记" }}"
                            } else if (isCurrent(request)) {
                                message = failure.ifBlank { "解锁失败" }
                            } else if (note != null) {
                                viewModel.lockEncryptedNote(note.id, workspaceKey)
                            }
                            unlockingId = null
                        }
                    }
                ) { Text(if (unlockingId == null) "解锁" else "解锁中") }
            },
            dismissButton = {
                TextButton(onClick = { noteToUnlock = null; password = "" }, enabled = unlockingId == null) {
                    Text("取消")
                }
            }
        )
    }

    val pendingDelete = deleteReportId
    if (pendingDelete != null) {
        AlertDialog(
            onDismissRequest = { deleteReportId = null },
            title = { Text("删除报告") },
            text = { Text(if (syncSession.loggedIn)
                "删除会同步到同一账户的其他设备，无法从本机恢复。"
                else "此报告删除后无法从本机恢复。") },
            confirmButton = {
                TextButton(onClick = {
                    deleteReportId = null
                    scope.launch {
                        runCatching { withContext(Dispatchers.IO) {
                            LegalLocalStore.deleteReport(appContext, workspaceKey, pendingDelete)
                            LegalLocalStore.listReports(appContext, workspaceKey) to
                                LegalLocalStore.pendingSyncCount(appContext, workspaceKey)
                        } }.onSuccess { (loaded, pending) ->
                            if (viewModel.currentWorkspaceKey() == workspaceKey) {
                                reports.clear(); reports.addAll(loaded)
                                if (selectedReportId == pendingDelete) selectedReportId = null
                                pendingSyncCount = pending
                                if (syncSession.loggedIn) viewModel.syncNow()
                            }
                        }.onFailure { message = "删除失败：${it.message.orEmpty()}" }
                    }
                }) { Text("删除") }
            },
            dismissButton = { TextButton(onClick = { deleteReportId = null }) { Text("保留") } }
        )
    }
}

@Composable
private fun LegalSection(content: @Composable ColumnScope.() -> Unit) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(18.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        tonalElevation = 2.dp
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            content = content
        )
    }
}

@Composable
private fun LegalManifestView(manifest: JSONObject) {
    LegalSection {
        Text("覆盖范围", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        val coverage = manifest.optJSONObject("coverage") ?: JSONObject()
        val names = mutableListOf<String>()
        val keys = coverage.keys()
        while (keys.hasNext()) names += keys.next()
        names.sorted().forEach { name ->
            val item = coverage.optJSONObject(name) ?: JSONObject()
            Text("$name：纳入 ${item.optInt("included")}，未覆盖 ${item.optInt("unavailable")}，已删除 ${item.optInt("excludedDeleted")}")
        }
        val omissions = manifest.optJSONArray("omissions") ?: JSONArray()
        if (omissions.length() > 0) {
            Text("未覆盖", fontWeight = FontWeight.SemiBold)
            for (index in 0 until omissions.length()) {
                val item = omissions.optJSONObject(index) ?: continue
                Text("${item.optString("sourcePath")}：${item.optString("reason")}")
            }
        }
    }
}

private fun legalEvidencePathAvailable(path: String, appData: AppData): Boolean {
    val parts = path.split('/')
    return when (parts.firstOrNull()) {
        "slot" -> parts.getOrNull(1)?.toIntOrNull()?.let { id -> appData.slots.any { it.id == id } } == true
        "session" -> appData.sessions.any { it.id == parts.getOrNull(1) }
        "archivedTask" -> appData.archivedTasks.any { it.id == parts.getOrNull(1) }
        "finance" -> when (parts.getOrNull(1)) {
            "day" -> parts.getOrNull(2)?.let { appData.financeProfile.dailyLedgers.containsKey(it) } == true
            "month" -> parts.getOrNull(2)?.let { appData.financeProfile.monthlySnapshots.containsKey(it) } == true
            else -> false
        }
        "note" -> {
            val note = appData.notes.firstOrNull {
                it.id == parts.getOrNull(1) && it.deletedAtEpochMillis == null
            } ?: return false
            val revision = if (parts.getOrNull(2) == "revisions") {
                note.revisions.firstOrNull { it.id == parts.getOrNull(3) }
            } else null
            val version = if (parts.getOrNull(2) == "versions") {
                note.versions.firstOrNull { it.id == parts.getOrNull(3) && it.deletedAtEpochMillis == null }
            } else null
            if (parts.getOrNull(2) == "revisions" && revision == null) return false
            if (parts.getOrNull(2) == "versions" && version == null) return false
            val attachmentId = parts.lastOrNull()?.takeIf { parts.contains("attachment") }
            attachmentId == null || (
                note.attachments.any { it.id == attachmentId } ||
                    note.revisions.any { snapshot -> snapshot.attachments.any { it.id == attachmentId } } ||
                    note.versions.any { snapshot -> snapshot.attachments.any { it.id == attachmentId } }
            )
        }
        else -> false
    }
}

@Composable
private fun LegalReportView(report: StoredLegalReport, onOpenEvidence: ((String) -> Unit)?,
    sourceAvailable: Boolean, canOpenEvidence: (String) -> Boolean) {
    val context = LocalContext.current
    val parsed = remember(report.reportJson) { runCatching { JSONObject(report.reportJson) }.getOrNull() }
    if (parsed == null) {
        LegalSection { Text("报告内容无法读取。") }
        return
    }
    val findings = parsed.optJSONArray("findings") ?: JSONArray()
    LegalSection {
        Text(if (parsed.optBoolean("completed")) "已完成的线索报告" else "未完成的线索报告",
            style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
        if (!sourceAvailable) Text("此报告来自其他设备，本机没有对应原记录；仍可查看报告中的原文摘录。")
        Text("扫描时间：${formatLegalTime(parsed.optLong("capturedAtEpochMillis"))}")
        if (findings.length() == 0) Text(
            if (parsed.optBoolean("completed")) "在本次覆盖范围内，未发现可核实的线索。"
            else "分析未完成，目前没有可展示的已核实线索。"
        )
        val errors = parsed.optJSONArray("errors") ?: JSONArray()
        for (index in 0 until errors.length()) Text(errors.optString(index))
    }
    LegalManifestView(parsed.optJSONObject("manifest") ?: JSONObject())
    for (index in 0 until findings.length()) {
        val finding = findings.optJSONObject(index) ?: continue
        LegalSection {
            Text(finding.optString("title"), style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold)
            Text(finding.optString("area"))
            Text("待核查描述：${finding.optString("fact")}")
            val at = finding.optLong("eventAtEpochMillis")
            if (at > 0L) Text("事发时间：${formatLegalTime(at)}")
            val evidence = finding.optJSONArray("evidence") ?: JSONArray()
            for (itemIndex in 0 until evidence.length()) {
                val item = evidence.optJSONObject(itemIndex) ?: continue
                Text("${item.optString("evidenceId")} · ${item.optString("title")}")
                Text("“${item.optString("quote")}”")
                val path = item.optString("sourcePath")
                if (path.isNotBlank()) {
                    Text("记录路径：$path", style = MaterialTheme.typography.labelSmall)
                    if (onOpenEvidence != null && canOpenEvidence(path)) {
                        TextButton(onClick = { onOpenEvidence(path) }) { Text("打开所属记录") }
                    } else Text("本机没有可回跳的原记录，请核对上方摘录。")
                }
            }
            Text("还需核对：${finding.optString("missingFacts")}")
            Text("建议：${finding.optString("recommendation")}")
            val laws = finding.optJSONArray("laws") ?: JSONArray()
            for (lawIndex in 0 until laws.length()) {
                val law = laws.optJSONObject(lawIndex) ?: continue
                Text("${law.optString("title")} · ${law.optString("version")}", fontWeight = FontWeight.SemiBold)
                val articleNumber = law.optString("articleNumber").trim()
                val articleText = law.optString("articleText").trim()
                if (articleNumber.isNotEmpty() && articleText.isNotEmpty()) {
                    Text("第${articleNumber}条 · 原文摘录：$articleText")
                    Text("摘录核对日期：${law.optString("checkedOn")}")
                } else {
                    Text("此报告未保存法条摘录，请打开官方来源核对。")
                }
                val url = law.optString("url")
                if (url.startsWith("https://")) {
                    TextButton(onClick = {
                        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
                    }) { Text("打开官方来源") }
                }
            }
        }
    }
}

private fun unlockedNotesJson(notes: Map<String, NoteEntry>): String {
    val array = JSONArray()
    notes.values.forEach { note ->
        val data = JSONObject(legalInputJson.encodeToString(note))
        data.put("noteId", note.id)
        array.put(data)
    }
    return array.toString()
}

/** Existing media validation is repeated in Rust before a file enters an AI request. */
private fun attachmentDescriptorsJson(
    context: android.content.Context,
    workspaceKey: String,
    appData: AppData,
    unlockedNotes: Map<String, NoteEntry>
): String {
    val descriptors = JSONArray()
    val seen = HashSet<String>()
    for (stored in appData.notes) {
        if (stored.deletedAtEpochMillis != null) continue
        val note = if (stored.encryption == null) stored else unlockedNotes[stored.id] ?: continue
        val attachments = note.attachments +
            note.revisions.flatMap { it.attachments } +
            note.versions.filter { it.deletedAtEpochMillis == null }.flatMap { it.attachments }
        for (attachment in attachments) {
            val key = "${note.id}\u0000${attachment.id}\u0000${attachment.sha256.lowercase()}"
            if (!seen.add(key) || attachment.sha256.isBlank()) continue
            val file = NoteMediaStore.verifiedAttachmentFile(context, workspaceKey, attachment) ?: continue
            descriptors.put(JSONObject()
                .put("noteId", note.id)
                .put("attachmentId", attachment.id)
                .put("sha256", attachment.sha256)
                .put("path", file.absolutePath)
                .put("mimeType", attachment.mimeType)
                .put("sizeBytes", attachment.sizeBytes))
        }
    }
    return descriptors.toString()
}

private fun legalRecipient(baseUrl: String): String? = runCatching {
    val uri = URI(baseUrl.trim())
    val host = uri.host?.lowercase()?.takeIf { it.isNotBlank() } ?: return@runCatching null
    val scheme = uri.scheme?.lowercase()
    if (uri.userInfo != null || uri.fragment != null ||
        (scheme != "https" && !(scheme == "http" && host in setOf("127.0.0.1", "localhost")))) {
        return@runCatching null
    }
    if (uri.port > 0) "$host:${uri.port}" else host
}.getOrNull()

private fun formatLegalBytes(bytes: Long): String = when {
    bytes < 1024L -> "$bytes B"
    bytes < 1_048_576L -> "%.1f KiB".format(bytes / 1024.0)
    else -> "%.1f MiB".format(bytes / 1_048_576.0)
}

private fun formatLegalTime(time: Long): String = if (time > 0L) {
    DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(time))
} else "时间未记录"

private fun incompleteLegalReport(workspaceKey: String, manifest: JSONObject, reason: String): String =
    JSONObject()
        .put("workspaceId", workspaceKey)
        .put("capturedAtEpochMillis", manifest.optLong("capturedAtEpochMillis"))
        .put("completed", false)
        .put("findings", JSONArray())
        .put("manifest", manifest)
        .put("errors", JSONArray().put(reason.ifBlank { "分析请求未完成" }))
        .toString()
"####;
