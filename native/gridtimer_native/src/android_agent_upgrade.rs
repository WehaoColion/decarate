use crate::ai_client::{self, AndroidAgentDraft, AndroidAgentResult, AndroidAgentToolTrace};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

const REVIEW_GOAL_CHARS: usize = 520;

#[derive(Clone, Debug)]
struct AgentScopeMeta {
    question: String,
    authorized_ids: HashSet<String>,
    document_count: usize,
    deep_review: bool,
}

pub fn run_enhanced_android_knowledge_agent(
    api_key: &str,
    base_url: &str,
    model: &str,
    scope_json: &str,
    task_id: &str,
    cancelled: &AtomicBool,
) -> Value {
    let scope = parse_scope_meta(scope_json);
    let deep_review = scope.as_ref().is_some_and(|value| value.deep_review);
    let plan = build_plan(scope.as_ref());

    let mut first = ai_client::run_android_knowledge_agent(
        api_key,
        base_url,
        model,
        scope_json,
        cancelled,
    );
    first.tool_trace = prefix_trace(first.tool_trace, "首轮");
    let first_requests = first.requests;
    let first_tool_calls = first.tool_calls;

    let mut review_attempted = false;
    let mut review_completed = false;
    let mut review_message = String::new();
    let mut final_result = first;

    if deep_review && final_result.ok && final_result.draft.is_some() && !cancelled.load(Ordering::Acquire) {
        review_attempted = true;
        if let Some(review_scope) = build_review_scope(scope_json, scope.as_ref()) {
            let mut reviewed = ai_client::run_android_knowledge_agent(
                api_key,
                base_url,
                model,
                &review_scope,
                cancelled,
            );
            reviewed.tool_trace = prefix_trace(reviewed.tool_trace, "复核");
            let review_requests = reviewed.requests;
            let review_tool_calls = reviewed.tool_calls;
            if reviewed.ok && reviewed.draft.is_some() {
                review_completed = true;
                let mut combined_trace = final_result.tool_trace;
                combined_trace.extend(reviewed.tool_trace);
                reviewed.requests = first_requests.saturating_add(review_requests);
                reviewed.tool_calls = first_tool_calls.saturating_add(review_tool_calls);
                reviewed.tool_trace = combined_trace;
                reviewed.message = "Agent 已完成首轮草稿与第二阶段深度核验，尚未保存到知识库".to_string();
                final_result = reviewed;
            } else {
                final_result.requests = first_requests.saturating_add(review_requests);
                final_result.tool_calls = first_tool_calls.saturating_add(review_tool_calls);
                final_result.tool_trace.extend(reviewed.tool_trace);
                review_message = reviewed.message.trim().to_string();
                final_result.message = "首轮草稿已生成，但第二阶段深度核验没有完成；请重试，或关闭深度核验后重新运行。".to_string();
            }
        } else {
            review_message = "无法建立第二阶段核验上下文".to_string();
            final_result.message = "首轮草稿已生成，但第二阶段深度核验上下文无效；请重新运行。".to_string();
        }
    }

    let verification = verify_result(
        &final_result,
        scope.as_ref(),
        deep_review,
        review_attempted,
        review_completed,
    );
    let mut value = serde_json::to_value(&final_result).unwrap_or_else(|_| {
        json!({
            "ok": false,
            "message": "Agent 结果无法编码，请重试",
            "answer": "",
            "draft": Value::Null,
            "requests": 0,
            "toolCalls": 0,
            "toolTrace": [],
            "recipientHost": "",
            "model": ""
        })
    });
    if let Some(object) = value.as_object_mut() {
        object.insert("taskId".to_string(), Value::String(task_id.to_string()));
        object.insert("plan".to_string(), Value::Array(plan.into_iter().map(Value::String).collect()));
        object.insert("deepReviewRequested".to_string(), Value::Bool(deep_review));
        object.insert("reviewAttempted".to_string(), Value::Bool(review_attempted));
        object.insert("reviewCompleted".to_string(), Value::Bool(review_completed));
        object.insert("verification".to_string(), verification);
        if !review_message.is_empty() {
            object.insert("reviewMessage".to_string(), Value::String(review_message));
        }
    }
    value
}

fn parse_scope_meta(scope_json: &str) -> Option<AgentScopeMeta> {
    let value: Value = serde_json::from_str(scope_json).ok()?;
    let question = value.get("question")?.as_str()?.trim().to_string();
    let documents = value.get("documents")?.as_array()?;
    let authorized_ids = documents
        .iter()
        .filter_map(|document| document.get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();
    Some(AgentScopeMeta {
        question,
        authorized_ids,
        document_count: documents.len(),
        deep_review: value.get("deepReview").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn build_plan(scope: Option<&AgentScopeMeta>) -> Vec<String> {
    let document_count = scope.map_or(0, |value| value.document_count);
    let deep_review = scope.is_some_and(|value| value.deep_review);
    let goal = scope.map_or("", |value| value.question.as_str());
    let mut plan = vec![
        format!("确认本次授权范围（{document_count} 条资料）"),
        "在授权资料中检索与目标最相关的内容".to_string(),
        "按需分段读取原文并保留真实来源".to_string(),
    ];
    if contains_any(goal, &["比较", "对比", "冲突", "一致", "差异"]) {
        plan.push("交叉比较多个来源，明确一致、冲突和证据不足之处".to_string());
    }
    if contains_any(goal, &["待办", "计划", "下一步", "未完成", "行动"]) {
        plan.push("提取尚未完成的行动项，不编造负责人和期限".to_string());
    }
    if contains_any(goal, &["复盘", "总结", "归纳", "整理"]) {
        plan.push("区分已确认事实、问题和下一步，避免把推测写成结论".to_string());
    }
    plan.push("生成新的知识页草稿和可选待办，不修改来源".to_string());
    if deep_review {
        plan.push("第二阶段独立重新检索同一资料，复核冲突、遗漏和无来源结论".to_string());
    }
    plan.extend([
        "本机核验来源、工具链和草稿结构".to_string(),
        "等待你检查、编辑并确认保存".to_string(),
    ]);
    plan
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn build_review_scope(scope_json: &str, scope: Option<&AgentScopeMeta>) -> Option<String> {
    let mut value: Value = serde_json::from_str(scope_json).ok()?;
    let goal = clip_chars(scope.map_or("", |value| value.question.as_str()), REVIEW_GOAL_CHARS);
    let question = format!(
        "第二阶段深度核验。原任务：{goal}\n请独立重新执行这个目标：重新检索并读取同一批授权资料，重点寻找事实冲突、遗漏、无来源结论和不必要推断，然后生成一份完整替代草稿。不得依赖首轮草稿内容，不得引入授权资料之外的事实，也不得声称已经保存或执行待办。"
    );
    if question.chars().count() > 1000 {
        return None;
    }
    value["question"] = Value::String(question);
    value["deepReview"] = Value::Bool(false);
    Some(value.to_string())
}

fn clip_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let clipped = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn prefix_trace(trace: Vec<AndroidAgentToolTrace>, stage: &str) -> Vec<AndroidAgentToolTrace> {
    trace
        .into_iter()
        .map(|mut item| {
            item.summary = format!("{stage} · {}", item.summary);
            item
        })
        .collect()
}

fn verify_result(
    result: &AndroidAgentResult,
    scope: Option<&AgentScopeMeta>,
    deep_review: bool,
    review_attempted: bool,
    review_completed: bool,
) -> Value {
    let draft = result.draft.as_ref();
    let authorized_ids = scope.map(|value| &value.authorized_ids);
    let source_ids = draft.map(|value| value.source_ids.as_slice()).unwrap_or(&[]);
    let unique_sources = source_ids.iter().collect::<HashSet<_>>().len() == source_ids.len();
    let sources_authorized = !source_ids.is_empty()
        && authorized_ids.is_some_and(|allowed| source_ids.iter().all(|id| allowed.contains(id)));
    let action_items_valid = draft.is_some_and(|value| {
        let normalized = value
            .action_items
            .iter()
            .map(|item| item.trim().to_lowercase())
            .collect::<Vec<_>>();
        normalized.iter().all(|item| !item.is_empty())
            && normalized.iter().collect::<HashSet<_>>().len() == normalized.len()
    });
    let has_search = result.tool_trace.iter().any(|trace| trace.tool == "search_knowledge");
    let has_read = result.tool_trace.iter().any(|trace| trace.tool == "read_document");
    let has_draft_tool = result.tool_trace.iter().any(|trace| trace.tool == "propose_new_document");
    let draft_complete = draft.is_some_and(|value| {
        !value.title.trim().is_empty() && !value.content.trim().is_empty() && !result.answer.trim().is_empty()
    });
    let review_ok = !deep_review || (review_attempted && review_completed);

    let checks = vec![
        json!({"id":"draft_complete","label":"草稿结构完整","passed":draft_complete,"detail":if draft_complete {"标题、正文和最终说明均存在"} else {"缺少标题、正文或最终说明"}}),
        json!({"id":"tool_chain","label":"工具链完整","passed":has_search && has_read && has_draft_tool,"detail":if has_search && has_read && has_draft_tool {"已检索、读取并生成内存草稿"} else {"缺少检索、读取或草稿步骤"}}),
        json!({"id":"sources","label":"来源仍在授权范围","passed":unique_sources && sources_authorized,"detail":if unique_sources && sources_authorized {format!("{} 个来源均来自本次授权资料", source_ids.len())} else {"来源缺失、重复或超出授权范围".to_string()}}),
        json!({"id":"tasks","label":"待办结构可用","passed":action_items_valid,"detail":if action_items_valid {"待办为空或每项均非空且没有重复"} else {"待办存在空项或重复项"}}),
        json!({"id":"deep_review","label":"第二阶段深度核验","passed":review_ok,"detail":if !deep_review {"本次未要求第二阶段核验"} else if review_completed {"已独立重新检索同一授权范围并生成复核草稿"} else {"深度核验未完成，保存前需要重新运行"}}),
        json!({"id":"write_boundary","label":"没有自动写入来源","passed":true,"detail":"Agent 只生成草稿；保存仍由你在界面确认"}),
    ];
    let passed = checks.iter().all(|check| check.get("passed").and_then(Value::as_bool) == Some(true));
    json!({
        "passed": passed,
        "reviewed": review_completed,
        "checks": checks
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scope(deep_review: bool) -> String {
        json!({
            "question":"整理项目进展并找出未完成事项和冲突",
            "deepReview":deep_review,
            "documents":[
                {"id":"a","title":"项目记录","folder":"工作","content":"第一阶段完成，测试尚未完成。"},
                {"id":"b","title":"会议","folder":"工作","content":"需要补测试和发布说明。"}
            ]
        }).to_string()
    }

    fn sample_result() -> AndroidAgentResult {
        AndroidAgentResult {
            ok: true,
            message: "完成".to_string(),
            answer: "已整理并核对来源。".to_string(),
            draft: Some(AndroidAgentDraft {
                title: "项目复盘".to_string(),
                content: "第一阶段完成，测试仍待补齐。".to_string(),
                action_items: vec!["补齐测试".to_string()],
                source_ids: vec!["a".to_string(), "b".to_string()],
            }),
            requests: 4,
            tool_calls: 3,
            tool_trace: vec![
                AndroidAgentToolTrace { tool: "search_knowledge".to_string(), summary: "检索".to_string(), result_count: 2, characters: 10 },
                AndroidAgentToolTrace { tool: "read_document".to_string(), summary: "读取".to_string(), result_count: 1, characters: 10 },
                AndroidAgentToolTrace { tool: "propose_new_document".to_string(), summary: "草稿".to_string(), result_count: 1, characters: 10 },
            ],
            recipient_host: "api.example.test".to_string(),
            model: "model".to_string(),
        }
    }

    #[test]
    fn review_scope_is_bounded_independent_and_keeps_the_same_documents() {
        let scope_json = sample_scope(true);
        let scope = parse_scope_meta(&scope_json).unwrap();
        let review = build_review_scope(&scope_json, Some(&scope)).unwrap();
        let value: Value = serde_json::from_str(&review).unwrap();
        assert_eq!(value["documents"].as_array().unwrap().len(), 2);
        assert_eq!(value["deepReview"], false);
        assert!(value["question"].as_str().unwrap().chars().count() <= 1000);
        assert!(value["question"].as_str().unwrap().contains("独立重新执行"));
        assert!(!value["question"].as_str().unwrap().contains("项目复盘"));
    }

    #[test]
    fn task_plan_adds_goal_specific_comparison_and_action_steps() {
        let scope_json = sample_scope(true);
        let scope = parse_scope_meta(&scope_json).unwrap();
        let plan = build_plan(Some(&scope));
        assert!(plan.iter().any(|step| step.contains("交叉比较")));
        assert!(plan.iter().any(|step| step.contains("行动项")));
        assert!(plan.iter().any(|step| step.contains("第二阶段")));
    }

    #[test]
    fn local_verification_rejects_missing_review_when_requested() {
        let scope_json = sample_scope(true);
        let scope = parse_scope_meta(&scope_json).unwrap();
        let result = sample_result();
        let verification = verify_result(&result, Some(&scope), true, true, false);
        assert_eq!(verification["passed"], false);
        assert_eq!(verification["reviewed"], false);
    }

    #[test]
    fn local_verification_accepts_complete_authorized_result() {
        let scope_json = sample_scope(false);
        let scope = parse_scope_meta(&scope_json).unwrap();
        let result = sample_result();
        let verification = verify_result(&result, Some(&scope), false, false, false);
        assert_eq!(verification["passed"], true);
    }

    #[test]
    fn unauthorized_or_duplicate_sources_fail_local_verification() {
        let scope_json = sample_scope(false);
        let scope = parse_scope_meta(&scope_json).unwrap();
        let mut result = sample_result();
        result.draft.as_mut().unwrap().source_ids = vec!["a".to_string(), "outside".to_string()];
        assert_eq!(verify_result(&result, Some(&scope), false, false, false)["passed"], false);
        result.draft.as_mut().unwrap().source_ids = vec!["a".to_string(), "a".to_string()];
        assert_eq!(verify_result(&result, Some(&scope), false, false, false)["passed"], false);
    }
}
