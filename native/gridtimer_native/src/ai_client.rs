use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const DEFAULT_AI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_AI_MODEL: &str = "gpt-5.2";
pub const DEFAULT_AI_REASONING_EFFORT: &str = "low";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCompletionResult {
    pub ok: bool,
    pub message: String,
    pub content: String,
}

impl AiCompletionResult {
    pub fn ok(content: String) -> Self {
        Self {
            ok: true,
            message: "已完成".to_string(),
            content,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            content: String::new(),
        }
    }
}

pub fn complete_note(
    api_key: &str,
    base_url: &str,
    model: &str,
    title: &str,
    content: &str,
    user_instruction: &str,
) -> AiCompletionResult {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return AiCompletionResult::error("请先填写 OpenAI API Key");
    }

    let base_url = match normalize_base_url(base_url) {
        Ok(value) => value,
        Err(message) => return AiCompletionResult::error(message),
    };
    let model = model.trim();
    let model = if model.is_empty() {
        DEFAULT_AI_MODEL
    } else {
        model
    };

    let instruction = user_instruction.trim();
    let instruction = if instruction.is_empty() {
        "整理这篇笔记，保留事实，不编造细节。输出可直接替换正文的内容。"
    } else {
        instruction
    };

    send_responses_request(
        api_key,
        &base_url,
        model,
        1400,
        "你是一个安静、克制的笔记整理助手。只输出笔记正文，不解释过程，不加寒暄。保留用户原意，中文表达自然，必要时整理成标题、要点或待办。",
        &format!(
            "动作：{instruction}\n\n标题：{}\n\n正文：\n{}",
            title.trim(),
            content.trim()
        ),
    )
}

pub fn complete_knowledge_query(
    api_key: &str,
    base_url: &str,
    model: &str,
    question: &str,
    source_titles: &[String],
    source_folders: &[String],
    source_excerpts: &[String],
) -> AiCompletionResult {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return AiCompletionResult::error("请先填写 OpenAI API Key");
    }

    let base_url = match normalize_base_url(base_url) {
        Ok(value) => value,
        Err(message) => return AiCompletionResult::error(message),
    };
    let model = model.trim();
    let model = if model.is_empty() {
        DEFAULT_AI_MODEL
    } else {
        model
    };
    let question = question.trim();
    if question.is_empty() {
        return AiCompletionResult::error("先输入一个问题");
    }

    let Some(user_prompt) =
        build_knowledge_user_prompt(question, source_titles, source_folders, source_excerpts)
    else {
        return AiCompletionResult::error("没有找到可用知识来源");
    };

    send_responses_request(
        api_key,
        &base_url,
        model,
        1800,
        "你是私人知识库问答助手。只能依据用户提供的知识来源回答；来源不足时直接说缺什么，不编造。回答要短、准、可执行。引用事实时用 [1]、[2] 这样的来源编号。",
        &user_prompt,
    )
}

fn send_responses_request(
    api_key: &str,
    base_url: &str,
    model: &str,
    max_output_tokens: usize,
    developer_prompt: &str,
    user_prompt: &str,
) -> AiCompletionResult {
    let request_body = json!({
        "model": model,
        "reasoning": { "effort": DEFAULT_AI_REASONING_EFFORT },
        "max_output_tokens": max_output_tokens,
        "input": [
            {
                "role": "developer",
                "content": developer_prompt
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ]
    });

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .build();
    let endpoint = format!("{base_url}/responses");
    let response = agent
        .post(&endpoint)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_string(&request_body.to_string());

    let raw = match response {
        Ok(response) => match response.into_string() {
            Ok(raw) => raw,
            Err(error) => return AiCompletionResult::error(format!("读取响应失败：{error}")),
        },
        Err(ureq::Error::Status(status, response)) => {
            let raw = response.into_string().unwrap_or_default();
            let detail = openai_error_message(&raw).unwrap_or_else(|| raw.trim().to_string());
            let message = if detail.is_empty() {
                format!("OpenAI 请求失败：HTTP {status}")
            } else {
                format!("OpenAI 请求失败：HTTP {status}，{detail}")
            };
            return AiCompletionResult::error(message);
        }
        Err(error) => {
            return AiCompletionResult::error(format!("OpenAI 请求失败：{error}"));
        }
    };

    match extract_output_text(&raw) {
        Some(text) if !text.trim().is_empty() => AiCompletionResult::ok(text.trim().to_string()),
        _ => AiCompletionResult::error(
            openai_error_message(&raw).unwrap_or_else(|| "OpenAI 响应里没有可用文本".to_string()),
        ),
    }
}

fn build_knowledge_user_prompt(
    question: &str,
    source_titles: &[String],
    source_folders: &[String],
    source_excerpts: &[String],
) -> Option<String> {
    let mut sources = Vec::new();
    for index in 0..source_titles.len().min(source_excerpts.len()).min(6) {
        let title = trim_to_chars(source_titles[index].trim(), 80);
        let folder = source_folders
            .get(index)
            .map(|value| trim_to_chars(value.trim(), 40))
            .unwrap_or_default();
        let excerpt = trim_to_chars(source_excerpts[index].trim(), 900);
        if excerpt.is_empty() {
            continue;
        }
        let display_title = if title.is_empty() {
            format!("知识来源 {}", index + 1)
        } else {
            title
        };
        let folder_suffix = if folder.is_empty() {
            String::new()
        } else {
            format!(" / {folder}")
        };
        sources.push(format!(
            "[{}] {}{}\n{}",
            sources.len() + 1,
            display_title,
            folder_suffix,
            excerpt
        ));
    }
    if sources.is_empty() {
        return None;
    }

    Some(format!(
        "问题：{}\n\n知识来源：\n{}\n\n回答要求：\n- 只根据上面的来源回答。\n- 不确定就说缺少哪类来源。\n- 关键结论后标注来源编号。",
        trim_to_chars(question, 300),
        sources.join("\n\n")
    ))
}

fn trim_to_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect::<String>()
}

pub fn complete_note_json(
    api_key: &str,
    base_url: &str,
    model: &str,
    title: &str,
    content: &str,
    user_instruction: &str,
) -> String {
    serde_json::to_string(&complete_note(
        api_key,
        base_url,
        model,
        title,
        content,
        user_instruction,
    ))
    .unwrap_or_else(|_| r#"{"ok":false,"message":"无法编码 OpenAI 响应","content":""}"#.to_string())
}

fn normalize_base_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_end_matches('/');
    let base_url = if trimmed.is_empty() {
        DEFAULT_AI_BASE_URL
    } else {
        trimmed
    };
    if base_url.starts_with("https://") || is_local_http_base_url(base_url) {
        Ok(base_url.to_string())
    } else {
        Err("接口地址需要使用 https，或本机 http 调试地址".to_string())
    }
}

fn is_local_http_base_url(value: &str) -> bool {
    value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("http://[::1]:")
}

fn openai_error_message(raw: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_output_text(raw: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    if let Some(text) = value.get("output_text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    let mut parts = Vec::new();
    collect_output_text(&value, &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(""))
    }
}

fn collect_output_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_output_text(item, parts);
            }
        }
        Value::Object(map) => {
            let is_output_text = map
                .get("type")
                .and_then(Value::as_str)
                .map(|kind| kind == "output_text")
                .unwrap_or(false);
            if is_output_text {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            if let Some(output) = map.get("output") {
                collect_output_text(output, parts);
            }
            if let Some(content) = map.get("content") {
                collect_output_text(content, parts);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_top_level_output_text() {
        let raw = r#"{"output_text":"整理好了"}"#;
        assert_eq!("整理好了", extract_output_text(raw).unwrap());
    }

    #[test]
    fn extracts_nested_output_text() {
        let raw = r#"{"output":[{"content":[{"type":"output_text","text":"第一段"},{"type":"output_text","text":"第二段"}]}]}"#;
        assert_eq!("第一段第二段", extract_output_text(raw).unwrap());
    }

    #[test]
    fn rejects_non_https_remote_base_url() {
        assert!(normalize_base_url("http://example.com/v1").is_err());
        assert!(normalize_base_url("http://127.0.0.1:8080/v1").is_ok());
    }

    #[test]
    fn builds_knowledge_prompt_with_numbered_sources() {
        let titles = vec!["项目页".to_string(), "空来源".to_string()];
        let folders = vec!["学习".to_string()];
        let excerpts = vec!["Rust 负责核心逻辑。".to_string(), "".to_string()];
        let prompt = build_knowledge_user_prompt("这个项目怎么实现？", &titles, &folders, &excerpts)
            .unwrap();
        assert!(prompt.contains("[1] 项目页 / 学习"));
        assert!(prompt.contains("Rust 负责核心逻辑。"));
        assert!(!prompt.contains("[2]"));
    }
}
