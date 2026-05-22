use crate::agent::provider::ProviderDecision;
use crate::agent::session::TurnHistoryMessage;
use crate::agent::tools::ToolCall;
use serde_json::json;

pub trait TurnPlanner: Send {
    fn preflight_decision(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
    ) -> Option<ProviderDecision>;

    fn select_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall>;
}

pub struct LocalTurnPlanner;

impl TurnPlanner for LocalTurnPlanner {
    fn preflight_decision(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
    ) -> Option<ProviderDecision> {
        Self::preflight_tool_decision(user_message, history)
    }

    fn select_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        provider_tool_call: Option<ToolCall>,
    ) -> Option<ToolCall> {
        provider_tool_call.or_else(|| Self::infer_local_tool_call(user_message, history))
    }
}

impl LocalTurnPlanner {
    fn infer_local_tool_call(
        user_message: &str,
        history: &[TurnHistoryMessage],
    ) -> Option<ToolCall> {
        let lowered = user_message.to_lowercase();
        let explicit_path = Self::extract_explicit_path(user_message);
        let referenced_path = explicit_path.or_else(|| Self::infer_last_referenced_path(history));

        if contains_any(&lowered, &["time", "时间", "几点", "timestamp"]) {
            return Some(ToolCall {
                call_id: None,
                name: "time_now".to_string(),
                arguments: json!({}),
            });
        }

        if contains_any(
            &lowered,
            &["有哪些文件", "文件夹里", "目录里", "list files", "列出文件"],
        ) {
            return Some(ToolCall {
                call_id: None,
                name: "workspace_list_files".to_string(),
                arguments: json!({
                    "path": referenced_path.unwrap_or_else(|| ".".to_string()),
                    "limit": 60,
                }),
            });
        }

        if contains_any(&lowered, &["搜索", "查找", "find ", "grep", "包含"]) {
            if let Some(query) =
                Self::extract_search_query(user_message, referenced_path.as_deref())
            {
                if let Some(path) = referenced_path.clone() {
                    return Some(ToolCall {
                        call_id: None,
                        name: "workspace_gather_context".to_string(),
                        arguments: json!({
                            "path": path,
                            "query": query,
                            "lineCount": 80,
                            "limit": 20,
                        }),
                    });
                }

                return Some(ToolCall {
                    call_id: None,
                    name: "workspace_search_text".to_string(),
                    arguments: json!({
                        "query": query,
                        "path": ".",
                        "limit": 20,
                        "ignoreCase": true,
                    }),
                });
            }
        }

        if contains_any(
            &lowered,
            &[
                "定义",
                "引用",
                "调用",
                "出现",
                "报错",
                "错误",
                "error",
                "where is",
                "where are",
            ],
        ) {
            if let Some(query) =
                Self::extract_search_query(user_message, referenced_path.as_deref())
            {
                if let Some(path) = referenced_path {
                    return Some(ToolCall {
                        call_id: None,
                        name: "workspace_gather_context".to_string(),
                        arguments: json!({
                            "path": path,
                            "query": query,
                            "lineCount": 80,
                            "limit": 20,
                        }),
                    });
                }

                return Some(ToolCall {
                    call_id: None,
                    name: "workspace_search_text".to_string(),
                    arguments: json!({
                        "query": query,
                        "path": ".",
                        "limit": 20,
                        "ignoreCase": true,
                    }),
                });
            }
        }

        if let Some(path) = referenced_path {
            if contains_any(
                &lowered,
                &[
                    "什么文件",
                    "是什么文件",
                    "这个文件",
                    "这个目录",
                    "what is",
                    "path info",
                    "看一个",
                    "看看",
                    "查看",
                    "内容",
                    "实现",
                    "源码",
                    "read",
                    "show",
                    "分析",
                    "解释",
                ],
            ) {
                return Some(ToolCall {
                    call_id: None,
                    name: "workspace_gather_context".to_string(),
                    arguments: json!({
                        "path": path,
                        "lineCount": 80,
                        "limit": 40,
                    }),
                });
            }

            return Some(ToolCall {
                call_id: None,
                name: "workspace_gather_context".to_string(),
                arguments: json!({
                    "path": path,
                    "lineCount": 80,
                    "limit": 40,
                }),
            });
        }

        None
    }

    fn preflight_tool_decision(
        user_message: &str,
        history: &[TurnHistoryMessage],
    ) -> Option<ProviderDecision> {
        let tool_call = Self::infer_local_tool_call(user_message, history)?;
        let context_hint = history
            .iter()
            .rev()
            .find(|message| message.role == "assistant")
            .map(|message| preview_text(&message.content, 80))
            .unwrap_or_else(|| "none".to_string());

        Some(ProviderDecision {
            output_text: format!(
                "Using tool `{}` for more context. Previous assistant context: {}.",
                tool_call.name, context_hint
            ),
            tool_call: Some(tool_call),
            reasoning_content: None,
            assistant_message: None,
            provider_source: "local_planner".to_string(),
            provider_mode: "tool-first".to_string(),
            fallback_reason: None,
            token_usage: None,
        })
    }

    fn extract_explicit_path(text: &str) -> Option<String> {
        let candidates = Self::extract_path_candidates(text);
        candidates
            .into_iter()
            .find(|segment| looks_like_path(segment))
    }

    fn infer_last_referenced_path(history: &[TurnHistoryMessage]) -> Option<String> {
        history
            .iter()
            .rev()
            .find_map(|message| Self::extract_explicit_path(&message.content))
    }

    fn extract_search_query(text: &str, referenced_path: Option<&str>) -> Option<String> {
        if let Some(quoted) = Self::extract_quoted_segment(text) {
            let trimmed = quoted.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        let mut cleaned = text.replace('\n', " ");
        if let Some(path) = referenced_path {
            cleaned = cleaned.replace(path, " ");
        }
        for path in Self::extract_path_candidates(text) {
            if looks_like_path(&path) {
                cleaned = cleaned.replace(&path, " ");
            }
        }
        for keyword in [
            "搜索", "查找", "find", "grep", "包含", "帮我", "一个", "看看", "定义", "引用", "调用",
            "出现", "报错", "错误", "where", "is", "are", "在哪", "哪里", "里",
        ] {
            cleaned = cleaned.replace(keyword, " ");
        }

        if let Some(symbol) = Self::extract_symbol_candidate(&cleaned) {
            return Some(symbol);
        }

        let query = cleaned
            .split_whitespace()
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        if query.is_empty() {
            None
        } else {
            Some(query)
        }
    }

    fn extract_path_candidates(text: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        let mut current = String::new();

        for ch in text.chars() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\') {
                current.push(ch);
            } else if !current.is_empty() {
                candidates.push(std::mem::take(&mut current));
            }
        }

        if !current.is_empty() {
            candidates.push(current);
        }

        candidates
    }

    fn extract_quoted_segment(text: &str) -> Option<String> {
        let quotes = [('`', '`'), ('"', '"'), ('\'', '\''), ('“', '”')];
        for (open, close) in quotes {
            let start = text.find(open)?;
            let remainder = &text[start + open.len_utf8()..];
            let end = remainder.find(close)?;
            let segment = &remainder[..end];
            if !segment.trim().is_empty() {
                return Some(segment.to_string());
            }
        }
        None
    }

    fn extract_symbol_candidate(text: &str) -> Option<String> {
        text.split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ',' | '，' | '。' | ':' | '：' | '?' | '？' | '(' | ')' | '（' | '）'
                )
        })
        .filter_map(|token| {
            let trimmed = token.trim_matches(|ch: char| {
                !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '.' | '/' | '\\')
            });
            if trimmed.len() < 2
                || looks_like_path(trimmed)
                || !trimmed
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
                || !trimmed
                    .chars()
                    .any(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                return None;
            }
            let score = usize::from(trimmed.contains('_'))
                + usize::from(trimmed.chars().any(|ch| ch.is_ascii_uppercase()))
                + usize::from(trimmed.chars().any(|ch| ch.is_ascii_lowercase()))
                + usize::from(trimmed.chars().any(|ch| ch.is_ascii_digit()))
                + trimmed.len();
            Some((score, trimmed.to_string()))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, token)| token)
    }
}

fn looks_like_path(segment: &str) -> bool {
    if segment.starts_with("http://") || segment.starts_with("https://") {
        return false;
    }

    segment.contains('/')
        || segment.contains('\\')
        || segment
            .rsplit('.')
            .next()
            .map(|ext| {
                !ext.is_empty()
                    && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
                    && segment.contains('.')
            })
            .unwrap_or(false)
}

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\n', "\\n");
    let count = normalized.chars().count();
    if count <= max_chars {
        normalized
    } else {
        let preview = normalized.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(role: &str, content: &str) -> TurnHistoryMessage {
        TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn planner_prefers_gather_context_for_explicit_file_questions() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "src-tauri/src/agent/tools.rs 是什么文件？",
            &[],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_gather_context");
        assert_eq!(
            call.arguments
                .get("path")
                .and_then(serde_json::Value::as_str),
            Some("src-tauri/src/agent/tools.rs")
        );
    }

    #[test]
    fn planner_can_follow_last_referenced_path_from_history() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "这个文件是什么？",
            &[
                history("user", "先看看 src-tauri/src/agent/planner.rs"),
                history("assistant", "好的"),
            ],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_gather_context");
        assert_eq!(
            call.arguments
                .get("path")
                .and_then(serde_json::Value::as_str),
            Some("src-tauri/src/agent/planner.rs")
        );
    }

    #[test]
    fn planner_routes_path_scoped_search_to_gather_context() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "在 src-tauri/src/agent 里搜索 `tool_result`",
            &[],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_gather_context");
        assert_eq!(
            call.arguments
                .get("query")
                .and_then(serde_json::Value::as_str),
            Some("tool_result")
        );
        assert_eq!(
            call.arguments
                .get("path")
                .and_then(serde_json::Value::as_str),
            Some("src-tauri/src/agent")
        );
    }

    #[test]
    fn planner_prefers_symbol_query_for_definition_questions() {
        let call = LocalTurnPlanner::infer_local_tool_call("ToolRouter 在哪里定义？", &[])
            .expect("tool call");

        assert_eq!(call.name, "workspace_search_text");
        assert_eq!(
            call.arguments
                .get("query")
                .and_then(serde_json::Value::as_str),
            Some("ToolRouter")
        );
    }

    #[test]
    fn planner_uses_gather_context_for_search_inside_explicit_file() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "在 src-tauri/src/agent/tools.rs 里查找 error_result 定义",
            &[],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_gather_context");
        assert_eq!(
            call.arguments
                .get("path")
                .and_then(serde_json::Value::as_str),
            Some("src-tauri/src/agent/tools.rs")
        );
        assert_eq!(
            call.arguments
                .get("query")
                .and_then(serde_json::Value::as_str),
            Some("error_result")
        );
    }
}
