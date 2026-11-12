//! wave-interrupted-turn-resume：L2-T 功能契约 + L2-AT 确定性破坏注入**双模态**验收。
//!
//! 契约真值源：`.dev-team/contract-matrix-interrupted-turn-resume.md` §2（C1–C8）与
//! `.dev-team/nfr-baseline-interrupted-turn-resume.json`；API 真值源：
//! `crates/pony-agent-core/src/agent/session/interrupted_turn.rs`（T0 冻结的纯函数出口，
//! 本文件按签名调用，**不** mock 网络、**不**改任何 `src/**` 实现）。
//!
//! 设计纪律（test-expert L2 + 零假测试律）：
//! - 每个用例 AAA 分段，断言只看可机器判定的黑盒事实（请求组装产物 / store 结构 / 纯函数返回值）；
//! - 零 `#[ignore]`、零吞异常、零 `assert!(true)`；SUT 空桩期以 `unimplemented!()` panic 作为
//!   红相失败原语（panic 帧在 SUT 内，属"错误"而非编译失败/harness 超时）；
//! - 不触碰用户真实数据目录：`isolate_process_data_dir()` 把 XDG/HOME/LOCALAPPDATA 指向临时沙箱，
//!   临时目录随夹具 `Drop` 清理（谁创建谁清理）。
//!
//! C1 断言面（Lead 已裁决）：请求组装层 —— 组装出的 messages 必须携带每个已闭合调用的
//! 工具名 + 结果哨兵串；native 家族 `tool_calls` / `role=tool` 成对闭合，非 native 家族由
//! `Previous assistant reply:` 承载文本（可见步骤）携带，**禁止**只剩一条检查点句。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

#[path = "acceptance/adversarial/interrupted_turn_resume/mod.rs"]
mod adversarial;

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::json;

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    ContextStateQuery, ContextStateRetriever, DefaultContextStateRetriever, DefaultTurnContextBuilder,
    TurnContextBuilder,
};
use pony_agent_core::agent::provider::{
    ProviderAuthType, ProviderManager, ProviderProtocol, ProviderRequest,
};
use pony_agent_core::agent::session::interrupted_turn::{
    interrupted_cause_note, interrupted_turn_assistant_text, interrupted_turn_transcript_closure,
    pending_tool_result_note, persisted_assistant_text, visible_step_lines, ClosedToolStep,
    InterruptedTurnCause, PendingToolCall, INTERRUPTED_PENDING_NOTE_PREFIX,
    INTERRUPTED_RESULT_SECTION_HEADER,
};
use pony_agent_core::agent::session::{
    build_missing_run_control_audit_summary, failed_turn_checkpoint_text,
    interrupted_turn_checkpoint_text, HistoryBranch, HistoryCursor, HistoryMessageKind,
    HistoryNode, HistoryNodeKind, PersistedStore, SessionBackend, SessionSnapshot, SessionState,
    SessionStore, SeparateTraceTableMode, TurnHistoryMessage, TurnTraceRecord,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

// ---------------------------------------------------------------------------
// 夹具（沙箱 / provider / 请求组装 / 会话快照）
// ---------------------------------------------------------------------------

const SID: &str = "pa_interrupted_turn_resume_acceptance";
const INTERRUPTED_USER: &str = "第一轮：重启 staging 服务并确认端口释放";
const NEXT_USER: &str = "继续：把服务重启完并汇总";
const DELIVERED_TEXT: &str = "我先看了 8080 端口占用，随后执行了 kill。";
const RESULT_ALPHA: &str = "RESULT_ALPHA_8080 被 pid 4242 占用";
const RESULT_BETA: &str = "RESULT_BETA_kill -TERM 4242 已执行，等待 2s 后复查";

/// 进程级沙箱（集成测试绝不触碰真实 `~/.local/share/PonyAgent`）。
fn isolate_process_data_dir() -> PathBuf {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    static SANDBOX: Mutex<Option<PathBuf>> = Mutex::new(None);
    ONCE.call_once(|| {
        let unique = format!(
            "pony-agent-it-interrupted-turn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        );
        let sandbox = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
        *SANDBOX.lock().expect("sandbox lock poisoned") = Some(sandbox);
    });
    SANDBOX
        .lock()
        .expect("sandbox lock poisoned")
        .clone()
        .expect("sandbox initialized")
}

fn closed(name: &str, result_summary: &str) -> ClosedToolStep {
    ClosedToolStep {
        name: name.to_string(),
        result_summary: result_summary.to_string(),
    }
}

fn closed_steps() -> Vec<ClosedToolStep> {
    vec![closed("Read", RESULT_ALPHA), closed("Bash", RESULT_BETA)]
}

fn pending_calls() -> Vec<PendingToolCall> {
    vec![PendingToolCall {
        name: "Write".to_string(),
    }]
}

fn provider_with(protocol: ProviderProtocol, supports_reasoning: bool) -> ProviderManager {
    ProviderManager::new(ResolvedProviderSelection {
        requested_name: "it-interrupted-turn".to_string(),
        provider_name: "it-interrupted-turn".to_string(),
        protocol,
        model: "deepseek-v4.1-flash".to_string(),
        base_url: "https://api.example.com/v1".to_string(),
        api_key: Some("test-token".to_string()),
        api_key_env_var: "TEST_API_KEY".to_string(),
        auth_type: ProviderAuthType::Bearer,
        temperature: 0.2,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        thinking_param_pattern: ThinkingParamPattern::None,
        capabilities: ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning,
            // 大窗口：本套用例只验证"内容在不在请求里"，不允许切窗把哨兵裁掉（那会污染判据）。
            context_window_tokens: Some(200_000),
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
    })
}

/// native 家族（openai completions + tools ⇒ 原生工具流回放）。
fn native_provider() -> ProviderManager {
    provider_with(ProviderProtocol::OpenAiCompletions, false)
}

/// 非 native 家族（anthropic messages ⇒ 纯文本历史承载）。
fn text_provider() -> ProviderManager {
    provider_with(ProviderProtocol::AnthropicMessages, false)
}

fn build_request(provider: &ProviderManager, session: &SessionSnapshot, user: &str) -> ProviderRequest {
    let retriever = DefaultContextStateRetriever;
    let retrieved = retriever.retrieve(ContextStateQuery {
        user_message: user,
        images: &[],
        workspace_mode: None,
        session,
        run: None,
        checkpoint: None,
    });
    DefaultTurnContextBuilder.build_request("state-machine-v1", provider, &retrieved, &[])
}

fn request_body(provider: &ProviderManager, session: &SessionSnapshot, user: &str) -> String {
    let request = build_request(provider, session, user);
    serde_json::to_string(&request).expect("provider request 必须可序列化")
}

fn turn_message(role: &str, content: &str, turn_id: Option<&str>, kind: Option<HistoryMessageKind>) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        turn_id: turn_id.map(str::to_string),
        kind,
        ..Default::default()
    }
}

/// 事故形态历史：中断轮 user + 合成 assistant 行（`kind` 坐实载体种类）。
fn interrupted_history(assistant_text: &str) -> Vec<TurnHistoryMessage> {
    vec![
        turn_message("user", INTERRUPTED_USER, Some("turn-1"), None),
        turn_message(
            "assistant",
            assistant_text,
            Some("turn-1"),
            Some(HistoryMessageKind::Cancelled),
        ),
    ]
}

fn snapshot_of(conversation_id: &str, history: Vec<TurnHistoryMessage>, transcript: Vec<serde_json::Value>) -> SessionSnapshot {
    SessionSnapshot {
        conversation_id: conversation_id.to_string(),
        title: "中断轮续跑验收".to_string(),
        summary: "摘要".to_string(),
        history,
        attachment_assets: Vec::new(),
        provider_native_transcript: transcript,
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: Default::default(),
        run_control_audit_summary: build_missing_run_control_audit_summary(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        resolved_node_id: None,
        latest_node_id: None,
        env_info: None,
        workspace_id: None,
    }
}

fn snapshot_from_state(state: &SessionState) -> SessionSnapshot {
    SessionSnapshot {
        conversation_id: state.conversation_id.clone(),
        title: state.title.clone(),
        summary: state.summary.clone(),
        history: state.history.clone(),
        attachment_assets: Vec::new(),
        provider_native_transcript: state.provider_native_transcript.clone(),
        turn_trace_history: state.turn_trace_history.clone(),
        long_term_memory_entries: state.long_term_memory_entries.clone(),
        memory_write_evidence: state.memory_write_evidence.clone(),
        memory_write_hook_trace_records: state.memory_write_hook_trace_records.clone(),
        history_state_evidence: state.history_state_evidence.clone(),
        history_state_audit_summary: Default::default(),
        run_control_audit_summary: build_missing_run_control_audit_summary(),
        turn_count: state.turn_count,
        last_referenced_file: state.last_referenced_file.clone(),
        updated_at_ms: state.updated_at_ms,
        history_nodes: state.history_nodes.clone(),
        history_branches: state.history_branches.clone(),
        history_cursor: state.history_cursor.clone(),
        resolved_node_id: state.history_cursor.visible_node_id.clone(),
        latest_node_id: state.history_cursor.branch_head_node_id.clone(),
        env_info: None,
        workspace_id: state.workspace_id.clone(),
    }
}

fn trace(turn_id: &str) -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: turn_id.to_string(),
        session_id: Some(SID.to_string()),
        title: "中断轮续跑验收".to_string(),
        phase: "interrupted".to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 预置状态后端（模拟"重启后从磁盘装载"的会话形态；不写 src、不碰真实库）
// ---------------------------------------------------------------------------

struct PreloadedBackend {
    store: Mutex<Option<PersistedStore>>,
    attachment_root: PathBuf,
}

struct PreloadedBackendGuard {
    root: PathBuf,
}

impl PreloadedBackendGuard {
    fn from_state(sandbox: &PathBuf, label: &str, state: &SessionState) -> (SessionStore, Self) {
        let root = sandbox.join(format!("preloaded-{label}"));
        std::fs::create_dir_all(&root).expect("create preloaded root");
        let sessions = json!({ state.conversation_id.clone(): serde_json::to_value(state).expect("session state 必须可序列化") });
        let store: PersistedStore = serde_json::from_value(json!({ "sessions": sessions }))
            .expect("预置会话库 JSON 必须能被 PersistedStore 反序列化");
        let backend = PreloadedBackend {
            store: Mutex::new(Some(store)),
            attachment_root: root.join("attachments"),
        };
        let guard = Self { root };
        (SessionStore::with_backend(Box::new(backend)), guard)
    }
}

impl Drop for PreloadedBackendGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl SessionBackend for PreloadedBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        self.store
            .lock()
            .expect("preloaded store lock poisoned")
            .take()
    }

    fn save_store(&self, _store: &PersistedStore) -> Result<(), String> {
        Ok(())
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

/// 事故形态的持久化会话：`branch-main` head = node-2（上一轮提交），但 visible 停在
/// node-1（较早的检查点节点）——事故库里 `…-branch-2` 就是在这条分叉上被隐式建出来的。
fn crafted_accident_state() -> SessionState {
    let node_one = HistoryNode {
        node_id: "node-1".to_string(),
        session_id: SID.to_string(),
        parent_node_id: None,
        branch_id: "branch-main".to_string(),
        forked_from_node_id: None,
        kind: HistoryNodeKind::Checkpoint,
        summary: "摘要".to_string(),
        title: "中断轮续跑验收".to_string(),
        history: vec![turn_message("user", "更早的一问", Some("turn-0"), None)],
        provider_native_transcript: Vec::new(),
        turn_id: Some("turn-0".to_string()),
        turn_count: 0,
        ..Default::default()
    };
    let node_two = HistoryNode {
        node_id: "node-2".to_string(),
        session_id: SID.to_string(),
        parent_node_id: Some("node-1".to_string()),
        branch_id: "branch-main".to_string(),
        forked_from_node_id: None,
        kind: HistoryNodeKind::TurnCommitted,
        summary: "摘要".to_string(),
        title: "中断轮续跑验收".to_string(),
        history: vec![
            turn_message("user", INTERRUPTED_USER, Some("turn-1"), None),
            turn_message("assistant", "第一轮已完成的部分结论", Some("turn-1"), None),
        ],
        provider_native_transcript: Vec::new(),
        turn_id: Some("turn-1".to_string()),
        turn_count: 1,
        ..Default::default()
    };
    SessionState {
        conversation_id: SID.to_string(),
        title: "中断轮续跑验收".to_string(),
        summary: "摘要".to_string(),
        history: node_two.history.clone(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![trace("turn-1")],
        trace_migration_state: Default::default(),
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 1_700_000_000_000,
        history_nodes: vec![node_one, node_two],
        history_branches: vec![HistoryBranch {
            branch_id: "branch-main".to_string(),
            session_id: SID.to_string(),
            base_node_id: Some("node-1".to_string()),
            head_node_id: Some("node-2".to_string()),
            forked_from_branch_id: None,
            forked_from_node_id: None,
            label: "main".to_string(),
            created_at_ms: 1,
            updated_at_ms: 2,
        }],
        history_cursor: HistoryCursor {
            session_id: SID.to_string(),
            // 事故形态：可见点停在较早的检查点节点，分支头已前进到上一轮提交。
            visible_node_id: Some("node-1".to_string()),
            active_branch_id: Some("branch-main".to_string()),
            branch_head_node_id: Some("node-2".to_string()),
            workspace_node_id: Some("node-2".to_string()),
            ..Default::default()
        },
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    }
}

// ---------------------------------------------------------------------------
// 闭合结构断言工具
// ---------------------------------------------------------------------------

fn role_of(message: &serde_json::Value) -> &str {
    message.get("role").and_then(|role| role.as_str()).unwrap_or("")
}

/// 严格配对闭合扫描：每个 `tool_calls[].id` 必须恰好有一条 `role=tool` 结果，
/// 且不得出现孤儿 tool 结果（DSH `openTurnClosers` 的"补齐优先于截断"判据）。
fn assert_roundtrips_closed(messages: &[serde_json::Value], ticket: &str) {
    let mut open: Vec<String> = Vec::new();
    let mut orphans: Vec<String> = Vec::new();
    for message in messages {
        match role_of(message) {
            "assistant" => {
                if let Some(calls) = message.get("tool_calls").and_then(|calls| calls.as_array()) {
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(|id| id.as_str())
                            .unwrap_or("<missing id>")
                            .to_string();
                        open.push(id);
                    }
                }
            }
            "tool" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string);
                match id {
                    Some(id) if open.contains(&id) => {
                        open.retain(|pending| pending != &id);
                    }
                    Some(id) => orphans.push(id),
                    None => orphans.push("<missing tool_call_id>".to_string()),
                }
            }
            _ => {}
        }
    }
    assert!(
        open.is_empty(),
        "{ticket}：存在未闭合的 tool_call {open:?}（悬空调用必须补 error 结果，不得截断）"
    );
    assert!(
        orphans.is_empty(),
        "{ticket}：存在孤儿 tool 结果 {orphans:?}（没有配对调用的结果不得落库）"
    );
}

// ===========================================================================
// C1：中断轮工具往返必须进下一轮上下文
// ===========================================================================

/// C1-native：openai 家族 ⇒ `tool_calls` ↔ `role=tool` 成对出现在下一轮请求里。
#[test]
fn c1_native_next_request_carries_closed_tool_roundtrip() {
    // Arrange
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("C1：已闭合步骤非空 ⇒ 必须产出 native 闭合片段（None 只在全空时允许）");
    assert!(
        !closure.is_empty(),
        "C1：闭合片段不得为空，实际 = {closure:?}"
    );
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let session = snapshot_of("c1-native", interrupted_history(&assistant_text), closure);
    let provider = native_provider();

    // Act
    let request = build_request(&provider, &session, NEXT_USER);
    let native = &request.native_messages;
    let serialized = serde_json::to_string(native).expect("native_messages 必须可序列化");

    // Assert 1：native 通道必须真的被用上（否则本用例失去验证目标）
    assert!(
        !native.is_empty(),
        "C1：openai 家族续跑请求必须走 native 通道，实际 native_messages 为空，请求体 = {}",
        serde_json::to_string(&request).unwrap_or_default()
    );

    // Assert 2：每个已闭合调用的工具名 + 结果哨兵串都必须出现在请求体里
    for step in &closed_steps {
        assert!(
            serialized.contains(&step.name),
            "C1：工具名 {:?} 必须出现在下一轮请求里（中断轮工具往返不得消失），实际 = {serialized}",
            step.name
        );
        assert!(
            serialized.contains(&step.result_summary),
            "C1：工具结果 {:?} 必须出现在下一轮请求里（结果不是可选装饰），实际 = {serialized}",
            step.result_summary
        );
    }

    // Assert 3：成对闭合（悬空调用补 error 结果，不得只剩孤儿 tool_calls）
    assert_roundtrips_closed(native, "C1");

    // Assert 4：角色序列必须是"工具往返"形状，而不是只有一条检查点句
    let tool_message_count = native.iter().filter(|m| role_of(m) == "tool").count();
    assert!(
        tool_message_count >= closed_steps.len(),
        "C1：role=tool 消息数必须 ≥ 已闭合调用数 {}（实际 {tool_message_count}）——只剩一条检查点句即回归",
        closed_steps.len()
    );

    // Assert 5：请求体显著大于"只有检查点句"的退化场景（token/字节维度对照，不依赖真实网络）
    let legacy_step_lines = vec![format!("Read {RESULT_ALPHA}"), format!("Bash {RESULT_BETA}")];
    let checkpoint_only = interrupted_turn_checkpoint_text(DELIVERED_TEXT, &legacy_step_lines)
        .expect("既有检查点纯函数必须为同形入参产出文本");
    let degraded = snapshot_of(
        "c1-native-degraded",
        interrupted_history(&checkpoint_only),
        Vec::new(),
    );
    let degraded_body = request_body(&provider, &degraded, NEXT_USER);
    let live_body = request_body(&provider, &session, NEXT_USER);
    assert!(
        live_body.len() > degraded_body.len(),
        "C1：闭合结构请求体必须大于仅检查点场景（live={}, degraded={}）",
        live_body.len(),
        degraded_body.len()
    );
}

/// C1-text：非 native 家族 ⇒ 可见步骤文本必须携带工具名 + 结果哨兵串（不是单条检查点句）。
#[test]
fn c1_text_family_next_request_carries_visible_step_results() {
    // Arrange
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let session = snapshot_of(
        "c1-text",
        interrupted_history(&assistant_text),
        Vec::new(),
    );
    let provider = text_provider();

    // Act
    let body = request_body(&provider, &session, NEXT_USER);

    // Assert 1：文本承载路径确实被使用（历史 carry 以 `Previous assistant reply:` 呈现）
    assert!(
        body.contains("Previous assistant reply:"),
        "C1：非 native 家族必须由历史承载文本进入请求，实际请求体 = {body}"
    );
    // Assert 2：工具名 + 结果哨兵串逐条出现
    for step in &closed_steps {
        assert!(
            body.contains(&step.name),
            "C1：文本腿必须携带工具名 {:?}，实际请求体 = {body}",
            step.name
        );
        assert!(
            body.contains(&step.result_summary),
            "C1：文本腿必须携带结果 {:?}，实际请求体 = {body}",
            step.result_summary
        );
    }
    // Assert 3：结果段标题存在（结构化的过程明细，不是一句"已执行工具步骤"）
    assert!(
        body.contains(INTERRUPTED_RESULT_SECTION_HEADER),
        "C1：文本腿必须含结果段标题 {INTERRUPTED_RESULT_SECTION_HEADER:?}，实际请求体 = {body}"
    );
    // Assert 4：不得只出现一条检查点句（检查点前缀出现时结果段也必须在）
    assert!(
        !body.contains("[中断轮次检查点]") || body.contains(INTERRUPTED_RESULT_SECTION_HEADER),
        "C1：禁止「只剩一条检查点句」的退化形态，实际请求体 = {body}"
    );
    // Assert 5：未开始调用必须显式表达"结果未知/未开始"事实
    assert!(
        body.contains(INTERRUPTED_PENDING_NOTE_PREFIX),
        "C1：未开始调用必须带显式事实前缀 {INTERRUPTED_PENDING_NOTE_PREFIX:?}，实际请求体 = {body}"
    );
}

// ===========================================================================
// C2：历史恰好一条中断轮且 turn_id 归属本轮
// ===========================================================================

#[test]
fn c2_history_has_exactly_one_interrupted_turn_labelled_with_its_own_turn_id() {
    // Arrange：先落一轮成功轮（含 trace 尾 = turn-1），再收口中断轮 turn-2
    isolate_process_data_dir();
    let mut store = SessionStore::memory_only();
    store.append_turn(Some(SID), "第一轮问题", "第一轮回答", None, Vec::new());
    store.record_turn_trace_in_memory(Some(SID), trace("turn-1"));
    let closed_steps = closed_steps();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending_calls());
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending_calls())
        .expect("C2：已闭合步骤非空 ⇒ 必须产出 native 闭合片段");

    // Act
    let snapshot = store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &assistant_text,
        Some(closure),
        Vec::new(),
        Some(HistoryMessageKind::Cancelled),
        Some("turn-2"),
    );

    // Assert 1：历史节点里 turn-2 恰好一条（不得因隐式 fork / 错标出现两条）
    let turn_two_nodes: Vec<&HistoryNode> = snapshot
        .history_nodes
        .iter()
        .filter(|node| node.turn_id.as_deref() == Some("turn-2"))
        .collect();
    assert_eq!(
        turn_two_nodes.len(),
        1,
        "C2：中断轮 turn-2 在 history_nodes 里必须恰好一条，实际 {} 条，节点 turn_id = {:?}",
        turn_two_nodes.len(),
        snapshot
            .history_nodes
            .iter()
            .map(|node| (node.node_id.clone(), node.turn_id.clone()))
            .collect::<Vec<_>>()
    );

    // Assert 2：不得错标为上一轮（事故现场 node-5 被标成 turn-2ca51165）
    let last_node = snapshot
        .history_nodes
        .last()
        .expect("C2：收口后必须存在历史节点");
    assert_eq!(
        last_node.turn_id.as_deref(),
        Some("turn-2"),
        "C2：新节点 turn_id 必须取本轮 turn-2（不得借 turn_trace_history.last() 错标成上一轮 turn-1），实际 = {:?}",
        last_node.turn_id
    );
    assert_ne!(
        last_node.turn_id.as_deref(),
        Some("turn-1"),
        "C2：中断轮节点不得被错标成上一轮 turn-1"
    );

    // Assert 3：历史里该轮恰好一条合成 assistant 行（一个 turn 一条记录）
    let interrupted_rows = snapshot
        .history
        .iter()
        .filter(|message| matches!(message.kind, Some(HistoryMessageKind::Cancelled)))
        .count();
    assert_eq!(
        interrupted_rows, 1,
        "C2：history 里中断轮合成 assistant 行必须恰好一条，实际 {interrupted_rows} 条"
    );
    let turn_two_user_rows = snapshot
        .history
        .iter()
        .filter(|message| message.role == "user" && message.turn_id.as_deref() == Some("turn-2"))
        .count();
    assert_eq!(
        turn_two_user_rows, 1,
        "C2：turn-2 的 user 行必须恰好一条，实际 {turn_two_user_rows} 条"
    );

    // Assert 4：既有成功轮不被中断收口抹掉
    assert!(
        snapshot
            .history
            .iter()
            .any(|message| message.content.contains("第一轮回答")),
        "C2：中断收口不得丢弃已提交的成功轮，实际 history = {:?}",
        snapshot.history.iter().map(|m| m.content.clone()).collect::<Vec<_>>()
    );
}

/// C2 基线回归腿：**不经过新纯函数**，直接走 store 的既有收口入口。
///
/// 价值：把「节点 turn_id 借 turn_trace_history.last() 错标」这条 RC2 缺陷钉成
/// **真实断言失败**（而不是被 SUT 空桩 panic 掩盖）；修好之后本用例必须绿。
#[test]
fn c2b_store_close_legacy_path_labels_node_with_current_turn_id() {
    // Arrange
    isolate_process_data_dir();
    let mut store = SessionStore::memory_only();
    store.append_turn(Some(SID), "第一轮问题", "第一轮回答", None, Vec::new());
    store.record_turn_trace_in_memory(Some(SID), trace("turn-1"));
    let legacy_text = interrupted_turn_checkpoint_text(
        DELIVERED_TEXT,
        &[format!("Read {RESULT_ALPHA}"), format!("Bash {RESULT_BETA}")],
    )
    .expect("C2b：既有检查点纯函数必须为同形入参产出文本");

    // Act（既有收口入口，零新纯函数参与）
    let snapshot = store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &legacy_text,
        None,
        Vec::new(),
        Some(HistoryMessageKind::Cancelled),
        Some("turn-2"),
    );

    // Assert：新节点的 turn_id 必须取本轮 turn-2，不得错标成上一轮 turn-1
    let last_node = snapshot.history_nodes.last().expect("C2b：收口后必须有节点");
    assert_eq!(
        last_node.turn_id.as_deref(),
        Some("turn-2"),
        "C2b：收口节点 turn_id 必须是本轮 turn-2（现状借 turn_trace_history.last() 错标成上一轮），实际 = {:?}",
        last_node.turn_id
    );
}

// ===========================================================================
// C3：正常续跑不得隐式 fork
// ===========================================================================

#[test]
fn c3_interrupted_resume_on_accident_shaped_cursor_does_not_fork_implicitly() {
    // Arrange：以事故形态持久化状态装载（visible 停在较早检查点节点，branch head 已前进）
    let sandbox = isolate_process_data_dir();
    let state = crafted_accident_state();
    let (mut store, _guard) = PreloadedBackendGuard::from_state(&sandbox, "c3", &state);
    let before = store
        .get_session(SID)
        .expect("C3：预置会话必须能被装载（夹具自检）")
        .clone();
    assert_eq!(
        before.history_branches.len(),
        1,
        "C3 夹具自检：装载时只允许存在 1 条分支，实际 {:?}",
        before.history_branches
    );
    assert_ne!(
        before.history_cursor.visible_node_id, before.history_cursor.branch_head_node_id,
        "C3 夹具自检：本用例必须复现 visible ≠ branch_head 的事故形态"
    );
    let active_branch_before = before.history_cursor.active_branch_id.clone();
    let closed_steps = closed_steps();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending_calls());
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending_calls())
        .expect("C3：已闭合步骤非空 ⇒ 必须产出 native 闭合片段");

    // Act：中断收口（不是显式 rewind/checkout）
    let snapshot = store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &assistant_text,
        Some(closure),
        Vec::new(),
        Some(HistoryMessageKind::Cancelled),
        Some("turn-2"),
    );

    // Assert 1：分支数不增加（只有显式 rewind/checkout 才允许 fork）
    assert_eq!(
        snapshot.history_branches.len(),
        1,
        "C3：中断收口不得隐式 fork，实际分支 = {:?}",
        snapshot
            .history_branches
            .iter()
            .map(|branch| (branch.branch_id.clone(), branch.base_node_id.clone(), branch.forked_from_branch_id.clone()))
            .collect::<Vec<_>>()
    );

    // Assert 2：active_branch_id 不变
    assert_eq!(
        snapshot.history_cursor.active_branch_id, active_branch_before,
        "C3：中断收口不得切换活动分支，实际 = {:?}（收口前 {:?}）",
        snapshot.history_cursor.active_branch_id, active_branch_before
    );

    // Assert 3：收口后光标自洽（不得停留在 HistoricalDirty 漂移态）
    assert_eq!(
        snapshot.history_cursor.visible_node_id, snapshot.history_cursor.branch_head_node_id,
        "C3：收口后 visible 必须回到 branch_head（否则下一轮继续漂移）"
    );
}

#[test]
fn c3b_explicit_fork_path_still_creates_a_branch() {
    // Arrange：显式 fork 是唯一允许增加分支的入口（防止 C3 修法把正当能力一并砍掉）
    let sandbox = isolate_process_data_dir();
    let mut state = crafted_accident_state();
    // 显式 fork 从"当前可见点"起步：此处让它与 branch head 一致，去掉干扰变量。
    state.history_cursor.visible_node_id = Some("node-2".to_string());
    let (mut store, _guard) = PreloadedBackendGuard::from_state(&sandbox, "c3b", &state);
    assert_eq!(
        store.get_session(SID).map(|s| s.history_branches.len()),
        Some(1),
        "C3b 夹具自检：装载后应只有 1 条分支"
    );

    // Act
    let forked = store.fork_from_history_node(Some(SID), "node-2", None);

    // Assert
    let forked = forked.expect("C3b：显式 fork 必须成功（不得因 C3 修法被关死）");
    assert_eq!(
        forked.history_branches.len(),
        2,
        "C3b：显式 fork 必须新增 1 条分支，实际 = {:?}",
        forked.history_branches.iter().map(|b| b.branch_id.clone()).collect::<Vec<_>>()
    );
}

/// C3 基线回归腿：**不经过新纯函数**，直接复现事故形态（visible ≠ branch_head）再收口。
///
/// 价值：把「中断收口因 visible≠head 隐式 fork」这条 RC2 缺陷钉成真实断言失败；
/// 修好之后本用例必须绿（可见点/分支头自洽，且只有显式 rewind 才 fork）。
#[test]
fn c3c_legacy_interrupted_append_does_not_fork_on_accident_shaped_cursor() {
    // Arrange
    let sandbox = isolate_process_data_dir();
    let state = crafted_accident_state();
    let (mut store, _guard) = PreloadedBackendGuard::from_state(&sandbox, "c3c", &state);
    let before = store.get_session(SID).expect("C3c：预置会话必须能装载").clone();
    assert_ne!(
        before.history_cursor.visible_node_id, before.history_cursor.branch_head_node_id,
        "C3c 夹具自检：必须复现 visible ≠ branch_head"
    );
    let legacy_text = interrupted_turn_checkpoint_text(DELIVERED_TEXT, &[])
        .expect("C3c：既有检查点纯函数必须产出文本");

    // Act（既有收口入口，零新纯函数参与）
    let snapshot = store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &legacy_text,
        None,
        Vec::new(),
        Some(HistoryMessageKind::Cancelled),
        Some("turn-2"),
    );

    // Assert：中断收口不得隐式 fork
    assert_eq!(
        snapshot.history_branches.len(),
        1,
        "C3c：中断收口不得隐式 fork（现状会建 …-branch-2），实际分支 = {:?}",
        snapshot
            .history_branches
            .iter()
            .map(|b| (b.branch_id.clone(), b.forked_from_branch_id.clone(), b.base_node_id.clone()))
            .collect::<Vec<_>>()
    );
}

// ===========================================================================
// C4：展示一行状态 + 步骤可查，渲染文本无占位符连排/参数残片
// ===========================================================================

#[test]
fn c4_interrupted_turn_renders_one_status_line_and_keeps_steps_in_detail() {
    // Arrange
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);

    // Act：取消轮 / 失败轮的一行状态（既有展示入口）
    let cancelled_line = tui::interrupted_record_status_line(&assistant_text, Some(HistoryMessageKind::Cancelled));
    let failed_line = tui::interrupted_record_status_line(&assistant_text, Some(HistoryMessageKind::Interrupted));

    // Assert 1：两种中断载体的状态行都只出一行，且措辞正确
    let cancelled_line = cancelled_line.expect("C4：kind=Cancelled 必须渲染一行状态（不得把过程清单当正文直出）");
    assert!(
        cancelled_line.contains("已停止"),
        "C4：取消轮状态行必须表达`已停止`，实际 = {cancelled_line:?}"
    );
    assert!(
        !cancelled_line.contains('\n'),
        "C4：状态行必须是单行，实际 = {cancelled_line:?}"
    );
    assert!(
        cancelled_line.chars().count() <= 60,
        "C4：状态行必须是一句话（≤60 字符），实际 {} 字符 = {cancelled_line:?}",
        cancelled_line.chars().count()
    );
    let failed_line = failed_line.expect("C4：kind=Interrupted 必须渲染一行状态");
    assert!(
        failed_line.contains("已中断"),
        "C4：失败/上游中断轮状态行必须表达`已中断`，实际 = {failed_line:?}"
    );
    assert!(!failed_line.contains('\n'), "C4：失败轮状态行必须是单行，实际 = {failed_line:?}");

    // Assert 2：步骤明细可查（进"过程明细"而不是丢进信息流正文）
    let detail_lines = visible_step_lines(&closed_steps, &pending);
    assert_eq!(
        detail_lines.len(),
        closed_steps.len() + pending.len(),
        "C4：过程明细必须逐条可查（闭合步骤 + 未开始调用），实际 = {detail_lines:?}"
    );
    for (index, step) in closed_steps.iter().enumerate() {
        assert!(
            detail_lines[index].contains(&step.name),
            "C4：明细第 {index} 行必须带工具名，实际 = {:?}",
            detail_lines[index]
        );
        assert!(
            !detail_lines[index].contains('\n'),
            "C4：明细每行必须单行，实际 = {:?}",
            detail_lines[index]
        );
    }

    // Assert 3：渲染文本不得含 `［工具调用］` 连排或参数残片
    let dirty_render = format!(
        "{assistant_text}\n［工具调用］［工具调用］［工具调用］\nparameter name=\"command\">cargo test"
    );
    let rendered = tui::sanitize_model_protocol_text(&dirty_render).0;
    assert!(
        !rendered.contains("parameter name="),
        "C4：渲染文本不得残留 DSML 参数残片，实际 = {rendered:?}"
    );
    assert!(
        !rendered.contains("［工具调用］［工具调用］"),
        "C4：渲染文本不得出现 `［工具调用］` 连排，实际 = {rendered:?}"
    );
    let placeholder_runs = rendered.matches("［工具调用］").count();
    assert!(
        placeholder_runs <= 1,
        "C4：占位符最多一个（禁止连排放大），实际 {placeholder_runs} 个 = {rendered:?}"
    );
    // 明细与正文都不得把协议原文带给用户
    for marker in ["|DSML|", "｜DSML｜｜", "invoke name="] {
        assert!(
            !assistant_text.contains(marker),
            "C4：承载文本不得含协议片段 {marker:?}，实际 = {assistant_text:?}"
        );
    }
}

// ===========================================================================
// C5：折叠不写回持久化正文
// ===========================================================================

const RAW_WITH_DSML: &str = "先说明计划：保留正文。\n<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n<｜｜DSML｜｜ parameter name=\"command\">cargo test --all</｜｜DSML｜｜ parameter>\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>\n然后给结论。";

#[test]
fn c5_folding_never_writes_back_into_persisted_body() {
    // Arrange
    let raw = RAW_WITH_DSML;

    // Act
    let persisted = persisted_assistant_text(raw);
    let display = tui::sanitize_model_protocol_text(raw).0;

    // Assert 1：持久化正文只保留已交付正文（协议块整体丢弃，不留占位符）
    assert!(
        persisted.contains("先说明计划：保留正文。"),
        "C5：已交付正文不得被折叠误伤，实际 = {persisted:?}"
    );
    assert!(
        persisted.contains("然后给结论。"),
        "C5：DSML 块之后的正文必须保留，实际 = {persisted:?}"
    );
    for forbidden in [
        "［工具调用］",
        "|DSML|",
        "｜DSML｜｜",
        "invoke name=",
        "parameter name=",
        "cargo test --all",
    ] {
        assert!(
            !persisted.contains(forbidden),
            "C5：持久化正文不得含协议折叠产物 {forbidden:?}（折叠只发生在展示侧），实际 = {persisted:?}"
        );
    }

    // Assert 2：纯协议正文 ⇒ 持久化为空（不得出现只剩占位符的垃圾正文）
    let protocol_only = persisted_assistant_text("<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>");
    assert!(
        protocol_only.trim().is_empty(),
        "C5：纯协议正文必须折叠成空串（不留占位符），实际 = {protocol_only:?}"
    );

    // Assert 3：幂等（重复写入不得放大）
    assert_eq!(
        persisted_assistant_text(&persisted),
        persisted,
        "C5：持久化清洗必须幂等（旧脏行不得被再次放大）"
    );

    // Assert 4：展示侧允许折叠而不是把协议原文给用户
    for marker in ["|DSML|", "｜DSML｜｜", "invoke name=", "parameter name=", "cargo test --all"] {
        assert!(
            !display.contains(marker),
            "C5：展示文本不得泄漏协议片段 {marker:?}，实际 = {display:?}"
        );
    }
    assert!(
        display.matches("［工具调用］").count() <= 1,
        "C5：展示占位符不得连排，实际 = {display:?}"
    );

    // Assert 5：旧库既有脏行只读兼容（占位符 + 参数残片不得被再放大）
    let legacy_dirty = "［工具调用］\nparameter name=\"command\">cargo test\n正文残留";
    let legacy_cleaned = persisted_assistant_text(legacy_dirty);
    assert!(
        legacy_cleaned.matches("［工具调用］").count() <= 1,
        "C5：旧脏行的占位符不得被放大，实际 = {legacy_cleaned:?}"
    );
    assert!(
        !legacy_cleaned.contains("parameter name="),
        "C5：旧脏行的参数残片必须被丢弃，实际 = {legacy_cleaned:?}"
    );
}

// ===========================================================================
// C6：恢复幂等
// ===========================================================================

#[test]
fn c6_repeated_recovery_is_idempotent_and_does_not_duplicate_turns() {
    // Arrange：先落一次中断收口
    let sandbox = isolate_process_data_dir();
    let mut store = SessionStore::memory_only();
    store.append_turn(Some(SID), "第一轮问题", "第一轮回答", None, Vec::new());
    store.record_turn_trace_in_memory(Some(SID), trace("turn-1"));
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("C6：已闭合步骤非空 ⇒ 必须产出 native 闭合片段");
    let _ = store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &assistant_text,
        Some(closure.clone()),
        Vec::new(),
        Some(HistoryMessageKind::Cancelled),
        Some("turn-2"),
    );
    let persisted_state = store
        .get_session(SID)
        .expect("C6：收口后会话必须存在")
        .clone();

    // Act：连续两次"重启 + 恢复续跑"
    let mut requests = Vec::new();
    let mut native_bodies = Vec::new();
    let mut snapshots = Vec::new();
    for round in 0..2 {
        let (mut reopened, _guard) =
            PreloadedBackendGuard::from_state(&sandbox, &format!("c6-{round}"), &persisted_state);
        let state = reopened
            .get_session(SID)
            .expect("C6：重启后会话必须仍可装载")
            .clone();
        let snapshot = snapshot_from_state(&state);
        let request = build_request(&native_provider(), &snapshot, NEXT_USER);
        native_bodies.push(
            serde_json::to_string(&request.native_messages).expect("C6：native_messages 必须可序列化"),
        );
        requests.push(serde_json::to_string(&request).expect("C6：请求必须可序列化"));
        snapshots.push(snapshot);
    }

    // Assert 1：两次恢复产出的请求逐字相同（不重复注入 transcript）
    assert_eq!(
        requests[0], requests[1],
        "C6：同一状态两次恢复必须产出逐字相同的请求（禁止重复注入 transcript）"
    );

    // Assert 2：闭合结构在 **native 通道内**恰好出现一次（不重复注入 transcript）。
    // 注：`ProviderRequest` 同时序列化 input carry 与 observation 文本段（1518 基线钉住的
    // 既有形状），故全局计数不具判据意义；重复注入只可能发生在 native 通道。
    let sentinel_occurrences = native_bodies[0].matches(RESULT_ALPHA).count();
    assert_eq!(
        sentinel_occurrences, 1,
        "C6：结果哨兵串在 native 通道内必须恰好出现 1 次（实际 {sentinel_occurrences} 次），native = {}",
        native_bodies[0]
    );
    assert_eq!(
        native_bodies[0], native_bodies[1],
        "C6：两次恢复的 native 通道必须逐字一致（禁止重复注入 transcript）"
    );
    assert!(
        requests[0].contains(RESULT_BETA),
        "C6：恢复请求必须仍携带全部已闭合结果，实际请求体 = {}",
        requests[0]
    );

    // Assert 3：不新增重复轮次（历史条数 / 节点条数稳定）
    assert_eq!(
        snapshots[0].history.len(),
        snapshots[1].history.len(),
        "C6：两次恢复的 history 长度必须一致（不得每次恢复都新增轮次）"
    );
    assert_eq!(
        snapshots[0].history_nodes.len(),
        snapshots[1].history_nodes.len(),
        "C6：两次恢复的 history_nodes 数必须一致"
    );
    let turn_two_rows = snapshots[0]
        .history
        .iter()
        .filter(|message| message.turn_id.as_deref() == Some("turn-2"))
        .count();
    assert_eq!(
        turn_two_rows, 2,
        "C6：turn-2 在历史里必须恰好 user+assistant 两条，实际 {turn_two_rows} 条"
    );

    // Assert 4：seq/水位单调且不重复注入（恢复不得回拨水位、不得重新生成节点）
    let watermark_first = snapshots[0].history_cursor.event_watermark;
    let watermark_second = snapshots[1].history_cursor.event_watermark;
    assert!(
        watermark_second >= watermark_first,
        "C6：恢复后事件水位必须单调不回退（{watermark_first} → {watermark_second}）"
    );
    let node_ids_first: Vec<String> = snapshots[0]
        .history_nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    let node_ids_second: Vec<String> = snapshots[1]
        .history_nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect();
    assert_eq!(
        node_ids_first, node_ids_second,
        "C6：两次恢复的节点 id 序列必须逐字一致（不得重新生成/追加节点）"
    );
    assert_eq!(
        node_ids_first.len(),
        node_ids_first.iter().collect::<std::collections::HashSet<_>>().len(),
        "C6：节点 id 不得重复（重复即 seq/节点编号不单调）"
    );
}

// ===========================================================================
// C7：失败轮与用户中断同构（同一闭合出口，失败原因保留）
// ===========================================================================

#[test]
fn c7_failed_turn_is_isomorphic_to_user_cancelled_and_keeps_reason() {
    // Arrange
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let causes = [
        InterruptedTurnCause::UserCancelled,
        InterruptedTurnCause::TurnFailed {
            reason: "tool `Bash` exited 137".to_string(),
        },
        InterruptedTurnCause::UpstreamAborted {
            reason: "upstream socket reset".to_string(),
        },
    ];

    // Act + Assert 1：三条路径的闭合结构必须同构（角色序列一致）
    let mut shapes: Vec<Vec<String>> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for cause in &causes {
        let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
            .expect("C7：已闭合步骤非空 ⇒ 三条原因路径都必须产出闭合片段");
        assert_roundtrips_closed(&closure, "C7");
        shapes.push(closure.iter().map(|m| role_of(m).to_string()).collect());
        notes.push(interrupted_cause_note(cause));
    }
    assert_eq!(
        shapes[0], shapes[1],
        "C7：失败轮与用户中断的闭合结构必须同构（角色序列），实际 = {shapes:?}"
    );
    assert_eq!(
        shapes[0], shapes[2],
        "C7：上游中断与用户中断的闭合结构必须同构（角色序列），实际 = {shapes:?}"
    );

    // Assert 2：原因措辞分流且失败原因逐字保留
    assert!(
        notes[0].contains("用户") || notes[0].contains("取消") || notes[0].contains("终止"),
        "C7：用户取消轮必须有「用户终止」语义的事实注记，实际 = {:?}",
        notes[0]
    );
    assert!(
        notes[1].contains("tool `Bash` exited 137"),
        "C7：失败轮的失败原因必须逐字保留，实际 = {:?}",
        notes[1]
    );
    assert!(
        notes[2].contains("upstream socket reset"),
        "C7：上游中断的原因必须逐字保留，实际 = {:?}",
        notes[2]
    );
    assert_ne!(
        notes[0], notes[1],
        "C7：用户取消不得伪装成失败（两种原因的事实注记必须可区分）"
    );

    // Assert 3：落库路径同构（kind 坐实载体，节点分类同口径，失败原因由既有检查点承载）
    isolate_process_data_dir();
    let mut cancelled_store = SessionStore::memory_only();
    let mut failed_store = SessionStore::memory_only();
    let cancelled_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let reason = "tool `Bash` exited 137";
    let failed_base = failed_turn_checkpoint_text(DELIVERED_TEXT, &[], reason)
        .expect("C7：失败轮检查点必须产出文本并保留原因");
    assert!(
        failed_base.contains("失败原因：") && failed_base.contains(reason),
        "C7：失败轮检查点必须逐字保留失败原因，实际 = {failed_base:?}"
    );
    let cancelled_snapshot = cancelled_store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &cancelled_text,
        None,
        Vec::new(),
        Some(HistoryMessageKind::Cancelled),
        Some("turn-c"),
    );
    let failed_snapshot = failed_store.append_turn_with_kind(
        Some(SID),
        INTERRUPTED_USER,
        &failed_base,
        None,
        Vec::new(),
        Some(HistoryMessageKind::Interrupted),
        Some("turn-f"),
    );
    let cancelled_node = cancelled_snapshot.history_nodes.last().expect("C7：取消轮必须落节点");
    let failed_node = failed_snapshot.history_nodes.last().expect("C7：失败轮必须落节点");
    assert_eq!(
        cancelled_node.kind, failed_node.kind,
        "C7：取消轮与失败轮必须走同一节点分类口径（实际 {:?} vs {:?}）",
        cancelled_node.kind, failed_node.kind
    );
    assert_eq!(
        cancelled_snapshot.history.len(),
        failed_snapshot.history.len(),
        "C7：两条路径写进历史的条数必须同构"
    );
    assert!(
        failed_snapshot
            .history
            .iter()
            .any(|message| message.content.contains(reason)),
        "C7：失败原因必须留在失败轮历史里，实际 = {:?}",
        failed_snapshot.history.iter().map(|m| m.content.clone()).collect::<Vec<_>>()
    );

    // Assert 4：悬空调用的合成结果必须显式表达"结果未知/未开始"，不伪装成功
    let note = pending_tool_result_note(&pending[0].name);
    assert!(
        note.contains(&pending[0].name),
        "C7：悬空调用的合成结果必须点名工具，实际 = {note:?}"
    );
    assert!(
        note.contains("结果未知") || note.contains("未开始") || note.contains("未返回"),
        "C7：悬空调用的合成结果必须携带不确定性事实（不得伪装成功），实际 = {note:?}"
    );
    assert!(
        !note.contains("成功") && !note.contains("succeeded"),
        "C7：悬空调用的合成结果不得伪装成功，实际 = {note:?}"
    );
}

// ===========================================================================
// C8：向后兼容（旧会话可装载、成功轮上下文逐字不变）
// ===========================================================================

#[test]
fn c8_legacy_session_loads_and_successful_turn_context_stays_verbatim() {
    // Arrange（一）：旧会话 JSON（无本轮新增字段）必须能装载
    let legacy_json = json!({
        "conversationId": "pa_legacy",
        "summary": "",
        "history": [
            { "role": "user", "content": "老会话提问" },
            { "role": "assistant", "content": "老会话回答" }
        ],
        "turnCount": 1,
        "lastReferencedFile": null
    });
    let legacy_state: SessionState =
        serde_json::from_value(legacy_json).expect("C8：无新增字段的旧会话必须仍可反序列化（零迁移）");

    // Assert 1：旧会话装载后字段逐字保真
    assert_eq!(legacy_state.conversation_id, "pa_legacy", "C8：会话 id 必须保真");
    assert_eq!(legacy_state.history.len(), 2, "C8：旧会话历史条数必须保真");
    assert_eq!(
        legacy_state.history[1].content, "老会话回答",
        "C8：旧会话正文必须逐字保真"
    );
    assert!(
        legacy_state.history_nodes.is_empty() && legacy_state.history_branches.is_empty(),
        "C8：旧会话不得被凭空补出历史图节点/分支"
    );

    // Arrange（二）：成功轮（无中断痕迹）的上下文必须逐字不变
    let success_history = vec![
        turn_message("user", "成功轮问题", Some("turn-ok"), None),
        turn_message("assistant", "成功轮回答正文", Some("turn-ok"), None),
    ];
    let success_session = snapshot_of("c8-success", success_history, Vec::new());

    // Act
    let success_body = request_body(&native_provider(), &success_session, NEXT_USER);
    let legacy_snapshot = snapshot_from_state(&legacy_state);
    let legacy_body = request_body(&native_provider(), &legacy_snapshot, NEXT_USER);

    // Assert 2：成功轮正文逐字进入请求，且不被中断相关结构污染
    assert!(
        success_body.contains("成功轮回答正文"),
        "C8：成功轮 assistant 正文必须逐字进入请求，实际 = {success_body}"
    );
    assert!(
        success_body.contains("Previous assistant reply: 成功轮回答正文"),
        "C8：成功轮的承载文本格式必须逐字不变（`Previous assistant reply: ` 前缀），实际 = {success_body}"
    );
    for forbidden in ["[中断轮次检查点]", INTERRUPTED_RESULT_SECTION_HEADER, INTERRUPTED_PENDING_NOTE_PREFIX] {
        assert!(
            !success_body.contains(forbidden),
            "C8：成功轮请求不得出现中断轮结构 {forbidden:?}，实际 = {success_body}"
        );
        assert!(
            !legacy_body.contains(forbidden),
            "C8：旧会话请求不得出现中断轮结构 {forbidden:?}，实际 = {legacy_body}"
        );
    }
    assert!(
        legacy_body.contains("老会话回答"),
        "C8：旧会话续跑必须仍能读到旧正文，实际 = {legacy_body}"
    );

    // Assert 3：无新增列/迁移的机械证据 —— 旧 JSON 里没有的字段全部走 serde 缺省
    assert!(
        legacy_state.provider_native_transcript.is_empty(),
        "C8：旧会话无 transcript 时必须缺省为空（不得伪造）"
    );

    // Assert 4：清洗对"干净正文"必须逐字不动（不得因 C5 修法误伤成功轮）
    let clean = "这是一段完全干净的模型正文：没有协议块。";
    assert_eq!(
        persisted_assistant_text(clean),
        clean,
        "C8：干净正文必须逐字保真（C5 修法不得误伤成功轮）"
    );
}

// ===========================================================================
// C9（AMEND-2）：生产收口路径接线守卫 + 闭合结构行为断言
// ===========================================================================

/// **wiring guard，不是行为断言**：本用例前半段只检查"生产收口代码真的调用了冻结纯函数"。
///
/// 存在理由（Lead 发现）：纯函数实现完整、单元级断言全绿，但 `persist_cancelled_turn_outcome` /
/// `persist_turn_outcome` 仍然只写检查点文本 ⇒ 生产语义未变（假绿）。行为断言在函数内部全绿
/// 也无法暴露"没接线"，只有对收口入口源码做结构性检查才能挡住。
#[test]
fn c9_runtime_close_out_path_actually_uses_closed_structure() {
    // ── 结构性守卫（wiring guard）──────────────────────────────────────────
    let close_out_source = include_str!("../src/agent/runtime/turn_persist.rs");
    assert!(
        close_out_source.contains("interrupted_turn_assistant_text"),
        "C9：生产收口路径（agent/runtime/turn_persist.rs）必须调用 interrupted_turn_assistant_text，\
         否则纯函数实现不接线（假绿）——检查 persist_cancelled_turn_outcome / persist_turn_outcome"
    );
    assert!(
        close_out_source.contains("interrupted_turn_transcript_closure"),
        "C9：生产收口路径（agent/runtime/turn_persist.rs）必须调用 interrupted_turn_transcript_closure，\
         否则中断轮工具往返仍无 native 载体（上下文丢失根因未解决）"
    );

    // ── 行为断言：真实形态活动 → 承接文本 + 闭合片段 ───────────────────────
    let closed_steps = vec![
        closed("Read", "C9_RESULT_READ 配置里 port=8080"),
        closed("Bash", "C9_RESULT_BASH systemctl restart staging 已执行"),
    ];
    let pending = vec![PendingToolCall {
        name: "Write".to_string(),
    }];
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("C9：已闭合步骤非空 ⇒ 必须产出 native 闭合片段");

    // Assert 1：承载文本复用既有检查点头（1518 基线逐字格式不得改）
    assert!(
        assistant_text.starts_with("[中断轮次检查点]"),
        "C9：承载文本必须逐字复用既有检查点头 `[中断轮次检查点]`，实际 = {assistant_text:?}"
    );
    assert!(
        assistant_text.contains(DELIVERED_TEXT),
        "C9：承载文本必须保留已交付正文，实际 = {assistant_text:?}"
    );
    // Assert 2：每条结果摘要可查（过程事实不是可选装饰）
    for step in &closed_steps {
        assert!(
            assistant_text.contains(&step.result_summary),
            "C9：承载文本必须逐条携带结果摘要 {:?}，实际 = {assistant_text:?}",
            step.result_summary
        );
    }
    // Assert 3：未开始调用必须带显式"结果未知/未开始"注记
    assert!(
        assistant_text.contains(INTERRUPTED_PENDING_NOTE_PREFIX),
        "C9：未开始调用必须带注记 {INTERRUPTED_PENDING_NOTE_PREFIX:?}，实际 = {assistant_text:?}"
    );
    assert!(
        assistant_text.contains(&pending[0].name),
        "C9：未开始调用必须点名工具 {:?}，实际 = {assistant_text:?}",
        pending[0].name
    );

    // Assert 4：native 闭合片段严格配对，悬空调用必须有 error 结果
    assert_roundtrips_closed(&closure, "C9");
    let serialized = serde_json::to_string(&closure).expect("C9：闭合片段必须可序列化");
    let pending_note = pending_tool_result_note(&pending[0].name);
    assert!(
        serialized.contains(&pending_note),
        "C9：悬空调用必须以合成的 error 结果闭合（内容 = pending_tool_result_note），实际 = {serialized}"
    );
    let tool_messages = closure.iter().filter(|message| role_of(message) == "tool").count();
    assert_eq!(
        tool_messages,
        closed_steps.len() + pending.len(),
        "C9：tool 结果数必须等于已闭合步骤 + 未开始调用（补齐优先于截断），实际 {tool_messages}"
    );
    for step in &closed_steps {
        assert!(
            serialized.contains(&step.name) && serialized.contains(&step.result_summary),
            "C9：闭合片段必须携带 {:?} 的调用与结果，实际 = {serialized}",
            step.name
        );
    }
}

// ===========================================================================
// C10（AMEND-3）：真实 sqlite `load_store` 重启路径（不是内存 dump 的"假重启"）
// ===========================================================================

/// C6 的重启走的是 `PreloadedBackendGuard`（把内存 dump 回吐给 `with_backend`），
/// 未覆盖真实落盘 → `SqliteSessionBackend::load_store` 的装载路径。本用例补上：
/// 用真实 sqlite 库（临时目录，绝不触碰用户库）落一次中断收口，关闭后**重新打开**装载，
/// 断言中断轮行仍在、`turn_id` 归属本轮、历史无重复轮。
#[test]
fn c10_real_sqlite_load_store_preserves_interrupted_turn_without_duplication() {
    // Arrange：真实 sqlite 落盘（XDG 沙箱内临时库）
    let sandbox = isolate_process_data_dir();
    let db_path = sandbox.join("c10-interrupted-turn.db");
    let closed_steps = closed_steps();
    let pending = pending_calls();
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("C10：已闭合步骤非空 ⇒ 必须产出 native 闭合片段");
    {
        let backend = SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let mut store = SessionStore::with_backend(Box::new(backend));
        // AMEND-4：夹具第一轮改用**带轮次身份**的写入口。`append_turn` 是无 turn_id 的
        // legacy 直连签名（真实 sqlite 取证显示该轮两行 turn_id 写入时即 NULL），而生产路径
        // `persist_turn_outcome` 恒传 turn_id ⇒ 夹具必须显式声明轮次身份，否则测的是伪造场景。
        store.append_turn_with_kind(
            Some(SID),
            "第一轮问题",
            "第一轮回答",
            None,
            Vec::new(),
            None,
            Some("turn-1"),
        );
        store.record_turn_trace_in_memory(Some(SID), trace("turn-1"));
        let written = store.append_turn_with_kind(
            Some(SID),
            INTERRUPTED_USER,
            &assistant_text,
            Some(closure),
            Vec::new(),
            Some(HistoryMessageKind::Cancelled),
            Some("turn-2"),
        );
        assert!(
            written
                .history
                .iter()
                .any(|message| message.turn_id.as_deref() == Some("turn-2")),
            "C10 夹具自检：写入后本进程视图里必须能看到 turn-2，实际 = {:?}",
            written.history.iter().map(|m| (m.role.clone(), m.turn_id.clone())).collect::<Vec<_>>()
        );
        store.release_log_writers_for_test();
    }

    // Act：走真实 `load_store`（新句柄、新连接、从磁盘读）
    let reopened_backend = SqliteSessionBackend::new_with_trace_mode(
        db_path.clone(),
        SeparateTraceTableMode::WriteSeparate,
    );
    let mut reopened = SessionStore::with_backend(Box::new(reopened_backend));
    let state = reopened
        .get_session(SID)
        .expect("C10：真实 sqlite 重启后会话必须仍可装载（load_store 路径）")
        .clone();

    // Assert 1：中断轮行仍在，且 kind/turn_id 归属本轮
    let interrupted_rows: Vec<&TurnHistoryMessage> = state
        .history
        .iter()
        .filter(|message| message.turn_id.as_deref() == Some("turn-2"))
        .collect();
    assert_eq!(
        interrupted_rows.len(),
        2,
        "C10：turn-2 必须恰好 user+assistant 两条（持久化后仍完整），实际 = {:?}",
        state.history.iter().map(|m| (m.role.clone(), m.turn_id.clone())).collect::<Vec<_>>()
    );
    let interrupted_assistant = interrupted_rows
        .iter()
        .find(|message| message.role == "assistant")
        .expect("C10：turn-2 的 assistant 合成行必须存在");
    assert_eq!(
        interrupted_assistant.kind,
        Some(HistoryMessageKind::Cancelled),
        "C10：重启后合成行的 kind 必须保真（否则展示层退化为文本嗅探）"
    );
    assert!(
        interrupted_assistant.content.contains("[中断轮次检查点]"),
        "C10：重启后承载文本必须仍带检查点头，实际 = {:?}",
        interrupted_assistant.content
    );

    // Assert 1b（装载侧交叉断言，AMEND-3 补强）：「行在但内容丢」也必须挡——
    // 重启后的中断轮 assistant 正文必须仍带结果段与未开始注记，而不只是"两行存在"。
    assert!(
        interrupted_assistant.content.contains(INTERRUPTED_RESULT_SECTION_HEADER),
        "C10：重启后中断轮正文必须含结果段标题 {INTERRUPTED_RESULT_SECTION_HEADER:?}（内容不得丢），实际 = {:?}",
        interrupted_assistant.content
    );
    assert!(
        interrupted_assistant.content.contains(INTERRUPTED_PENDING_NOTE_PREFIX),
        "C10：重启后中断轮正文必须含未开始注记 {INTERRUPTED_PENDING_NOTE_PREFIX:?}，实际 = {:?}",
        interrupted_assistant.content
    );
    for step in &closed_steps {
        assert!(
            interrupted_assistant.content.contains(&step.result_summary),
            "C10：重启后中断轮正文必须仍携带结果摘要 {:?}（装载不得裁掉过程事实），实际 = {:?}",
            step.result_summary,
            interrupted_assistant.content
        );
    }

    // Assert 2：无重复轮（同一 assistant 合成正文只出现一次；turn-1 也未被复制）
    let duplicated = state
        .history
        .iter()
        .filter(|message| message.content == interrupted_assistant.content)
        .count();
    assert_eq!(
        duplicated, 1,
        "C10：中断轮合成正文在历史里必须恰好一条（重启不得重复注入），实际 {duplicated} 条"
    );
    let turn_one_rows = state
        .history
        .iter()
        .filter(|message| message.turn_id.as_deref() == Some("turn-1"))
        .count();
    assert_eq!(
        turn_one_rows, 2,
        "C10：turn-1 必须恰好 user+assistant 两条（不得被重启复制），实际 {turn_one_rows} 条"
    );

    // Assert 3：历史节点 turn_id 归属本轮（真实库也要正确）
    if let Some(node) = state
        .history_nodes
        .iter()
        .find(|node| node.turn_id.as_deref() == Some("turn-2"))
    {
        assert_eq!(
            node.turn_id.as_deref(),
            Some("turn-2"),
            "C10：重启后节点 turn_id 必须仍是 turn-2，实际 = {:?}",
            node.turn_id
        );
    }

    // Assert 4：重启后的下一轮请求仍携带工具往返（上下文真值不因重启丢失）
    let snapshot = snapshot_from_state(&state);
    let body = serde_json::to_string(
        &build_request(&native_provider(), &snapshot, NEXT_USER).native_messages,
    )
    .expect("C10：native_messages 必须可序列化");
    for step in &closed_steps {
        assert!(
            body.contains(&step.name) && body.contains(&step.result_summary),
            "C10：重启后 native 通道必须仍带 {:?} 的调用与结果，实际 = {body}",
            step.name
        );
    }
}
