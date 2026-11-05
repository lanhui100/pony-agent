//! Wave-3 / L3-B 遗留真实反例的补丁红相（独立 Test Agent 追加，Executor 只读）。
//!
//! 反例来源：Lead 实跑 L3-B 复现脚本 `/tmp/l3b_cmds.sh` 与本机 sessions.db
//! （`normalized_messages` rowid 166/184 已存 DSML 原文）。四类缺口：
//!  A1 `provider::sanitize_dsml_protocol_text` 对大小写/半角/混合 DSML 变体绕过；
//!  A2 `context.rs::to_provider_history_message` / `history_to_native_messages`
//!     不清洗历史 ⇒ 已落盘 DSML 原文回灌 provider；
//!  A3 `tui.rs::hydrate_state_from_session` 的 message_state fallback 直接
//!     `content: entry.content` 未清洗；
//!  A4 `store.rs::classify_turn_node_kind` 把含 `[中断轮次检查点]` 的失败轮
//!     改判 `TurnCommitted`（既有 `TurnCancelled` 语义漂移）。
//!
//! 铁律：全部真 assert；不改生产代码、不改已冻结验收文件、不给生产代码加 pub、
//! 不 `#[ignore]`、不吞异常；临时存储谁创建谁清理。
//! 私有函数一律通过**公开接缝**（`DefaultContextStateRetriever` /
//! `DefaultTurnContextBuilder::build_request` / `SessionStore::append_failed_turn_fallible` /
//! `SessionStore::load_history_graph`）观测真实行为；A3 无公开接缝，采用静态源码
//! 断言并显式标注 `[RED DEGRADED: STATIC-ONLY]`，附判别力自检，不谎称动态通过。

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    ContextStateQuery, ContextStateRetriever, DefaultContextStateRetriever,
    DefaultTurnContextBuilder, TurnContextBuilder,
};
use pony_agent_core::agent::provider::{
    sanitize_dsml_protocol_text, ProviderAuthType, ProviderManager, ProviderProtocol,
};
use pony_agent_core::agent::session::{
    FileSessionBackend, HistoryNodeKind, SessionStore, TurnHistoryMessage, TurnTraceRecord,
};
use std::fs;
use std::path::PathBuf;

const DSML_INVOKE: &str = "<｜｜DSML｜｜ invoke name=\"Run\">\n<｜｜DSML｜｜ parameter name=\"command\">cargo test</｜｜DSML｜｜ parameter>\n</｜｜DSML｜｜ invoke>";
const LEAK_MARKERS: [&str; 4] = [
    "invoke name=",
    "parameter name=",
    "｜DSML｜｜",
    "｜dsml｜｜",
];

fn assert_no_dsml_leak(ticket: &str, out: &str) {
    for marker in LEAK_MARKERS {
        assert!(
            !out.contains(marker),
            "{ticket}：不得泄漏 DSML 协议片段 {marker:?}，实际 = {out:?}"
        );
    }
}

/// 隔离临时会话库（test-expert tear-down guarantee：Drop 即清理，不污染真实 ~/.local）。
struct TempSessions(PathBuf);

impl TempSessions {
    fn new(tag: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时钟必须可用")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "l3b-session-resilience-{tag}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("创建隔离会话目录");
        Self(path)
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(FileSessionBackend::new(
            self.0.join("sessions.json"),
        )))
    }
}

impl Drop for TempSessions {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn history_message(role: &str, content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

fn provider_manager(supports_reasoning: bool) -> ProviderManager {
    ProviderManager::new(ResolvedProviderSelection {
        requested_name: "l3b-probe".to_string(),
        provider_name: "l3b-probe".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: "https://example.invalid/v1".to_string(),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "L3B_PROBE_API_KEY".to_string(),
        // 仅构造用：build_request 不发网络请求，断言全部落在离线请求投影上。
        api_key: Some("l3b-test-token".to_string()),
        model: "l3b-probe-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 1024,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(256_000),
            supports_reasoning,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::None,
    })
}

// ── A1：大小写 / 半角 / 混合 DSML 变体不得绕过清洗入口 ────────────────────────

/// A1 / L3B-A01：三类变体经 `sanitize_dsml_protocol_text` 均不得泄漏协议原文。
#[test]
fn a1_dsml_case_and_width_variants_do_not_bypass_sanitizer() {
    let variants = [
        ("lower", DSML_INVOKE.replace("DSML", "dsml")),
        ("halfwidth", DSML_INVOKE.replace('｜', "|")),
        ("mixed", DSML_INVOKE.replace("｜｜", "|｜")),
    ];
    assert_eq!(variants.len(), 3, "A1 harness：必须覆盖三类变体");
    assert!(
        DSML_INVOKE.contains("invoke name="),
        "A1 harness：原始 DSML 夹具必须含协议字段（否则断言恒真）"
    );

    for (name, variant) in variants.iter() {
        // 判别力夹具：三个变体确实互不相同且都不是规范块。
        assert_ne!(
            variant, DSML_INVOKE,
            "A1 harness：变体 {name} 必须与规范块不同"
        );
        let out = sanitize_dsml_protocol_text(variant);
        assert_no_dsml_leak(&format!("A1/{name}"), &out);
        assert_ne!(
            out, *variant,
            "A1/{name}：清洗入口不得对协议变体原样放行，实际 = {out:?}"
        );
    }
}

// ── A2：已落盘 DSML 历史经公开 context 接缝不得回灌 provider ──────────────────

/// A2 / L3B-A02：以真实 `SessionSnapshot` 走 `DefaultContextStateRetriever` +
/// `DefaultTurnContextBuilder::build_request`，断言 normalized input 与
/// provider-native transcript 两路都不含 DSML 原文。
#[test]
fn a2_persisted_dsml_history_is_sanitized_before_provider_replay() {
    // Arrange：隔离会话库写入一行 user + 一行含 DSML 原文的 assistant（对应 sessions.db 存量）。
    let temp = TempSessions::new("a2");
    let mut store = temp.store();
    let session_id = "session-l3b-a2";
    store.replace_session_history(
        Some(session_id),
        vec![
            history_message("user", "请运行测试"),
            history_message("assistant", DSML_INVOKE),
        ],
    );
    let snapshot = store.snapshot(Some(session_id), &[]);
    assert!(
        snapshot
            .history
            .iter()
            .any(|message| message.content.contains("invoke name=")),
        "A2 harness：快照必须真的含未清洗 DSML 历史（否则断言恒真）"
    );

    // Act 1：公开 retriever（`history_to_native_messages` 落点）。
    let retrieved = DefaultContextStateRetriever.retrieve(ContextStateQuery {
        user_message: "继续",
        images: &[],
        workspace_mode: None,
        session: &snapshot,
        run: None,
        checkpoint: None,
    });
    let native_history =
        serde_json::to_string(&retrieved.transcript.provider_native_messages)
            .expect("native transcript 必须可序列化");
    assert!(
        native_history.contains("请运行测试"),
        "A2 harness：native transcript 必须真的由 history 构造（否则断言恒真）"
    );
    assert_no_dsml_leak("A2/history_to_native_messages", &native_history);

    // Act 2：公开 builder（`to_provider_history_message` 落点）；native 模式同时覆盖两路。
    let provider = provider_manager(true);
    let request =
        DefaultTurnContextBuilder.build_request("l3b-graph", &provider, &retrieved, &[]);
    let input_text = request
        .input
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        input_text.contains("Previous assistant reply"),
        "A2 harness：to_provider_history_message 必须真的把 assistant 历史投影进 input"
    );
    assert_no_dsml_leak("A2/to_provider_history_message", &input_text);
    let native_messages =
        serde_json::to_string(&request.native_messages).expect("native_messages 可序列化");
    assert_no_dsml_leak("A2/native_conversation_carry", &native_messages);
}

// ── A3：message_state fallback 清洗（静态源码断言，显式降级） ─────────────────

/// 判别力可复用的判据：fallback 块里必须对 `entry.content` 显式调用清洗入口。
fn fallback_is_sanitized(block: &str) -> bool {
    block.contains("sanitize_dsml_protocol_text")
        || block.contains("sanitize_model_protocol_text")
}

/// A3 / L3B-A03 [RED DEGRADED: STATIC-ONLY]：`hydrate_state_from_session` 是私有
/// 函数且 `message_state` 夹具无法在无 PTY 的集成测试里构造，故读生产源码的
/// fallback 代码块做静态判据；附「已知良/已知劣」双样本判别力自检，避免空断言。
#[test]
fn a3_message_state_fallback_sanitizes_entry_content() {
    let src = include_str!("../src/bin/tui.rs");
    let marker = "// 若 history 为空，再尝试 fallback 到 message_state";
    let start = src
        .find(marker)
        .expect("[RED DEGRADED: STATIC-ONLY] A3 harness：必须能定位 message_state fallback 块");
    let tail = &src[start..];
    let end = tail
        .find("state.scroll_offset = u32::MAX;")
        .expect("[RED DEGRADED: STATIC-ONLY] A3 harness：必须能定位 fallback 块结束边界");
    let block = &tail[..end];

    // 判别力夹具：块里确实存在裸 `entry.content` 赋值（缺陷形态），不是空块。
    assert!(
        block.contains("entry.content"),
        "[RED DEGRADED: STATIC-ONLY] A3 harness：fallback 块必须真的处理 entry.content"
    );
    let known_good = "\"user\" => { state.messages.push(ChatMessage { role: MessageRole::User, content: pony_agent_core::agent::provider::sanitize_dsml_protocol_text(&entry.content),";
    let known_bad = "content: entry.content,";
    assert!(
        fallback_is_sanitized(known_good),
        "[RED DEGRADED: STATIC-ONLY] A3 判别力自检：已知良样本必须被判为已清洗"
    );
    assert!(
        !fallback_is_sanitized(known_bad),
        "[RED DEGRADED: STATIC-ONLY] A3 判别力自检：已知劣样本不得被判为已清洗"
    );

    assert!(
        fallback_is_sanitized(block),
        "[RED DEGRADED: STATIC-ONLY] A3：message_state fallback 必须对 entry.content 调用 \
         sanitize_dsml_protocol_text / sanitize_model_protocol_text，禁止把未清洗的历史 \
         直接推入用户可见消息区。实际块 = {block:?}"
    );
}

// ── A4：检查点文本不得把失败轮改判为 TurnCommitted ───────────────────────────

/// 从真实 store 的 history graph 里找到承载指定 assistant 文本的节点。
fn node_kind_for(
    store: &mut SessionStore,
    session_id: &str,
    needle: &str,
) -> HistoryNodeKind {
    let (nodes, _branches, _cursor) = store.load_history_graph(Some(session_id));
    nodes
        .iter()
        .rev()
        .find(|node| {
            node.history
                .iter()
                .any(|message| message.content.contains(needle))
        })
        .map(|node| node.kind.clone())
        .unwrap_or_else(|| {
            panic!("A4 harness：history graph 必须存在承载 {needle:?} 的节点（nodes={}）", nodes.len())
        })
}

/// A4 / L3B-A04：`append_failed_turn_fallible` 对带 `[中断轮次检查点]` 的失败轮
/// 必须保持 `TurnCancelled`；契约**未**要求追加 System 消息，本用例不据此判 FAIL。
#[test]
fn a4_interrupted_checkpoint_turn_stays_cancelled_not_committed() {
    let temp = TempSessions::new("a4");
    let mut store = temp.store();
    let session_id = "session-l3b-a4";
    const CHECKPOINT_MARKER: &str = "[中断轮次检查点]";
    let checkpoint_text = format!(
        "{CHECKPOINT_MARKER} 已落盘步骤：Read package.json\nBash cargo test"
    );

    // Arrange/Act：真实失败轮写入（classify_turn_node_kind 的唯一生产调用点）。
    let mut trace = TurnTraceRecord::default();
    trace.turn_id = "turn-l3b-a4-checkpoint".to_string();
    trace.session_id = Some(session_id.to_string());
    let result = store.append_failed_turn_fallible(
        Some(session_id),
        "请运行测试",
        &checkpoint_text,
        trace,
    );
    assert!(
        result.is_ok(),
        "A4 harness：append_failed_turn_fallible 必须成功，实际 = {result:?}"
    );

    // Assert 1（红相核心）：带检查点的失败轮不得被改判为 Committed。
    assert_eq!(
        node_kind_for(&mut store, session_id, CHECKPOINT_MARKER),
        HistoryNodeKind::TurnCancelled,
        "A4：含检查点文本的失败轮必须保持 TurnCancelled（既有语义），不得漂移为 TurnCommitted"
    );

    // Assert 2（判别力自检/绿守卫）：既有精确取消文案仍必须判 TurnCancelled，
    // 证明上面的判据确实能区分两种 kind，不是恒真/恒假。
    let control_session = "session-l3b-a4-control";
    let mut control_trace = TurnTraceRecord::default();
    control_trace.turn_id = "turn-l3b-a4-control".to_string();
    control_trace.session_id = Some(control_session.to_string());
    assert!(
        store
            .append_failed_turn_fallible(
                Some(control_session),
                "请运行测试",
                "用户终止，发送消息可继续。",
                control_trace,
            )
            .is_ok(),
        "A4 判别力自检：控制轮必须写入成功"
    );
    assert_eq!(
        node_kind_for(&mut store, control_session, "用户终止，发送消息可继续。"),
        HistoryNodeKind::TurnCancelled,
        "A4 判别力自检：既有精确取消文案必须仍投影为 TurnCancelled"
    );
}
