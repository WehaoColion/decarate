// v2.23.2.6 - Display AI answers with offline Markdown and formula layout.
// v2.23.2.5 - Separate direct AI questions from source-grounded knowledge answers.
// v2.23.2.4 - Restore question keyboard only after the editor and dialog are ready.
// v2.23.2.3 - Make Android AI actions discoverable and bind answers to their request.
pub const PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeAiRequestBoundary.kt";
pub const TEST_PATH: &str = "com/ofairyo/gridtimer/ui/KnowledgeAiRequestBoundaryTest.kt";

pub const CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui

internal data class KnowledgeAiOutcome(val ok: Boolean, val text: String, val message: String)

internal enum class KnowledgeAiMode(val wireValue: String) { DIRECT("direct"), KNOWLEDGE("knowledge") }

internal fun <T> knowledgeAiSourcesForMode(mode: KnowledgeAiMode, loadSources: () -> List<T>): List<T> =
    if (mode == KnowledgeAiMode.DIRECT) emptyList() else loadSources()

internal class KnowledgeAiRequestBoundary(initialMode: KnowledgeAiMode = KnowledgeAiMode.KNOWLEDGE) {
    private var generation = 0L
    private var pending: Long? = null
    @Volatile private var closed = false
    private var mode = initialMode

    @Synchronized fun begin(authorized: Boolean, configured: Boolean, question: String, sourceCount: Int): Long? {
        if (closed || pending != null || !authorized || !configured || question.isBlank() ||
            (mode == KnowledgeAiMode.KNOWLEDGE && sourceCount <= 0) ||
            (mode == KnowledgeAiMode.DIRECT && sourceCount != 0)) return null
        return (++generation).also { pending = it }
    }

    @Synchronized fun changeMode(next: KnowledgeAiMode) {
        if (mode != next) { mode = next; generation++ }
    }
    @Synchronized fun isCurrent(ticket: Long): Boolean = !closed && pending == ticket && generation == ticket
    @Synchronized fun invalidate() { generation++ }
    @Synchronized fun close() { closed = true; generation++ }

    @Synchronized fun finish(ticket: Long, identityCurrent: Boolean, ok: Boolean, output: String, message: String): KnowledgeAiOutcome? {
        if (pending != ticket) return null
        pending = null
        if (closed || generation != ticket || !identityCurrent) return null
        val text = output.trim()
        return if (ok && text.isNotBlank()) KnowledgeAiOutcome(true, text, "")
        else KnowledgeAiOutcome(false, "", if (ok) "模型没有返回可读回答，请重试。" else message.ifBlank { "AI 请求未完成，请检查连接后重试。" })
    }
}

internal class KnowledgeKeyboardRequest {
    private var generation = 0L
    private var pending: Long? = null
    private var closed = false

    fun request(): Long = (++generation).also { if (!closed) pending = it }
    fun cancel() { generation++; pending = null }
    fun close() { closed = true; cancel() }

    fun takeReady(ticket: Long, fieldFocused: Boolean, windowFocused: Boolean, resumed: Boolean): Boolean {
        if (closed || pending != ticket || generation != ticket || !fieldFocused || !windowFocused || !resumed) return false
        pending = null
        return true
    }
}
"####;

pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.ui
import org.junit.Assert.*
import org.junit.Test

class KnowledgeAiRequestBoundaryTest {
    @Test fun noAuthorizationOrInputCannotSend() {
        for (input in listOf(Triple(false, true, 1), Triple(true, false, 1), Triple(true, true, 0))) {
            assertNull(KnowledgeAiRequestBoundary().begin(input.first, input.second, "问题", input.third))
        }
        assertNull(KnowledgeAiRequestBoundary().begin(true, true, "  ", 1))
    }
    @Test fun duplicateClickCannotStartAnotherRequest() {
        val state = KnowledgeAiRequestBoundary()
        assertNotNull(state.begin(true, true, "总结", 2))
        assertNull(state.begin(true, true, "总结", 2))
    }
    @Test fun emptyOrFailedOutputCannotBecomeAnswer() {
        for ((ok, output) in listOf(true to " ", false to "残留文字")) {
            val state = KnowledgeAiRequestBoundary()
            val ticket = state.begin(true, true, "问题", 1)!!
            val result = state.finish(ticket, true, ok, output, "失败")!!
            assertFalse(result.ok); assertEquals("", result.text)
        }
    }
    @Test fun editedQuestionAndCancelledRequestCannotPublish() {
        val state = KnowledgeAiRequestBoundary()
        val ticket = state.begin(true, true, "旧问题", 1)!!
        state.invalidate()
        assertNull(state.begin(true, true, "新问题", 1))
        assertNull(state.finish(ticket, true, true, "旧回答", ""))
        val next = state.begin(true, true, "新问题", 1)!!
        assertEquals("新回答", state.finish(next, true, true, "新回答", "")!!.text)
    }
    @Test fun cancelledQueueCannotStartTransport() {
        val state = KnowledgeAiRequestBoundary()
        val ticket = state.begin(true, true, "问题", 1)!!
        assertTrue(state.isCurrent(ticket))
        state.invalidate()
        var calls = 0
        if (state.isCurrent(ticket)) calls++
        assertEquals(0, calls)
        state.close()
        assertFalse(state.isCurrent(ticket))
    }
    @Test fun closedOrChangedAccountCannotPublish() {
        val closed = KnowledgeAiRequestBoundary()
        val a = closed.begin(true, true, "问题", 1)!!
        closed.close()
        assertNull(closed.finish(a, true, true, "回答", ""))
        val changed = KnowledgeAiRequestBoundary()
        val b = changed.begin(true, true, "问题", 1)!!
        assertNull(changed.finish(b, false, true, "回答", ""))
    }
    @Test fun directModeAllowsNoKnowledgeButStillRequiresAuthorizationAndInput() {
        assertNotNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT).begin(true, true, "一加一等于几", 0))
        for ((authorized, configured, question) in listOf(Triple(false, true, "问题"), Triple(true, false, "问题"), Triple(true, true, "  "))) {
            assertNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT).begin(authorized, configured, question, 0))
        }
        assertNull(KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT).begin(true, true, "问题", 1))
    }
    @Test fun directModeDoesNotReadOrIncludeKnowledgeSources() {
        var reads = 0
        val direct = knowledgeAiSourcesForMode(KnowledgeAiMode.DIRECT) { reads++; listOf("私人笔记") }
        assertTrue(direct.isEmpty()); assertEquals(0, reads)
        val knowledge = knowledgeAiSourcesForMode(KnowledgeAiMode.KNOWLEDGE) { reads++; listOf("已选节选") }
        assertEquals(listOf("已选节选"), knowledge); assertEquals(1, reads)
    }
    @Test fun modeSwitchInvalidatesQueuedTransportAndStaleAnswer() {
        val state = KnowledgeAiRequestBoundary(KnowledgeAiMode.KNOWLEDGE)
        val ticket = state.begin(true, true, "问题", 1)!!
        state.changeMode(KnowledgeAiMode.DIRECT)
        assertFalse(state.isCurrent(ticket))
        assertNull(state.begin(true, true, "问题", 0))
        assertNull(state.finish(ticket, true, true, "旧知识库回答", ""))
        val next = state.begin(true, true, "问题", 0)!!
        assertEquals("直接回答", state.finish(next, true, true, "直接回答", "")!!.text)
    }
    @Test fun switchingBackDoesNotReviveOldModeOrBypassKnowledgeSources() {
        val state = KnowledgeAiRequestBoundary(KnowledgeAiMode.DIRECT)
        val ticket = state.begin(true, true, "问题", 0)!!
        state.changeMode(KnowledgeAiMode.KNOWLEDGE)
        state.changeMode(KnowledgeAiMode.DIRECT)
        assertFalse(state.isCurrent(ticket))
        assertNull(state.finish(ticket, true, true, "过期回答", ""))
        state.changeMode(KnowledgeAiMode.KNOWLEDGE)
        assertNull(state.begin(true, true, "问题", 0))
        assertNotNull(state.begin(true, true, "问题", 1))
    }
    @Test fun keyboardWaitsForFocusedResumedWindow() {
        val state = KnowledgeKeyboardRequest()
        val ticket = state.request()
        assertFalse(state.takeReady(ticket, true, false, true))
        assertFalse(state.takeReady(ticket, false, true, true))
        assertFalse(state.takeReady(ticket, true, true, false))
        assertTrue(state.takeReady(ticket, true, true, true))
    }
    @Test fun consumedKeyboardRequestDoesNotReopenAfterBackOrRecomposition() {
        val state = KnowledgeKeyboardRequest()
        val ticket = state.request()
        assertTrue(state.takeReady(ticket, true, true, true))
        assertFalse(state.takeReady(ticket, true, false, true))
        assertFalse(state.takeReady(ticket, true, true, true))
    }
    @Test fun tappingFocusedEditorCanRequestKeyboardAgain() {
        val state = KnowledgeKeyboardRequest()
        val first = state.request()
        assertTrue(state.takeReady(first, true, true, true))
        val second = state.request()
        assertFalse(state.takeReady(first, true, true, true))
        assertTrue(state.takeReady(second, true, true, true))
    }
    @Test fun lostFocusStoppedOrDisposedEditorRejectsDeferredKeyboard() {
        val state = KnowledgeKeyboardRequest()
        val old = state.request()
        state.cancel()
        assertFalse(state.takeReady(old, true, true, true))
        val next = state.request()
        assertTrue(state.takeReady(next, true, true, true))
        val disposed = state.request()
        state.close()
        assertFalse(state.takeReady(disposed, true, true, true))
        assertFalse(state.takeReady(state.request(), true, true, true))
    }
}
"####;

fn replace(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!("AI workflow anchor is not unique: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    let mut source = source.to_owned();
    match path {
        "com/ofairyo/gridtimer/ui/GridTimerScreen.kt" => {
            replace(&mut source,
                "    var selectedDestination by rememberSaveable { mutableStateOf(HomeNavigationDestination.BOARD) }",
                "    var selectedDestination by rememberSaveable { mutableStateOf(HomeNavigationDestination.BOARD) }\n    var pendingKnowledgeAi by rememberSaveable(activeWorkspaceKey) { mutableStateOf(false) }\n    var pendingLegalAi by rememberSaveable(activeWorkspaceKey) { mutableStateOf(false) }\n    var openAiSettings by rememberSaveable(activeWorkspaceKey) { mutableStateOf(false) }")?;
            replace(&mut source,
                "                                NoteStudioSheet(\n                                    initialNoteId = pendingLegalNoteId,",
                "                                NoteStudioSheet(\n                                    openAiOnEntry = pendingKnowledgeAi,\n                                    onAiEntryConsumed = { pendingKnowledgeAi = false },\n                                    onOpenAiSettings = { openAiSettings = true; selectedDestination = HomeNavigationDestination.MY },\n                                    initialNoteId = pendingLegalNoteId,")?;
            replace(&mut source,
                "                            FinanceSheet(\n                                appData = appData,",
                "                            FinanceSheet(\n                                openLegalOnEntry = pendingLegalAi,\n                                onLegalEntryConsumed = { pendingLegalAi = false },\n                                onOpenAiSettings = { openAiSettings = true; selectedDestination = HomeNavigationDestination.MY },\n                                appData = appData,")?;
            replace(&mut source,
                "                                onAiConfigurationSave = { apiKey, baseUrl, model ->",
                "                                initialAiExpanded = openAiSettings,\n                                onOpenKnowledgeAi = { pendingKnowledgeAi = true; selectedDestination = HomeNavigationDestination.NOTEBOOK },\n                                onOpenLegalRisk = { pendingLegalAi = true; selectedDestination = HomeNavigationDestination.FINANCE },\n                                onAiConfigurationSave = { apiKey, baseUrl, model ->")?;
            replace(&mut source,
                "private fun FinanceSheet(\n    appData: AppData,",
                "private fun FinanceSheet(\n    openLegalOnEntry: Boolean = false,\n    onLegalEntryConsumed: () -> Unit = {},\n    onOpenAiSettings: () -> Unit = {},\n    appData: AppData,")?;
            replace(&mut source,
                "    var showLegalRisk by rememberSaveable(workspaceKey) { mutableStateOf(false) }",
                "    var showLegalRisk by rememberSaveable(workspaceKey) { mutableStateOf(false) }\n    LaunchedEffect(openLegalOnEntry, workspaceKey) {\n        if (openLegalOnEntry) { showLegalRisk = true; onLegalEntryConsumed() }\n    }")?;
            replace(&mut source,
                "            onDismiss = { showLegalRisk = false },\n            onOpenEvidence =",
                "            onDismiss = { showLegalRisk = false },\n            onOpenAiSettings = onOpenAiSettings,\n            onOpenEvidence =")?;
            replace(&mut source,
                "                Text(\"账目浏览周期\", style = MaterialTheme.typography.labelMedium)",
                r####"                Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("AI 法律风险分析", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                        Text("扫描任务、笔记、知识页和账目，逐条给出待核查线索与原始记录。", style = MaterialTheme.typography.bodyMedium)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            TextButton(onClick = { showLegalRisk = true }) { Text("开始 AI 分析") }
                            TextButton(onClick = onOpenAiSettings) { Text("AI 设置") }
                        }
                        Text(if (syncSession.aiApiKey.isBlank()) "先配置 AI，扫描预览可以在本机查看。" else "${syncSession.aiModel} · 发送前查看范围并确认", style = MaterialTheme.typography.bodySmall)
                    }
                }
                Text("账目浏览周期", style = MaterialTheme.typography.labelMedium)"####)?;
        }
        "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt" => {
            for import in [
                "androidx.compose.foundation.interaction.PressInteraction",
                "androidx.compose.foundation.layout.ime",
                "androidx.compose.foundation.layout.imePadding",
                "androidx.compose.foundation.relocation.BringIntoViewRequester",
                "androidx.compose.foundation.relocation.bringIntoViewRequester",
                "androidx.compose.runtime.withFrameNanos",
                "androidx.compose.ui.focus.FocusRequester",
                "androidx.compose.ui.focus.focusRequester",
                "androidx.compose.ui.platform.LocalDensity",
                "androidx.compose.ui.platform.LocalSoftwareKeyboardController",
                "androidx.compose.ui.platform.LocalWindowInfo",
                "androidx.compose.ui.window.DialogProperties",
                "kotlinx.coroutines.flow.collect",
            ] {
                let statement = format!("import {import}\n");
                if !source.contains(&statement) {
                    replace(
                        &mut source,
                        "package com.ofairyo.gridtimer.ui\n",
                        &format!("package com.ofairyo.gridtimer.ui\n{statement}"),
                    )?;
                }
            }
            if !source.contains("import androidx.compose.material3.TextButton\n") {
                replace(&mut source, "package com.ofairyo.gridtimer.ui\n", "package com.ofairyo.gridtimer.ui\nimport androidx.compose.material3.TextButton\n")?;
            }
            replace(&mut source,
                "internal fun NoteStudioSheet(\n    initialNoteId: String? = null,",
                "internal fun NoteStudioSheet(\n    openAiOnEntry: Boolean = false,\n    onAiEntryConsumed: () -> Unit = {},\n    onOpenAiSettings: () -> Unit = {},\n    initialNoteId: String? = null,")?;
            replace(&mut source,
                "    var knowledgePriorityNoteId by remember(workspaceKey) { mutableStateOf<String?>(null) }",
                "    var knowledgePriorityNoteId by remember(workspaceKey) { mutableStateOf<String?>(null) }\n    LaunchedEffect(openAiOnEntry, workspaceKey) {\n        if (openAiOnEntry) { knowledgeDialogVisible = true; knowledgePriorityNoteId = null; onAiEntryConsumed() }\n    }")?;
            replace(&mut source,
                "            priorityNoteId = knowledgePriorityNoteId,\n            viewModel = viewModel,",
                "            priorityNoteId = knowledgePriorityNoteId,\n            onOpenAiSettings = onOpenAiSettings,\n            viewModel = viewModel,")?;
            let start = source
                .find("@Composable\nprivate fun KnowledgeAiDialog(")
                .ok_or("missing knowledge AI dialog")?;
            let end = source[start..]
                .find("@Composable\nprivate fun KnowledgeSourceCard(")
                .map(|offset| start + offset)
                .ok_or("missing knowledge source card")?;
            source.replace_range(start..end, KNOWLEDGE_DIALOG);
            // Provider-neutral errors also apply to the note editor.
            source = source.replace(
                "请先在“我的”里填写 OpenAI API Key",
                "请先在“我的 → AI”中配置密钥、接口和模型。",
            );
        }
        "com/ofairyo/gridtimer/ui/SmartisanNoteUi.kt" => {
            source = source.replace(
                "请先在“我的”里填写 OpenAI API Key",
                "请先在“我的 → AI”中配置密钥、接口和模型。",
            );
        }
        _ => {}
    }
    Ok(source)
}

const KNOWLEDGE_DIALOG: &str = r####"@Composable
private fun KnowledgeAiDialog(
    appData: AppData,
    syncSession: SyncAccountSession,
    selectedFolderId: String?,
    priorityNoteId: String?,
    onOpenAiSettings: () -> Unit,
    viewModel: TimerViewModel,
    onDismiss: () -> Unit,
    onOpenSource: (KnowledgeSourceCandidate) -> Unit,
    onSaveAnswer: (String, String, List<KnowledgeSourceCandidate>) -> Unit
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val lifecycleOwner = LocalLifecycleOwner.current
    val workspaceKey = LocalNoteMediaWorkspaceKey.current
    val identity = listOf(syncSession.token, syncSession.userId, syncSession.serverInstanceId, syncSession.accountNamespace).joinToString("\u0000")
    val configuration = AiConfiguration(syncSession.aiApiKey, syncSession.aiBaseUrl, syncSession.aiModel)
    val latestIdentity by rememberUpdatedState(identity)
    val latestConfiguration by rememberUpdatedState(configuration)
    var queryMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(KnowledgeAiMode.DIRECT) }
    val latestMode by rememberUpdatedState(queryMode)
    val boundary = remember(workspaceKey, identity, priorityNoteId) { KnowledgeAiRequestBoundary(queryMode) }
    val latestBoundary by rememberUpdatedState(boundary)
    var question by rememberSaveable(workspaceKey, priorityNoteId) { mutableStateOf("") }
    var searchScope by rememberSaveable(workspaceKey, priorityNoteId, selectedFolderId) {
        mutableStateOf(if (selectedFolderId == null) KnowledgeSearchScope.ALL else KnowledgeSearchScope.CURRENT_FOLDER)
    }
    var answer by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var answerQuestion by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var answerSources by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<KnowledgeSourceCandidate>>(emptyList()) }
    var statusText by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf("") }
    var busy by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var cancelling by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    var showSendingContent by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }
    val selectedFolderName = appData.findNoteFolder(selectedFolderId)?.name
    val sourceCandidates = remember(queryMode, appData.notes, appData.noteFolders, question, searchScope, selectedFolderId, priorityNoteId) {
        knowledgeAiSourcesForMode(queryMode) {
            buildKnowledgeSourceCandidates(appData, question, selectedFolderId, searchScope, priorityNoteId)
        }
    }
    val sourceSignature = sourceCandidates.map { listOf(it.note.id, it.note.displayTitle(), it.folderName, it.excerpt) }
    val latestSourceSignature by rememberUpdatedState(sourceSignature)
    val latestQuestion by rememberUpdatedState(question)
    var observedSources by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(sourceSignature) }
    var observedConfiguration by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(configuration) }

    fun invalidateAnswer() {
        boundary.invalidate()
        answer = ""; answerQuestion = ""; answerSources = emptyList()
        statusText = if (busy) "问题或资料已改变，本次回答将不再采用。" else ""
        cancelling = busy
    }
    fun changeMode(next: KnowledgeAiMode) {
        if (queryMode == next) return
        boundary.changeMode(next)
        invalidateAnswer()
        queryMode = next
        showSendingContent = false
        if (busy) statusText = "模式已切换，旧请求结束前不能重复发送。"
    }
    DisposableEffect(boundary, lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) {
                boundary.invalidate()
                if (busy) { cancelling = true; statusText = "已停止采用本次回答，返回后可重新发送。" }
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { boundary.close(); lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(sourceSignature, configuration) {
        if (observedSources != sourceSignature || observedConfiguration != configuration) {
            invalidateAnswer()
            observedSources = sourceSignature
            observedConfiguration = configuration
        }
    }

    fun requestAnswer() {
        val safeQuestion = question.trim()
        val sources = sourceCandidates.toList()
        observedSources = sourceSignature
        observedConfiguration = configuration
        val ticket = boundary.begin(true, configuration.testable, safeQuestion, sources.size) ?: return
        val requestMode = queryMode
        val requestIdentity = identity
        val requestConfiguration = configuration
        val requestSignature = sourceSignature
        answer = ""; answerQuestion = ""; answerSources = emptyList()
        busy = true; cancelling = false; statusText = ""
        scope.launch {
            try {
                val result = withContext(Dispatchers.IO) {
                    if (!boundary.isCurrent(ticket) || latestBoundary !== boundary || viewModel.currentWorkspaceKey() != workspaceKey ||
                        latestIdentity != requestIdentity || latestConfiguration != requestConfiguration ||
                        latestQuestion.trim() != safeQuestion || latestMode != requestMode || latestSourceSignature != requestSignature) null
                    else NativeOptimizerBridge.completeAndroidAiQuery(
                        apiKey = requestConfiguration.apiKey.trim(),
                        baseUrl = requestConfiguration.baseUrl.trim(),
                        model = requestConfiguration.model.trim(),
                        mode = requestMode.wireValue,
                        question = safeQuestion,
                        sourceTitles = sources.map { it.note.displayTitle() }.toTypedArray(),
                        sourceFolders = sources.map(KnowledgeSourceCandidate::folderName).toTypedArray(),
                        sourceExcerpts = sources.map(KnowledgeSourceCandidate::excerpt).toTypedArray()
                    )
                }
                val current = latestBoundary === boundary && latestIdentity == requestIdentity && latestConfiguration == requestConfiguration &&
                    viewModel.currentWorkspaceKey() == workspaceKey && latestQuestion.trim() == safeQuestion &&
                    latestMode == requestMode && latestSourceSignature == requestSignature
                val outcome = boundary.finish(ticket, current, result?.getOrNull(0)?.toBooleanStrictOrNull() == true,
                    result?.getOrNull(2).orEmpty(), result?.getOrNull(1).orEmpty())
                if (outcome != null && latestBoundary === boundary) {
                    if (outcome.ok) {
                        answer = outcome.text; answerQuestion = safeQuestion; answerSources = sources
                        statusText = if (requestMode == KnowledgeAiMode.DIRECT) "AI 回答完成" else "回答完成 · ${sources.size} 个来源"
                    } else statusText = outcome.message
                }
            } finally {
                if (latestBoundary === boundary) {
                    val interrupted = cancelling
                    busy = false; cancelling = false
                    if (interrupted) statusText = "请求已结束，可以重新发送。"
                }
            }
        }
    }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(decorFitsSystemWindows = false)) {
        Box(modifier = Modifier.fillMaxWidth().imePadding().navigationBarsPadding()
            .windowInsetsPadding(WindowInsets.statusBars.only(WindowInsetsSides.Top))) {
        Surface(modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(22.dp), color = flowusPanelBackground(), border = BorderStroke(1.dp, flowusBorderColor())) {
            LazyColumn(modifier = Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                item {
                    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                        Text("AI 问答", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                        CapsuleAction(text = "关闭", icon = Icons.Rounded.Close, onClick = onDismiss)
                    }
                }
                item {
                    Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                        Column(modifier = Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(if (configuration.testable) "${configuration.model} · ${configuration.recipientHost}" else "AI 尚未配置完整", fontWeight = FontWeight.SemiBold)
                            Text(if (queryMode == KnowledgeAiMode.DIRECT) "直接向 AI 提问，仅发送你的问题。" else "依据所选知识页节选回答，并列出来源。", style = MaterialTheme.typography.bodySmall)
                            TextButton(onClick = { boundary.invalidate(); onDismiss(); onOpenAiSettings() }) { Text(if (configuration.testable) "更改 AI 设置" else "配置 DeepSeek / 其他 AI") }
                        }
                    }
                }
                item {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        ChoicePill(text = "直接问 AI", selected = queryMode == KnowledgeAiMode.DIRECT, accent = notesAccentColor,
                            onClick = { changeMode(KnowledgeAiMode.DIRECT) })
                        ChoicePill(text = "问知识库", selected = queryMode == KnowledgeAiMode.KNOWLEDGE, accent = accentFor("blue"),
                            onClick = { changeMode(KnowledgeAiMode.KNOWLEDGE) })
                    }
                }
                if (queryMode == KnowledgeAiMode.KNOWLEDGE) item {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (selectedFolderId != null) ChoicePill(text = selectedFolderName ?: "当前文档库", selected = searchScope == KnowledgeSearchScope.CURRENT_FOLDER, accent = notesAccentColor,
                            onClick = { invalidateAnswer(); searchScope = KnowledgeSearchScope.CURRENT_FOLDER })
                        ChoicePill(text = "全部知识页", selected = searchScope == KnowledgeSearchScope.ALL, accent = accentFor("blue"),
                            onClick = { invalidateAnswer(); searchScope = KnowledgeSearchScope.ALL })
                    }
                }
                item(key = "knowledge-question") {
                    KnowledgeQuestionField(question = question,
                        hint = if (queryMode == KnowledgeAiMode.DIRECT) "例如：一加一等于几" else "例如：总结这些记录里需要我处理的事情",
                        onQuestionChange = { invalidateAnswer(); question = it })
                }
                if (question.isBlank()) item {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        for (example in if (queryMode == KnowledgeAiMode.DIRECT) listOf("一加一等于几", "解释复利是怎么计算的") else listOf("总结最近的记录", "找出待办和未解决的问题", "整理主要观点")) {
                            TextButton(onClick = { invalidateAnswer(); question = example }) { Text(example) }
                        }
                    }
                }
                item {
                    Text(if (queryMode == KnowledgeAiMode.DIRECT) "将发送：仅你的问题" else "将发送：你的问题 + ${sourceCandidates.size} 个知识页节选", style = MaterialTheme.typography.bodySmall)
                    if (!configuration.testable) Text("先点上方配置入口，填写密钥、接口根地址和模型。", style = MaterialTheme.typography.bodySmall)
                    else if (question.isBlank()) Text("输入问题，或点选上面的示例。", style = MaterialTheme.typography.bodySmall)
                    else if (queryMode == KnowledgeAiMode.KNOWLEDGE && sourceCandidates.isEmpty()) Text("当前范围没有可读知识页。可切到全部知识页，或改用直接问 AI。", style = MaterialTheme.typography.bodySmall)
                    TextButton(onClick = { showSendingContent = !showSendingContent }) { Text(if (showSendingContent) "收起发送内容" else "查看将发送的资料") }
                    PhysicalButton(label = if (busy) (if (cancelling) "等待本次请求结束" else "AI 正在回答") else "发送问题", icon = Icons.Rounded.Title,
                        accent = notesAccentColor, filled = true, enabled = !busy && configuration.testable && question.isNotBlank() &&
                            (queryMode == KnowledgeAiMode.DIRECT || sourceCandidates.isNotEmpty()),
                        modifier = Modifier.fillMaxWidth(), onClick = ::requestAnswer)
                }
                if (busy) item {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                        TextButton(onClick = { boundary.invalidate(); cancelling = true; statusText = "已取消，结束前不能发起重复请求。" }, enabled = !cancelling) { Text("取消本次回答") }
                    }
                }
                if (statusText.isNotBlank()) item {
                    Text(statusText, style = MaterialTheme.typography.bodyMedium, color = if (answer.isBlank() && !busy) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)
                }
                if (answer.isNotBlank()) {
                    item {
                        FlowusPanel(accent = notesAccentColor) { AndroidRenderedMarkdown(answer, modifier = Modifier.padding(14.dp)) }
                        TextButton(onClick = {
                            val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as? android.content.ClipboardManager
                            clipboard?.setPrimaryClip(android.content.ClipData.newPlainText("AI 回答", answer))
                            Toast.makeText(context, "已复制回答原文", Toast.LENGTH_SHORT).show()
                        }) { Text("复制回答原文") }
                        PhysicalButton(label = "保存回答为知识页", icon = Icons.Rounded.SaveAlt, accent = accentFor("green"), filled = false, enabled = !busy,
                            modifier = Modifier.fillMaxWidth(), onClick = { onSaveAnswer(answerQuestion, answer, answerSources) })
                    }
                    if (answerSources.isNotEmpty()) item { Text("回答依据 ${answerSources.size}", fontWeight = FontWeight.SemiBold) }
                    items(answerSources, key = { "answer-${it.note.id}" }) { source -> KnowledgeSourceCard(source, answerSources.indexOf(source) + 1) { onOpenSource(source) } }
                }
                if (showSendingContent) {
                    item { Text("待发送的问题", fontWeight = FontWeight.SemiBold); Text(question.ifBlank { "尚未输入问题" }) }
                    if (queryMode == KnowledgeAiMode.KNOWLEDGE) item { Text("待发送的来源节选 ${sourceCandidates.size}", fontWeight = FontWeight.SemiBold) }
                    items(sourceCandidates, key = { "preview-${it.note.id}" }) { source -> KnowledgeSourceCard(source, sourceCandidates.indexOf(source) + 1) { onOpenSource(source) } }
                }
            }
        }
        }
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)
@Composable
private fun KnowledgeQuestionField(question: String, hint: String, onQuestionChange: (String) -> Unit) {
    // These locals must belong to the Dialog, not the Activity underneath it.
    val keyboard = LocalSoftwareKeyboardController.current
    val windowInfo = LocalWindowInfo.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val focusRequester = remember { FocusRequester() }
    val bringIntoView = remember { BringIntoViewRequester() }
    val interactions = remember { MutableInteractionSource() }
    val request = remember(lifecycleOwner) { KnowledgeKeyboardRequest() }
    var fieldFocused by remember { mutableStateOf(false) }
    var ticket by remember(request) { mutableStateOf(0L) }
    var resumed by remember(lifecycleOwner) {
        mutableStateOf(lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED))
    }
    val currentFocused by rememberUpdatedState(fieldFocused)
    val windowFocused = windowInfo.isWindowFocused
    val imeBottom = WindowInsets.ime.getBottom(LocalDensity.current)

    DisposableEffect(request, lifecycleOwner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) resumed = true
            if (event == Lifecycle.Event.ON_PAUSE || event == Lifecycle.Event.ON_STOP) resumed = false
            if (event == Lifecycle.Event.ON_STOP) request.cancel()
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { request.close(); lifecycleOwner.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(interactions, request) {
        interactions.interactions.collect { interaction ->
            if (interaction is PressInteraction.Release) {
                focusRequester.requestFocus()
                ticket = request.request()
            }
        }
    }
    LaunchedEffect(request, ticket, fieldFocused, windowFocused, resumed) {
        if (!fieldFocused || !windowFocused || !resumed) return@LaunchedEffect
        // Let the dialog editor establish its input connection before showing IME.
        withFrameNanos { }
        if (request.takeReady(ticket, currentFocused, windowInfo.isWindowFocused,
                lifecycleOwner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED))) {
            keyboard?.show()
            bringIntoView.bringIntoView()
        }
    }
    LaunchedEffect(imeBottom, fieldFocused) {
        if (imeBottom > 0 && fieldFocused) bringIntoView.bringIntoView()
    }
    SmartisanTextField(value = question, onValueChange = onQuestionChange, label = "你的问题",
        hint = hint, maxLength = 1000, minLines = 2, maxLines = 5,
        modifier = Modifier.fillMaxWidth(),
        fieldModifier = Modifier.focusRequester(focusRequester).bringIntoViewRequester(bringIntoView),
        interactionSource = interactions,
        onFocusChanged = { focused ->
            fieldFocused = focused
            if (focused) ticket = request.request() else request.cancel()
        })
}

"####;
