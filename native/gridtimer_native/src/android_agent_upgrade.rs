use crate::ai_client::{self, AndroidAgentResult, AndroidAgentToolTrace};
use crate::android_agent_progress::{AgentProgress, AgentProgressEvent, AgentStage, AgentTerminal};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

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
    run_enhanced_android_knowledge_agent_with_progress(
        api_key, base_url, model, scope_json, task_id, cancelled, None,
    )
}

pub(crate) fn run_enhanced_android_knowledge_agent_with_progress(
    api_key: &str,
    base_url: &str,
    model: &str,
    scope_json: &str,
    task_id: &str,
    cancelled: &AtomicBool,
    progress: Option<&AgentProgress>,
) -> Value {
    let observer = |event| progress.is_none_or(|state| state.apply(event));
    run_enhanced_with_runner_and_progress(
        scope_json,
        task_id,
        cancelled,
        progress,
        |scope, review| {
            ai_client::run_android_knowledge_agent_observed(
                api_key, base_url, model, scope, cancelled, review, &observer,
            )
        },
    )
}

#[cfg(test)]
fn run_enhanced_with_runner<F>(
    scope_json: &str,
    task_id: &str,
    cancelled: &AtomicBool,
    runner: F,
) -> Value
where
    F: FnMut(&str, bool) -> AndroidAgentResult,
{
    run_enhanced_with_runner_and_progress(scope_json, task_id, cancelled, None, runner)
}

fn run_enhanced_with_runner_and_progress<F>(
    scope_json: &str,
    task_id: &str,
    cancelled: &AtomicBool,
    progress: Option<&AgentProgress>,
    mut runner: F,
) -> Value
where
    F: FnMut(&str, bool) -> AndroidAgentResult,
{
    let scope = parse_scope_meta(scope_json);
    let deep_review = scope.as_ref().is_some_and(|value| value.deep_review);
    let plan = build_plan(scope.as_ref());

    if let Some(progress) = progress {
        progress.apply(AgentProgressEvent::StageStarted(AgentStage::FirstPass));
    }
    let mut first = if cancelled.load(Ordering::Acquire) {
        AndroidAgentResult {
            ok: false,
            message: "本次 Agent 任务已取消，未发送请求".to_string(),
            answer: String::new(),
            draft: None,
            requests: 0,
            tool_calls: 0,
            tool_trace: Vec::new(),
            recipient_host: String::new(),
            model: String::new(),
        }
    } else {
        runner(scope_json, false)
    };
    let first_verification = verify_stage(&first, scope.as_ref());
    first.tool_trace = prefix_trace(first.tool_trace, "首轮");
    let first_requests = first.requests;
    let first_tool_calls = first.tool_calls;

    let mut review_attempted = false;
    let mut review_completed = false;
    let mut review_verification = None;
    let mut review_message = String::new();
    let mut final_result = first;

    if deep_review && first_verification.passed() && !cancelled.load(Ordering::Acquire) {
        review_attempted = true;
        // Both stages receive exactly the authorized scope and complete original goal.
        // Review instructions are supplied by the trusted client, outside source data.
        if let Some(progress) = progress {
            progress.apply(AgentProgressEvent::StageStarted(AgentStage::Review));
        }
        let mut reviewed = runner(scope_json, true);
        let stage = verify_stage(&reviewed, scope.as_ref());
        review_completed = stage.passed() && !cancelled.load(Ordering::Acquire);
        review_verification = Some(stage);
        reviewed.tool_trace = prefix_trace(reviewed.tool_trace, "复核");
        let review_requests = reviewed.requests;
        let review_tool_calls = reviewed.tool_calls;
        if review_completed {
            let mut combined_trace = final_result.tool_trace;
            combined_trace.extend(reviewed.tool_trace);
            reviewed.requests = first_requests.saturating_add(review_requests);
            reviewed.tool_calls = first_tool_calls.saturating_add(review_tool_calls);
            reviewed.tool_trace = combined_trace;
            reviewed.message =
                "Agent 已完成首轮草稿与第二阶段深度核验，尚未保存到知识库".to_string();
            final_result = reviewed;
        } else {
            final_result.requests = first_requests.saturating_add(review_requests);
            final_result.tool_calls = first_tool_calls.saturating_add(review_tool_calls);
            final_result.tool_trace.extend(reviewed.tool_trace);
            review_message = reviewed.message.trim().to_string();
            final_result.message =
                "首轮草稿已生成，但第二阶段深度核验没有完成；请重试，或关闭深度核验后重新运行。"
                    .to_string();
        }
    }

    if let Some(progress) = progress {
        progress.apply(AgentProgressEvent::Verifying);
    }
    let mut was_cancelled = cancelled.load(Ordering::Acquire);
    if was_cancelled {
        review_completed = false;
        final_result.ok = false;
        final_result.message = "本次 Agent 任务已取消，结果不能保存，请重新运行。".to_string();
    }

    let mut verification = verify_result(
        &verify_stage(&final_result, scope.as_ref()),
        &first_verification,
        review_verification.as_ref(),
        deep_review,
        review_completed,
        was_cancelled,
    );
    if let Some(progress) = progress {
        let terminal = if was_cancelled {
            AgentTerminal::Cancelled
        } else if verification.get("passed").and_then(Value::as_bool) == Some(true) {
            AgentTerminal::Ready
        } else if deep_review && first_verification.passed() && !review_completed {
            AgentTerminal::ReviewIncomplete
        } else {
            AgentTerminal::Failed
        };
        if progress.finish(terminal) == AgentTerminal::Cancelled && !was_cancelled {
            was_cancelled = true;
            review_completed = false;
            final_result.ok = false;
            final_result.message = "本次 Agent 任务已取消，结果不能保存，请重新运行。".to_string();
            verification = verify_result(
                &verify_stage(&final_result, scope.as_ref()),
                &first_verification,
                review_verification.as_ref(),
                deep_review,
                review_completed,
                was_cancelled,
            );
        }
    }
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
        object.insert(
            "plan".to_string(),
            Value::Array(plan.into_iter().map(Value::String).collect()),
        );
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
        deep_review: value
            .get("deepReview")
            .and_then(Value::as_bool)
            .unwrap_or(false),
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

fn prefix_trace(trace: Vec<AndroidAgentToolTrace>, stage: &str) -> Vec<AndroidAgentToolTrace> {
    trace
        .into_iter()
        .map(|mut item| {
            item.summary = format!("{stage} · {}", item.summary);
            item
        })
        .collect()
}

#[derive(Clone, Debug)]
struct AgentStageVerification {
    succeeded: bool,
    draft_complete: bool,
    tool_chain: bool,
    sources_valid: bool,
    source_count: usize,
    action_items_valid: bool,
}

impl AgentStageVerification {
    fn passed(&self) -> bool {
        self.succeeded
            && self.draft_complete
            && self.tool_chain
            && self.sources_valid
            && self.action_items_valid
    }
}

fn verify_stage(
    result: &AndroidAgentResult,
    scope: Option<&AgentScopeMeta>,
) -> AgentStageVerification {
    let draft = result.draft.as_ref();
    let authorized_ids = scope.map(|value| &value.authorized_ids);
    let source_ids = draft
        .map(|value| value.source_ids.as_slice())
        .unwrap_or(&[]);
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
    let completed_tool = |tool: &str| {
        result
            .tool_trace
            .iter()
            .any(|trace| trace.tool == tool && trace.result_count > 0 && trace.characters > 0)
    };
    let has_search = completed_tool("search_knowledge");
    let has_read = completed_tool("read_document");
    let has_draft_tool = completed_tool("propose_new_document");
    let draft_complete = draft.is_some_and(|value| {
        !value.title.trim().is_empty()
            && !value.content.trim().is_empty()
            && !result.answer.trim().is_empty()
    });
    AgentStageVerification {
        succeeded: result.ok,
        draft_complete,
        tool_chain: has_search && has_read && has_draft_tool,
        sources_valid: unique_sources && sources_authorized,
        source_count: source_ids.len(),
        action_items_valid,
    }
}

fn verify_result(
    final_stage: &AgentStageVerification,
    first_stage: &AgentStageVerification,
    review_stage: Option<&AgentStageVerification>,
    deep_review: bool,
    review_completed: bool,
    cancelled: bool,
) -> Value {
    let review_ok = !deep_review
        || (review_completed && review_stage.is_some_and(AgentStageVerification::passed));
    let tool_chain = first_stage.tool_chain
        && (!deep_review || review_stage.is_some_and(|stage| stage.tool_chain));
    let run_complete = !cancelled && final_stage.succeeded && first_stage.passed() && review_ok;
    let draft_complete = final_stage.draft_complete;
    let sources_valid = final_stage.sources_valid;
    let action_items_valid = final_stage.action_items_valid;

    let checks = vec![
        json!({"id":"run_complete","label":"任务有效完成","passed":run_complete,"detail":if cancelled {"任务已取消，不能保存"} else if run_complete {"所有要求的阶段均成功且通过本机核验"} else {"至少一个阶段失败或未通过本机核验"}}),
        json!({"id":"draft_complete","label":"草稿结构完整","passed":draft_complete,"detail":if draft_complete {"标题、正文和最终说明均存在"} else {"缺少标题、正文或最终说明"}}),
        json!({"id":"tool_chain","label":"工具链完整","passed":tool_chain,"detail":if tool_chain {"每个要求的阶段均独立检索、读取并生成内存草稿"} else {"至少一个阶段缺少成功的检索、读取或草稿步骤"}}),
        json!({"id":"sources","label":"来源仍在授权范围","passed":sources_valid,"detail":if sources_valid {format!("{} 个来源均来自本次授权资料", final_stage.source_count)} else {"来源缺失、重复或超出授权范围".to_string()}}),
        json!({"id":"tasks","label":"待办结构可用","passed":action_items_valid,"detail":if action_items_valid {"待办为空或每项均非空且没有重复"} else {"待办存在空项或重复项"}}),
        json!({"id":"deep_review","label":"第二阶段深度核验","passed":review_ok,"detail":if !deep_review {"本次未要求第二阶段核验"} else if review_completed {"已独立重新检索同一授权范围并生成复核草稿"} else {"深度核验未完成，保存前需要重新运行"}}),
        json!({"id":"write_boundary","label":"没有自动写入来源","passed":true,"detail":"Agent 只生成草稿；保存仍由你在界面确认"}),
    ];
    let passed = checks
        .iter()
        .all(|check| check.get("passed").and_then(Value::as_bool) == Some(true));
    json!({
        "passed": passed,
        "reviewed": review_completed,
        "checks": checks
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_client::AndroidAgentDraft;

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
                AndroidAgentToolTrace {
                    tool: "search_knowledge".to_string(),
                    summary: "检索".to_string(),
                    result_count: 2,
                    characters: 10,
                },
                AndroidAgentToolTrace {
                    tool: "read_document".to_string(),
                    summary: "读取".to_string(),
                    result_count: 1,
                    characters: 10,
                },
                AndroidAgentToolTrace {
                    tool: "propose_new_document".to_string(),
                    summary: "草稿".to_string(),
                    result_count: 1,
                    characters: 10,
                },
            ],
            recipient_host: "api.example.test".to_string(),
            model: "model".to_string(),
        }
    }

    #[test]
    fn missing_api_key_never_sends_requests_or_opens_the_save_gate() {
        let result = run_enhanced_android_knowledge_agent(
            "",
            "https://api.example.test/v1",
            "model",
            &sample_scope(true),
            "task",
            &AtomicBool::new(false),
        );
        assert_eq!(result["ok"], false);
        assert_eq!(result["requests"], 0);
        assert_eq!(result["reviewAttempted"], false);
        assert_eq!(result["verification"]["passed"], false);
    }

    #[test]
    fn both_stages_keep_the_complete_goal_and_identical_authorized_scope() {
        let mut scope: Value = serde_json::from_str(&sample_scope(true)).unwrap();
        let goal = format!("{}末尾限制必须保留", "目".repeat(992));
        assert_eq!(goal.chars().count(), 1000);
        scope["question"] = json!(goal);
        let scope_json = scope.to_string();
        let mut stages = Vec::new();
        let result = run_enhanced_with_runner(
            &scope_json,
            "task",
            &AtomicBool::new(false),
            |actual, review| {
                assert_eq!(actual, scope_json);
                assert_eq!(
                    serde_json::from_str::<Value>(actual).unwrap()["question"],
                    goal
                );
                stages.push(review);
                sample_result()
            },
        );
        assert_eq!(stages, vec![false, true]);
        assert_eq!(result["verification"]["passed"], true);
    }

    #[test]
    fn review_failure_keeps_first_draft_but_closes_the_save_gate_and_counts_both_stages() {
        let result = run_enhanced_with_runner(
            &sample_scope(true),
            "task",
            &AtomicBool::new(false),
            |_, review| {
                let mut result = sample_result();
                if review {
                    result.ok = false;
                    result.draft = None;
                    result.requests = 2;
                    result.tool_calls = 1;
                }
                result
            },
        );
        assert_eq!(result["draft"]["title"], "项目复盘");
        assert_eq!(result["reviewAttempted"], true);
        assert_eq!(result["reviewCompleted"], false);
        assert_eq!(result["verification"]["passed"], false);
        assert_eq!(result["requests"], 6);
        assert_eq!(result["toolCalls"], 4);
    }

    #[test]
    fn review_must_have_its_own_successful_search_read_and_draft_tools() {
        for tool in ["search_knowledge", "read_document", "propose_new_document"] {
            for failed_trace in [false, true] {
                let result = run_enhanced_with_runner(
                    &sample_scope(true),
                    "task",
                    &AtomicBool::new(false),
                    |_, review| {
                        let mut result = sample_result();
                        if review {
                            if failed_trace {
                                result
                                    .tool_trace
                                    .iter_mut()
                                    .find(|trace| trace.tool == tool)
                                    .unwrap()
                                    .result_count = 0;
                            } else {
                                result.tool_trace.retain(|trace| trace.tool != tool);
                            }
                        }
                        result
                    },
                );
                assert_eq!(result["verification"]["passed"], false, "{tool}");
                assert_eq!(result["reviewCompleted"], false, "{tool}");
            }
        }
    }

    #[test]
    fn failed_first_result_or_incomplete_tool_chain_never_starts_review_or_allows_save() {
        for failed_ok in [false, true] {
            let mut calls = 0;
            let result = run_enhanced_with_runner(
                &sample_scope(true),
                "task",
                &AtomicBool::new(false),
                |_, review| {
                    calls += 1;
                    assert!(!review);
                    let mut result = sample_result();
                    if failed_ok {
                        result.ok = false;
                    } else {
                        result
                            .tool_trace
                            .retain(|trace| trace.tool != "read_document");
                    }
                    result
                },
            );
            assert_eq!(calls, 1);
            assert_eq!(result["reviewAttempted"], false);
            assert_eq!(result["verification"]["passed"], false);
        }
    }

    #[test]
    fn cancellation_before_start_never_calls_a_runner() {
        let result = run_enhanced_with_runner(
            &sample_scope(true),
            "task",
            &AtomicBool::new(true),
            |_, _| panic!("cancelled task ran"),
        );
        assert_eq!(result["ok"], false);
        assert_eq!(result["requests"], 0);
        assert_eq!(result["verification"]["passed"], false);
    }

    #[test]
    fn cancellation_between_stages_stops_review_and_closes_the_save_gate() {
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let result =
            run_enhanced_with_runner(&sample_scope(true), "task", &cancelled, |_, review| {
                calls += 1;
                assert!(!review);
                cancelled.store(true, Ordering::Release);
                sample_result()
            });
        assert_eq!(calls, 1);
        assert_eq!(result["reviewAttempted"], false);
        assert_eq!(result["ok"], false);
        assert_eq!(result["verification"]["passed"], false);
    }

    #[test]
    fn cancellation_while_final_response_finishes_closes_single_and_reviewed_save_gates() {
        for deep_review in [false, true] {
            let cancelled = AtomicBool::new(false);
            let result = run_enhanced_with_runner(
                &sample_scope(deep_review),
                "task",
                &cancelled,
                |_, review| {
                    if !deep_review || review {
                        cancelled.store(true, Ordering::Release);
                    }
                    sample_result()
                },
            );
            assert_eq!(result["ok"], false);
            assert_eq!(result["reviewCompleted"], false);
            assert_eq!(result["verification"]["passed"], false);
            assert_eq!(result["requests"], if deep_review { 8 } else { 4 });
        }
    }

    #[test]
    fn successful_single_stage_or_reviewed_result_opens_the_save_gate_with_correct_counts() {
        for deep_review in [false, true] {
            let result = run_enhanced_with_runner(
                &sample_scope(deep_review),
                "task",
                &AtomicBool::new(false),
                |_, review| {
                    let mut result = sample_result();
                    if review {
                        result.draft.as_mut().unwrap().title = "复核草稿".to_string();
                    }
                    result
                },
            );
            assert_eq!(result["verification"]["passed"], true);
            assert_eq!(result["reviewCompleted"], deep_review);
            assert_eq!(result["requests"], if deep_review { 8 } else { 4 });
            assert_eq!(result["toolCalls"], if deep_review { 6 } else { 3 });
            assert_eq!(
                result["toolTrace"].as_array().unwrap().len(),
                if deep_review { 6 } else { 3 }
            );
            assert_eq!(
                result["draft"]["title"],
                if deep_review {
                    "复核草稿"
                } else {
                    "项目复盘"
                }
            );
        }
    }

    #[test]
    fn unauthorized_or_duplicate_sources_fail_local_verification() {
        for deep_review in [false, true] {
            for ids in [vec!["a", "outside"], vec!["a", "a"], vec![]] {
                let result = run_enhanced_with_runner(
                    &sample_scope(deep_review),
                    "task",
                    &AtomicBool::new(false),
                    |_, review| {
                        let mut result = sample_result();
                        if !deep_review || review {
                            result.draft.as_mut().unwrap().source_ids =
                                ids.iter().map(|id| id.to_string()).collect();
                        }
                        result
                    },
                );
                assert_eq!(result["verification"]["passed"], false);
            }
        }
    }

    #[test]
    fn duplicate_or_empty_actions_close_the_save_gate() {
        for actions in [vec!["补齐测试", " 补齐测试 "], vec![" "]] {
            let result = run_enhanced_with_runner(
                &sample_scope(false),
                "task",
                &AtomicBool::new(false),
                |_, _| {
                    let mut result = sample_result();
                    result.draft.as_mut().unwrap().action_items =
                        actions.iter().map(|item| item.to_string()).collect();
                    result
                },
            );
            assert_eq!(result["verification"]["passed"], false);
        }
    }
    #[test]
    fn progress_two_stages_emit_independent_reads_and_finish_after_verification() {
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let progress = AgentProgress::new("task", std::sync::Arc::clone(&flag));
        let result = run_enhanced_with_runner_and_progress(
            &sample_scope(true),
            "task",
            &flag,
            Some(&progress),
            |_, review| {
                let snapshot = serde_json::to_value(progress.snapshot()).unwrap();
                assert_eq!(
                    snapshot["stage"],
                    if review { "review" } else { "first_pass" }
                );
                assert_eq!(snapshot["documentsRead"], 0);
                assert!(progress.apply(AgentProgressEvent::RequestStarted));
                assert!(progress.apply(AgentProgressEvent::RequestFinished));
                for documents_read in [0, 2, 2] {
                    assert!(progress.apply(AgentProgressEvent::ToolFinished { documents_read }));
                }
                assert_eq!(
                    serde_json::to_value(progress.snapshot()).unwrap()["documentsRead"],
                    2
                );
                let mut result = sample_result();
                result.requests = 1;
                result
            },
        );
        let snapshot = serde_json::to_value(progress.snapshot()).unwrap();
        assert_eq!(snapshot["terminal"], "ready");
        assert_eq!(snapshot["stage"], "verifying");
        assert_eq!(snapshot["activity"], "finished");
        assert_eq!(snapshot["requestInFlight"], false);
        assert_eq!(snapshot["requests"], result["requests"]);
        assert_eq!(snapshot["toolCalls"], result["toolCalls"]);
        assert_eq!(snapshot["documentsRead"], 0);
        assert_eq!(result["verification"]["passed"], true);
    }

    #[test]
    fn progress_failed_and_incomplete_review_never_report_ready() {
        for review_failure in [false, true] {
            let flag = std::sync::Arc::new(AtomicBool::new(false));
            let progress = AgentProgress::new("task", std::sync::Arc::clone(&flag));
            let result = run_enhanced_with_runner_and_progress(
                &sample_scope(true),
                "task",
                &flag,
                Some(&progress),
                |_, review| {
                    if !review_failure || review {
                        {
                            let mut failed = sample_result();
                            failed.ok = false;
                            failed.message = "transport failed".to_string();
                            failed.answer.clear();
                            failed.draft = None;
                            failed
                        }
                    } else {
                        sample_result()
                    }
                },
            );
            let snapshot = serde_json::to_value(progress.snapshot()).unwrap();
            assert_eq!(
                snapshot["terminal"],
                if review_failure {
                    "review_incomplete"
                } else {
                    "failed"
                }
            );
            assert_eq!(snapshot["requestInFlight"], false);
            assert_eq!(result["verification"]["passed"], false);
            assert_eq!(result["ok"], review_failure);
        }
    }

    #[test]
    fn progress_cancellation_before_registration_or_during_stage_cannot_open_save_gate() {
        for before_start in [false, true] {
            let flag = std::sync::Arc::new(AtomicBool::new(before_start));
            let progress = AgentProgress::new("task", std::sync::Arc::clone(&flag));
            let mut calls = 0;
            let result = run_enhanced_with_runner_and_progress(
                &sample_scope(true),
                "task",
                &flag,
                Some(&progress),
                |_, _| {
                    calls += 1;
                    progress.request_cancel();
                    sample_result()
                },
            );
            assert_eq!(calls, usize::from(!before_start));
            assert_eq!(result["ok"], false);
            assert_eq!(result["verification"]["passed"], false);
            let snapshot = serde_json::to_value(progress.snapshot()).unwrap();
            assert_eq!(snapshot["terminal"], "cancelled");
            assert_eq!(snapshot["cancelRequested"], true);
            assert_eq!(snapshot["requestInFlight"], false);
        }
    }
}
