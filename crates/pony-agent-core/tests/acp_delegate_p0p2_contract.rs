//! ACP 委派缺陷修复 P0–P2 —— **第一批验收契约**（wave-acp-delegate，L2-T 独立测试者）。
//!
//! 契约真源（Lead 冻结，只读）：`.dev-team/contract-matrix-acp-delegate-p0p2.md`。
//! 本文件覆盖其中 **C0.1 / C0.2 / C0.3 / C0.4 / C2.1 / C3.1 / C4.1 / C5.1**；
//! C1.* 与 C6.* 在第二批 `acp_delegate_persistence_contract.rs`。
//!
//! ## 判据口径（只锁可观察行为，不锁实现细节）
//!
//! - **C0/C2 走真实工具路径**：`ToolRouter::execute_with_context` 调 `acp_delegate`，
//!   外部 agent 是 `tests/fixtures/acp_fake_agent.sh`（本地 `sh`，零外网）；
//!   断言对象是工具结果 JSON 本身（不是 session 内部状态）。
//! - **C3 是静态源码扫描**：`src/acp/**` 非测试代码零裸 `eprintln!`（诊断统一走
//!   `agent::otel::log`，json + trace_id）。扫描器显式跳过 `#[cfg(test)] mod` 块，
//!   避免把测试断言里的 `eprintln!` 误判为违约。
//! - **C4 走真实 turn 链路**：回环 mock provider（`127.0.0.1:0`）+ 注入式 `ToolExecutor`，
//!   经 `turn_flow::register_event_persist` 捕获**持久化事件流**，断言
//!   `ToolCall/ToolResult.step` 与 `StepStart.step` 同源（不得取事件 sequence）。
//! - **C5 走真实重试链路**：回环 mock provider 先 503（可辨识耗时）再 200，断言
//!   `ProviderCallCacheRecord.attempts/failed_attempt_ms/prelude_ms` 的「仅重试时出现」语义。
//!
//! ## 红相纪律
//!
//! 每条用例的**主断言**都必须因产品能力缺失而失败（断言失败，不是编译错误），
//! 且失败文案直接指明违约契约 ID。基线事实（`c5761b27` + W1 冻结骨架）：
//! - `build_ok_result` / `build_failure_result` 不产出 `finalText` / `finalTextTruncated` / `usage`；
//! - `src/acp/**` 有 32 处裸 `eprintln!`；
//! - `build_tool_event` 的 `step` 取 `payload.sequence`（事件序号）而非 hop 索引；
//! - `ProviderCallCacheRecord.attempts/failed_attempt_ms/prelude_ms` 只有字段声明，无赋值。
//!
//! 纪律：无 `#[ignore]`、无盲等 sleep（等待同步一律走有界条件轮询）、无 `assert!(true)`
//! 类恒真断言、临时目录用完即删。
//!
//! 本文件由 Test Agent 独占写域；实施者只读不可改。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::context::DefaultTurnContextBuilder;
use pony_agent_core::agent::planner::LocalTurnPlanner;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntime, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::telemetry::DefaultTurnTelemetryBuilder;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult, ToolRouter};
use pony_agent_core::agent::turn_event::TurnEvent;
use pony_agent_core::agent::turn_flow::{
    clear_event_persist, register_event_persist, TurnEventSink,
};
use serde_json::{json, Value};

// ══════════════════════════ 公共常量 / 契约真值 ══════════════════════════

/// C0.3：`finalText` 的字符上限（真源 `acp::result::ACP_DELEGATE_FINAL_TEXT_MAX_CHARS`，
/// 该模块 `pub(crate)` ⇒ 集成测试侧按契约值冻结同一数字）。
const FINAL_TEXT_MAX_CHARS: usize = 32_000;

/// C0.3 超长正文的正文体量（必须显著越过上限，才能证明是「截断」而非「恰好」）。
const OVERSIZE_BODY_CHARS: usize = 40_000;

/// C0.1 的分块顺序标记（拼接顺序错乱会立刻改变期望串）。
const CHUNK_MARKERS: [&str; 3] = ["ALPHA-1|", "BETA-2|", "GAMMA-3|"];

/// C2.1 usage_update 的机器可辨识真值。
const USAGE_USED: u64 = 12_345;
const USAGE_SIZE: u64 = 200_000;

/// C5.1 失败 attempt 的可辨识耗时：显著大于任何一次回环 HTTP 调用，
/// 又不足以拖慢套件（300ms 失败 + 一次退避）。
const FAILED_ATTEMPT_MS: u64 = 300;

// ══════════════════════════ 一、ACP fake agent 设施（C0/C2） ══════════════════════════

/// 进程级一次性初始化：写 acp-agents.json + 设 `XDG_CONFIG_HOME`（`acp_delegate` 经
/// `acp::config::resolve_agent` 读这里）。`OnceLock` 保证只发生一次，后续用例只读。
static AGENTS_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// 全局 env / 配置目录初始化锁（`set_var` 是进程级副作用，串行化写入窗口）。
static ENV_INIT_LOCK: Mutex<()> = Mutex::new(());

fn fixture_script() -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(
        script.is_file(),
        "fake ACP agent fixture 缺失（测试设施问题，不是产品违约）: {}",
        script.display()
    );
    script.to_string_lossy().into_owned()
}

/// 各用例的 updates / initialize 证据文件路径（配置里按固定路径登记，用例内再写内容）。
fn chunks_file(root: &Path) -> PathBuf {
    root.join("chunks.jsonl")
}
fn long_file(root: &Path) -> PathBuf {
    root.join("long.jsonl")
}
fn usage_file(root: &Path) -> PathBuf {
    root.join("usage.jsonl")
}

fn init_capture(root: &Path, tag: &str) -> PathBuf {
    root.join(format!("init-{tag}.jsonl"))
}

/// 一次性装配 fake agent 清单（8 个模式各一个 id，全部指向本地 `sh` fixture）。
fn agents_root() -> PathBuf {
    AGENTS_ROOT
        .get_or_init(|| {
            let _guard = ENV_INIT_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let root = std::env::temp_dir().join(format!(
                "pony-acp-delegate-p0p2-{}",
                std::process::id()
            ));
            let workspace = root.join("workspace");
            std::fs::create_dir_all(&workspace).expect("创建 fake agent 临时工作区");
            let config_dir = root.join("config");
            std::fs::create_dir_all(config_dir.join("pony-agent")).expect("创建隔离 XDG 配置目录");

            let script = fixture_script();
            let agents = json!([
                {
                    "id": "fake-order",
                    "label": "C0.1 分块顺序",
                    "command": "sh",
                    "args": [script, "emit_updates", init_capture(&root, "order").to_string_lossy(), chunks_file(&root).to_string_lossy()]
                },
                {
                    "id": "fake-empty",
                    "label": "C0.2 无正文",
                    "command": "sh",
                    "args": [script, "flow", "end_turn", "acp-sess-empty"]
                },
                {
                    "id": "fake-long",
                    "label": "C0.3 超长正文",
                    "command": "sh",
                    "args": [script, "emit_updates", init_capture(&root, "long").to_string_lossy(), long_file(&root).to_string_lossy()]
                },
                {
                    "id": "fake-truncated",
                    "label": "C0.4 max_tokens",
                    "command": "sh",
                    "args": [script, "flow", "max_tokens", "acp-sess-trunc"]
                },
                {
                    "id": "fake-refused",
                    "label": "C0.4 refusal",
                    "command": "sh",
                    "args": [script, "flow", "refusal", "acp-sess-refused"]
                },
                {
                    "id": "fake-cancelled",
                    "label": "C0.4 cancelled",
                    "command": "sh",
                    "args": [script, "flow", "cancelled", "acp-sess-cancelled"]
                },
                {
                    "id": "fake-crash",
                    "label": "C0.4 错误路径",
                    "command": "sh",
                    "args": [script, "crash"]
                },
                {
                    "id": "fake-usage",
                    "label": "C2.1 usage_update",
                    "command": "sh",
                    "args": [script, "emit_updates", init_capture(&root, "usage").to_string_lossy(), usage_file(&root).to_string_lossy()]
                }
            ]);
            std::fs::write(
                config_dir.join("pony-agent").join("acp-agents.json"),
                serde_json::to_string_pretty(&agents).expect("fake agent 清单必须可序列化"),
            )
            .expect("写入隔离 acp-agents.json");

            std::env::set_var("XDG_CONFIG_HOME", &config_dir);
            root
        })
        .clone()
}

/// 走**真实工具路径**调 `acp_delegate`，返回 `(status, 结果 JSON)`。
fn run_acp_delegate(agent_id: &str, prompt: &str) -> (String, Value) {
    let root = agents_root();
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("创建 workspace");

    let router = ToolRouter::with_workspace_root(workspace.clone());
    let call = ToolCall {
        call_id: Some(format!("call-{agent_id}")),
        name: "acp_delegate".to_string(),
        arguments: json!({
            "agent": agent_id,
            "prompt": prompt,
            "timeoutMs": 30_000,
        }),
        plan: None,
    };
    let context = ToolExecutionContext {
        workspace_root: Some(workspace),
        session_id: Some("sess-parent-c0".to_string()),
        ..Default::default()
    };

    let result = router.execute_with_context(&call, &context);
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "acp_delegate 结果必须是 JSON（契约 C0.1）；agent={agent_id} status={} output={} 解析失败：{error}",
            result.status, result.output
        )
    });
    (result.status, parsed)
}

/// 写一条 NDJSON 的 ACP session update 行（fake agent 的 `emit_updates` 逐行发出）。
fn write_updates(path: &Path, updates: &[Value]) {
    let mut body = String::new();
    for update in updates {
        body.push_str(&serde_json::to_string(update).expect("update 必须可序列化"));
        body.push('\n');
    }
    std::fs::write(path, body)
        .unwrap_or_else(|error| panic!("写 updates 证据文件失败 {}: {error}", path.display()));
}

fn text_chunk(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": text }
    })
}

fn json_str<'a>(value: &'a Value, pointer: &str, contract: &str) -> &'a str {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or_else(|| {
        panic!("{contract} 违约：结果 JSON 缺字符串字段 `{pointer}`，实际 = {value}")
    })
}

// ══════════════════════════ C0.1 正文归约（顺序拼接） ══════════════════════════

#[test]
fn c0_1_final_text_concatenates_chunks_in_arrival_order() {
    // Arrange：fake agent 依次发 3 条 agent_message_chunk（顺序标记互不相同）。
    let root = agents_root();
    let updates: Vec<Value> = CHUNK_MARKERS.iter().map(|text| text_chunk(text)).collect();
    write_updates(&chunks_file(&root), &updates);

    // Act
    let (status, result) = run_acp_delegate("fake-order", "把三块正文按到达顺序拼给我");

    // Assert ①：成功语义与既有键面一律保留（additive 变更不得挤掉旧键）。
    assert_eq!(status, "ok", "C0.1 违约：end_turn 委派必须 status=ok，实际 = {result}");
    assert_eq!(
        result.pointer("/ok").and_then(Value::as_bool),
        Some(true),
        "C0.1 违约：结果 JSON 必须保留 `ok: true`，实际 = {result}"
    );
    for pointer in [
        "/sessionId",
        "/stopReason",
        "/warnings",
        "/summary/text",
    ] {
        assert!(
            result.pointer(pointer).is_some(),
            "C0.1 违约：结果 JSON 必须保留既有键 `{pointer}`，实际 = {result}"
        );
    }
    assert_eq!(
        json_str(&result, "/stopReason", "C0.1"),
        "end_turn",
        "C0.1 违约：stopReason 必须原样透传 end_turn，实际 = {result}"
    );
    assert_eq!(
        result.pointer("/updates").and_then(Value::as_u64),
        Some(CHUNK_MARKERS.len() as u64),
        "C0.1 违约：`updates` 必须仍是收到的 update 帧数 = {}，实际 = {result}",
        CHUNK_MARKERS.len()
    );

    // Assert ②：新增 `finalText` 必须是按到达顺序拼接的正文（顺序错乱即违约）。
    let final_text = json_str(&result, "/finalText", "C0.1");
    assert_eq!(
        final_text,
        CHUNK_MARKERS.concat(),
        "C0.1 违约：`finalText` 必须按到达顺序拼接 AgentMessageChunk 文本（期望 {:?}），实际 = {final_text:?}",
        CHUNK_MARKERS.concat()
    );

    // Assert ③：`finalTextTruncated` 必须是 bool 且为 false（未越上限）。
    assert_eq!(
        result.pointer("/finalTextTruncated"),
        Some(&Value::Bool(false)),
        "C0.1 违约：`finalTextTruncated` 必须存在且为 false（布尔，不得省略/字符串化），实际 = {result}"
    );
}

// ══════════════════════════ C0.2 无正文 ══════════════════════════

#[test]
fn c0_2_absent_chunks_yield_empty_final_text_and_honest_summary() {
    // Arrange：fake agent 直接回 end_turn，一条 AgentMessageChunk 都不发。
    // Act
    let (status, result) = run_acp_delegate("fake-empty", "这一轮不会有任何正文");

    // Assert ①：空正文必须显式为空串，不得伪装内容、不得缺键。
    assert_eq!(status, "ok", "C0.2 违约：无正文的 end_turn 仍是成功轮，实际 = {result}");
    assert_eq!(
        result.pointer("/finalText"),
        Some(&Value::String(String::new())),
        "C0.2 违约：无 AgentMessageChunk ⇒ `finalText` 必须是空串（不得缺键/不得 null/不得伪造内容），实际 = {result}"
    );

    // Assert ②：未发生截断。
    assert_eq!(
        result.pointer("/finalTextTruncated"),
        Some(&Value::Bool(false)),
        "C0.2 违约：无正文时 `finalTextTruncated` 必须为 false，实际 = {result}"
    );

    // Assert ③：summary 必须明示「未返回文本正文」，不得复用「已完成本轮任务」的通用文案
    //           （那会让调用方以为拿到了结果）。
    let summary = json_str(&result, "/summary/text", "C0.2");
    assert!(
        summary.contains("未返回文本"),
        "C0.2 违约：summary.text 必须明示未返回文本正文（含「未返回文本」），实际 = {summary:?}（整包 {result}）"
    );
}

// ══════════════════════════ C0.3 超长正文截断 ══════════════════════════

#[test]
fn c0_3_oversize_final_text_keeps_prefix_and_marks_truncated() {
    // Arrange：单条正文 = 头标记 + 40k 正文 + 尾标记（尾标记落在上限之外）。
    let root = agents_root();
    let oversize = format!(
        "BEGIN-{}-TAIL-MARKER-MUST-NOT-SURVIVE",
        "y".repeat(OVERSIZE_BODY_CHARS)
    );
    assert!(
        oversize.chars().count() > FINAL_TEXT_MAX_CHARS,
        "测试设施前提：注入正文必须越过上限"
    );
    write_updates(&long_file(&root), &[text_chunk(&oversize)]);

    // Act
    let (status, result) = run_acp_delegate("fake-long", "这一轮会返回超长正文");

    // Assert ①：仍是成功轮（截断不是失败）。
    assert_eq!(status, "ok", "C0.3 违约：超长正文仍应 status=ok，实际 = {result}");

    // Assert ②：截断标志必须为 true。
    assert_eq!(
        result.pointer("/finalTextTruncated"),
        Some(&Value::Bool(true)),
        "C0.3 违约：正文 > {FINAL_TEXT_MAX_CHARS} 字符 ⇒ `finalTextTruncated` 必须为 true，实际 = {result}"
    );

    // Assert ③：保留的必须是**前缀**且长度恰为上限（单一常量真源）。
    let final_text = json_str(&result, "/finalText", "C0.3");
    assert_eq!(
        final_text.chars().count(),
        FINAL_TEXT_MAX_CHARS,
        "C0.3 违约：`finalText` 必须保留恰好 {FINAL_TEXT_MAX_CHARS} 字符前缀，实际 {} 字符",
        final_text.chars().count()
    );
    assert!(
        final_text.starts_with("BEGIN-"),
        "C0.3 违约：截断必须保留**前缀**（以 BEGIN- 开头），实际开头 = {:?}",
        final_text.chars().take(16).collect::<String>()
    );
    assert!(
        !final_text.contains("TAIL-MARKER-MUST-NOT-SURVIVE"),
        "C0.3 违约：上限之外的尾标记不得出现在 finalText 里（说明未真正截断）"
    );
}

// ══════════════════════════ C0.4 非 end_turn 归约不变 ══════════════════════════

/// C0.4 公共断言：非 end_turn 归约的 status / 错误码保持既有口径，且**不出现** `finalText`。
fn assert_no_final_text_contract(result: &Value, case: &str) {
    for key in ["finalText", "finalTextTruncated"] {
        assert!(
            result.get(key).is_none(),
            "C0.4 违约：`{case}` 归约不得出现 `{key}`（非 end_turn 不产正文），实际 = {result}"
        );
    }
    assert!(
        result
            .pointer("/summary/text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty()),
        "C0.4 违约：`{case}` 归约必须仍带非空 summary.text，实际 = {result}"
    );
}

#[test]
fn c0_4a_truncated_stop_reason_unchanged() {
    // Arrange：stopReason = max_tokens（归约口径：ok + truncated: true）。
    // Act
    let (status, result) = run_acp_delegate("fake-truncated", "外部 agent 会撞上限");

    // Assert：既有截断语义一字不变，且不出现 finalText。
    assert_eq!(status, "ok", "C0.4 违约：max_tokens 仍是成功轮，实际 = {result}");
    assert_eq!(
        result.pointer("/truncated").and_then(Value::as_bool),
        Some(true),
        "C0.4 违约：max_tokens ⇒ 既有 `truncated: true` 必须保留，实际 = {result}"
    );
    assert_eq!(
        json_str(&result, "/stopReason", "C0.4"),
        "max_tokens",
        "C0.4 违约：stopReason 必须原样透传，实际 = {result}"
    );
    assert_no_final_text_contract(&result, "truncated");
}

#[test]
fn c0_4b_refused_stop_reason_unchanged() {
    // Act
    let (status, result) = run_acp_delegate("fake-refused", "外部 agent 会拒绝");

    // Assert
    assert_eq!(status, "error", "C0.4 违约：refusal ⇒ status=error，实际 = {result}");
    assert_eq!(
        json_str(&result, "/error/code", "C0.4b"),
        "acp_agent_refused",
        "C0.4 违约：refusal 的错误码必须保持 acp_agent_refused，实际 = {result}"
    );
    assert_no_final_text_contract(&result, "refused");
}

#[test]
fn c0_4c_cancelled_stop_reason_unchanged() {
    // Act
    let (status, result) = run_acp_delegate("fake-cancelled", "外部 agent 轮次被取消");

    // Assert
    assert_eq!(
        status, "aborted",
        "C0.4 违约：cancelled ⇒ status=aborted，实际 = {result}"
    );
    assert_eq!(
        json_str(&result, "/error/code", "C0.4c"),
        "tool_cancelled",
        "C0.4 违约：cancelled 的错误码必须保持 tool_cancelled，实际 = {result}"
    );
    assert_no_final_text_contract(&result, "cancelled");
}

#[test]
fn c0_4d_error_path_unchanged() {
    // Arrange：fake agent 握手前即以非零码退出（spawn/协议失败路径）。
    // Act
    let (status, result) = run_acp_delegate("fake-crash", "外部 agent 起不来");

    // Assert：错误面保持 `ok:false` + `acp_*` 错误码来源，且不出现 finalText。
    assert_eq!(status, "error", "C0.4 违约：进程/协议失败 ⇒ status=error，实际 = {result}");
    assert_eq!(
        result.pointer("/ok").and_then(Value::as_bool),
        Some(false),
        "C0.4 违约：错误结果必须 `ok:false`，实际 = {result}"
    );
    let code = json_str(&result, "/error/code", "C0.4d");
    assert!(
        code.starts_with("acp_"),
        "C0.4 违约：错误码必须保留 `acp_*` 分源（不得塌缩成通用错误），实际 = {code}"
    );
    assert_no_final_text_contract(&result, "error");
}

// ══════════════════════════ C2.1 usage_update → 结果 JSON usage ══════════════════════════

#[test]
fn c2_1_usage_update_lands_in_result_json_usage() {
    // Arrange：fake agent 先发 usage_update（used/size/cost），再发一条正文 chunk。
    let root = agents_root();
    write_updates(
        &usage_file(&root),
        &[
            json!({
                "sessionUpdate": "usage_update",
                "used": USAGE_USED,
                "size": USAGE_SIZE,
                "cost": { "amount": 0.0123, "currency": "USD" }
            }),
            text_chunk("usage-probe-done"),
        ],
    );

    // Act
    let (status, result) = run_acp_delegate("fake-usage", "量一下上下文用量");

    // Assert ①：轮成功。
    assert_eq!(status, "ok", "C2.1 违约：usage_update 不改变成功语义，实际 = {result}");

    // Assert ②：used/size 必须进入结果 JSON 的 `usage` 字段。
    let usage = result
        .get("usage")
        .unwrap_or_else(|| panic!("C2.1 违约：收到 usage_update ⇒ 结果 JSON 必须带 `usage` 字段，实际 = {result}"));
    assert_eq!(
        usage.get("used").and_then(Value::as_u64),
        Some(USAGE_USED),
        "C2.1 违约：`usage.used` 必须等于 usage_update.used = {USAGE_USED}，实际 usage = {usage}"
    );
    assert_eq!(
        usage.get("size").and_then(Value::as_u64),
        Some(USAGE_SIZE),
        "C2.1 违约：`usage.size` 必须等于 usage_update.size = {USAGE_SIZE}，实际 usage = {usage}"
    );

    // Assert ③：可选 cost —— 只锁「若透传则必须是 amount+currency 结构」，不强制存在
    //           （ACP 的 cost 本就可选；契约写「及可选 cost」）。
    if let Some(cost) = usage.get("cost") {
        assert!(
            cost.get("amount").and_then(Value::as_f64).is_some()
                && cost.get("currency").and_then(Value::as_str).is_some(),
            "C2.1 违约：若透传 cost，必须是 {{amount, currency}} 结构，实际 = {cost}"
        );
    }

    // Assert ④：正文归约仍然工作（usage 与正文是两条独立通道，互不吞并）。
    assert_eq!(
        result.pointer("/finalText"),
        Some(&Value::String("usage-probe-done".to_string())),
        "C2.1 违约：usage_update 不得挤掉正文归约，实际 = {result}"
    );
}

// ══════════════════════════ C3.1 源码扫描：非测试代码零裸 eprintln! ══════════════════════════

/// 扫描单个源文件，返回**非测试区域**里出现裸 `eprintln!` 的行号与原文。
///
/// 跳过规则：`#[cfg(test)] mod ... { ... }` 块（按花括号配对整体跳过）、行注释。
fn bare_eprintln_hits(source: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    let mut depth: i32 = 0;
    let mut cfg_test_pending = false;
    let mut test_block_base: Option<i32> = None;

    for (index, raw) in source.lines().enumerate() {
        let trimmed = raw.trim();

        // 进入测试模块：`#[cfg(test)]` 之后的第一条带 `mod … {` 的行。
        if cfg_test_pending && trimmed.contains("mod ") && raw.contains('{') {
            test_block_base = Some(depth);
            cfg_test_pending = false;
        } else if trimmed.starts_with("#[cfg(test)]") {
            cfg_test_pending = true;
        }

        let inside_test = test_block_base.is_some();
        let is_comment = trimmed.starts_with("//") || trimmed.starts_with('*');
        if !inside_test && !is_comment && raw.contains("eprintln!") {
            hits.push((index + 1, trimmed.to_string()));
        }

        depth += raw.matches('{').count() as i32 - raw.matches('}').count() as i32;
        if let Some(base) = test_block_base {
            if depth <= base {
                test_block_base = None;
            }
        }
    }

    hits
}

#[test]
fn c3_1_acp_sources_have_no_bare_eprintln() {
    // Arrange：`src/acp/**` 的全部 Rust 源文件。
    let acp_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("acp");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&acp_dir)
        .unwrap_or_else(|error| panic!("读取 ACP 源码目录失败 {}: {error}", acp_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();

    // Assert ①：扫描面必须真实存在（防「目录改名 ⇒ 空扫描恒绿」的假测试）。
    assert!(
        files.len() >= 8,
        "C3.1 测试设施：`src/acp/` 至少应有 8 个 .rs 文件（client/config/protocol/session/transport/handlers/mod/…），实际 {}",
        files.len()
    );
    let names: Vec<String> = files
        .iter()
        .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .collect();
    for required in ["client.rs", "session.rs", "transport.rs", "handlers.rs", "config.rs"] {
        assert!(
            names.iter().any(|name| name == required),
            "C3.1 测试设施：`src/acp/{required}` 必须在扫描面内，实际 = {names:?}"
        );
    }

    // Act：逐文件扫描非测试区域的裸 eprintln!。
    let mut violations: Vec<String> = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("读取 {} 失败: {error}", path.display()));
        for (line, text) in bare_eprintln_hits(&source) {
            let relative = path
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(path)
                .display()
                .to_string();
            violations.push(format!("{relative}:{line}: {text}"));
        }
    }

    // Assert ②：零裸 eprintln!（诊断统一走 `agent::otel::log`：json + trace_id）。
    assert!(
        violations.is_empty(),
        "C3.1 违约：`crates/pony-agent-core/src/acp/**` 非测试代码存在 {} 处裸 `eprintln!`，\
         诊断必须走 `agent::otel::log`（结构化 json + trace_id）。违约点：\n{}",
        violations.len(),
        violations.join("\n")
    );
}

// ══════════════════════════ 回环 mock provider（C4 / C5 共用） ══════════════════════════

#[derive(Clone)]
struct MockReply {
    status: u16,
    reason: &'static str,
    body: String,
    content_type: &'static str,
    /// 应答前的固定延迟 —— 模拟「失败 attempt 自身耗时」，是**被测输入**而非等待同步。
    delay: Duration,
}

#[derive(Clone)]
enum Script {
    /// 按连接次序取剧本（越界复用最后一条）。
    ByIndex(Vec<MockReply>),
    /// 按请求语义取剧本：请求体已含 tool 结果 ⇒ followup，否则 initial。
    ByPhase {
        initial: MockReply,
        followup: MockReply,
    },
}

struct ScriptedProvider {
    base_url: String,
    addr: String,
    requests: Arc<Mutex<Vec<Value>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ScriptedProvider {
    fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .expect("mock provider 必须能绑定回环端口（零外网）");
        let port = listener
            .local_addr()
            .expect("mock provider 必须能取本地地址")
            .port();
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            let mut served = 0usize;
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                let raw = read_http_request(&mut stream);
                if raw.is_empty() {
                    // 收尾唤醒连接，不计入真实请求。
                    continue;
                }
                let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
                requests_for_thread
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(body.clone());

                let reply = match &script {
                    Script::ByIndex(replies) => {
                        let index = served.min(replies.len().saturating_sub(1));
                        replies[index].clone()
                    }
                    Script::ByPhase { initial, followup } => {
                        if request_has_tool_result(&body) {
                            followup.clone()
                        } else {
                            initial.clone()
                        }
                    }
                };
                served += 1;

                if !reply.delay.is_zero() {
                    std::thread::sleep(reply.delay);
                }
                let head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.reason,
                    reply.content_type,
                    reply.body.as_bytes().len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(reply.body.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                drain_incoming(&mut stream);
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            shutdown,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 一次空连接唤醒 accept，让 mock 线程退出。
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn request_has_tool_result(body: &Value) -> bool {
    body.get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages
                .iter()
                .any(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        })
}

fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut expected_total: Option<usize> = None;
    loop {
        if let Some(total) = expected_total {
            if raw.len() >= total {
                break;
            }
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if expected_total.is_none() {
                    if let Some(header_end) = find_subslice(&raw, b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..header_end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        expected_total = Some(header_end + 4 + length);
                    }
                }
            }
            Err(_) => break,
        }
    }
    match find_subslice(&raw, b"\r\n\r\n") {
        Some(index) => raw[index + 4..].to_vec(),
        None => Vec::new(),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn drain_incoming(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut chunk = [0u8; 4096];
    while matches!(stream.read(&mut chunk), Ok(read) if read > 0) {}
}

// ══════════════════════════ runtime 装配（C4 / C5 共用） ══════════════════════════

struct StaticResolver {
    selection: ResolvedProviderSelection,
}

impl ProviderSelectionResolver for StaticResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

/// 记录真实执行过的工具调用（C4「一次真实工具调用」的证据面）。
#[derive(Default)]
struct RecordingExecutor {
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingExecutor {
    fn recorded(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ToolExecutor for RecordingExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((call.name.clone(), call.arguments.to_string()));
        ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            output: "{\"entries\":[\"a.rs\"]}".to_string(),
            duration_ms: 1,
        }
    }
}

/// buffered（非流式）provider 选择：`supports_streaming=false` ⇒ 走同步链，
/// 时序可辨识（失败 attempt 耗时无处躲藏）。
fn buffered_selection(base_url: String, tag: &str) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: tag.to_string(),
        provider_name: tag.to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "P0P2_TEST_API_KEY".to_string(),
        api_key: Some("p0p2-dummy-key".to_string()),
        model: "p0p2-probe-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn turn_input(session_id: &str) -> TurnInput {
    TurnInput {
        message: "p0p2 契约探针".to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn names(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn terminal(&self) -> Option<TurnStreamEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .rev()
            .find(|(name, _)| {
                matches!(
                    name.as_str(),
                    "turn:completed" | "turn:failed" | "turn:cancelled"
                )
            })
            .map(|(_, payload)| payload.clone())
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((name.to_string(), payload));
    }
}

fn backend_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pony-p0p2-{}-{tag}.db",
        std::process::id()
    ))
}

/// 跑一轮 turn，返回 (sink, 执行器记录) 供断言。
fn run_turn(
    provider: &ScriptedProvider,
    tag: &str,
    session_id: &str,
    executor: Arc<RecordingExecutor>,
) -> RecordingSink {
    let path = backend_path(tag);
    let _ = std::fs::remove_file(&path);
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone()))),
        Box::new(StaticResolver {
            selection: buffered_selection(provider.base_url.clone(), tag),
        }),
        Box::new(ArcExecutor(executor)),
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    let sink = RecordingSink::default();
    runtime.start_turn_stream(
        &sink,
        format!("turn-{tag}"),
        turn_input(session_id),
    );
    let _ = std::fs::remove_file(&path);
    sink
}

/// 把 `Arc<RecordingExecutor>` 适配成 `Box<dyn ToolExecutor>` 的持有者。
struct ArcExecutor(Arc<RecordingExecutor>);

impl ToolExecutor for ArcExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        self.0.execute(call)
    }
}

// ══════════════════════════ C4.1 step 同源 ══════════════════════════

/// 持久化通道是**进程级全局单槽** ⇒ C4 用例必须独占（并保证测试结束后清空）。
static EVENT_PERSIST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn c4_1_tool_event_step_is_the_turn_step_index() {
    let _guard = EVENT_PERSIST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Arrange：捕获持久化事件流。
    let captured: Arc<Mutex<Vec<TurnEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_for_persist = Arc::clone(&captured);
    register_event_persist(Arc::new(
        move |_session_id: &str, _turn_id: &str, event: TurnEvent, _is_terminal: bool| {
            captured_for_persist
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event);
        },
    ));

    // Arrange：mock provider 第一轮返回 tool_call，followup 返回收尾正文。
    let provider = ScriptedProvider::start(Script::ByPhase {
        initial: MockReply {
            status: 200,
            reason: "OK",
            content_type: "application/json",
            delay: Duration::ZERO,
            body: json!({
                "id": "p0p2-step-initial",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_step_probe",
                            "type": "function",
                            "function": { "name": "workspace_list_files", "arguments": "{\"path\":\".\"}" }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
            })
            .to_string(),
        },
        followup: MockReply {
            status: 200,
            reason: "OK",
            content_type: "application/json",
            delay: Duration::ZERO,
            body: json!({
                "id": "p0p2-step-followup",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "p0p2 step probe done" },
                    "finish_reason": "stop"
                }],
                "usage": { "prompt_tokens": 20, "completion_tokens": 6, "total_tokens": 26 }
            })
            .to_string(),
        },
    });
    let executor = Arc::new(RecordingExecutor::default());

    // Act
    let sink = run_turn(&provider, "step", "session-p0p2-step", Arc::clone(&executor));
    clear_event_persist();
    let events = captured
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();

    // 前置证据：一轮必须真的完成，且工具真的被执行（否则下面的断言在证明空气）。
    assert!(
        sink.names().iter().any(|name| name == "turn:completed"),
        "C4.1 夹具前提：本剧本必须跑完一轮 turn，实际事件序列 = {:?}",
        sink.names()
    );
    let executed = executor.recorded();
    assert!(
        executed
            .iter()
            .any(|(name, _)| name == "workspace_list_files"),
        "C4.1 夹具前提：must 真实执行一次工具调用（provider tool_call → executor），实际 = {executed:?}"
    );

    // 事件流按 turn 过滤（持久化通道是进程级，其他用例的 ACP 事件可能同批出现）。
    let turn_id = "turn-step";
    let step_starts: Vec<u32> = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::StepStart { turn_id: id, step, .. } if id == turn_id => Some(*step),
            _ => None,
        })
        .collect();
    let tool_events: Vec<(&'static str, String, u32)> = events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::ToolCall {
                turn_id: id,
                step,
                call_id,
                ..
            } if id == turn_id => Some(("ToolCall", call_id.clone(), *step)),
            TurnEvent::ToolResult {
                turn_id: id,
                step,
                call_id,
                ..
            } if id == turn_id => Some(("ToolResult", call_id.clone(), *step)),
            _ => None,
        })
        .collect();

    // Assert ①：step 事件的真值面存在（初始 hop 的 step 0）。
    assert!(
        step_starts.contains(&0),
        "C4.1 夹具前提：持久化事件流必须含初始 hop 的 StepStart{{step:0}}，实际 StepStart steps = {step_starts:?}"
    );

    // Assert ②：ToolCall / ToolResult 必须入事件流（否则没有 step 可断言）。
    assert!(
        tool_events
            .iter()
            .any(|(kind, call_id, _)| *kind == "ToolCall" && call_id == "call_step_probe"),
        "C4.1 夹具前提：持久化事件流必须含 ToolCall(call_step_probe)，实际 = {:?}",
        tool_events
            .iter()
            .map(|(kind, call_id, step)| format!("{kind}({call_id}, step={step})"))
            .collect::<Vec<_>>()
    );

    // Assert ③（核心）：ToolCall/ToolResult.step 必须取自**turn step 索引**（与 StepStart 同源），
    //          不得用事件 sequence。判据：工具事件的 step 必须落在 StepStart 的 step 集合里；
    //          且初始 hop 产生的调用必须精确等于 0（产生它的 step）。
    for (kind, call_id, step) in &tool_events {
        assert!(
            step_starts.contains(step),
            "C4.1 违约：{kind}({call_id}).step = {step} 不在 StepStart 的 step 集合 {:?} 中\
             —— step 取了事件 sequence，而不是产生该调用的 turn step index",
            step_starts
        );
    }
    let call_steps: Vec<(&str, u32)> = tool_events
        .iter()
        .map(|(kind, _, step)| (*kind, *step))
        .collect();
    assert!(
        call_steps.iter().all(|(_, step)| *step == 0),
        "C4.1 违约：本剧本的工具调用产生于初始 hop（step 0），ToolCall/ToolResult.step 必须都为 0，实际 = {call_steps:?}"
    );
}

// ══════════════════════════ C5.1 重试窗口 metadata ══════════════════════════

const OPENAI_OK_BODY: &str = "{\"id\":\"p0p2-ok\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"p0p2-retry-done\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":40,\"total_tokens\":140}}";

const SERVER_ERROR_BODY: &str =
    "{\"error\":{\"type\":\"server_error\",\"message\":\"upstream temporarily unavailable\"}}";

/// 终态事件里的 per-call 记录（唯一取证面）。
fn single_call_record<'a>(
    terminal: &'a TurnStreamEvent,
    case: &str,
) -> &'a pony_agent_core::agent::telemetry::ProviderCallCacheRecord {
    let records = terminal
        .provider_call_records
        .as_ref()
        .unwrap_or_else(|| panic!("{case}：终态事件必须携带 providerCallRecords"));
    assert_eq!(
        records.len(),
        1,
        "{case}：本剧本只有一次（成功结算的）provider 调用，实际 {} 条记录",
        records.len()
    );
    &records[0]
}

fn call_records_json(terminal: &TurnStreamEvent) -> Value {
    let records = terminal
        .provider_call_records
        .as_ref()
        .expect("终态事件必须携带 providerCallRecords");
    serde_json::to_value(records).expect("providerCallRecords 必须可序列化")
}

#[test]
fn c5_1_retry_populates_attempts_and_retry_windows() {
    // Arrange：第一次 attempt 503（耗时 300ms 可辨识），第二次 200。
    let provider = ScriptedProvider::start(Script::ByIndex(vec![
        MockReply {
            status: 503,
            reason: "Service Unavailable",
            content_type: "application/json",
            delay: Duration::from_millis(FAILED_ATTEMPT_MS),
            body: SERVER_ERROR_BODY.to_string(),
        },
        MockReply {
            status: 200,
            reason: "OK",
            content_type: "application/json",
            delay: Duration::ZERO,
            body: OPENAI_OK_BODY.to_string(),
        },
    ]));
    let executor = Arc::new(RecordingExecutor::default());

    // Act
    let sink = run_turn(&provider, "retry", "session-p0p2-retry", executor);
    let terminal = sink
        .terminal()
        .unwrap_or_else(|| panic!("C5.1：一轮必须产出终态事件，实际 = {:?}", sink.names()));

    // 前置证据：真的走了「失败 → 重试 → 成功」（否则下面的断言在证明空气）。
    assert!(
        provider.request_count() >= 2,
        "C5.1 夹具前提：503 属既有可重试瞬时错误，必须真实重试（≥2 次 HTTP），实际 {} 次",
        provider.request_count()
    );
    // 终态可能是 completed（重试成功）；若失败则是产品违约，单独报出。
    assert_eq!(
        terminal.kind, "completed",
        "C5.1 夹具前提：503→200 剧本必须以 turn:completed 收尾，实际 kind = {} (error={:?})",
        terminal.kind, terminal.error
    );

    let record = single_call_record(&terminal, "C5.1");

    // Assert ①：attempts 恒 Some(≥1)，重试剧本必须 >1。
    let attempts = record.attempts.unwrap_or_else(|| {
        panic!(
            "C5.1 违约：`ProviderCallCacheRecord.attempts` 必须恒为 Some(≥1)，实际 None\
             （重试剧本应记录 2 次尝试）"
        )
    });
    assert!(
        attempts > 1,
        "C5.1 违约：失败 1 次后成功 ⇒ attempts 必须 > 1，实际 = {attempts}"
    );
    assert_eq!(
        attempts as usize,
        provider.request_count(),
        "C5.1 违约：attempts 必须等于真实 HTTP 尝试次数，实际 attempts={attempts} vs mock 收到 {} 次",
        provider.request_count()
    );

    // Assert ②：failed_attempt_ms 仅在重试时出现，且 > 0。
    let failed_ms = record.failed_attempt_ms.unwrap_or_else(|| {
        panic!("C5.1 违约：发生失败重试 ⇒ `failed_attempt_ms` 必须为 Some(>0)，实际 None")
    });
    assert!(
        failed_ms > 0,
        "C5.1 违约：`failed_attempt_ms` 必须 > 0（失败尝试真实耗时约 {FAILED_ATTEMPT_MS}ms），实际 = {failed_ms}"
    );

    // Assert ③：prelude_ms（首个成功 attempt 之前的等待/预热）仅在重试时出现，且 > 0。
    let prelude_ms = record.prelude_ms.unwrap_or_else(|| {
        panic!("C5.1 违约：发生失败重试 ⇒ `prelude_ms` 必须为 Some(>0)，实际 None")
    });
    assert!(
        prelude_ms > 0,
        "C5.1 违约：`prelude_ms` 必须 > 0，实际 = {prelude_ms}"
    );

    // Assert ④：wire 面必须透出三个 additive 字段（camelCase），供回读消费。
    let wire = call_records_json(&terminal);
    let first = &wire[0];
    assert_eq!(
        first.get("attempts").and_then(Value::as_u64),
        Some(attempts as u64),
        "C5.1 违约：wire 面 `providerCallRecords[0].attempts` 必须为 {attempts}，实际 = {first}"
    );
    assert!(
        first.get("failedAttemptMs").and_then(Value::as_u64) == Some(failed_ms),
        "C5.1 违约：wire 面必须含 `failedAttemptMs = {failed_ms}`（camelCase additive 字段），实际 = {first}"
    );
    assert!(
        first.get("preludeMs").and_then(Value::as_u64) == Some(prelude_ms),
        "C5.1 违约：wire 面必须含 `preludeMs = {prelude_ms}`（camelCase additive 字段），实际 = {first}"
    );

    // Assert ⑤：`session_turn_traces.providerCallRecords` 的回读契约 —— 同一条记录经
    //           serde 往返后三个字段必须原样可读（持久化即走这条路径）。
    let roundtrip: pony_agent_core::agent::telemetry::ProviderCallCacheRecord =
        serde_json::from_value(first.clone()).unwrap_or_else(|error| {
            panic!("C5.1 违约：providerCallRecords 记录必须可反序列化回读（{error}）：{first}")
        });
    assert_eq!(roundtrip.attempts, Some(attempts), "C5.1 违约：回读 attempts 不一致");
    assert_eq!(
        roundtrip.failed_attempt_ms,
        Some(failed_ms),
        "C5.1 违约：回读 failed_attempt_ms 不一致"
    );
    assert_eq!(
        roundtrip.prelude_ms,
        Some(prelude_ms),
        "C5.1 违约：回读 prelude_ms 不一致"
    );
}

#[test]
fn c5_1_no_retry_keeps_single_attempt_and_omits_retry_windows() {
    // Arrange：一次成功，无重试。
    let provider = ScriptedProvider::start(Script::ByIndex(vec![MockReply {
        status: 200,
        reason: "OK",
        content_type: "application/json",
        delay: Duration::ZERO,
        body: OPENAI_OK_BODY.to_string(),
    }]));
    let executor = Arc::new(RecordingExecutor::default());

    // Act
    let sink = run_turn(&provider, "norety", "session-p0p2-norety", executor);
    let terminal = sink
        .terminal()
        .unwrap_or_else(|| panic!("C5.1：一轮必须产出终态事件，实际 = {:?}", sink.names()));
    assert_eq!(
        terminal.kind, "completed",
        "C5.1 夹具前提：单次成功剧本必须 turn:completed，实际 = {} (error={:?})",
        terminal.kind, terminal.error
    );
    assert_eq!(
        provider.request_count(),
        1,
        "C5.1 夹具前提：本剧本只应有 1 次 HTTP，实际 {} 次",
        provider.request_count()
    );

    let record = single_call_record(&terminal, "C5.1-no-retry");

    // Assert ①：无重试 ⇒ attempts == Some(1)（恒 Some，不得省略）。
    assert_eq!(
        record.attempts,
        Some(1),
        "C5.1 违约：无重试路径 `attempts` 必须为 Some(1)，实际 = {:?}",
        record.attempts
    );

    // Assert ②：无重试 ⇒ 另两键必须缺席（None），禁止 0 占位冒充真值。
    assert_eq!(
        record.failed_attempt_ms, None,
        "C5.1 违约：无重试路径 `failed_attempt_ms` 必须缺席（None），实际 = {:?}",
        record.failed_attempt_ms
    );
    assert_eq!(
        record.prelude_ms, None,
        "C5.1 违约：无重试路径 `prelude_ms` 必须缺席（None），实际 = {:?}",
        record.prelude_ms
    );

    // Assert ③：wire 面上 attempts 必须存在（=1），另两键必须**键缺席**。
    let wire = call_records_json(&terminal);
    let first = &wire[0];
    assert_eq!(
        first.get("attempts").and_then(Value::as_u64),
        Some(1),
        "C5.1 违约：wire 面 `providerCallRecords[0].attempts` 必须为 1，实际 = {first}"
    );
    for absent in ["failedAttemptMs", "preludeMs"] {
        assert!(
            first.get(absent).is_none(),
            "C5.1 违约：无重试时 wire 面不得出现 `{absent}`（键必须缺席，不得 null/0 占位），实际 = {first}"
        );
    }
}
