use crate::agent::graph::{
    GraphDecision, GraphDecisionKind, GraphDecisionReason, GraphRun, GraphRunPhase,
    GraphTurnHandoff,
};
use crate::agent::provider::ProviderDecision;
use crate::agent::session::TurnHistoryMessage;
use crate::agent::tools::{ToolCall, ToolPlan, ToolPlanStep};
use serde_json::json;

const MAX_LOCAL_BATCH_PATHS: usize = 6;
const MAX_GRAPH_AUTO_CONTINUE_STEPS: usize = 8;

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

pub struct GraphPlanningContext<'a> {
    pub run: GraphPlanningRunView<'a>,
    pub handoff: &'a GraphTurnHandoff,
}

#[derive(Clone, Copy)]
pub struct GraphPlanningRunView<'a> {
    pub goal: &'a str,
    pub step_count: usize,
}

impl<'a> GraphPlanningContext<'a> {
    pub fn from_run(run: &'a GraphRun, handoff: &'a GraphTurnHandoff) -> Self {
        Self {
            run: GraphPlanningRunView {
                goal: run.goal.as_str(),
                step_count: run.steps.len(),
            },
            handoff,
        }
    }
}

pub trait GraphPlanner: Send + Sync {
    fn decide_after_turn(&self, context: GraphPlanningContext<'_>) -> GraphDecision;
}

pub struct DefaultGraphPlanner;

pub struct LocalTurnPlanner;

impl GraphPlanner for DefaultGraphPlanner {
    fn decide_after_turn(&self, context: GraphPlanningContext<'_>) -> GraphDecision {
        let assistant_message = context.handoff.assistant_message.trim();
        let goal = context.run.goal.trim();

        if assistant_requests_user_input(assistant_message) {
            return GraphDecision {
                kind: GraphDecisionKind::WaitUser,
                reason: GraphDecisionReason::TurnCompletedAwaitingUser,
                summary: String::new(),
                target_phase: GraphRunPhase::WaitingUser,
            };
        }

        if goal_supports_auto_continue(goal)
            && context.run.step_count < MAX_GRAPH_AUTO_CONTINUE_STEPS
        {
            return GraphDecision {
                kind: GraphDecisionKind::Continue,
                reason: GraphDecisionReason::PlannerRequestedContinue,
                summary: build_next_action_summary(goal, context.handoff),
                target_phase: GraphRunPhase::Ready,
            };
        }

        GraphDecision {
            kind: GraphDecisionKind::WaitUser,
            reason: GraphDecisionReason::TurnCompletedAwaitingUser,
            summary: String::new(),
            target_phase: GraphRunPhase::WaitingUser,
        }
    }
}

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
        let local_tool_call = Self::infer_local_tool_call(user_message, history);
        match (provider_tool_call, local_tool_call) {
            (Some(provider), Some(local))
                if Self::should_prefer_local_tool_call(&provider, &local) =>
            {
                Some(local)
            }
            (Some(provider), _) => Some(provider),
            (None, Some(local)) => Some(local),
            (None, None) => None,
        }
    }
}

impl LocalTurnPlanner {
    fn should_prefer_local_tool_call(
        provider_tool_call: &ToolCall,
        local_tool_call: &ToolCall,
    ) -> bool {
        let local_has_explicit_plan = local_tool_call.plan.is_some();
        let local_is_multi_path = local_tool_call.name == "workspace_batch"
            || local_tool_call.arguments.get("paths").is_some();
        let provider_is_single_path = provider_tool_call.arguments.get("paths").is_none();

        local_has_explicit_plan
            && local_is_multi_path
            && provider_is_single_path
            && (provider_tool_call.name != local_tool_call.name
                || provider_tool_call.plan.is_none())
    }

    fn infer_local_tool_call(
        user_message: &str,
        history: &[TurnHistoryMessage],
    ) -> Option<ToolCall> {
        let lowered = user_message.to_lowercase();
        let explicit_paths = Self::extract_explicit_paths(user_message);
        let explicit_path = explicit_paths.first().cloned();
        let referenced_path = explicit_path.or_else(|| Self::infer_last_referenced_path(history));

        if let Some(batch_call) =
            Self::infer_multi_path_tool_call(user_message, &lowered, &explicit_paths)
        {
            return Some(batch_call);
        }

        if contains_any(&lowered, &["time", "时间", "几点", "timestamp"]) {
            return Some(ToolCall {
                call_id: None,
                name: "time_now".to_string(),
                arguments: json!({}),
                plan: None,
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
                plan: None,
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
                        plan: None,
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
                    plan: None,
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
                        plan: None,
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
                    plan: None,
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
                    "看一下",
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
                    plan: None,
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
                plan: None,
            });
        }

        None
    }

    fn infer_multi_path_tool_call(
        user_message: &str,
        lowered: &str,
        explicit_paths: &[String],
    ) -> Option<ToolCall> {
        if explicit_paths.len() < 2 {
            return None;
        }

        let has_compare_intent = contains_any(
            lowered,
            &[
                "compare",
                "vs",
                "diff",
                "对比",
                "比较",
                "一起看",
                "同时看",
                "分别看",
            ],
        );
        let has_overview_intent = contains_any(
            lowered,
            &[
                "看", "看看", "查看", "分析", "解释", "实现", "内容", "what is", "show", "read",
            ],
        );
        let search_query = Self::extract_search_query(user_message, None);

        if !has_compare_intent && !has_overview_intent && search_query.is_none() {
            return None;
        }

        let calls = explicit_paths
            .iter()
            .take(MAX_LOCAL_BATCH_PATHS)
            .map(|path| {
                let mut arguments = json!({
                    "path": path,
                    "lineCount": 80,
                    "limit": 20,
                });
                if let Some(query) = &search_query {
                    arguments["query"] = json!(query);
                }
                json!({
                    "name": "workspace_gather_context",
                    "arguments": arguments,
                })
            })
            .collect::<Vec<_>>();
        let tool_plan_steps = explicit_paths
            .iter()
            .take(MAX_LOCAL_BATCH_PATHS)
            .enumerate()
            .map(|(index, path)| ToolPlanStep {
                name: "workspace_gather_context".to_string(),
                arguments: json!({
                    "path": path,
                    "query": search_query.clone(),
                    "lineCount": 80,
                    "limit": 20,
                }),
                summary: format!("第 {} 个路径聚合：`{}`。", index + 1, path),
            })
            .collect::<Vec<_>>();

        Some(ToolCall {
            call_id: None,
            name: "workspace_batch".to_string(),
            arguments: json!({
                "parallel": true,
                "continueOnError": true,
                "calls": calls,
            }),
            plan: Some(ToolPlan {
                kind: "batch".to_string(),
                summary: format!("批量聚合 {} 个显式路径。", tool_plan_steps.len()),
                parallel: true,
                continue_on_error: true,
                steps: tool_plan_steps,
            }),
        })
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
        Self::extract_explicit_paths(text).into_iter().next()
    }

    fn extract_explicit_paths(text: &str) -> Vec<String> {
        let mut seen = Vec::new();
        for segment in Self::extract_path_candidates(text) {
            if looks_like_path(&segment) && !seen.iter().any(|existing| existing == &segment) {
                seen.push(segment);
            }
        }
        seen
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
            if !trimmed.is_empty() && !looks_like_path(trimmed) {
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

fn assistant_requests_user_input(message: &str) -> bool {
    let normalized = message.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }

    if normalized.contains('?') || normalized.contains('？') {
        return true;
    }

    [
        "需要我",
        "是否继续",
        "请提供",
        "告诉我",
        "如果你愿意",
        "want me to",
        "please provide",
        "can you",
        "should i",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn goal_supports_auto_continue(goal: &str) -> bool {
    let normalized = goal.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }

    [
        "逐步",
        "继续",
        "系统",
        "完整",
        "全面",
        "端到端",
        "多轮",
        "直到",
        "最终",
        "推进",
        "收口",
        "排查并修复",
        "验证并修复",
        "梳理",
        "continue",
        "step by step",
        "iterative",
        "end-to-end",
        "investigate",
        "debug and fix",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn build_next_action_summary(goal: &str, handoff: &GraphTurnHandoff) -> String {
    let goal_excerpt = truncate_text(goal.trim(), 72);
    let summary_excerpt = truncate_text(handoff.session_summary.trim(), 96);
    let acceptance_note = handoff
        .acceptance_focus
        .as_deref()
        .map(|note| format!(" 验收要求：{}。", truncate_text(note, 88)))
        .unwrap_or_default();
    let closeout_note = handoff
        .closeout_focus
        .as_deref()
        .map(|note| format!(" 收口要求：{}。", truncate_text(note, 88)))
        .unwrap_or_default();
    let active_task_note = handoff
        .active_task_focus
        .as_deref()
        .map(|task| format!(" 当前激活任务：{}。", truncate_text(task, 32)))
        .unwrap_or_default();
    let focus_note = handoff
        .last_referenced_file
        .as_deref()
        .map(|path| format!(" 当前焦点文件：{}。", truncate_text(path, 72)))
        .unwrap_or_default();
    let focus_note = format!("{acceptance_note}{closeout_note}{focus_note}");
    let memory_note = if handoff.long_term_memory_entry_count > 0 {
        format!(
            " 已保留 {} 条长期记忆可供下一轮继续消费。",
            handoff.long_term_memory_entry_count
        )
    } else {
        String::new()
    };

    if summary_excerpt.is_empty() {
        return format!(
            "继续围绕目标“{}”推进下一轮，并把新的发现回写到 run 状态。{}{}{}",
            goal_excerpt, active_task_note, focus_note, memory_note
        );
    }

    format!(
        "继续围绕目标“{}”推进下一轮，优先扩展当前上下文：{}。{}{}{}",
        goal_excerpt, summary_excerpt, active_task_note, focus_note, memory_note
    )
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    let mut count = 0;

    for ch in text.chars() {
        if count >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
        count += 1;
    }

    truncated
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
    use crate::agent::graph::{GraphRun, GraphRunPhase};

    fn history(role: &str, content: &str) -> TurnHistoryMessage {
        TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
        }
    }

    fn sample_run(goal: &str) -> GraphRun {
        GraphRun {
            id: "run-1".to_string(),
            goal: goal.to_string(),
            session_id: Some("session-1".to_string()),
            phase: GraphRunPhase::Running,
            steps: Vec::new(),
            active_turn_id: None,
            last_completed_turn_id: None,
            stop_reason: None,
            last_handoff: None,
            resume_count: 0,
            last_decision: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn sample_handoff(assistant_message: &str) -> GraphTurnHandoff {
        GraphTurnHandoff {
            contract_version: "graph-run-contract-v1".to_string(),
            turn_id: Some("turn-1".to_string()),
            session_id: Some("session-1".to_string()),
            turn_phase: "ready".to_string(),
            checkpoint_status: Some("completed".to_string()),
            checkpoint_phase: Some("ready".to_string()),
            user_message: "请继续排查 provider 适配".to_string(),
            assistant_message: assistant_message.to_string(),
            session_summary: "已经完成第一轮排查，发现需要继续核对 provider fallback 行为。"
                .to_string(),
            conversation_id: "session-1".to_string(),
            session_turn_count: 1,
            run_id: Some("run-1".to_string()),
            run_phase: Some("waiting_user".to_string()),
            active_task_focus: Some("PA-018".to_string()),
            acceptance_focus: None,
            closeout_focus: None,
            last_referenced_file: Some("src-tauri/src/agent/provider.rs".to_string()),
            recent_attachment_asset_count: 0,
            long_term_memory_status: "empty".to_string(),
            long_term_memory_entry_count: 0,
            trace_step_count: 4,
            tool_activity_count: 1,
            provider_name: "OpenAI".to_string(),
            provider_model: "gpt-5".to_string(),
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

    #[test]
    fn planner_uses_batch_for_multi_path_overview_requests() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "同时看看 src-tauri/src/agent/tools.rs 和 src-tauri/src/agent/planner.rs",
            &[],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_batch");
        assert!(call.plan.is_some());
        assert!(call.arguments.get("toolPlan").is_none());
        let calls = call
            .arguments
            .get("calls")
            .and_then(serde_json::Value::as_array)
            .expect("batch calls");
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|entry| {
            entry.get("name").and_then(serde_json::Value::as_str)
                == Some("workspace_gather_context")
        }));
    }

    #[test]
    fn planner_uses_batch_for_multi_path_search_requests() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "对比 src-tauri/src/agent/tools.rs 和 src-tauri/src/agent/planner.rs 里 `ToolCall` 的用法",
            &[],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_batch");
        assert!(call.plan.is_some());
        assert!(call.arguments.get("toolPlan").is_none());
        let calls = call
            .arguments
            .get("calls")
            .and_then(serde_json::Value::as_array)
            .expect("batch calls");
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|entry| {
            entry
                .get("arguments")
                .and_then(|args| args.get("query"))
                .and_then(serde_json::Value::as_str)
                == Some("ToolCall")
        }));
    }

    #[test]
    fn planner_ignores_quoted_path_when_extracting_search_query() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "在 `src-tauri/src/agent/tools.rs` 里搜索 ToolResult",
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
            Some("ToolResult")
        );
    }

    #[test]
    fn planner_caps_multi_path_batch_size_to_tool_limit() {
        let call = LocalTurnPlanner::infer_local_tool_call(
            "同时看 a.rs b.rs c.rs d.rs e.rs f.rs g.rs",
            &[],
        )
        .expect("tool call");

        assert_eq!(call.name, "workspace_batch");
        let calls = call
            .arguments
            .get("calls")
            .and_then(serde_json::Value::as_array)
            .expect("batch calls");
        assert_eq!(calls.len(), MAX_LOCAL_BATCH_PATHS);
    }

    #[test]
    fn planner_prefers_local_multi_path_plan_over_provider_single_path_call() {
        let provider_tool_call = ToolCall {
            call_id: Some("call_provider".to_string()),
            name: "workspace_read_file".to_string(),
            arguments: json!({
                "path": "src-tauri/src/agent/tools.rs"
            }),
            plan: None,
        };

        let call = LocalTurnPlanner
            .select_tool_call(
                "请同时查看 src-tauri/src/agent/tools.rs、src-tauri/src/agent/planner.rs，并概括差异",
                &[],
                Some(provider_tool_call),
            )
            .expect("selected tool call");

        assert_eq!(call.name, "workspace_batch");
        assert!(call.plan.is_some());
        assert!(call.arguments.get("calls").is_some());
    }

    #[test]
    fn graph_planner_waits_when_assistant_is_asking_user() {
        let planner = DefaultGraphPlanner;
        let run = sample_run("逐步排查 provider 配置问题并收口");
        let handoff = sample_handoff(
            "我已经定位到第一处配置差异。是否继续帮你核对下一层 provider fallback？",
        );

        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

        assert_eq!(decision.kind, GraphDecisionKind::WaitUser);
        assert_eq!(
            decision.reason,
            GraphDecisionReason::TurnCompletedAwaitingUser
        );
        assert_eq!(decision.target_phase, GraphRunPhase::WaitingUser);
    }

    #[test]
    fn graph_planner_can_request_continue_for_iterative_goal() {
        let planner = DefaultGraphPlanner;
        let run = sample_run("逐步排查 provider 配置问题并收口");
        let handoff = sample_handoff("我已经完成第一轮核对，并整理出继续排查的上下文。");

        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

        assert_eq!(decision.kind, GraphDecisionKind::Continue);
        assert_eq!(
            decision.reason,
            GraphDecisionReason::PlannerRequestedContinue
        );
        assert_eq!(decision.target_phase, GraphRunPhase::Ready);
        assert!(decision.summary.contains("继续围绕目标"));
        assert!(decision.summary.contains("当前激活任务：PA-018"));
        assert!(decision.summary.contains("当前焦点文件"));
    }

    #[test]
    fn graph_planner_continue_summary_can_surface_long_term_memory_facts() {
        let planner = DefaultGraphPlanner;
        let run = sample_run("逐步排查 provider 配置问题并收口");
        let mut handoff = sample_handoff("我已经完成第一轮核对，并整理出继续排查的上下文。");
        handoff.long_term_memory_status = "available".to_string();
        handoff.long_term_memory_entry_count = 2;

        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

        assert_eq!(decision.kind, GraphDecisionKind::Continue);
        assert!(decision.summary.contains("已保留 2 条长期记忆"));
    }

    #[test]
    fn graph_planner_continue_summary_can_surface_acceptance_focus() {
        let planner = DefaultGraphPlanner;
        let run = sample_run("逐步推进 PA-018 并完成交付");
        let mut handoff = sample_handoff("我已经补齐了当前验收证据，并准备继续推进。");
        handoff.acceptance_focus = Some(
            "Establish acceptance criteria and run a closeout audit before claiming delivery."
                .to_string(),
        );

        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

        assert_eq!(decision.kind, GraphDecisionKind::Continue);
        assert!(decision.summary.contains("验收要求"));
        assert!(decision.summary.contains("closeout audit"));
    }

    #[test]
    fn graph_planner_continue_summary_can_surface_closeout_focus() {
        let planner = DefaultGraphPlanner;
        let run = sample_run("逐步推进 PA-018 并完成交付");
        let mut handoff = sample_handoff("我已经补齐了当前验收证据，并准备继续推进。");
        handoff.closeout_focus = Some(
            "Summarize changed files, verification performed, and unresolved risks at closeout."
                .to_string(),
        );

        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

        assert_eq!(decision.kind, GraphDecisionKind::Continue);
        assert!(decision.summary.contains("收口要求"));
        assert!(decision.summary.contains("verification performed"));
    }

    #[test]
    fn graph_planner_defaults_to_wait_user_without_iterative_goal_signal() {
        let planner = DefaultGraphPlanner;
        let run = sample_run("解释 tauri.conf.json 是什么");
        let handoff = sample_handoff("tauri.conf.json 是 Tauri 应用的主配置文件。");

        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

        assert_eq!(decision.kind, GraphDecisionKind::WaitUser);
        assert_eq!(decision.target_phase, GraphRunPhase::WaitingUser);
    }
}
