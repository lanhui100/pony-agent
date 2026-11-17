//! ACP 委派（`acp_delegate`）L2-AT 对抗破坏测试 —— wave-acp-delegate P0–P2，独立于常驻名册。
//!
//! 真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md`（C0.1–C0.4 / C1.1 / C1.5）与
//! `.dev-team/nfr-baseline.json`（阈值唯一真源：根 `external_call_timeout_ms`、`acp.request_timeout_ms`）。
//!
//! 三类**确定性**破坏面：
//! 1. **超时穿透**（`at_01`）：agent 侧 sleep = `external_call_timeout_ms + 1000ms`，委派预算 =
//!    baseline ⇒ 必须归约 `acp_timeout`（或 `aborted`），且总耗时 < 注入延迟（不 hang）；
//! 2. **畸形 session/update**（`at_02a/b/c`）：未知变体 / 缺字段 / 非文本 ContentBlock / >32k 正文
//!    ⇒ 不 panic、不静默丢弃整条、`finalText` 有界且 `finalTextTruncated=true`（超长时）；
//! 3. **并发委派**（`at_03a/b`）：同一 ACP session 两路并发 prompt 必须在**线上**串行；两路并发
//!    委派的父 turn 事件不得串会话（`parent_activity_id` / session / turn 归属正确）。
//!
//! 纪律：
//! - 阈值只读 `.dev-team/nfr-baseline.json`；注入延迟 = baseline + 1000ms 是任务卡冻结口径
//!   （非本文件自选），等待上界取 `acp.request_timeout_ms`；
//! - 同步一律「有界条件轮询 / `Barrier` / `mpsc::recv_timeout`」，不用 sleep 猜时序、不做概率重试；
//! - 全链路真实：真实 ACP 子进程 + 真实 `ToolRouter` / `AcpAgentSession`，不 mock 传输层；
//! - 红相语义 = **断言失败**（非编译错误）。W1 骨架期 `acp::result::reduce_final_text`
//!   （`src/acp/result.rs:39`）与 `acp::delegate_events::emit_parent_tool_activity`
//!   （`src/acp/delegate_events.rs:46`）仍是 `unimplemented!()` 且未接入工具结果/事件通道，
//!   故 `finalText` 与「父 turn 事件」断言必红；已实现行为（会话串行闸
//!   `src/acp/session.rs:849`、整轮预算超时归约）作为回归守护，不在红相之列。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::AcpAgentSession;
use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolResult, ToolRouter};
use pony_agent_core::agent::turn_event::TurnEvent;
use pony_agent_core::agent::turn_flow::{
    clear_event_persist, register_event_persist, NoopTurnEventSink,
};
use serde_json::{json, Value};

// ───────────────────────────── 通用设施（独立于其它验收文件，避免跨 target 共享） ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// `finalText` 长度上限的镜像常量。
///
/// 单一真源是 `crates/pony-agent-core/src/acp/result.rs:25`
/// （`ACP_DELEGATE_FINAL_TEXT_MAX_CHARS = 32_000`）；W1 冻结期该模块是 `pub(crate)`
/// （`src/acp/mod.rs:21`），集成测试**无法**导入，因此这里只镜像契约数值并在断言消息里
/// 指回真源。若 W2a 把常量开放为 `pub`，本常量应改为直接导入（断言语义不变）。
const FINAL_TEXT_MAX_CHARS: usize = 32_000;

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-at-delegate-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// NFR 真源整体（根 `external_call_timeout_ms` + `acp` 段）。
fn nfr_json() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.dev-team/nfr-baseline.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 NFR 真源失败 {}: {error}", path.display()));
    serde_json::from_str(&body).expect("nfr-baseline.json 必须是合法 JSON")
}

/// 外部调用超时基线（根键，任务卡口径）。
fn nfr_external_call_timeout_ms() -> u64 {
    nfr_json()
        .get("external_call_timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 external_call_timeout_ms"))
}

fn nfr_acp_ms(key: &str) -> u64 {
    nfr_json()
        .get("acp")
        .and_then(|acp| acp.get(key))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 acp.{key}"))
}

/// 本文件所有阻塞等待的上界：NFR 的 `acp.request_timeout_ms`。
fn op_timeout() -> Duration {
    Duration::from_millis(nfr_acp_ms("request_timeout_ms"))
}

/// 有界条件轮询（到达条件即返回；超时才算失败）。
fn wait_until<F: FnMut() -> bool>(mut condition: F, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

/// 共享 fixture 路径（冻结件，禁改）。
fn fixture_script() -> PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script
}

fn prompt_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

// ── 用户清单注入（`resolve_agent` 的唯一公开入口是 `<XDG_CONFIG_HOME>/pony-agent/acp-agents.json`） ──

/// 进程级 XDG 隔离的串行闸：清单路径是进程环境，测试并行时必须互斥。
static CONFIG_ENV_LOCK: Mutex<()> = Mutex::new(());

/// `XDG_CONFIG_HOME` 守卫（Drop 时还原环境并释放串行闸）。
struct XdgConfig {
    _lock: MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl XdgConfig {
    fn install(config_home: &Path) -> Self {
        let lock = CONFIG_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", config_home);
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for XdgConfig {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

fn write_acp_agents_config(config_home: &Path, specs: Value) {
    let dir = config_home.join("pony-agent");
    std::fs::create_dir_all(&dir).expect("创建 pony-agent 配置目录");
    std::fs::write(
        dir.join("acp-agents.json"),
        serde_json::to_string_pretty(&specs).expect("清单必须可序列化"),
    )
    .expect("写入 acp-agents.json");
}

fn agent_spec_json(id: &str, args: &[String]) -> Value {
    json!({
        "id": id,
        "label": format!("Fake ACP agent {id}"),
        "command": "sh",
        "args": args,
    })
}

// ── 委派调用装配（真实 `ToolRouter` 工具面，与生产同一条 `execute_acp_delegate` 臂） ──

fn delegate_call(call_id: &str, agent: &str, prompt: &str, timeout_ms: Option<u64>) -> ToolCall {
    let mut arguments = json!({
        "agent": agent,
        "prompt": prompt,
        "description": "委派给外部 ACP agent",
    });
    if let Some(ms) = timeout_ms {
        arguments["timeoutMs"] = json!(ms);
    }
    ToolCall {
        call_id: Some(call_id.to_string()),
        name: "acp_delegate".to_string(),
        arguments,
        plan: None,
    }
}

/// 父 turn 上下文：`session_id` 是事件归属真源，`dispatch_context.turn_id` 是父 turn 真源
/// （C1.1 / C1.5 要求归集事件的 session/turn = 父会话/父 turn）。
fn parent_context(cwd: &Path, session_id: &str, turn_id: &str) -> ToolExecutionContext {
    ToolExecutionContext {
        workspace_root: Some(cwd.to_path_buf()),
        session_id: Some(session_id.to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn result_json(result: &ToolResult) -> Value {
    serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "acp_delegate 结果必须是 JSON（{error}），实际输出：{}",
            result.output
        )
    })
}

/// `finalText` 读取（C0.1 的键名是契约冻结字面量，故直接按名取，不做容错猜测）。
fn final_text(output: &Value) -> &str {
    match output.get("finalText") {
        Some(Value::String(text)) => text,
        other => panic!(
            "C0.1 违约：结果 JSON 必含字符串 `finalText`（AgentMessageChunk 按到达顺序拼接），\
             实际 `finalText` = {other:?}；顶层键 = {:?}",
            output
                .as_object()
                .map(|map| map.keys().cloned().collect::<Vec<_>>())
        ),
    }
}

fn final_text_truncated(output: &Value) -> bool {
    match output.get("finalTextTruncated") {
        Some(Value::Bool(flag)) => *flag,
        other => panic!(
            "C0.1/C0.3 违约：结果 JSON 必含布尔 `finalTextTruncated`，实际 {other:?}"
        ),
    }
}

fn warnings(output: &Value) -> Vec<String> {
    output
        .get("warnings")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn updates_count(output: &Value) -> u64 {
    output.get("updates").and_then(Value::as_u64).unwrap_or(0)
}

fn frame_text(text: &str) -> Value {
    json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": text },
    })
}

fn frame_tool(tool_call_id: &str, title: &str) -> Value {
    json!({
        "sessionUpdate": "tool_call",
        "toolCallId": tool_call_id,
        "title": title,
        "kind": "edit",
        "status": "completed",
    })
}

fn write_frames(path: &Path, frames: &[Value]) {
    let body: String = frames
        .iter()
        .map(|frame| {
            serde_json::to_string(frame).expect("update 帧必须可序列化") + "\n"
        })
        .collect();
    std::fs::write(path, body).expect("写入 session/update 帧文件");
}

/// 走真实工具面跑一次委派（`emit_updates` 模式把 `frames` 逐条作为 `session/update` 发出）。
///
/// 临时目录在返回前回收：本助手只对外暴露归约结果（不泄漏工作目录）。
fn delegate_with_updates(tag: &str, agent_id: &str, frames: &[Value], prompt: &str) -> ToolResult {
    let cwd = temp_dir(tag);
    let config_home = cwd.join("config");
    let init_file = cwd.join("initialize.jsonl");
    let updates_file = cwd.join("updates.jsonl");
    write_frames(&updates_file, frames);
    write_acp_agents_config(
        &config_home,
        json!([agent_spec_json(
            agent_id,
            &[
                fixture_script().to_string_lossy().into_owned(),
                "emit_updates".to_string(),
                init_file.to_string_lossy().into_owned(),
                updates_file.to_string_lossy().into_owned(),
            ],
        )]),
    );

    let result = {
        let _xdg = XdgConfig::install(&config_home);
        let router = ToolRouter::with_workspace_root(cwd.clone());
        let context = parent_context(&cwd, "parent-sess-at", "turn-at-1");
        let call = delegate_call("call-at-1", agent_id, prompt, None);
        router.execute_with_context(&call, &context)
    };
    cleanup(&cwd);
    result
}

// ───────────────────────── 1. 超时穿透（agent 侧 sleep > baseline） ─────────────────────────

/// `at_01`：注入延迟 = `external_call_timeout_ms + 1000ms`，委派预算 = baseline。
///
/// 契约口径（C0.4 失败归约 + 任务卡「不 hang」）：
/// - `status ∈ {error, aborted}` 且错误码 ∈ {`acp_timeout`, `tool_cancelled`}（归约确定，不塌缩成泛化错误）；
/// - 总耗时 < 注入延迟（预算生效，不陪 agent 睡满；也不 hang）；
/// - 失败归约不得出现 `finalText`（C0.4：只有 `end_turn` 成功路径才携带正文）。
#[test]
fn at_01_timeout_injection_reduces_to_timeout_without_hang() {
    let baseline_ms = nfr_external_call_timeout_ms();
    let injection_ms = baseline_ms + 1_000;
    let injection_seconds = format!("{:.3}", injection_ms as f64 / 1000.0);

    let cwd = temp_dir("timeout-injection");
    let config_home = cwd.join("config");
    // `slow_prompt <delaySeconds> <sessionId>`：initialize/session/new 即时，
    // `session/prompt` 先发一条 update，再 sleep <delay> 才回 end_turn（agent 侧延迟注入）。
    write_acp_agents_config(
        &config_home,
        json!([agent_spec_json(
            "fake-slow",
            &[
                fixture_script().to_string_lossy().into_owned(),
                "slow_prompt".to_string(),
                injection_seconds,
                "acp-sess-1".to_string(),
            ],
        )]),
    );

    let (result, elapsed) = {
        let _xdg = XdgConfig::install(&config_home);
        let router = ToolRouter::with_workspace_root(cwd.clone());
        let context = parent_context(&cwd, "parent-sess-timeout", "turn-timeout-1");
        // 预算 = NFR 基线（不是自选数值）：agent 延迟 > 预算 ⇒ 必在预算处归约。
        let call = delegate_call("call-timeout", "fake-slow", "长任务：外部 agent 会睡过头", Some(baseline_ms));
        let started = Instant::now();
        let result = router.execute_with_context(&call, &context);
        (result, started.elapsed())
    };

    assert_eq!(
        result.tool_name, "acp_delegate",
        "at_01：结果必须来自 acp_delegate，实际 {result:?}"
    );

    let output = result_json(&result);
    let code = output
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    assert!(
        result.status == "error" || result.status == "aborted",
        "at_01 违约：超预算必须归约为失败态（error/aborted），实际 status={} / {output}",
        result.status
    );
    assert!(
        code == "acp_timeout" || code == "tool_cancelled",
        "at_01 违约：超预算的错误码必须是 acp_timeout（或取消语义的 tool_cancelled），\
         不得塌缩成其它码，实际 `{code}` / {output}"
    );

    // 不 hang：预算 baseline 必须先生效，而不是陪 agent 睡满注入延迟。
    assert!(
        elapsed < Duration::from_millis(injection_ms),
        "at_01 违约：委派耗时 {elapsed:?} 不得 ≥ 注入延迟 {injection_ms}ms（预算未生效 ⇒ hang 风险）"
    );

    // C0.4：非 end_turn 归约不出现 finalText（失败路径不得伪装正文）。
    assert!(
        output.get("finalText").is_none(),
        "C0.4 违约：失败归约不得出现 finalText，实际 {output}"
    );

    cleanup(&cwd);
}

// ───────────────────────── 2. 畸形 session/update 穿透 ─────────────────────────

/// `at_02a`：未知 `sessionUpdate` 变体 + 缺字段 update + 非文本块混入同一条流。
///
/// 威胁：一条畸形 update 若让整轮流被丢弃（或 panic），委派结果就只剩硬编码 summary。
/// 断言（C0.1 + 任务卡「不 panic / 不静默丢弃整条」）：
/// - 轮次仍成功归约（`status=ok`），进程不 panic；
/// - 畸形帧前后两段合法正文都保留且顺序正确（`before|after`）⇒ 流没有被整条丢弃；
/// - 缺字段帧必须留痕（`acp_protocol_error` warning），未知/非文本帧必须计入 `updates`
///   ⇒「每条帧要么进 update 流、要么进 warning」，不存在静默消失的帧。
#[test]
fn at_02a_malformed_updates_do_not_drop_the_stream() {
    let frames = vec![
        frame_text("before|"),
        // 未来/厂商私有变体：`#[serde(other)] Unknown` 兜底（不得 panic、不得整条丢弃）。
        json!({"sessionUpdate": "vendor_private_telemetry", "payload": {"x": 1}}),
        // 缺必填 `content`：v1 schema 校验失败，只能进诊断（warning），不得中断本轮流。
        json!({"sessionUpdate": "agent_message_chunk"}),
        // 非文本 ContentBlock：不得被误读为正文。
        json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
        }),
        frame_text("after"),
    ];

    let result = delegate_with_updates("malformed", "fake-malformed", &frames, "畸形 update 穿透");
    let output = result_json(&result);

    assert_eq!(
        result.status, "ok",
        "at_02a 违约：畸形 update 不得让整轮失败/中断，实际 status={} / {output}",
        result.status
    );

    // C0.1：正文 = 合法文本块按到达顺序拼接（畸形帧前后的内容都不许丢）。
    assert_eq!(
        final_text(&output),
        "before|after",
        "C0.1 违约：畸形帧前后的合法正文必须都保留且保序，实际 {output}"
    );
    assert!(
        !final_text_truncated(&output),
        "C0.3：未超上限时 finalTextTruncated 必须为 false，实际 {output}"
    );

    let warnings = warnings(&output);
    let protocol_warnings = warnings
        .iter()
        .filter(|warning| warning.contains("acp_protocol_error"))
        .count();
    assert!(
        protocol_warnings >= 1,
        "at_02a 违约：缺字段的 session/update 必须留下 acp_protocol_error 诊断（不静默丢弃），\
         实际 warnings={warnings:?}"
    );

    // 逐帧记账：5 条帧中，1 条进 warning、其余进 update 流 ⇒ 计数之和必须覆盖全部帧。
    let accounted = updates_count(&output) as usize + protocol_warnings;
    assert!(
        accounted >= frames.len(),
        "at_02a 违约：存在被静默丢弃的 update 帧（accounted={accounted} < sent={}，\
         updates={} + protocol_warnings={protocol_warnings}）",
        frames.len(),
        updates_count(&output)
    );

    // 非文本块内容不得混进正文（图像 base64 不是正文）。
    assert!(
        !final_text(&output).contains("iVBORw0KGgo="),
        "C0.1 违约：非文本 ContentBlock 不得进入 finalText，实际 {output}"
    );
}

/// `at_02b`：单个 `agent_message_chunk` 正文 > 32k。
///
/// 契约 C0.3：保留前缀 + `finalTextTruncated=true`，且**有界**（上限是单一常量真源，
/// 见 `src/acp/result.rs:25`）。不得整段返回（会把 session blob 撑爆），也不得整段丢弃。
#[test]
fn at_02b_oversized_chunk_is_bounded_and_flagged() {
    let oversized = "x".repeat(FINAL_TEXT_MAX_CHARS + 1_000);
    let frames = vec![frame_text(&oversized), frame_text("TAIL")];

    let result = delegate_with_updates("oversized", "fake-oversized", &frames, "超长正文穿透");
    let output = result_json(&result);

    assert_eq!(
        result.status, "ok",
        "at_02b 前置：超长正文仍应是成功轮次，实际 status={} / {output}",
        result.status
    );

    let text = final_text(&output);
    let chars = text.chars().count();
    assert!(
        chars > 0,
        "C0.3 违约：超长正文必须保留前缀（不得整段丢弃），实际 finalText 为空 / {output}"
    );
    assert!(
        chars <= FINAL_TEXT_MAX_CHARS,
        "C0.3 违约：finalText 必须被 {FINAL_TEXT_MAX_CHARS} 字符上限截断（真源 src/acp/result.rs:25），\
         实际 {chars} 字符 / {output}"
    );
    assert_eq!(
        text,
        "x".repeat(chars),
        "C0.3 违约：截断必须是**前缀保留**（不得穿插/重排），实际前缀不连续"
    );
    assert!(
        final_text_truncated(&output),
        "C0.3 违约：超长截断必须置 finalTextTruncated=true，实际 {output}"
    );
}

/// `at_02c`：非文本 ContentBlock（image）夹在正文中间。
///
/// 断言：不 panic、轮次成功、只有文本块进入 `finalText` 且保序（`AB`），非文本帧仍被记账。
#[test]
fn at_02c_non_text_content_block_is_not_text() {
    let frames = vec![
        frame_text("A"),
        json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
        }),
        frame_text("B"),
    ];

    let result = delegate_with_updates("non-text", "fake-non-text", &frames, "非文本内容块");
    let output = result_json(&result);

    assert_eq!(
        result.status, "ok",
        "at_02c 违约：非文本块不得让整轮失败，实际 status={} / {output}",
        result.status
    );
    assert_eq!(
        final_text(&output),
        "AB",
        "C0.1 违约：非文本块既不进正文也不得吞掉相邻文本块，实际 {output}"
    );
    assert!(
        updates_count(&output) >= frames.len() as u64,
        "at_02c 违约：非文本帧必须仍被记账（不静默丢弃），实际 updates={} / {output}",
        updates_count(&output)
    );
}

// ───────────────────────── 3. 并发委派 ─────────────────────────

/// 为「同一 session 两路并发 prompt」现场生成的**顺序记录型** fake agent。
///
/// 为什么不复用 `tests/fixtures/acp_fake_agent.sh`：本文件只允许落 `tests/acp_delegate_adversarial.rs`
/// 与红相日志，不得改冻结 fixture；而串行闸的**线上证据**需要「每收到一行请求就留痕、且
/// 在 hold 期继续读 stdin」的模式（现存模式均无此能力）。
///
/// 协议：`sh <script> <recordFile> <holdSeconds>`
/// - `initialize` / `session/new` 立即应答；
/// - `session/prompt <id>`：先 append `REQ prompt <id>`，再交给**后台**子 shell：
///   `sleep <hold>` → append `ANSWER prompt <id>` → 回 `end_turn`；
///   主 shell 立刻回到读循环 ⇒ 若宿主**没有**串行闸，第二路 prompt 就会在这段 hold 内被读到
///   并留下 `REQ prompt`（顺序即结论，不依赖时间度量）。
const GATE_PROBE_AGENT: &str = r#"set -u
rec="$1"
hold="$2"
note() { printf '%s\n' "$1" >> "$rec"; }
id_of() { printf '%s' "$1" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p'; }
method_of() { printf '%s' "$1" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p'; }
while IFS= read -r line; do
  [ -z "$line" ] && continue
  m=$(method_of "$line")
  i=$(id_of "$line")
  case "$m" in
    initialize)
      note "REQ initialize"
      [ -n "$i" ] && printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":false},"authMethods":[]}}\n' "$i"
      ;;
    session/new)
      note "REQ session/new"
      [ -n "$i" ] && printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"acp-sess-gate"}}\n' "$i"
      ;;
    session/prompt)
      note "REQ prompt $i"
      ( sleep "$hold"; note "ANSWER prompt $i"; printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$i" ) &
      ;;
    *)
      note "REQ other $m"
      ;;
  esac
done
wait
"#;

fn write_gate_probe_agent(dir: &Path) -> PathBuf {
    let path = dir.join("gate_probe_agent.sh");
    std::fs::write(&path, GATE_PROBE_AGENT).expect("写入 gate probe fake agent");
    path
}

fn wire_records(record: &Path) -> Vec<String> {
    std::fs::read_to_string(record)
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// `at_03a`：同一 `AcpAgentSession` 两路并发 `prompt` ⇒ 会话串行闸必须在**线上**生效。
///
/// 确定性：固定顺序（先 A 后 B）+ 固定迭代（各一轮）；A 进入 agent 用「wire 记录出现
/// `REQ prompt`」同步（有界轮询），不靠 sleep 猜时序。
/// 负样本（无闸）时 B 的 `session/prompt` 会在 A 的 hold 期被 agent 读到，
/// 于是 `REQ prompt B` 出现在 `ANSWER prompt A` 之前 ⇒ 断言必然失败。
#[test]
fn at_03a_same_session_prompts_are_serialized_on_the_wire() {
    let cwd = temp_dir("session-gate");
    let script = write_gate_probe_agent(&cwd);
    let record = cwd.join("wire-record.txt");
    let mut args = vec![script.to_string_lossy().into_owned()];
    args.push(record.to_string_lossy().into_owned());
    args.push("0.800".to_string());

    let spec = AcpAgentSpec {
        id: "gate-probe".to_string(),
        label: "Gate probe".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
        inherit_proxy_env: true,
        mcp_servers: None,
    };
    let mut session = match AcpAgentSession::spawn(&spec, &cwd) {
        Ok(session) => session,
        Err(error) => panic!("at_03a 前置：gate probe agent 必须可握手，实际 {error:?}"),
    };
    let bound = op_timeout();

    let (first_tx, first_rx) = mpsc::channel();
    let (second_tx, second_rx) = mpsc::channel();
    let (first_result, second_result) = std::thread::scope(|scope| {
        let shared = &session;
        scope.spawn(move || {
            let result = shared.prompt(prompt_blocks("第一路委派"), &NoopTurnEventSink);
            let _ = first_tx.send(result);
        });

        // 同步点：第一路 prompt 已抵达 agent（线上留痕），此时串行闸必须已被第一路持有。
        assert!(
            wait_until(
                || wire_records(&record)
                    .iter()
                    .any(|line| line.starts_with("REQ prompt ")),
                bound
            ),
            "at_03a 前置：第一路 prompt 必须在 {bound:?} 内抵达 agent，实际记录 {:?}",
            wire_records(&record)
        );

        scope.spawn(move || {
            let result = shared.prompt(prompt_blocks("第二路委派"), &NoopTurnEventSink);
            let _ = second_tx.send(result);
        });

        let first = first_rx
            .recv_timeout(bound + bound)
            .expect("at_03a 违约：第一路 prompt 必须及时返回（不 hang）");
        let second = second_rx
            .recv_timeout(bound + bound)
            .expect("at_03a 违约：第二路 prompt 必须及时返回（不 hang）");
        (first, second)
    });

    let lines = wire_records(&record);
    let requests: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("REQ prompt "))
        .map(|(index, _)| index)
        .collect();
    let answers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("ANSWER prompt "))
        .map(|(index, _)| index)
        .collect();

    assert_eq!(
        requests.len(),
        2,
        "at_03a 前置：两路 prompt 都必须抵达 agent，实际线上记录 {lines:?}"
    );
    assert_eq!(
        answers.len(),
        2,
        "at_03a 前置：两路 prompt 都必须被应答，实际线上记录 {lines:?}"
    );
    assert!(
        requests[0] < requests[1],
        "at_03a：固定顺序（先 A 后 B）必须体现在线上顺序上，实际 {lines:?}"
    );
    assert!(
        requests[1] > answers[0],
        "at_03a 违约：同一 ACP session 上出现并发 prompt —— 第二路 `REQ prompt` 早于第一路 \
         `ANSWER prompt`（串行闸未生效，直接并发打了同一 session），线上记录 {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line == "REQ cancel"),
        "at_03a：串行排队不得退化成取消，实际 {lines:?}"
    );

    for (label, result) in [("第一路", &first_result), ("第二路", &second_result)] {
        assert_eq!(
            result.status, "ok",
            "at_03a 违约：{label} prompt 必须成功（排队而非失败），实际 {result:?}"
        );
    }

    session.close();
    cleanup(&cwd);
}

// ── 父 turn 事件捕获（C1.1 / C1.5：生产持久化通道是唯一公开的观测缝） ──

#[derive(Clone, Debug)]
struct CapturedToolEvent {
    /// `emit_external_turn_event` 透传的父会话 id（落库键）。
    persist_session_id: String,
    /// 透传的父 turn id。
    persist_turn_id: String,
    /// 事件自带的 `turn_id` 字段（C1.5：不得是 ACP session id）。
    event_turn_id: String,
    /// C1.1：ACP 子活动归属的父活动 id（= `acp_delegate` call_id）。
    parent_activity_id: Option<String>,
    /// 事件的活动 id（子活动 = ACP `toolCallId`）。
    activity_id: String,
}

fn capture_tool_event(
    session_id: &str,
    turn_id: &str,
    event: &TurnEvent,
) -> Option<CapturedToolEvent> {
    let (event_turn_id, parent_activity_id, activity_id) = match event {
        TurnEvent::ToolCall {
            turn_id,
            call_id,
            parent_activity_id,
            ..
        } => (turn_id.clone(), parent_activity_id.clone(), call_id.clone()),
        TurnEvent::ToolResult {
            turn_id,
            call_id,
            parent_activity_id,
            ..
        } => (turn_id.clone(), parent_activity_id.clone(), call_id.clone()),
        _ => return None,
    };
    Some(CapturedToolEvent {
        persist_session_id: session_id.to_string(),
        persist_turn_id: turn_id.to_string(),
        event_turn_id,
        parent_activity_id,
        activity_id,
    })
}

/// `at_03b`：两路**并发**委派（不同父会话/父 turn/call_id）的事件归属。
///
/// 契约 C1.1 / C1.5：
/// - 每路委派的 ACP `tool_call` 必须归集为**自身父 turn** 的 `turn:tool` 事件
///   （`parent_activity_id == 本方 acp_delegate call_id`）；
/// - 事件 session/turn = 父会话/父 turn，`turn_id` 不得是 ACP session id，`parent_activity_id` 不得为空；
/// - 两路不得串会话（A 的事件不得带 B 的 session/turn/活动 id，反之亦然）；
/// - 正文也不得串（`finalText` 各归各家，C0.1）。
///
/// 确定性：`Barrier` 同时起跑 + 固定迭代（各一轮），事件从生产持久化通道
/// `turn_flow::register_event_persist` 收取（唯一公开观测缝），不用 sleep 竞态。
#[test]
fn at_03b_concurrent_delegations_do_not_cross_session_events() {
    let cwd = temp_dir("cross-session");
    let config_home = cwd.join("config");
    let updates_alpha = cwd.join("updates-alpha.jsonl");
    let updates_beta = cwd.join("updates-beta.jsonl");
    write_frames(
        &updates_alpha,
        &[frame_text("alpha-text"), frame_tool("tc-alpha", "edit alpha.rs")],
    );
    write_frames(
        &updates_beta,
        &[frame_text("beta-text"), frame_tool("tc-beta", "edit beta.rs")],
    );

    write_acp_agents_config(
        &config_home,
        json!([
            agent_spec_json("fake-alpha", &[
                fixture_script().to_string_lossy().into_owned(),
                "emit_updates".to_string(),
                cwd.join("init-alpha.jsonl").to_string_lossy().into_owned(),
                updates_alpha.to_string_lossy().into_owned(),
            ]),
            agent_spec_json("fake-beta", &[
                fixture_script().to_string_lossy().into_owned(),
                "emit_updates".to_string(),
                cwd.join("init-beta.jsonl").to_string_lossy().into_owned(),
                updates_beta.to_string_lossy().into_owned(),
            ]),
        ]),
    );

    let captured: Arc<Mutex<Vec<CapturedToolEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let (results, events) = {
        let _xdg = XdgConfig::install(&config_home);
        let router = Arc::new(ToolRouter::with_workspace_root(cwd.clone()));
        let sink = Arc::clone(&captured);
        register_event_persist(Arc::new(
            move |session_id: &str, turn_id: &str, event: TurnEvent, _terminal: bool| {
                if let Some(record) = capture_tool_event(session_id, turn_id, &event) {
                    sink.lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(record);
                }
            },
        ));

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (tx, rx) = mpsc::channel::<(&'static str, ToolResult)>();
        let results = std::thread::scope(|scope| {
            for (label, agent, session_id, turn_id, call_id) in [
                ("alpha", "fake-alpha", "sess-alpha", "turn-alpha", "call-alpha"),
                ("beta", "fake-beta", "sess-beta", "turn-beta", "call-beta"),
            ] {
                let router = Arc::clone(&router);
                let barrier = Arc::clone(&barrier);
                let tx = tx.clone();
                let cwd = cwd.clone();
                scope.spawn(move || {
                    let context = parent_context(&cwd, session_id, turn_id);
                    let call = delegate_call(call_id, agent, "并发委派", None);
                    barrier.wait();
                    let result = router.execute_with_context(&call, &context);
                    let _ = tx.send((label, result));
                });
            }
            drop(tx);
            let mut collected = Vec::new();
            for _ in 0..2 {
                collected.push(
                    rx.recv_timeout(op_timeout() * 2)
                        .expect("at_03b 违约：并发委派必须都及时返回（不 hang）"),
                );
            }
            collected
        });

        let events = captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        clear_event_persist();
        (results, events)
    };

    let pick = |label: &str| -> ToolResult {
        results
            .iter()
            .find(|entry| entry.0 == label)
            .map(|(_, result)| result.clone())
            .unwrap_or_else(|| panic!("at_03b 前置：缺少 {label} 的结果"))
    };
    let alpha = pick("alpha");
    let beta = pick("beta");
    for (label, result) in [("alpha", &alpha), ("beta", &beta)] {
        assert_eq!(
            result.tool_name, "acp_delegate",
            "at_03b：{label} 结果必须来自 acp_delegate，实际 {result:?}"
        );
        assert_eq!(
            result.status, "ok",
            "at_03b 违约：{label} 委派必须成功，实际 status={} / {}",
            result.status, result.output
        );
    }

    // 逐条收集违约（不早退）：一条用例要同时暴露 C0.1（正文归约）与 C1.1/C1.5（父 turn 事件归属）
    // 两个威胁面，否则红相只看到第一个断言，证据面被吃掉。
    let mut violations: Vec<String> = Vec::new();

    // C0.1：正文本体不得串（各自 agent 发出的 chunk 各归各家）。
    let alpha_output = result_json(&alpha);
    let beta_output = result_json(&beta);
    for (label, output, expected) in [
        ("alpha", &alpha_output, "alpha-text"),
        ("beta", &beta_output, "beta-text"),
    ] {
        match output.get("finalText").and_then(Value::as_str) {
            Some(text) if text == expected => {}
            other => violations.push(format!(
                "C0.1 违约：{label} 委派 finalText 应为 `{expected}`（各归各家，不得串会话），\
                 实际 {other:?} / 顶层键={:?}",
                output
                    .as_object()
                    .map(|map| map.keys().cloned().collect::<Vec<_>>())
            )),
        }
    }

    // C1.1：每路委派都必须留下属于**自身父活动**的事件（W1 骨架期 0 条 ⇒ 此处必红）。
    if events.is_empty() {
        violations.push(
            "C1.1 违约：委派期间的 ACP tool_call 必须归集为父 turn 事件并经持久化通道落盘，\
             实际捕获 0 条（delegate_events 未接入，见 src/acp/delegate_events.rs:46）"
                .to_string(),
        );
    }

    for (parent, session_id, turn_id) in [
        ("call-alpha", "sess-alpha", "turn-alpha"),
        ("call-beta", "sess-beta", "turn-beta"),
    ] {
        let own: Vec<&CapturedToolEvent> = events
            .iter()
            .filter(|event| event.parent_activity_id.as_deref() == Some(parent))
            .collect();
        if own.is_empty() && !events.is_empty() {
            violations.push(format!(
                "C1.1 违约：`parent_activity_id={parent}` 的父 turn 事件缺失\
                 （必须 == acp_delegate call_id），实际捕获事件 = {events:?}"
            ));
        }
        let foreign = if parent == "call-alpha" {
            "tc-beta"
        } else {
            "tc-alpha"
        };
        for event in own {
            if event.persist_session_id != session_id {
                violations.push(format!(
                    "C1.1 违约：事件 session 必须是父会话 `{session_id}`（不得串会话），实际 {event:?}"
                ));
            }
            if event.persist_turn_id != turn_id {
                violations.push(format!(
                    "C1.1 违约：事件落库 turn 必须是父 turn `{turn_id}`，实际 {event:?}"
                ));
            }
            if event.event_turn_id != turn_id {
                violations.push(format!(
                    "C1.5 违约：事件 `turn_id` 必须是父 turn id（不得为 ACP session id），\
                     实际 {event:?}"
                ));
            }
            if event.activity_id.contains(foreign) {
                violations.push(
                    "C1.1 违约：事件不得携带另一路委派的活动 id（串会话）".to_string()
                        + &format!("，实际 {event:?}"),
                );
            }
        }
    }

    // 反向：任何事件都不得同时属于两路（parent 与 session/turn 必须自洽）。
    for event in &events {
        if let Some(parent) = event.parent_activity_id.as_deref() {
            match parent {
                "call-alpha" | "call-beta" => {
                    let expected = if parent == "call-alpha" {
                        "sess-alpha"
                    } else {
                        "sess-beta"
                    };
                    if event.persist_session_id != expected {
                        violations.push(format!(
                            "C1.1 违约：父活动与会话归属不自洽（串会话），实际 {event:?}"
                        ));
                    }
                }
                other => violations.push(format!(
                    "C1.1 违约：出现未知 parent_activity_id `{other}`（必须恰为某次 acp_delegate 的 \
                     call_id）：{event:?}"
                )),
            }
        }
    }

    assert!(
        violations.is_empty(),
        "at_03b 违约（{} 条，按契约分组）：\n- {}",
        violations.len(),
        violations.join("\n- ")
    );

    cleanup(&cwd);
}
