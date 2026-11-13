//! ADR 0123：会话转录清洗契约（就地自愈 / 推理空洞判定 / 修剪不落盘）的回归测试。
//!
//! 从 `tests.rs` 独立成文件，避免会话测试主干继续膨胀，同时让 ADR 0123 的契约面自成一册。

use super::super::*;

#[test]
fn snapshot_heals_legacy_native_transcript_without_reasoning_content() {
    let session_id = "legacy-reasoning";
    let mut store = SessionStore::memory_only();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![TurnHistoryMessage {
                role: "user".to_string(),
                attachments: Vec::new(),
                content: "继续".to_string(),
                ..Default::default()
            }],
            provider_native_transcript: vec![
                serde_json::json!({
                    "role": "user",
                    "content": "看看文件"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_legacy",
                            "type": "function",
                            "function": {
                                "name": "workspace_list_files",
                                "arguments": "{}"
                            }
                        }
                    ]
                }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    // ADR 0123：非 reasoning 模型的历史工具调用不再整段清空，而是就地补齐 synthetic tool 结果。
    assert!(!snapshot.provider_native_transcript.is_empty());
    assert_eq!(snapshot.provider_native_transcript.len(), 4);
    let synthetic_tool = snapshot
        .provider_native_transcript
        .iter()
        .find(|msg| msg.get("role").and_then(serde_json::Value::as_str) == Some("tool"))
        .expect("必须补齐 synthetic tool 结果");
    assert_eq!(
        synthetic_tool
            .get("tool_call_id")
            .and_then(serde_json::Value::as_str),
        Some("call_legacy")
    );
    assert!(synthetic_tool
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .contains("interrupted"));
}

#[test]
fn snapshot_heals_incomplete_native_tool_roundtrip() {
    let session_id = "incomplete-tool-roundtrip";
    let mut store = SessionStore::memory_only();
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    attachments: Vec::new(),
                    content: "请记住 tauri.conf.json".to_string(),
                    ..Default::default()
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    attachments: Vec::new(),
                    content: "搜索没有直接命中，让我查看一下工作区的文件结构。".to_string(),
                    ..Default::default()
                },
            ],
            provider_native_transcript: vec![
                serde_json::json!({
                    "role": "user",
                    "content": "请记住 tauri.conf.json"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "先搜索文件",
                    "reasoning_content": "先搜索一下。",
                    "tool_calls": [
                        {
                            "id": "call_search",
                            "type": "function",
                            "function": {
                                "name": "workspace_search_text",
                                "arguments": "{\"query\":\"tauri.conf.json\"}"
                            }
                        }
                    ]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_search",
                    "content": "{\"matchCount\":0}"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "搜索没有直接命中，让我查看一下工作区的文件结构。",
                    "reasoning_content": "继续列目录。",
                    "tool_calls": [
                        {
                            "id": "call_list",
                            "type": "function",
                            "function": {
                                "name": "workspace_list_files",
                                "arguments": "{\"path\":\".\"}"
                            }
                        }
                    ]
                }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: Some("tauri.conf.json".to_string()),
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    // ADR 0123：未闭合往返就地自愈——补齐 call_list 的 synthetic tool 结果与中断检查点，绝不清空。
    assert!(!snapshot.provider_native_transcript.is_empty());
    assert_eq!(snapshot.provider_native_transcript.len(), 6);
    let healed_tail: Vec<&str> = snapshot.provider_native_transcript[4..]
        .iter()
        .filter_map(|msg| msg.get("role").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(healed_tail, vec!["tool", "assistant"]);
    assert_eq!(
        snapshot.provider_native_transcript[4]
            .get("tool_call_id")
            .and_then(serde_json::Value::as_str),
        Some("call_list")
    );
    assert!(snapshot.provider_native_transcript[5]
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .contains("[中断轮次检查点"));
}

#[test]
fn test_sanitize_transcript_exempts_non_reasoning_tool_calls() {
    let mut store = SessionStore::memory_only();
    let session = store.ensure_session("test-session");
    // 构造一个由非 reasoning 工具模型生成的典型工具调用往返（reasoning_content 显式为 null 或缺失）
    session.provider_native_transcript = vec![
        serde_json::json!({
            "role": "user",
            "content": "列出当前目录"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": serde_json::Value::Null,
            "tool_calls": [{
                "id": "call_123",
                "type": "function",
                "function": {
                    "name": "run_command",
                    "arguments": "{\"CommandLine\": \"ls\"}"
                }
            }]
        }),
        serde_json::json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "content": "Cargo.toml\nsrc\n"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "当前目录下有 Cargo.toml 和 src 目录。",
            "reasoning_content": serde_json::Value::Null
        }),
    ];

    // 在旧逻辑中，first_legacy_reasoning_gap 会把 null 判为 gap，导致整段清空！
    let mutated = store::sanitize_provider_native_transcript(session);
    assert!(!session.provider_native_transcript.is_empty(), "非 reasoning 模型的正常工具调用绝不能被清空！");
    assert_eq!(session.provider_native_transcript.len(), 4);
    assert!(!mutated);
}

#[test]
fn test_sanitize_transcript_in_place_heals_incomplete_tool_roundtrip() {
    let mut store = SessionStore::memory_only();
    let session = store.ensure_session("test-session");
    // 构造一个因中断导致未闭合的工具调用
    session.provider_native_transcript = vec![
        serde_json::json!({
            "role": "user",
            "content": "执行长时间任务"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "id": "call_unclosed",
                "type": "function",
                "function": {
                    "name": "run_command",
                    "arguments": "{\"CommandLine\": \"sleep 100\"}"
                }
            }]
        }),
    ];

    let mutated = store::sanitize_provider_native_transcript(session);
    assert!(mutated, "未闭合工具往返自愈必须标记为已修改");
    assert!(!session.provider_native_transcript.is_empty(), "中断未闭合工具往返必须自愈，绝不能被整段清空！");
    // 应该被补齐 synthetic tool result 占位与中断 assistant 检查点
    assert!(session.provider_native_transcript.iter().any(|msg| {
        msg.get("role").and_then(serde_json::Value::as_str) == Some("tool")
            && msg.get("tool_call_id").and_then(serde_json::Value::as_str) == Some("call_unclosed")
    }), "必须包含补齐的 tool_call_id=call_unclosed");
}

#[test]
fn test_sanitize_transcript_truncates_real_legacy_reasoning_gap() {
    let mut store = SessionStore::memory_only();
    let session = store.ensure_session("test-reasoning-session");
    // 构造一个前面有 reasoning_content，但在后续 tool 往返中缺失 reasoning_content 的破损会话
    session.provider_native_transcript = vec![
        serde_json::json!({
            "role": "user",
            "content": "深度思考问题"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "思考完毕",
            "reasoning_content": "这是初始思维链"
        }),
        serde_json::json!({
            "role": "user",
            "content": "继续工具调用"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": serde_json::Value::Null, // 破损：reasoning 模型缺失思维链
            "tool_calls": [{
                "id": "call_reasoning",
                "type": "function",
                "function": {
                    "name": "run_command",
                    "arguments": "{\"CommandLine\": \"cargo check\"}"
                }
            }]
        }),
        serde_json::json!({
            "role": "tool",
            "tool_call_id": "call_reasoning",
            "content": "ok"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "继续回答",
            "reasoning_content": "缺口之后的思维链"
        }),
    ];

    let mutated = store::sanitize_provider_native_transcript(session);
    assert!(mutated);
    // B114：不再整段清空，改为截断到首个缺口之前——缺口前的合法前缀必须保留供续跑复用。
    assert_eq!(
        session.provider_native_transcript.len(),
        3,
        "只能丢弃首个缺口及其之后的消息，缺口前的合法前缀必须保留"
    );
    assert_eq!(
        session.provider_native_transcript[2]
            .get("content")
            .and_then(serde_json::Value::as_str),
        Some("继续工具调用")
    );
    assert!(
        session.provider_native_transcript.iter().all(|msg| {
            msg.get("tool_call_id")
                .and_then(serde_json::Value::as_str)
                != Some("call_reasoning")
        }),
        "缺口消息的 assistant 及其 tool 结果必须一并丢弃，不得留下孤儿往返"
    );
}

#[test]
fn test_sanitize_transcript_truncates_gap_after_tool_and_keeps_closed_roundtrip() {
    let mut store = SessionStore::memory_only();
    let session = store.ensure_session("test-reasoning-gap-after-tool");
    // 缺口落在「tool 结果之后的第一条 assistant」：截断后末尾是已闭合的 tool 往返，必须原样保留。
    session.provider_native_transcript = vec![
        serde_json::json!({ "role": "user", "content": "第一问" }),
        serde_json::json!({
            "role": "assistant",
            "content": "第一答",
            "reasoning_content": "第一轮思维链"
        }),
        serde_json::json!({ "role": "user", "content": "跑个工具" }),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "第二轮思维链",
            "tool_calls": [{
                "id": "call_closed",
                "type": "function",
                "function": { "name": "run_command", "arguments": "{\"CommandLine\": \"true\"}" }
            }]
        }),
        serde_json::json!({ "role": "tool", "tool_call_id": "call_closed", "content": "ok" }),
        serde_json::json!({ "role": "assistant", "content": "缺口回答" }),
    ];

    let mutated = store::sanitize_provider_native_transcript(session);
    assert!(mutated);
    assert_eq!(session.provider_native_transcript.len(), 5);
    assert_eq!(
        session.provider_native_transcript[4]
            .get("tool_call_id")
            .and_then(serde_json::Value::as_str),
        Some("call_closed"),
        "缺口前已闭合的 tool 往返必须完整保留"
    );
}

#[test]
fn test_sanitize_transcript_in_place_heals_incomplete_reasoning_tool_roundtrip() {
    let mut store = SessionStore::memory_only();
    let session = store.ensure_session("test-reasoning-interrupted");
    // Turn 1: 正常推理轮次
    // Turn 2: 未闭合工具调用（中断）
    session.provider_native_transcript = vec![
        serde_json::json!({
            "role": "user",
            "content": "深度思考并搜索"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "我先检索一下",
            "reasoning_content": "这是第一轮推理思维链"
        }),
        serde_json::json!({
            "role": "user",
            "content": "执行某个长工具"
        }),
        serde_json::json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "开始调用工具",
            "tool_calls": [{
                "id": "call_reasoning_unclosed",
                "type": "function",
                "function": {
                    "name": "run_command",
                    "arguments": "{\"CommandLine\": \"sleep 100\"}"
                }
            }]
        }),
    ];

    let mutated = store::sanitize_provider_native_transcript(session);
    assert!(mutated, "未闭合 reasoning 工具往返自愈必须标记为已修改");
    assert!(
        !session.provider_native_transcript.is_empty(),
        "中断未闭合 reasoning 工具往返必须自愈，绝不能被 first_legacy_reasoning_gap 二次误杀清空！"
    );
    assert!(session.provider_native_transcript.iter().any(|msg| {
        msg.get("role").and_then(serde_json::Value::as_str) == Some("tool")
            && msg.get("tool_call_id").and_then(serde_json::Value::as_str) == Some("call_reasoning_unclosed")
    }));
}

#[test]
fn snapshot_does_not_persist_tool_result_pruning() {
    let session_id = "prune-not-persisted";
    let mut store = SessionStore::memory_only();
    let large_output = "x".repeat(20_000);
    store.sessions.insert(
        session_id.to_string(),
        SessionState {
            conversation_id: session_id.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: vec![TurnHistoryMessage {
                role: "user".to_string(),
                attachments: Vec::new(),
                content: "读取日志".to_string(),
                ..Default::default()
            }],
            provider_native_transcript: vec![
                serde_json::json!({ "role": "user", "content": "读取日志" }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": "call_log",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": "{}" }
                    }]
                }),
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": "call_log",
                    "content": large_output
                }),
                serde_json::json!({ "role": "assistant", "content": "日志已读取" }),
            ],
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            title_override: None,
            archived: false,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );

    let snapshot = store.snapshot(Some(session_id), &[]);
    let tool = snapshot
        .provider_native_transcript
        .iter()
        .find(|msg| msg.get("role").and_then(serde_json::Value::as_str) == Some("tool"))
        .expect("原始工具结果必须保留在持久化快照中");
    let content = tool
        .get("content")
        .and_then(serde_json::Value::as_str)
        .expect("工具内容必须是字符串");
    // ADR 0123：修剪只发生在读侧装载，完整原始日志必须原样落盘供追溯。
    assert_eq!(content.len(), 20_000);
    assert!(!content.contains(crate::agent::tool_pruner::PRUNE_MARKER));
}
