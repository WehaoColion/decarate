// v2.23.2.24 - Guard whole-document AI launch and keep transient selection out of saved state.
// v2.23.2.23 - Freeze review context and apply the tested save verification policy.
// Android Agent v2: expose a visible plan, optional second-pass review and local verification.

pub const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!(
            "Android agent upgrade anchor is not unique: {before}"
        ));
    }
    *source = source.replacen(before, after, 1);
    Ok(())
}

pub fn render(path: &str, source: &str) -> Result<String, String> {
    if path != SCREEN {
        return Ok(source.to_owned());
    }
    let mut rendered = source.to_owned();

    replace_once(
        &mut rendered,
        "    var agentTrace by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<String>>(emptyList()) }\n    val agentSelectedIds =",
        "    var agentTrace by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<String>>(emptyList()) }\n    var agentPlan by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<String>>(emptyList()) }\n    var agentVerification by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf<List<String>>(emptyList()) }\n    var agentVerificationPassed by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }\n    var agentHumanEdited by remember(workspaceKey, identity, priorityNoteId) { mutableStateOf(false) }\n    var agentDeepReview by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(true) }\n    val agentSelectedIds =",
    )?;

    replace_once(
        &mut rendered,
        "        val scopeJson = JSONObject().put(\"question\", safeQuestion).put(\"documents\", JSONArray().apply {",
        "        val scopeJson = JSONObject().put(\"question\", safeQuestion).put(\"deepReview\", runDeepReview).put(\"documents\", JSONArray().apply {",
    )?;

    replace_once(
        &mut rendered,
        "        agentResultSources = emptyList(); agentTrace = emptyList(); statusText = \"正在连接 ${runConfiguration.recipientHost}，最多执行 6 次模型请求。\"",
        "        agentResultSources = emptyList(); agentPlan = emptyList(); agentTrace = emptyList(); agentVerification = emptyList(); agentVerificationPassed = false; agentHumanEdited = false\n        statusText = if (agentDeepReview) \"正在连接 ${runConfiguration.recipientHost}；首轮完成后将自动进行第二阶段深度核验。\" else \"正在连接 ${runConfiguration.recipientHost}，最多执行 6 次模型请求。\"",
    )?;

    let reset = "agentResultSources = emptyList(); agentTrace = emptyList()";
    let reset_count = rendered.matches(reset).count();
    if reset_count != 2 {
        return Err(format!(
            "expected two Agent reset anchors after request setup, found {reset_count}"
        ));
    }
    rendered = rendered.replace(
        reset,
        "agentResultSources = emptyList(); agentTrace = emptyList(); agentPlan = emptyList(); agentVerification = emptyList(); agentVerificationPassed = false; agentHumanEdited = false",
    );

    replace_once(
        &mut rendered,
        r####"                    agentTrace = parsed?.optJSONArray("toolTrace")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optJSONObject(index)?.let { "${it.optString("summary")} · ${it.optInt("resultCount")} 项" } }
                    }.orEmpty()
                    answer = parsed?.optString("answer").orEmpty()
                    statusText = "Agent 已生成草稿，尚未保存到知识库。""####,
        r####"                    agentTrace = parsed?.optJSONArray("toolTrace")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optJSONObject(index)?.let { "${it.optString("summary")} · ${it.optInt("resultCount")} 项" } }
                    }.orEmpty()
                    agentTrace = listOf("模型请求 ${parsed?.optInt("requests", 0) ?: 0} 次 · 本机工具调用 ${parsed?.optInt("toolCalls", 0) ?: 0} 次") + agentTrace
                    agentPlan = parsed?.optJSONArray("plan")?.let { array ->
                        (0 until array.length()).mapNotNull { index -> array.optString(index).takeIf(String::isNotBlank) }
                    }.orEmpty()
                    val verificationJson = parsed?.optJSONObject("verification")
                    agentVerificationPassed = verificationJson?.optBoolean("passed", false) == true
                    agentVerification = verificationJson?.optJSONArray("checks")?.let { array ->
                        (0 until array.length()).mapNotNull { index ->
                            array.optJSONObject(index)?.let { check ->
                                val mark = if (check.optBoolean("passed", false)) "✓" else "!"
                                "$mark ${check.optString("label")} · ${check.optString("detail")}".trim()
                            }
                        }
                    }.orEmpty()
                    agentHumanEdited = false
                    answer = parsed?.optString("answer").orEmpty()
                    statusText = if (agentVerificationPassed) "Agent 已生成并核验草稿，尚未保存到知识库。"
                        else parsed?.optString("message")?.takeIf(String::isNotBlank) ?: "草稿已生成，但本机核验未通过；请重新运行 Agent。""####,
    )?;

    replace_once(
        &mut rendered,
        "        val hasDraft = agentDraft != null && agentDraftTitle.isNotBlank() && agentDraftBody.isNotBlank() && agentResultSources.isNotEmpty()",
        "        val hasDraft = agentDraft != null && agentDraftTitle.isNotBlank() && agentDraftBody.isNotBlank() && agentResultSources.isNotEmpty() && agentVerificationPassed",
    )?;

    replace_once(
        &mut rendered,
        r####"                    Text("可选当前未删除、未加密的便签和知识页；每条资料需单独勾选。附件、财务、计时、归档记录、密钥和诊断资料不在本次范围。", style = MaterialTheme.typography.bodySmall)
                }
                if (agentMode) items(agentAvailableDocuments, key = { "agent-doc-${it.id}" }) { note ->"####,
        r####"                    Text("可选当前未删除、未加密的便签和知识页；每条资料需单独勾选。附件、财务、计时、归档记录、密钥和诊断资料不在本次范围。", style = MaterialTheme.typography.bodySmall)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        ChoicePill(text = "深度核验", selected = agentDeepReview, accent = accentFor("green"), onClick = {
                            stopAgent("核验模式已改变")
                            agentDeepReview = !agentDeepReview
                        })
                    }
                    Text(if (agentDeepReview) "开启后，首轮草稿完成后会再次检索同一授权资料，专门检查冲突、遗漏和无来源结论；模型调用会增加。"
                        else "关闭后只执行首轮 Agent；仍会进行本机来源与工具链核验。", style = MaterialTheme.typography.bodySmall)
                    Text("执行计划：确认范围 → 检索 → 分段阅读 → 生成草稿${if (agentDeepReview) " → 深度复核" else ""} → 本机核验 → 等待确认保存", style = MaterialTheme.typography.bodySmall)
                }
                if (agentMode) items(agentAvailableDocuments, key = { "agent-doc-${it.id}" }) { note ->"####,
    )?;

    replace_once(
        &mut rendered,
        r####"                        agentMode -> "接收方：${configuration.recipientHost.ifBlank { "未配置" }} · 模型：${configuration.model.ifBlank { "未配置" }} · 已选 ${selectedAgentDocuments.size} 条资料，正文共 $selectedChars 字符；先发问题和工具说明，再按需发送读取片段，最多 6 次请求。""####,
        r####"                        agentMode -> "接收方：${configuration.recipientHost.ifBlank { "未配置" }} · 模型：${configuration.model.ifBlank { "未配置" }} · 已选 ${selectedAgentDocuments.size} 条资料，正文共 $selectedChars 字符；先发问题和工具说明，再按需发送读取片段；${if (agentDeepReview) "首轮最多 6 次请求，成功后再进行最多 6 次深度核验" else "最多 6 次请求"}。""####,
    )?;

    replace_once(
        &mut rendered,
        "                    if (agentMode) Text(\"每轮会携带此前的工具记录，资料读取累计上限 32,000 字符；请求设置 store:false，但自定义服务的留存规则由服务商决定。\", style = MaterialTheme.typography.bodySmall)",
        "                    if (agentMode) Text(\"每个阶段都会携带本阶段工具记录，单阶段资料读取累计上限 32,000 字符；请求设置 store:false，但自定义服务的留存规则由服务商决定。\", style = MaterialTheme.typography.bodySmall)",
    )?;

    replace_once(
        &mut rendered,
        "                        OutlinedTextField(value = agentDraftTitle, onValueChange = { agentDraftTitle = it }, label = { Text(\"新知识页标题\") }, modifier = Modifier.fillMaxWidth(), singleLine = true)\n                        OutlinedTextField(value = agentDraftBody, onValueChange = { agentDraftBody = it }, label = { Text(\"可编辑草稿内容与待办\") }, modifier = Modifier.fillMaxWidth(), minLines = 6, maxLines = 18)\n                        if (agentTrace.isNotEmpty()) {",
        "                        OutlinedTextField(value = agentDraftTitle, onValueChange = { agentDraftTitle = it; agentHumanEdited = true }, label = { Text(\"新知识页标题\") }, modifier = Modifier.fillMaxWidth(), singleLine = true)\n                        OutlinedTextField(value = agentDraftBody, onValueChange = { agentDraftBody = it; agentHumanEdited = true }, label = { Text(\"可编辑草稿内容与待办\") }, modifier = Modifier.fillMaxWidth(), minLines = 6, maxLines = 18)\n                        if (agentPlan.isNotEmpty()) {\n                            Text(\"执行计划\", fontWeight = FontWeight.SemiBold)\n                            agentPlan.forEachIndexed { index, step -> Text(\"${index + 1}. $step\", style = MaterialTheme.typography.bodySmall) }\n                        }\n                        if (agentVerification.isNotEmpty()) {\n                            Text(if (agentVerificationPassed) \"本机核验通过\" else \"本机核验未通过\", fontWeight = FontWeight.SemiBold,\n                                color = if (agentVerificationPassed) accentFor(\"green\") else MaterialTheme.colorScheme.error)\n                            agentVerification.forEach { line -> Text(line, style = MaterialTheme.typography.bodySmall) }\n                        }\n                        if (agentHumanEdited) Text(\"你已编辑 Agent 草稿；核验记录只覆盖模型生成版本，最终保存内容以当前编辑框为准。\", style = MaterialTheme.typography.bodySmall)\n                        if (agentTrace.isNotEmpty()) {",
    )?;

    replace_once(
        &mut rendered,
        "                                hasDraft = agentDraftBody.isNotBlank() && agentDraftTitle.isNotBlank() && agentResultSources.isNotEmpty(),",
        "                                hasDraft = agentVerificationPassed && agentDraftBody.isNotBlank() && agentDraftTitle.isNotBlank() && agentResultSources.isNotEmpty(),",
    )?;

    replace_once(&mut rendered,
        "    val latestAgentTaskId by rememberUpdatedState(agentTaskId)",
        "    val latestAgentTaskId by rememberUpdatedState(agentTaskId)\n    val latestAgentDeepReview by rememberUpdatedState(agentDeepReview)")?;
    replace_once(&mut rendered,
        "        val runConfiguration = configuration\n        val runContext = KnowledgeAgentDraftContext(workspaceKey, runIdentity, runConfiguration, safeQuestion, documents)",
        "        val runConfiguration = configuration\n        val runDeepReview = agentDeepReview\n        val runContext = KnowledgeAgentDraftContext(workspaceKey, runIdentity, runConfiguration, safeQuestion, documents, runDeepReview)")?;
    replace_once(&mut rendered,
        "        documents = latestSelectedAgentDocuments\n    )",
        "        documents = latestSelectedAgentDocuments,\n        deepReview = latestAgentDeepReview\n    )")?;
    let context_guard = "latestSelectedAgentDocuments == documents";
    if rendered.matches(context_guard).count() != 2 {
        return Err("Agent request context hooks are missing or ambiguous".to_owned());
    }
    rendered = rendered.replace(
        context_guard,
        "latestSelectedAgentDocuments == documents && latestAgentDeepReview == runDeepReview",
    );
    replace_once(&mut rendered,
        "        val hasDraft = agentDraft != null && agentDraftTitle.isNotBlank() && agentDraftBody.isNotBlank() && agentResultSources.isNotEmpty() && agentVerificationPassed",
        "        val hasDraft = KnowledgeAgentReviewPolicy.canSave(agentVerificationPassed, agentDraft != null && agentDraftTitle.isNotBlank() && agentDraftBody.isNotBlank() && agentResultSources.isNotEmpty())")?;
    replace_once(&mut rendered,
        "                                hasDraft = agentVerificationPassed && agentDraftBody.isNotBlank() && agentDraftTitle.isNotBlank() && agentResultSources.isNotEmpty(),",
        "                                hasDraft = KnowledgeAgentReviewPolicy.canSave(agentVerificationPassed, agentDraftBody.isNotBlank() && agentDraftTitle.isNotBlank() && agentResultSources.isNotEmpty()),")?;

    replace_once(&mut rendered,
        "        answer = \"\"; answerSources = emptyList()\n        scope.launch {",
        "        answer = \"\"; answerSources = emptyList()\n        scope.launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) {")?;

    // Whole-document AI is opened while the document editor still owns a saveable
    // state holder. SnapshotStateList is transient UI state and must not be registered
    // as a Bundle value when that holder is swapped for the AI dialog.
    replace_once(
        &mut rendered,
        "    val agentSelectedIds = rememberSaveable(workspaceKey, identity, selectedFolderId) { mutableStateListOf<String>() }",
        "    val agentSelectedIds = remember(workspaceKey, identity, selectedFolderId, priorityNoteId) { mutableStateListOf<String>() }",
    )?;

    // The document-level action must enter source-grounded mode immediately.
    replace_once(
        &mut rendered,
        "    var queryMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(KnowledgeAiMode.DIRECT) }",
        "    var queryMode by rememberSaveable(workspaceKey, identity, priorityNoteId) { mutableStateOf(knowledgeAiInitialMode(priorityNoteId)) }",
    )?;

    // Candidate extraction touches legacy and structured page projections. Treat a
    // malformed page as an unavailable source instead of allowing a composition-time
    // exception to terminate the Android process.
    replace_once(
        &mut rendered,
        r####"        knowledgeAiSourcesForMode(queryMode) {
            buildKnowledgeSourceCandidates(appData, question, selectedFolderId, searchScope, priorityNoteId)
        }"####,
        r####"        knowledgeAiSourcesForMode(queryMode) {
            runCatching {
                buildKnowledgeSourceCandidates(appData, question, selectedFolderId, searchScope, priorityNoteId)
            }.getOrElse { emptyList() }
        }"####,
    )?;

    // Re-resolve the page against current AppData before opening AI. The dialog only
    // carries a small id; the existing source builder keeps document text bounded.
    replace_once(
        &mut rendered,
        r####"                    onAskKnowledge = { note ->
                        knowledgePriorityNoteId = note.id
                        knowledgeDialogVisible = true
                    }"####,
        r####"                    onAskKnowledge = { note ->
                        val current = selectReadableKnowledgeSources(
                            appData.notes, note.id, NoteEntry::id,
                            isReadable = { it.kind == NoteEntryKind.DOCUMENT && !it.isDeleted() && it.encryption == null }
                        ).firstOrNull()
                        if (current == null) {
                            Toast.makeText(context, "当前文档不可读取；加密文档暂不纳入 AI 资料。", Toast.LENGTH_LONG).show()
                        } else {
                            knowledgePriorityNoteId = current.id
                            knowledgeDialogVisible = true
                        }
                    }"####,
    )?;

    // Resolve the selected document again on every source rebuild. Never fall back
    // to other pages when that document disappears, locks, or fails projection.
    replace_once(
        &mut rendered,
        r####"private fun buildKnowledgeSourceCandidates(
    appData: AppData,
    question: String,
    selectedFolderId: String?,
    searchScope: KnowledgeSearchScope,
    priorityNoteId: String?
): List<KnowledgeSourceCandidate> {
    val documents = appData.activeNotebookDocuments()
        .filter { note ->
            searchScope == KnowledgeSearchScope.ALL ||
                selectedFolderId == null ||
                note.folderId == selectedFolderId ||
                note.id == priorityNoteId
        }
    if (documents.isEmpty()) {
        return emptyList()
    }
    val foldersById = appData.noteFolders.associateBy(NoteFolder::id)
    val titles = documents.map { it.displayTitle() }.toTypedArray()
    val bodies = documents.map { it.searchableText().take(12_000) }.toTypedArray()
    val folderNames = documents
        .map { note -> foldersById[note.folderId]?.name.orEmpty() }
        .toTypedArray()
    val rankedNotes = NativeOptimizerBridge.rankKnowledgeSources(
        query = question,
        titles = titles,
        bodies = bodies,
        folders = folderNames
    ).asIterable().mapNotNull { index -> documents.getOrNull(index) }
    val priorityNote = priorityNoteId?.let { id -> documents.firstOrNull { it.id == id } }
    val orderedNotes = mutableListOf<NoteEntry>()
    priorityNote?.let(orderedNotes::add)
    rankedNotes.forEach { note ->
        if (orderedNotes.none { existing -> existing.id == note.id }) {
            orderedNotes.add(note)
        }
    }
    val finalNotes = orderedNotes.ifEmpty { documents }

    return finalNotes
        .take(5)
        .mapNotNull { note ->
            val excerpt = buildKnowledgeExcerpt(note, question)
            if (excerpt.isBlank()) {
                null
            } else {
                KnowledgeSourceCandidate(
                    note = note,
                    folderName = foldersById[note.folderId]?.name.orEmpty(),
                    excerpt = excerpt
                )
            }
        }
}"####,
        r####"private fun buildKnowledgeSourceCandidates(
    appData: AppData,
    question: String,
    selectedFolderId: String?,
    searchScope: KnowledgeSearchScope,
    priorityNoteId: String?
): List<KnowledgeSourceCandidate> {
    val documents = selectReadableKnowledgeSources(
        appData.activeNotebookDocuments(), priorityNoteId, NoteEntry::id,
        isReadable = { note ->
            note.kind == NoteEntryKind.DOCUMENT && !note.isDeleted() && note.encryption == null &&
                (searchScope == KnowledgeSearchScope.ALL || selectedFolderId == null ||
                    note.folderId == selectedFolderId || note.id == priorityNoteId)
        }
    )
    val foldersById = appData.noteFolders.associateBy(NoteFolder::id)
    val projections = projectKnowledgeSources(documents) { note ->
        Triple(note.displayTitle(), note.searchableText().take(12_000), note)
    }
    if (projections.isEmpty()) return emptyList()
    val rankedNotes = NativeOptimizerBridge.rankKnowledgeSources(
        query = question,
        titles = projections.map { it.first }.toTypedArray(),
        bodies = projections.map { it.second }.toTypedArray(),
        folders = projections.map { foldersById[it.third.folderId]?.name.orEmpty() }.toTypedArray()
    ).asIterable().mapNotNull { index -> projections.getOrNull(index)?.third }
    val priorityNote = priorityNoteId?.let { id -> projections.firstOrNull { it.third.id == id }?.third }
    val orderedNotes = mutableListOf<NoteEntry>()
    priorityNote?.let(orderedNotes::add)
    rankedNotes.forEach { note ->
        if (orderedNotes.none { existing -> existing.id == note.id }) orderedNotes.add(note)
    }
    val finalNotes = orderedNotes.ifEmpty { projections.map { it.third } }
    return projectKnowledgeSources(finalNotes.take(5)) { note ->
        val excerpt = buildKnowledgeExcerpt(note, question)
        if (excerpt.isBlank()) null else KnowledgeSourceCandidate(
            note = note,
            folderName = foldersById[note.folderId]?.name.orEmpty(),
            excerpt = excerpt
        )
    }
}"####,
    )?;

    Ok(rendered)
}
