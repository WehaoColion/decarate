// Android Agent interaction: actual task progress and explicit result/save states.
fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    let count = source.matches(before).count();
    if count != 1 {
        return Err(format!("Agent progress UI anchor count {count}: {before}"));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}
pub fn render(source: &str) -> Result<String, String> {
    let mut rendered = source.to_owned();
    replace_once(
        &mut rendered,
        r####"    val agentSelectedIds = remember(workspaceKey, identity, selectedFolderId, priorityNoteId) { mutableStateListOf<String>() }"####,
        r####"    val agentSelectedIds = remember(workspaceKey, identity, selectedFolderId, priorityNoteId) { mutableStateListOf<String>() }
    var agentProgress by remember(agentBoundary) { mutableStateOf<KnowledgeAgentProgress?>(null) }
    var agentTerminal by remember(agentBoundary) { mutableStateOf("idle") }
    var agentSaved by remember(agentBoundary) { mutableStateOf(false) }
    var agentRequests by remember(agentBoundary) { mutableStateOf(0) }
    var agentToolCalls by remember(agentBoundary) { mutableStateOf(0) }"####,
    )?;
    replace_once(
        &mut rendered,
        r####"    val selectedAgentDocuments = remember(agentAvailableDocuments, agentSelectedIds.toSet()) {"####,
        r####"    val agentSelectableDocuments = agentAvailableDocuments
    val selectedAgentDocuments = remember(agentSelectableDocuments, agentSelectedIds.toSet()) {"####,
    )?;
    replace_once(
        &mut rendered,
        r####"        agentAvailableDocuments.filter { it.id in selectedIds }.map { note ->"####,
        r####"        projectKnowledgeSources(agentSelectableDocuments.filter { it.id in selectedIds }) { note ->"####,
    )?;
    replace_once(
        &mut rendered,
        r####"    fun invalidateAnswer() {"####,
        r####"    LaunchedEffect(agentTaskId, agentBoundary) {
        val watchedId = agentTaskId ?: return@LaunchedEffect
        while (agentRunHandle.taskId == watchedId && latestAgentBoundary === agentBoundary &&
            latestIdentity == identity && viewModel.currentWorkspaceKey() == workspaceKey) {
            val rawProgress = withContext(Dispatchers.IO) {
                NativeOptimizerBridge.androidKnowledgeAgentProgress(watchedId)
            }
            val owned = agentRunHandle.taskId == watchedId && latestAgentBoundary === agentBoundary &&
                latestIdentity == identity && viewModel.currentWorkspaceKey() == workspaceKey
            val accepted = acceptKnowledgeAgentProgress(agentProgress, decodeKnowledgeAgentProgress(rawProgress), watchedId, owned)
            if (owned && accepted != null) {
                agentProgress = accepted
                agentRequests = maxOf(agentRequests, accepted.requests)
                agentToolCalls = maxOf(agentToolCalls, accepted.toolCalls)
            }
            delay(500L)
        }
    }

    fun invalidateAnswer() {"####,
    )?;
    replace_once(
        &mut rendered,
        r####"        val runId = UUID.randomUUID().toString()"####,
        r####"        agentProgress = null; agentTerminal = "running"; agentSaved = false; agentRequests = 0; agentToolCalls = 0
        val runId = UUID.randomUUID().toString()"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                val completed = agentBoundary.finish(ticket, current, wireOk && wireDraft != null)"####,
        r####"                if (ownedRun) {
                    agentRequests = maxOf(agentRequests, parsed?.optInt("requests", agentRequests) ?: agentRequests)
                    agentToolCalls = maxOf(agentToolCalls, parsed?.optInt("toolCalls", agentToolCalls) ?: agentToolCalls)
                    agentTrace = parsed?.optJSONArray("toolTrace")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optJSONObject(index)?.let {
                            "${it.optString("summary")} · ${it.optInt("resultCount")} 项"
                        } }
                    }.orEmpty()
                    agentPlan = parsed?.optJSONArray("plan")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optString(index).takeIf(String::isNotBlank) }
                    }.orEmpty()
                    val checks = parsed?.optJSONObject("verification")
                    agentVerificationPassed = checks?.optBoolean("passed", false) == true
                    agentVerification = checks?.optJSONArray("checks")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optJSONObject(index)?.let {
                            "${if (it.optBoolean("passed", false)) "✓" else "!"} ${it.optString("label")} · ${it.optString("detail")}".trim()
                        } }
                    }.orEmpty()
                    agentTerminal = when {
                        agentCancelling -> "cancelled"
                        !wireOk || wireDraft == null -> "failed"
                        !agentVerificationPassed -> if (parsed?.optBoolean("reviewAttempted", false) == true &&
                            parsed?.optBoolean("reviewCompleted", false) != true) "review_incomplete" else "failed"
                        else -> "ready"
                    }
                }
                val completed = agentBoundary.finish(ticket, current, wireOk && wireDraft != null)"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                    if (!wasCancelling && agentDraft == null && statusText.isBlank()) statusText = "Agent 请求已结束。""####,
        r####"                    if (wasCancelling) {
                        agentTerminal = "cancelled"
                        statusText = "已取消。后续模型请求已停止，本次结果不能保存。"
                    } else if (agentDraft == null && statusText.isBlank()) {
                        agentTerminal = "failed"
                        statusText = "Agent 请求已结束，没有可保存的完整草稿。"
                    }"####,
    )?;
    replace_once(
        &mut rendered,
        r####"    fun saveAgentDraft() {
        if (agentSaving) return"####,
        r####"    fun currentAgentSaveState() = knowledgeAgentSaveState(
        running = agentBusy, saving = agentSaving, saved = agentSaved, terminal = agentTerminal,
        identityCurrent = viewModel.currentWorkspaceKey() == workspaceKey && latestIdentity == identity,
        contextCurrent = agentDraftContext != null && agentDraftContext == currentAgentDraftContext(),
        verified = agentVerificationPassed,
        completeDraft = agentDraft != null && agentDraftTitle.isNotBlank() && agentDraftBody.isNotBlank() && agentResultSources.isNotEmpty()
    )

    fun saveAgentDraft() {
        if (currentAgentSaveState() != KnowledgeAgentSaveState.READY) {
            statusText = knowledgeAgentSaveReason(currentAgentSaveState())
            return
        }"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                agentDraftContext = null
                statusText = "已保存为新的知识页。""####,
        r####"                agentSaved = true
                agentDraftContext = null
                statusText = "已保存为新的知识页。关闭面板后可新建任务。""####,
    )?;
    replace_once(
        &mut rendered,
        r####"                            knowledgeDialogVisible = false
                            knowledgePriorityNoteId = null
                            Toast.makeText(context, "已保存为新的知识页", Toast.LENGTH_SHORT).show()
                            onComplete(true, "")"####,
        r####"                            Toast.makeText(context, "已保存为新的知识页", Toast.LENGTH_SHORT).show()
                            onComplete(true, "")"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                            if (selectedAgentDocuments.size == agentAvailableDocuments.size) agentSelectedIds.clear()
                            else { agentSelectedIds.clear(); agentAvailableDocuments.take(30).forEach { agentSelectedIds.add(it.id) } }"####,
        r####"                            val next = toggleKnowledgeAgentSelection(agentSelectedIds.toSet(), agentSelectableDocuments.map(NoteEntry::id))
                            agentSelectedIds.clear()
                            agentSelectedIds.addAll(next)"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                        }) { Text(if (selectedAgentDocuments.size == agentAvailableDocuments.size) "清空选择" else "全选当前范围") }"####,
        r####"                        }) { Text(if (agentSelectedIds.toSet() == agentSelectableDocuments.take(30).map(NoteEntry::id).toSet()) "清空选择" else "选择当前范围（最多 30 条）") }"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                if (agentMode) items(agentAvailableDocuments, key = { "agent-doc-${it.id}" }) { note ->"####,
        r####"                if (agentMode) items(agentSelectableDocuments, key = { "agent-doc-${it.id}" }) { note ->"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                    PhysicalButton(label = when { agentMode && agentBusy -> if (agentCancelling) "正在停止后续步骤" else "Agent 正在执行"; agentMode -> "授权本次范围并运行 Agent"; busy -> if (cancelling) "等待本次请求结束" else "AI 正在回答"; else -> "发送问题" }, icon = Icons.Rounded.Title,
                        accent = if (agentMode) accentFor("green") else notesAccentColor, filled = true,
                        enabled = if (agentMode) !agentBusy && !agentSaving && showSendingContent && agentPreviewAcknowledged && configuration.testable && question.isNotBlank() && selectedAgentDocuments.isNotEmpty()
                            else !busy && !agentBusy && configuration.testable && question.isNotBlank() && (queryMode == KnowledgeAiMode.DIRECT || sourceCandidates.isNotEmpty()),
                        modifier = Modifier.fillMaxWidth(), onClick = if (agentMode) ::requestAgent else ::requestAnswer)"####,
        r####"                    val launchAction = knowledgeAgentLaunchAction(busy || agentBusy, agentSaving,
                        !agentSaved && configuration.testable && question.isNotBlank() && selectedAgentDocuments.isNotEmpty(),
                        showSendingContent, agentPreviewAcknowledged)
                    PhysicalButton(label = when {
                            agentMode && agentBusy -> if (agentCancelling) "正在停止后续步骤" else "Agent 正在执行"
                            agentMode && agentSaved -> "本任务已保存"
                            agentMode && launchAction == KnowledgeAgentLaunchAction.START -> "开始执行 Agent"
                            agentMode -> "核对资料并继续"
                            busy -> if (cancelling) "等待本次请求结束" else "AI 正在回答"
                            else -> "发送问题"
                        }, icon = Icons.Rounded.Title, accent = if (agentMode) accentFor("green") else notesAccentColor, filled = true,
                        enabled = if (agentMode) launchAction != KnowledgeAgentLaunchAction.WAIT
                            else !busy && !agentBusy && configuration.testable && question.isNotBlank() && (queryMode == KnowledgeAiMode.DIRECT || sourceCandidates.isNotEmpty()),
                        modifier = Modifier.fillMaxWidth(), onClick = {
                            if (!agentMode) requestAnswer()
                            else when (launchAction) {
                                KnowledgeAgentLaunchAction.PREVIEW -> {
                                    showSendingContent = true
                                    agentPreviewAcknowledged = false
                                    statusText = "查看下方完整资料；在预览末尾勾选授权并开始执行。"
                                }
                                KnowledgeAgentLaunchAction.START -> requestAgent()
                                KnowledgeAgentLaunchAction.WAIT -> Unit
                            }
                        })"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                    Text(statusText, style = MaterialTheme.typography.bodyMedium, color = if (answer.isBlank() && !busy) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)"####,
        r####"                    Text(statusText, style = MaterialTheme.typography.bodyMedium,
                        color = if ((agentMode && agentTerminal in setOf("failed", "review_incomplete")) ||
                            (!agentMode && answer.isBlank() && !busy)) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                if (agentMode && agentDraft != null) {"####,
        r####"                if (agentMode && (agentBusy || agentTerminal != "idle" || agentProgress != null)) item(key = "agent-progress") {
                    FlowusPanel(accent = if (agentTerminal in setOf("failed", "review_incomplete")) accentFor("orange") else accentFor("green")) {
                        Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(knowledgeAgentTerminalLabel(agentTerminal, agentSaved, agentCancelling), fontWeight = FontWeight.SemiBold)
                            val progress = agentProgress
                            if (agentBusy) Text(if (progress == null) "等待本机任务开始。"
                                else knowledgeAgentActivityLabel(progress, agentCancelling), style = MaterialTheme.typography.bodySmall)
                            if (progress != null) Text("已等待 ${progress.elapsedSeconds} 秒", style = MaterialTheme.typography.bodySmall)
                            Text("模型请求 $agentRequests 次 · 工具调用 $agentToolCalls 次", style = MaterialTheme.typography.bodySmall)
                            if (agentBusy && progress != null && progress.stage != "verifying")
                                Text("本阶段已读取 ${progress.documentsRead} 条资料", style = MaterialTheme.typography.bodySmall)
                            if (agentTrace.isNotEmpty()) {
                                Text("实际工具记录", fontWeight = FontWeight.Medium)
                                agentTrace.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                            }
                            if (!agentBusy && agentVerification.isNotEmpty()) {
                                Text(if (agentVerificationPassed) "本机核验通过" else "本机核验未通过", fontWeight = FontWeight.Medium)
                                agentVerification.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
                            }
                        }
                    }
                }
                if (agentMode && agentDraft != null) {"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                        Text("新知识页草稿 · 尚未保存", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)"####,
        r####"                        Text(if (agentSaved) "新知识页 · 已保存" else "新知识页草稿 · 尚未保存", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                        if (agentVerification.isNotEmpty()) {
                            Text(if (agentVerificationPassed) "本机核验通过" else "本机核验未通过", fontWeight = FontWeight.SemiBold,
                                color = if (agentVerificationPassed) accentFor("green") else MaterialTheme.colorScheme.error)
                            agentVerification.forEach { line -> Text(line, style = MaterialTheme.typography.bodySmall) }
                        }
"####,
        r####""####,
    )?;
    replace_once(
        &mut rendered,
        r####"                        if (agentTrace.isNotEmpty()) {
                            Text("本次工具记录", fontWeight = FontWeight.SemiBold)
                            agentTrace.forEach { traceLine -> Text("· $traceLine", style = MaterialTheme.typography.bodySmall) }
                        }
"####,
        r####""####,
    )?;
    replace_once(
        &mut rendered,
        r####"                        PhysicalButton(label = if (agentSaving) "正在核验保存" else "确认保存为新知识页", icon = Icons.Rounded.SaveAlt, accent = accentFor("green"), filled = true,
                            enabled = !agentBusy && !agentSaving && agentBoundary.canSave("####,
        r####"                        val saveState = currentAgentSaveState()
                        Text(knowledgeAgentSaveReason(saveState), style = MaterialTheme.typography.bodySmall)
                        PhysicalButton(label = when (saveState) {
                                KnowledgeAgentSaveState.SAVED -> "已保存为新知识页"
                                KnowledgeAgentSaveState.SAVING -> "正在核验保存"
                                else -> "确认保存为新知识页"
                            }, icon = Icons.Rounded.SaveAlt, accent = accentFor("green"), filled = true,
                            enabled = saveState == KnowledgeAgentSaveState.READY && !agentBusy && !agentSaving && agentBoundary.canSave("####,
    )?;
    replace_once(
        &mut rendered,
        r####"                            Text(
                                "我已核对上方完整资料、接收方和模型，并授权本次发送。",
                                modifier = Modifier.clickable { agentPreviewAcknowledged = showSendingContent },"####,
        r####"                            Text(
                                "我已核对上方完整资料、接收方和模型，并授权本次发送。",
                                modifier = Modifier.clickable(enabled = !agentBusy && !agentSaving && !agentSaved) {
                                    agentPreviewAcknowledged = showSendingContent && !agentBusy && !agentSaving && !agentSaved
                                },"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                                enabled = !agentBusy,
                                onCheckedChange = { checked -> agentPreviewAcknowledged = checked && showSendingContent }"####,
        r####"                                enabled = !agentBusy && !agentSaving && !agentSaved,
                                onCheckedChange = { checked -> agentPreviewAcknowledged = checked && showSendingContent && !agentBusy && !agentSaving && !agentSaved }"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                            )
                        }
                    }
                }
            }
        }
        }
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)"####,
        r####"                            )
                        }
                        PhysicalButton(label = if (agentSaved) "本任务已保存" else if (agentBusy) "Agent 正在执行" else "授权并开始执行",
                            icon = Icons.Rounded.Title, accent = accentFor("green"), filled = true,
                            enabled = knowledgeAgentLaunchAction(busy || agentBusy, agentSaving,
                                !agentSaved && configuration.testable && question.isNotBlank() && selectedAgentDocuments.isNotEmpty(),
                                showSendingContent, agentPreviewAcknowledged) == KnowledgeAgentLaunchAction.START,
                            modifier = Modifier.fillMaxWidth(), onClick = ::requestAgent)
                    }
                }
            }
        }
        }
    }
}

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)"####,
    )?;
    replace_once(
        &mut rendered,
        r####"        if (agentBusy || agentSaving) return
        val safeQuestion = question.trim()"####,
        r####"        if (agentBusy || agentSaving || busy || agentSaved) return
        val safeQuestion = question.trim()"####,
    )?;
    replace_once(
        &mut rendered,
        r####"        agentRunHandle.started(runId)"####,
        r####"        showSendingContent = false
        agentRunHandle.started(runId)"####,
    )?;
    replace_once(
        &mut rendered,
        r####"                        model = runConfiguration.model.trim(), scopeJson = scopeJson, taskId = runId
                    )
                }
                val parsed = raw?.let { runCatching { JSONObject(it) }.getOrNull() }"####,
        r####"                        model = runConfiguration.model.trim(), scopeJson = scopeJson, taskId = runId
                    )
                }
                val finalProgressRaw = withContext(Dispatchers.IO) {
                    NativeOptimizerBridge.androidKnowledgeAgentProgress(runId)
                }
                val ownedRun = agentRunHandle.taskId == runId && latestAgentBoundary === agentBoundary &&
                    latestIdentity == runIdentity && viewModel.currentWorkspaceKey() == workspaceKey
                val finalProgress = acceptKnowledgeAgentProgress(agentProgress, decodeKnowledgeAgentProgress(finalProgressRaw), runId, ownedRun)
                if (ownedRun && finalProgress != null) {
                    agentProgress = finalProgress
                    agentRequests = maxOf(agentRequests, finalProgress.requests)
                    agentToolCalls = maxOf(agentToolCalls, finalProgress.toolCalls)
                }
                val parsed = raw?.let { runCatching { JSONObject(it) }.getOrNull() }"####,
    )?;
    replace_once(
        &mut rendered,
        r####"OutlinedTextField(value = agentDraftTitle, onValueChange = { agentDraftTitle = it; agentHumanEdited = true }, label ="####,
        r####"OutlinedTextField(value = agentDraftTitle, onValueChange = {
                            if (!agentSaving && !agentSaved) { agentDraftTitle = it; agentHumanEdited = true }
                        }, readOnly = agentSaving || agentSaved, label ="####,
    )?;
    replace_once(
        &mut rendered,
        r####"OutlinedTextField(value = agentDraftBody, onValueChange = { agentDraftBody = it; agentHumanEdited = true }, label ="####,
        r####"OutlinedTextField(value = agentDraftBody, onValueChange = {
                            if (!agentSaving && !agentSaved) { agentDraftBody = it; agentHumanEdited = true }
                        }, readOnly = agentSaving || agentSaved, label ="####,
    )?;
    replace_once(
        &mut rendered,
        r####"        statusText = if (busy) "问题或资料已改变，本次回答将不再采用。" else """####,
        r####"        statusText = if (busy) "问题或资料已改变，本次回答将不再采用。"
            else if (agentSaved) "本任务已保存。关闭面板后可新建任务。" else """####,
    )?;
    replace_once(
        &mut rendered,
        r####"            statusText = "$reason，旧草稿已失效，请重新运行 Agent。"
        }
    }
    fun setAgentMode"####,
        r####"            statusText = "$reason，旧草稿已失效，请重新运行 Agent。"
        }
        if (!agentBusy) {
            agentProgress = null; agentTerminal = "idle"; agentRequests = 0; agentToolCalls = 0
            agentTrace = emptyList(); agentPlan = emptyList(); agentVerification = emptyList(); agentVerificationPassed = false
            if (agentSaved) statusText = "本任务已保存。关闭面板后可新建任务。"
        }
    }
    fun setAgentMode"####,
    )?;
    replace_once(
        &mut rendered,
        r####"        if (enabled) statusText = "先选择资料并查看发送范围，再点击授权并执行。""####,
        r####"        if (!agentBusy) {
            agentProgress = null; agentTerminal = "idle"; agentRequests = 0; agentToolCalls = 0
        }
        if (enabled) statusText = if (agentSaved) "本任务已保存。关闭面板后可新建任务。"
            else "先选择资料并查看发送范围，再点击授权并执行。""####,
    )?;
    rendered.push_str(UI_FUNCTIONS);
    Ok(rendered)
}
const UI_FUNCTIONS: &str = r####"
private fun decodeKnowledgeAgentProgress(raw: String?): KnowledgeAgentProgress? = raw?.let {
    runCatching {
        val value = org.json.JSONObject(it)
        KnowledgeAgentProgress(
            value.optString("taskId"), value.optLong("sequence", -1), value.optString("stage"),
            value.optString("activity"), value.optInt("requests", -1), value.optInt("toolCalls", -1),
            value.optInt("documentsRead", -1), value.optBoolean("requestInFlight"), value.optBoolean("cancelRequested"),
            value.optString("terminal"), value.optLong("elapsedSeconds", -1)
        )
    }.getOrNull()
}
private fun knowledgeAgentTerminalLabel(terminal: String, saved: Boolean, cancelling: Boolean): String = when {
    saved -> "新知识页已保存"
    cancelling -> "已请求取消"
    terminal == "ready" -> "任务完成 · 可以检查并保存"
    terminal == "review_incomplete" -> "复核未完成 · 草稿仅供查看"
    terminal == "failed" -> "任务未完成"
    terminal == "cancelled" -> "任务已取消 · 结果不能保存"
    else -> "Agent 正在执行"
}
private fun knowledgeAgentActivityLabel(progress: KnowledgeAgentProgress, cancelling: Boolean): String {
    if (cancelling || progress.cancelRequested) return if (progress.requestInFlight)
        "取消已生效；等待当前模型请求结束，不再发起后续请求。" else "正在结束本次任务。"
    val stage = when (progress.stage) { "review" -> "深度复核"; "verifying" -> "本机核验"; else -> "首轮任务" }
    val activity = when (progress.activity) {
        "model" -> "等待模型响应"
        "tool" -> "执行本机资料工具"
        "verifying" -> "核验来源与草稿"
        "finished" -> "处理返回结果"
        else -> "准备请求"
    }
    return "$stage · $activity"
}
private fun knowledgeAgentSaveReason(state: KnowledgeAgentSaveState): String = when (state) {
    KnowledgeAgentSaveState.READY -> "核验通过。检查当前草稿后，点按确认保存。"
    KnowledgeAgentSaveState.RUNNING -> "任务尚在执行，完成核验后才能保存。"
    KnowledgeAgentSaveState.SAVING -> "正在确认本机保存，请等待。"
    KnowledgeAgentSaveState.SAVED -> "已完成本机保存。原始资料未修改。关闭面板后可新建任务。"
    KnowledgeAgentSaveState.CANCELLED -> "任务已取消，本次结果不能保存。"
    KnowledgeAgentSaveState.FAILED -> "任务未完成，请重新核对范围后运行。"
    KnowledgeAgentSaveState.REVIEW_INCOMPLETE -> "深度复核没有完成，当前草稿不能保存。"
    KnowledgeAgentSaveState.CONTEXT_CHANGED -> "账号、工作区、目标或资料已变化，请重新运行。"
    KnowledgeAgentSaveState.INCOMPLETE -> "标题、正文或有效来源不完整，不能保存。"
    KnowledgeAgentSaveState.UNVERIFIED -> "本机核验未通过，不能保存。"
}
"####;
