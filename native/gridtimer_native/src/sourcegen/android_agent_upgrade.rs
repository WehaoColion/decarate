// Android Agent v2: expose a visible plan, optional second-pass review and local verification.

pub const SCREEN: &str = "com/ofairyo/gridtimer/ui/NoteStudioSheet.kt";

fn replace_once(source: &mut String, before: &str, after: &str) -> Result<(), String> {
    if source.matches(before).count() != 1 {
        return Err(format!("Android agent upgrade anchor is not unique: {before}"));
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
        "        val scopeJson = JSONObject().put(\"question\", safeQuestion).put(\"deepReview\", agentDeepReview).put(\"documents\", JSONArray().apply {",
    )?;

    replace_once(
        &mut rendered,
        "        agentResultSources = emptyList(); agentTrace = emptyList(); statusText = \"正在连接 ${runConfiguration.recipientHost}，最多执行 6 次模型请求。\"",
        "        agentResultSources = emptyList(); agentPlan = emptyList(); agentTrace = emptyList(); agentVerification = emptyList(); agentVerificationPassed = false; agentHumanEdited = false\n        statusText = if (agentDeepReview) \"正在连接 ${runConfiguration.recipientHost}；首轮完成后将自动进行第二阶段深度核验。\" else \"正在连接 ${runConfiguration.recipientHost}，最多执行 6 次模型请求。\"",
    )?;

    let reset = "agentResultSources = emptyList(); agentTrace = emptyList()";
    let reset_count = rendered.matches(reset).count();
    if reset_count != 2 {
        return Err(format!("expected two Agent reset anchors after request setup, found {reset_count}"));
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

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrelated_files_pass_through() {
        assert_eq!(render("other.kt", "sentinel").unwrap(), "sentinel");
    }

    #[test]
    fn duplicate_anchor_is_rejected() {
        let mut source = "needle needle".to_string();
        assert!(replace_once(&mut source, "needle", "value").is_err());
    }
}
