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

        if lowered.contains("time") || lowered.contains("时间") || lowered.contains("几点") {
            return Some(ToolCall {
                call_id: None,
                name: "time.now".to_string(),
                arguments: json!({}),
            });
        }

        if let Some(path) =
            Self::extract_explicit_file_name(user_message).or_else(|| Self::infer_last_referenced_file(history))
        {
            return Some(ToolCall {
                call_id: None,
                name: "workspace.read_file".to_string(),
                arguments: json!({ "path": path }),
            });
        }

        if lowered.contains("目录")
            || lowered.contains("文件夹")
            || lowered.contains("有哪些文件")
            || lowered.contains("list files")
        {
            return Some(ToolCall {
                call_id: None,
                name: "workspace.list_files".to_string(),
                arguments: json!({ "path": "." }),
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
            provider_mode: "tool-first".to_string(),
            fallback_reason: None,
            token_usage: None,
        })
    }

    fn extract_explicit_file_name(text: &str) -> Option<String> {
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

        candidates.into_iter().find(|segment| {
            segment.contains('.')
                && !segment.starts_with("http://")
                && !segment.starts_with("https://")
                && segment
                    .rsplit('.')
                    .next()
                    .map(|ext| !ext.is_empty() && ext.chars().all(|ch| ch.is_ascii_alphanumeric()))
                    .unwrap_or(false)
        })
    }

    fn infer_last_referenced_file(history: &[TurnHistoryMessage]) -> Option<String> {
        history
            .iter()
            .rev()
            .filter(|message| message.role == "user")
            .find_map(|message| Self::extract_explicit_file_name(&message.content))
    }
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
