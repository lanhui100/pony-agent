//! Wave43 · Agent Trace 迁移（PonySentry → OTel/Langfuse）验收测试（红相）。
//!
//! 本文件是 **Test Agent 独占写域**，只读契约、不得由实现方修改。契约来源：
//! - `.dev-team/wave43-adjudication.md`（A2 纯映射 seam / A3 flush 与门禁 / A4 手动导出旁路 / A6 落盘键）
//! - `.dev-team/contract-matrix-wave43.md`（OT-1 / OT-3 / OT-4 / OT-6 / OT-7 / OT-8）
//!
//! 覆盖范围（均可用纯函数或本地注入证明，零外网依赖）：
//! 1. OT-1 门禁：`is_reporting_enabled()` 缺省 false、`set_reporting_enabled` 往返幂等、
//!    `force_flush` 未 init 返回 Err（A3），以及**关闭态零 egress**（本地 OTLP sink 计数）。
//! 2. OT-3 LLM 属性：`gen_ai.system` / `gen_ai.request.model` / 同值双写的
//!    `langfuse.observation.model.name` / token 用量键；缺失值不输出该键。
//! 3. OT-4 工具属性：`gen_ai.tool.name` / `langfuse.observation.type=tool` / status / duration / error。
//! 4. turn 属性：session/trace name/operation/turn id/sequence/phase + user_id 有则带。
//! 5. OT-6 手动导出旁路（A4）：签名冻结、`Ok(2)` 数量契约、**不改变 gate 末态**、
//!    真实落到本地 sink 的 `POST /v1/traces` + wire 级 body 断言。
//! 6. OT-7 退役守卫：13 个待退役符号 0 命中 + 8 个保留面符号仍存在（双向，防过度删除）。
//! 7. OT-8 设置持久化：缺省 false / camelCase 键往返 / legacy 无键容错（本地临时文件），
//!    以及 Tauri 宿主桥接的 P4 静态断言（8d 设置读写双路径同步 core 门禁、8e 退出路径释放 provider）。
//!
//! 断言纪律：属性键一律用**字面量**断言（避免「常量被改后自指自证」），另设一条常量字面量冻结
//! 测试；禁止 `#[ignore]`、禁止无断言、禁止 `unwrap()` 吞错。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::app_settings::{AppSettings, AppSettingsStore, WorkspaceMode};
use pony_agent_core::agent::otel::{
    self, AttrValue, LlmSpanInput, ToolSpanInput, TurnSpanInput, AUTH_HEADER_NAME,
    ATTR_AGENT_TOOL_DURATION_MS, ATTR_AGENT_TOOL_ERROR, ATTR_AGENT_TOOL_STATUS, ATTR_AGENT_TURN_ID,
    ATTR_AGENT_TURN_PHASE, ATTR_AGENT_TURN_SEQUENCE, ATTR_GEN_AI_OPERATION_NAME,
    ATTR_GEN_AI_REQUEST_MODEL, ATTR_GEN_AI_SYSTEM, ATTR_GEN_AI_TOOL_NAME,
    ATTR_GEN_AI_USAGE_INPUT_TOKENS, ATTR_GEN_AI_USAGE_OUTPUT_TOKENS,
    ATTR_LANGFUSE_OBSERVATION_MODEL_NAME, ATTR_LANGFUSE_OBSERVATION_TYPE, ATTR_LANGFUSE_SESSION_ID,
    ATTR_LANGFUSE_TRACE_NAME, ATTR_LANGFUSE_USER_ID, DEFAULT_ENDPOINT, DEFAULT_SAMPLING_RATIO,
    DEFAULT_SERVICE_NAME, ENV_OTLP_ENDPOINT, ENV_PONY_OTEL_TOKEN, ENV_SAMPLER_ARG, ENV_SERVICE_NAME,
    OBSERVATION_TYPE_TOOL, OPERATION_AGENT_TURN, SPAN_AGENT_TURN, SPAN_LLM_CHAT, SPAN_TOOL_CALL,
};
use pony_agent_core::agent::session::TurnTraceRecord;

// ---------------------------------------------------------------- 门禁串行锁
//
// 全局门禁是进程级 `AtomicBool`，cargo test 默认多线程并发跑用例。凡触达门禁/全局 provider
// 的用例必须持锁串行，且每条用例结束时把门禁复位为 false，保证「缺省 false」断言不被污染。

static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------- 属性断言小工具

fn attr<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> Option<&'a AttrValue> {
    attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

/// 断言某键存在且为字符串，返回其值（键名缺失时报出全集，便于定位）。
fn require_str<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> &'a str {
    match attr(attrs, key) {
        Some(AttrValue::Str(value)) => value.as_str(),
        other => panic!(
            "属性 `{key}` 必须以字符串属性输出，实际 {:?}；当前属性全集：{:?}",
            other,
            attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ),
    }
}

/// 断言某键存在且为整数（OTel 数值属性；落成字符串会破坏 Langfuse 数值过滤）。
fn require_i64(attrs: &[(String, AttrValue)], key: &str) -> i64 {
    match attr(attrs, key) {
        Some(AttrValue::I64(value)) => *value,
        other => panic!(
            "属性 `{key}` 必须以整数（AttrValue::I64）输出，实际 {:?}；当前属性全集：{:?}",
            other,
            attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ),
    }
}

/// A2：缺失值一律不输出该键（禁止空串占位）。
fn assert_absent(attrs: &[(String, AttrValue)], key: &str) {
    assert!(
        attr(attrs, key).is_none(),
        "缺失值不得输出键 `{key}`，实际存在：{:?}",
        attr(attrs, key)
    );
}

/// A2 后半条：任何属性都不得是空串占位（避免 Langfuse 侧出现空 observation 字段）。
fn assert_no_empty_string_attr(attrs: &[(String, AttrValue)]) {
    let empties: Vec<&str> = attrs
        .iter()
        .filter(|(_, value)| matches!(value, AttrValue::Str(text) if text.is_empty()))
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(empties.is_empty(), "禁止空串占位属性，命中键：{empties:?}");
}

/// 语义轨路由谓词（collector 侧不可改）：`gen_ai.system` 存在 **或** 任意 `langfuse.*`。
fn assert_langfuse_routing_predicate(attrs: &[(String, AttrValue)], span: &str) {
    assert!(
        attrs
            .iter()
            .any(|(name, _)| name == "gen_ai.system" || name.starts_with("langfuse.")),
        "{span} 必须至少命中一个 Langfuse 语义轨路由谓词（gen_ai.system 或 langfuse.*），实际键集：{:?}",
        attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
}

// -------------------------------------------------- 本地 OTLP/HTTP sink（P1 探针）
//
// 非阻塞 accept + 固定轮询 + 硬截止（仓内既有验收惯例，禁止无界等待），捕获
// method/path/body 后回 `200 + 空 ExportTraceServiceResponse`。
// 硬截止保证「关闭态零 egress」这类负向断言不会悬空等待。

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_millis(1_000);
    let mut expected_total: Option<usize> = None;

    while Instant::now() < deadline {
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
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }

    raw
}

fn parse_captured_request(raw: &[u8]) -> CapturedRequest {
    let header_end = find_subslice(raw, b"\r\n\r\n").unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let body = if header_end + 4 <= raw.len() {
        raw[header_end + 4..].to_vec()
    } else {
        Vec::new()
    };
    CapturedRequest { method, path, body }
}

fn spawn_otlp_sink(budget: Duration) -> (u16, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("本地 OTLP sink 必须能绑定回环端口");
    listener
        .set_nonblocking(true)
        .expect("本地 OTLP sink 必须为非阻塞监听");
    let port = listener.local_addr().expect("sink local_addr").port();

    let handle = std::thread::spawn(move || {
        let mut captured: Vec<CapturedRequest> = Vec::new();
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _peer)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                    let raw = read_http_request(&mut stream);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    );
                    let _ = stream.flush();
                    captured.push(parse_captured_request(&raw));
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        captured
    });

    (port, handle)
}

fn local_config(port: u16) -> otel::OtelConfig {
    otel::OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent-wave43-acceptance".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

fn turn_record(session_id: &str, turn_id: &str, phase: &str) -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: turn_id.to_string(),
        session_id: Some(session_id.to_string()),
        title: "wave43 otel acceptance".to_string(),
        phase: phase.to_string(),
        ..Default::default()
    }
}

// ============================================================== 冻结常量字面量
//
// 契约矩阵 P2/OT-2/OT-3/OT-4 的 wire 级断言依赖这些字面量；常量被改则 Langfuse 侧静默失联，
// 因此单列一条测试锁死取值（含 plan D3/D4 的端点、服务名、env 变量名、采样率缺省 1.0）。

#[test]
fn frozen_span_names_attribute_keys_and_defaults_keep_literals() {
    // span 名（D4 / mod.rs 冻结）
    assert_eq!(SPAN_AGENT_TURN, "agent.turn");
    assert_eq!(SPAN_LLM_CHAT, "llm.chat");
    assert_eq!(SPAN_TOOL_CALL, "tool.call");
    assert_eq!(OPERATION_AGENT_TURN, "agent.turn");
    assert_eq!(OBSERVATION_TYPE_TOOL, "tool");

    // OTel GenAI 惯例键
    assert_eq!(ATTR_GEN_AI_SYSTEM, "gen_ai.system");
    assert_eq!(ATTR_GEN_AI_REQUEST_MODEL, "gen_ai.request.model");
    assert_eq!(ATTR_GEN_AI_USAGE_INPUT_TOKENS, "gen_ai.usage.input_tokens");
    assert_eq!(ATTR_GEN_AI_USAGE_OUTPUT_TOKENS, "gen_ai.usage.output_tokens");
    assert_eq!(ATTR_GEN_AI_TOOL_NAME, "gen_ai.tool.name");
    assert_eq!(ATTR_GEN_AI_OPERATION_NAME, "gen_ai.operation.name");

    // Langfuse 语义轨键
    assert_eq!(ATTR_LANGFUSE_SESSION_ID, "langfuse.session.id");
    assert_eq!(ATTR_LANGFUSE_USER_ID, "langfuse.user.id");
    assert_eq!(ATTR_LANGFUSE_TRACE_NAME, "langfuse.trace.name");
    assert_eq!(ATTR_LANGFUSE_OBSERVATION_TYPE, "langfuse.observation.type");
    assert_eq!(
        ATTR_LANGFUSE_OBSERVATION_MODEL_NAME,
        "langfuse.observation.model.name"
    );

    // pony-agent 自有维度
    assert_eq!(ATTR_AGENT_TURN_ID, "agent.turn.id");
    assert_eq!(ATTR_AGENT_TURN_SEQUENCE, "agent.turn.sequence");
    assert_eq!(ATTR_AGENT_TURN_PHASE, "agent.turn.phase");
    assert_eq!(ATTR_AGENT_TOOL_STATUS, "agent.tool.status");
    assert_eq!(ATTR_AGENT_TOOL_DURATION_MS, "agent.tool.duration_ms");
    assert_eq!(ATTR_AGENT_TOOL_ERROR, "agent.tool.error");

    // D3 缺省值与环境变量
    assert_eq!(
        DEFAULT_ENDPOINT,
        "http://otel-collector.observability.svc.cluster.local:4318"
    );
    assert_eq!(DEFAULT_SERVICE_NAME, "pony-agent");
    assert_eq!(DEFAULT_SAMPLING_RATIO, 1.0);
    assert_eq!(ENV_OTLP_ENDPOINT, "OTEL_EXPORTER_OTLP_ENDPOINT");
    assert_eq!(ENV_SERVICE_NAME, "OTEL_SERVICE_NAME");
    assert_eq!(ENV_SAMPLER_ARG, "OTEL_TRACES_SAMPLER_ARG");
    assert_eq!(ENV_PONY_OTEL_TOKEN, "PONY_OTEL_TOKEN");
    assert_eq!(AUTH_HEADER_NAME, "authorization");
}

// ================================================================== OT-1 门禁
// A3 冻结的三条行为：未 init 时 `is_reporting_enabled()` 返回 false；`set_reporting_enabled`
// 幂等、末态等于最后一次写入值；未 init 时 `force_flush` 返回 Err。

#[test]
fn ot1_reporting_gate_defaults_closed() {
    let _guard = gate_lock();

    // Act + Assert：不做任何写入，直接读取缺省态。
    assert!(
        !otel::is_reporting_enabled(),
        "A3/OT-1：缺省（未 init、未写入）时全局上报门禁必须为 false"
    );
}

#[test]
fn ot1_reporting_gate_roundtrips_idempotently() {
    let _guard = gate_lock();

    otel::set_reporting_enabled(true);
    assert!(otel::is_reporting_enabled(), "set_reporting_enabled(true) 后必须读到 true");

    otel::set_reporting_enabled(true);
    assert!(
        otel::is_reporting_enabled(),
        "重复置 true 必须幂等（A3：末态等于最后一次写入值）"
    );

    otel::set_reporting_enabled(false);
    assert!(!otel::is_reporting_enabled(), "set_reporting_enabled(false) 后必须读到 false");

    otel::set_reporting_enabled(false);
    assert!(
        !otel::is_reporting_enabled(),
        "重复置 false 必须幂等（A3：末态等于最后一次写入值）"
    );
}

#[test]
fn ot1_force_flush_without_init_returns_err() {
    let _guard = gate_lock();
    otel::shutdown(); // A3：shutdown 幂等，作为「未 init 状态」的可复现前置。

    match otel::force_flush(100) {
        Ok(()) => panic!("A3：未调用过 init 时 force_flush 必须返回 Err"),
        Err(message) => assert!(
            !message.is_empty(),
            "A3：Err 必须携带可诊断原因，不得返回空字符串"
        ),
    }
}

/// OT-1 最高危契约：关闭态必须**零网络出口**（D2：GatedSampler Drop ⇒ 不分配 span、不排队）。
#[test]
fn ot1_closed_gate_exports_zero_requests_to_local_otlp_sink() {
    let _guard = gate_lock();
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_200));

    // Arrange：显式关闭门禁后装配本地端点（回环，零外网）。
    otel::set_reporting_enabled(false);
    otel::init(local_config(port))
        .expect("D3/A3：本地回环端点必须可装配（装配失败只降级不 panic）");
    assert!(
        !otel::is_reporting_enabled(),
        "OT-1：装配不得隐式打开门禁（缺省必须仍为 false）"
    );

    // Act：关闭态下完整走一遍 turn 守卫生命周期 + flush。
    let scope = otel::TurnScope::new("sess-otel-ot1", "turn-otel-ot1", 0);
    assert!(
        scope.trace_id().is_none(),
        "OT-1：关闭态零分配，TurnScope::trace_id() 必须为 None"
    );
    scope.end();
    otel::force_flush(500).expect("A3：已 init 时 force_flush 必须返回 Ok");
    otel::shutdown();

    // Assert：整个窗口（init → span 生命周期 → flush → shutdown）零 egress。
    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        captured.is_empty(),
        "OT-1：关闭态必须零网络出口，实际捕获 {} 个请求：{:?}",
        captured.len(),
        captured
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect::<Vec<_>>()
    );
}

// ================================================================== turn 属性

fn turn_input() -> TurnSpanInput {
    TurnSpanInput {
        session_id: "sess-otel-turn".to_string(),
        user_id: Some("user-otel-1".to_string()),
        turn_id: "turn-otel-1".to_string(),
        sequence: 7,
        phase: "completed".to_string(),
    }
}

#[test]
fn turn_span_attributes_carry_frozen_session_and_turn_keys() {
    // Act
    let attrs = otel::turn_span_attributes(&turn_input());

    // Assert：键与值成对断言（键在值错同样红）。
    assert_eq!(
        require_str(&attrs, "langfuse.session.id"),
        "sess-otel-turn",
        "langfuse.session.id 必须等于 session_id（同会话多轮聚合的前提）"
    );
    assert_eq!(
        require_str(&attrs, "gen_ai.operation.name"),
        "agent.turn",
        "turn root span 的 gen_ai.operation.name 恒为 agent.turn"
    );
    assert_eq!(require_str(&attrs, "agent.turn.id"), "turn-otel-1");
    assert_eq!(require_i64(&attrs, "agent.turn.sequence"), 7);
    assert_eq!(require_str(&attrs, "agent.turn.phase"), "completed");
    assert_eq!(require_str(&attrs, "langfuse.user.id"), "user-otel-1");

    // langfuse.trace.name 的取值未在 A2 冻结，只断言「存在且非空」（禁止空串占位）。
    assert!(
        !require_str(&attrs, "langfuse.trace.name").is_empty(),
        "langfuse.trace.name 不得为空串（A2：禁止空串占位）"
    );

    assert_no_empty_string_attr(&attrs);
    assert_langfuse_routing_predicate(&attrs, "agent.turn");
}

#[test]
fn turn_span_attributes_omit_absent_user_id() {
    let mut input = turn_input();
    input.user_id = None;

    let attrs = otel::turn_span_attributes(&input);

    assert_absent(&attrs, "langfuse.user.id");
    // 反向防过度删除：其余必备键仍必须在位。
    assert_eq!(require_str(&attrs, "langfuse.session.id"), "sess-otel-turn");
    assert_eq!(require_str(&attrs, "agent.turn.id"), "turn-otel-1");
    assert_no_empty_string_attr(&attrs);
}

// ================================================================== OT-3 LLM 属性

fn llm_input() -> LlmSpanInput {
    LlmSpanInput {
        provider_kind: "anthropic".to_string(),
        model: Some("claude-sonnet-4-otel".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(20),
        operation: "chat".to_string(),
    }
}

#[test]
fn ot3_llm_span_attributes_carry_model_and_usage_keys() {
    let attrs = otel::llm_span_attributes(&llm_input());

    // 双断言之一：强断言（键 + 值成对）。
    assert_eq!(
        require_str(&attrs, "gen_ai.system"),
        "anthropic",
        "语义轨路由强谓词 gen_ai.system 必须携带 provider kind"
    );

    // 契约名为 gen_ai.request.model；若只出现旧名 gen_ai.model，报出裁决口径而非泛化失败。
    if attr(&attrs, "gen_ai.request.model").is_none() && attr(&attrs, "gen_ai.model").is_some() {
        panic!(
            "只找到旧名 gen_ai.model：D4/A2 冻结契约为 gen_ai.request.model；如裁决改名，须先改契约再改断言（禁止单方改测试）"
        );
    }
    assert_eq!(
        require_str(&attrs, "gen_ai.request.model"),
        "claude-sonnet-4-otel"
    );

    // A2：同值双写，避免 Langfuse observation 模型名口径歧义。
    assert_eq!(
        require_str(&attrs, "langfuse.observation.model.name"),
        require_str(&attrs, "gen_ai.request.model"),
        "langfuse.observation.model.name 必须与 gen_ai.request.model 同值双写"
    );

    assert_eq!(require_i64(&attrs, "gen_ai.usage.input_tokens"), 10);
    assert_eq!(require_i64(&attrs, "gen_ai.usage.output_tokens"), 20);
    assert_eq!(require_str(&attrs, "gen_ai.operation.name"), "chat");

    assert_no_empty_string_attr(&attrs);
    // 双断言之二：谓词交集非空（弱断言，防止只满足其一就被判定通过）。
    assert_langfuse_routing_predicate(&attrs, "llm.chat");
}

#[test]
fn ot3_llm_span_attributes_omit_absent_model_and_token_keys() {
    let input = LlmSpanInput {
        provider_kind: "openai".to_string(),
        model: None,
        input_tokens: None,
        output_tokens: None,
        operation: "chat".to_string(),
    };

    let attrs = otel::llm_span_attributes(&input);

    assert_eq!(require_str(&attrs, "gen_ai.system"), "openai");
    // A2：缺失值不输出该键（模型名两处双写键同时消失）。
    assert_absent(&attrs, "gen_ai.request.model");
    assert_absent(&attrs, "langfuse.observation.model.name");
    assert_absent(&attrs, "gen_ai.usage.input_tokens");
    assert_absent(&attrs, "gen_ai.usage.output_tokens");
    // 恒输出项仍须在位。
    assert_eq!(require_str(&attrs, "gen_ai.operation.name"), "chat");
    assert_no_empty_string_attr(&attrs);
}

// ================================================================== OT-4 工具属性

#[test]
fn ot4_tool_span_attributes_carry_tool_semantics() {
    let attrs = otel::tool_span_attributes(&ToolSpanInput {
        tool_name: "bash".to_string(),
        status: "success".to_string(),
        duration_ms: Some(42),
        error: None,
    });

    assert_eq!(require_str(&attrs, "gen_ai.tool.name"), "bash");
    assert_eq!(
        require_str(&attrs, "langfuse.observation.type"),
        "tool",
        "tool span 的 langfuse.observation.type 恒为 tool（否则 Langfuse 侧不识别为工具观察）"
    );
    assert_eq!(require_str(&attrs, "agent.tool.status"), "success");
    assert_eq!(require_i64(&attrs, "agent.tool.duration_ms"), 42);
    assert_absent(&attrs, "agent.tool.error");

    assert_no_empty_string_attr(&attrs);
    assert_langfuse_routing_predicate(&attrs, "tool.call");
}

#[test]
fn ot4_tool_span_attributes_carry_error_and_omit_absent_duration() {
    let attrs = otel::tool_span_attributes(&ToolSpanInput {
        tool_name: "bash".to_string(),
        status: "failed".to_string(),
        duration_ms: None,
        error: Some("exit status 1".to_string()),
    });

    // 失败工具仍须可路由、可识别为 tool 观察（B5：不在本轮虚断言 span status 语义）。
    assert_eq!(require_str(&attrs, "gen_ai.tool.name"), "bash");
    assert_eq!(require_str(&attrs, "langfuse.observation.type"), "tool");
    assert_eq!(require_str(&attrs, "agent.tool.status"), "failed");
    assert_eq!(require_str(&attrs, "agent.tool.error"), "exit status 1");
    assert_absent(&attrs, "agent.tool.duration_ms");
    assert_no_empty_string_attr(&attrs);
}

// ================================================================== AttrValue 形状

#[test]
fn attr_value_variants_are_distinct_and_type_carrying() {
    // 四个变体各自承载值，且跨变体不相等 —— 否则「数值属性被落成字符串」也可蒙混通过。
    assert_eq!(AttrValue::Str("s".to_string()), AttrValue::Str("s".to_string()));
    assert_ne!(AttrValue::Str("s".to_string()), AttrValue::Str("t".to_string()));
    assert_eq!(AttrValue::I64(42), AttrValue::I64(42));
    assert_ne!(AttrValue::I64(42), AttrValue::I64(43));
    assert_eq!(AttrValue::F64(1.0), AttrValue::F64(1.0));
    assert_eq!(AttrValue::Bool(true), AttrValue::Bool(true));
    assert_ne!(AttrValue::Bool(true), AttrValue::Bool(false));

    assert_ne!(AttrValue::Str("42".to_string()), AttrValue::I64(42));
    assert_ne!(AttrValue::I64(1), AttrValue::F64(1.0));
    assert_ne!(AttrValue::Bool(true), AttrValue::Str("true".to_string()));
}

// ============================================================ OT-6 手动导出旁路

#[test]
fn ot6_force_export_bypasses_closed_gate_and_keeps_gate_state() {
    let _guard = gate_lock();
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_500));
    otel::init(local_config(port)).expect("D3/A3：本地回环端点必须可装配");

    let traces = vec![
        turn_record("sess-otel-ot6", "turn-otel-ot6-1", "completed"),
        turn_record("sess-otel-ot6", "turn-otel-ot6-2", "failed"),
    ];

    // 签名冻结（A4）：函数指针类型检查，签名漂移即编译失败。
    let seam: fn(&str, &[TurnTraceRecord]) -> Result<usize, String> =
        otel::force_export_session_trace;

    // Act 1：门禁**关闭**下逐次授权导出（旁路）。
    otel::set_reporting_enabled(false);
    assert_eq!(
        seam("sess-otel-ot6", &traces),
        Ok(2),
        "A4/OT-6：手动导出必须返回成功导出的 turn 数（2 条记录 ⇒ Ok(2)）"
    );
    assert!(
        !otel::is_reporting_enabled(),
        "A4：旁路导出不得翻转全局门禁（用后必须复原为 false）"
    );

    // Act 2：门禁**开启**下重复导出，数量契约不变且门禁末态保持 true（副作用检测）。
    otel::set_reporting_enabled(true);
    assert_eq!(
        seam("sess-otel-ot6", &traces),
        Ok(2),
        "OT-6：重复调用数量契约不变（幂等重放，允许产生多条 trace）"
    );
    assert!(
        otel::is_reporting_enabled(),
        "A4：旁路导出不得把已开启的门禁关掉（末态必须等于调用前的 true）"
    );

    otel::shutdown();
    otel::set_reporting_enabled(false);

    // Assert：真的走 OTLP/HTTP 出口，且 wire 级 body 含可验证字面量（P2）。
    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "OT-6：门禁关闭时手动导出仍必须产生 OTLP 出口（旁路失效则零请求）"
    );
    let request = captured.first().expect("上一断言已保证非空");
    assert_eq!(request.method, "POST", "OTLP/HTTP 导出必须为 POST");
    assert!(
        request.path.contains("/v1/traces"),
        "OTLP/HTTP trace 导出路径必须为 /v1/traces，实际 `{}`",
        request.path
    );
    assert!(
        !request.body.starts_with(&[0x1f, 0x8b]),
        "导出 body 不得为 gzip（D1 features 不含 gzip；若改 feature 须先补解压断言，否则本断言失效）"
    );
    let body = String::from_utf8_lossy(&request.body);
    assert!(
        body.contains("sess-otel-ot6"),
        "导出 body 必须含 session id 字面量（protobuf string 字段按 UTF-8 落在 wire 字节里）"
    );
    assert!(
        body.contains("agent.turn"),
        "导出 body 必须含 turn root span 名 agent.turn"
    );
}

// ============================================================ OT-7 退役守卫

/// D6 / A7 待退役符号（11 个 trace API 符号 + 2 个 PonySentry trace 路由残留）。
const RETIRED_TRACE_SYMBOLS: [&str; 13] = [
    "AgentTracePayload",
    "TurnTraceItem",
    "ToolCallTraceItem",
    "EvalStatus",
    "capture_agent_trace",
    "send_agent_trace_direct",
    "maybe_report_turn_trace",
    "build_agent_trace_from_event",
    "build_session_aggregated_trace_payload",
    "build_agent_trace_payloads_from_turns",
    "build_session_trace_payloads_sharded",
    "send_trace_direct_http",
    "/api/v1/traces",
];

/// D6 / A7 保留面符号（8 个）：6 个 ponysentry 导出面 + 2 个宿主 issue 上报调用点。
const KEPT_PONYSENTRY_SYMBOLS: [&str; 6] = [
    "IngestPayload",
    "pub fn capture_error",
    "pub fn capture_payload",
    "pub fn capture_panic",
    "pub fn add_breadcrumb",
    "pub fn install_panic_hook",
];

const KEPT_HOST_CALL_SITES: [&str; 2] = [
    "agent::ponysentry::capture_error(",
    "agent::ponysentry::capture_payload(",
];

/// 读取仓库内源码（相对 `crates/pony-agent-core`）。
fn crate_source(relative_to_crate_root: &str) -> String {
    let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), relative_to_crate_root);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("契约证据源必须可读（{path}）：{err}"))
}

#[test]
fn ot7_retired_ponysentry_trace_symbols_have_zero_hits() {
    let source = crate_source("src/agent/ponysentry/mod.rs");

    // 防假绿：证据源必须是真实、非空且含保留面锚点的文件（否则 0 命中毫无意义）。
    // D6 退役后本文件从 ~18KB 合法收缩到 ~2.2KB（13 个符号整段删除），原 4096 字节阈值
    // 已与新契约矛盾；改为「体积下限 + 两个保留面锚点」双证据，仍能击穿空文件/错路径。
    assert!(
        source.len() > 1_000,
        "证据源疑似空文件或路径失效（len={}），OT-7 的 0 命中断言不可信",
        source.len()
    );
    assert!(
        source.contains("pub fn capture_error") && source.contains("pub fn capture_panic"),
        "证据源缺少保留面锚点（capture_error / capture_panic），OT-7 的 0 命中断言不可信"
    );

    let mut hits: Vec<String> = Vec::new();
    for symbol in RETIRED_TRACE_SYMBOLS {
        if let Some(index) = source.find(symbol) {
            let line = source[..index].matches('\n').count() + 1;
            hits.push(format!("{symbol} @ src/agent/ponysentry/mod.rs:{line}"));
        }
    }

    assert!(
        hits.is_empty(),
        "D6/A7：PonySentry trace 符号必须全部退役（0 命中），实际残留：{hits:#?}"
    );
}

#[test]
fn ot7_issue_reporting_keep_surface_still_exists() {
    let ponysentry = crate_source("src/agent/ponysentry/mod.rs");
    let missing: Vec<&str> = KEPT_PONYSENTRY_SYMBOLS
        .iter()
        .copied()
        .filter(|symbol| !ponysentry.contains(*symbol))
        .collect();
    assert!(
        missing.is_empty(),
        "D6/A7 保留面被过度删除（issue/error 上报必须不受 trace 下线影响）：{missing:#?}"
    );

    let host = crate_source("../../src-tauri/src/lib.rs");
    let missing_call_sites: Vec<&str> = KEPT_HOST_CALL_SITES
        .iter()
        .copied()
        .filter(|site| !host.contains(*site))
        .collect();
    assert!(
        missing_call_sites.is_empty(),
        "D6/A7 宿主 issue 上报调用点被过度删除：{missing_call_sites:#?}"
    );
}

// ---------------------------------------- OT-7 扩扫：全仓机械守卫（Lead 裁决 2026-10-24）
//
// 为什么需要扩扫：残留引用会以编译错误暴露，但字符串/注释/文档级残留不会——只能机械扫描。
// 扫描面：`crates/**`、`src-tauri/src/**`、`src/**`、`tests/**`（仅 .rs/.ts/.tsx/.js/.vue）。
// 排除：`docs/`、`.dev-team/`、`openspec/changes/archive/`（历史归档不追改）、`target*`、
//       `node_modules`、`.git`、`dist`，以及**本守卫文件自身**（符号清单以字面量存放于此，属自指）。
//
// [task-4 DONE 2026-10-24] §5 清单中的 3 个存量测试已按契约处置（删除 2 个 + 重写 1 个），
// 因此本守卫已是**全仓 0 命中强断言**，不再有「已知待清理」豁免表：任何残留一律判失败。

const RETIRED_SCAN_ROOTS: [&str; 4] = ["crates", "src-tauri/src", "src", "tests"];
const RETIRED_SCAN_EXTENSIONS: [&str; 5] = ["rs", "ts", "tsx", "js", "vue"];

/// 目录名命中即整棵跳过（`target*` 前缀另行判断，覆盖 `target-test-exact-a` 等隔离构建目录）。
const RETIRED_SCAN_SKIP_DIRS: [&str; 7] = [
    "node_modules",
    ".git",
    "dist",
    "docs",
    ".dev-team",
    "archive",
    "openspec",
];

/// 自指处理：本文件以字面量保存 `RETIRED_TRACE_SYMBOLS`，故**只掩码该常量的定义块**，
/// 文件其余部分照常扫描——禁止整文件排除，否则会漏掉守卫自身其他位置的残留。
const GUARD_FILE_RELATIVE_PATH: &str = "crates/pony-agent-core/tests/otel_trace_migration_test.rs";

/// 符号清单常量定义块的起始标记；改名即触发守卫完整性失败，避免掩码静默失效。
const RETIRED_SYMBOL_LIST_MARKER: &str = "const RETIRED_TRACE_SYMBOLS";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn collect_scannable_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("OT-7 扩扫目录必须可读（{}）：{err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("OT-7 扩扫目录项必须可读：{err}"));
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name.starts_with("target") || RETIRED_SCAN_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect_scannable_files(&path, files);
            continue;
        }
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if RETIRED_SCAN_EXTENSIONS.contains(&extension) {
            files.push(path);
        }
    }
}

fn repo_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 把 `source[start..end]` 挖成空格（逐字节等长，保留换行 ⇒ 行号不漂移）。
fn mask_span(source: &str, start: usize, end: usize) -> String {
    let mut masked = String::with_capacity(source.len());
    masked.push_str(&source[..start]);
    for character in source[start..end].chars() {
        if character == '\n' {
            masked.push('\n');
        } else {
            for _ in 0..character.len_utf8() {
                masked.push(' ');
            }
        }
    }
    masked.push_str(&source[end..]);
    masked
}

/// 只掩码 `RETIRED_TRACE_SYMBOLS` 常量定义块，返回（掩码后源码，被掩码块本身）。
fn mask_retired_symbol_list(source: &str) -> (String, String) {
    let marker = source.find(RETIRED_SYMBOL_LIST_MARKER).unwrap_or_else(|| {
        panic!("守卫完整性：未找到 `{RETIRED_SYMBOL_LIST_MARKER}` 定义块，自指掩码会失效")
    });
    let start = source[..marker]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = source[marker..]
        .find("];")
        .map(|index| marker + index + 2)
        .unwrap_or_else(|| panic!("守卫完整性：`RETIRED_TRACE_SYMBOLS` 定义块未闭合（缺 `];`）"));
    (mask_span(source, start, end), source[start..end].to_string())
}

#[test]
fn ot7_retired_symbols_have_zero_hits_repo_wide() {
    let root = repo_root();

    // 防假绿：四个扫描根必须真实存在，且扫描面不能塌缩。
    for scan_root in RETIRED_SCAN_ROOTS {
        assert!(
            root.join(scan_root).exists(),
            "OT-7 扩扫根不存在：`{scan_root}`（全仓守卫不可信）"
        );
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for scan_root in RETIRED_SCAN_ROOTS {
        collect_scannable_files(&root.join(scan_root), &mut files);
    }
    assert!(
        files.len() > 150,
        "OT-7 扩扫面过小（{} 个源文件），全仓守卫不可信",
        files.len()
    );

    let mut violations: Vec<String> = Vec::new();
    let mut guard_file_scanned = false;

    for file in &files {
        let relative = repo_relative(&root, file);
        let mut source = std::fs::read_to_string(file)
            .unwrap_or_else(|err| panic!("OT-7 扩扫面文件必须可读（{relative}）：{err}"));

        if relative == GUARD_FILE_RELATIVE_PATH {
            // 自指掩码只覆盖符号清单定义块；文件其余内容仍在扫描面内。
            let (masked, block) = mask_retired_symbol_list(&source);
            for symbol in RETIRED_TRACE_SYMBOLS {
                assert!(
                    block.contains(symbol),
                    "守卫完整性：符号 `{symbol}` 不在 RETIRED_TRACE_SYMBOLS 定义块内，掩码会漏扫"
                );
            }
            assert_eq!(
                masked.len(),
                source.len(),
                "自指掩码必须逐字节等长（行号与证据不可漂移）"
            );
            source = masked;
            guard_file_scanned = true;
        }

        for symbol in RETIRED_TRACE_SYMBOLS {
            if let Some(index) = source.find(symbol) {
                let line = source[..index].matches('\n').count() + 1;
                violations.push(format!("{symbol} @ {relative}:{line}"));
            }
        }
    }

    assert!(
        guard_file_scanned,
        "守卫文件必须处于扫描面内（只掩码符号清单常量块，禁止整文件排除）"
    );

    assert!(
        violations.is_empty(),
        "OT-7 全仓守卫：D6 退役符号必须 0 命中，实际残留 {} 条：{violations:#?}",
        violations.len()
    );
}

// ================================================================== OT-8 设置持久化

fn unique_temp_settings_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "pony-wave43-otel-{}-{}-{}",
        std::process::id(),
        tag,
        nanos
    ))
}

/// 谁创建谁清理：临时目录存在才删除，删除失败必须显式失败（不静默污染环境）。
fn cleanup_dir(dir: &Path) {
    if dir.exists() {
        std::fs::remove_dir_all(dir).expect("测试临时目录必须可清理（谁创建谁清理）");
    }
}

#[test]
fn ot8_settings_trace_flag_defaults_false() {
    let dir = unique_temp_settings_dir("ot8a");
    let store = AppSettingsStore::with_path(dir.join("settings.json"));

    let loaded = store.load_view();

    assert!(
        !loaded.agent_trace_reporting,
        "OT-8a：settings.json 不存在时 agent_trace_reporting 必须缺省 false"
    );
    assert_eq!(loaded.workspace_mode, WorkspaceMode::Coding);
    cleanup_dir(&dir);
}

#[test]
fn ot8_settings_trace_flag_roundtrips_with_camel_case_key() {
    let dir = unique_temp_settings_dir("ot8b");
    let path = dir.join("settings.json");
    let store = AppSettingsStore::with_path(path.clone());

    let saved = store
        .save_view(AppSettings {
            agent_trace_reporting: true,
            ..Default::default()
        })
        .expect("OT-8b：save_view 必须成功");

    assert!(saved.agent_trace_reporting, "save_view 回显必须反映写入值");
    assert!(
        store.load_view().agent_trace_reporting,
        "OT-8b：写入 true 后 load_view 必须读回 true（From 双向同步）"
    );

    let raw = std::fs::read_to_string(&path).expect("settings.json 必须可读");
    let json: serde_json::Value =
        serde_json::from_str(&raw).expect("settings.json 必须是合法 JSON");
    assert_eq!(
        json.get("agentTraceReporting"),
        Some(&serde_json::Value::Bool(true)),
        "A6 冻结落盘键必须为 camelCase `agentTraceReporting`，实际内容：{raw}"
    );
    assert!(
        json.get("agent_trace_reporting").is_none(),
        "不得写出 snake_case 落盘键，实际内容：{raw}"
    );

    cleanup_dir(&dir);
}

#[test]
fn ot8_legacy_settings_without_trace_flag_reads_false() {
    let dir = unique_temp_settings_dir("ot8c");
    std::fs::create_dir_all(&dir).expect("临时设置目录必须可创建");
    let path = dir.join("settings.json");
    std::fs::write(&path, r#"{"workspaceMode":"work","toolRules":[]}"#)
        .expect("legacy settings.json 必须可写");

    let loaded = AppSettingsStore::with_path(path).load_view();

    assert_eq!(
        loaded.workspace_mode,
        WorkspaceMode::Work,
        "OT-8c：legacy 文件必须仍能解析出既有字段"
    );
    assert!(
        !loaded.agent_trace_reporting,
        "OT-8c：legacy settings.json 无该键时必须回落 false（serde default 容错）"
    );
    cleanup_dir(&dir);
}

// ------------------------------------------- OT-8d/8e 宿主桥接（P4 静态断言）

const TAURI_SRC_RELATIVE: &str = "src-tauri/src";
const CORE_AGENT_SRC_RELATIVE: &str = "crates/pony-agent-core/src/agent";

fn read_repo_source(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("OT-8 静态断言目标源码必须可读（{relative}）：{err}"))
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("OT-8 静态断言目录必须可读（{}）：{err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("OT-8 静态断言目录项必须可读：{err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

/// 从函数签名处按花括号配对截取函数体（含签名）。花括号不闭合直接失败，禁止静默截断。
fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("OT-8 静态断言：未找到 `{signature}`"));
    let open = start
        + source[start..]
            .find('{')
            .unwrap_or_else(|| panic!("OT-8 静态断言：`{signature}` 缺少函数体左花括号"));
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &source[open..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("OT-8 静态断言：`{signature}` 的花括号未闭合，断言不可信");
}

/// 在 `crates/pony-agent-core/src/agent/**` 内定位 core 公共 API 定义（模块位置不冻结）。
fn find_core_gate_api() -> (String, String) {
    let root = repo_root();
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rust_files(&root.join(CORE_AGENT_SRC_RELATIVE), &mut files);
    assert!(
        files.len() > 20,
        "OT-8d 核心源码扫描面过小（{} 个 .rs），静态断言不可信",
        files.len()
    );

    let signature = "pub fn set_agent_trace_reporting(";
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("OT-8d 扫描面文件必须可读（{}）：{err}", file.display()));
        if source.contains(signature) {
            return (repo_relative(&root, &file), source);
        }
    }
    panic!("OT-8d：core 公共 API `set_agent_trace_reporting(bool)` 未定义（D7 契约缺失）")
}

#[test]
fn ot8_settings_bridge_syncs_core_gate_on_load_and_save() {
    // 1) core 公共 API 存在，且真正桥接到 OTel 门禁（不是空实现）。
    let (api_file, api_source) = find_core_gate_api();
    let api_body = function_body(&api_source, "pub fn set_agent_trace_reporting(");
    assert!(
        api_body.contains("set_reporting_enabled("),
        "OT-8d：`{api_file}` 的 set_agent_trace_reporting 必须桥接到 otel 门禁，实际函数体：{api_body}"
    );

    // 2) 启动加载路径必须把持久化值同步给 core 门禁。
    let lib = read_repo_source("src-tauri/src/lib.rs");
    let load_body = function_body(&lib, "fn load_app_settings");
    assert!(
        load_body.contains("set_agent_trace_reporting("),
        "OT-8d：load_app_settings 必须调用 set_agent_trace_reporting（启动即按持久化值生效），实际函数体：{load_body}"
    );
    assert!(
        load_body.contains("agent_trace_reporting"),
        "OT-8d：load_app_settings 必须传入读回的持久化字段，不得硬编码常量，实际函数体：{load_body}"
    );

    // 3) 保存路径同样必须同步（否则开关开启后需重启才生效）。
    let save_body = function_body(&lib, "fn save_app_settings");
    assert!(
        save_body.contains("set_agent_trace_reporting("),
        "OT-8d：save_app_settings 必须调用 set_agent_trace_reporting（保存即生效），实际函数体：{save_body}"
    );
    assert!(
        save_body.contains("agent_trace_reporting"),
        "OT-8d：save_app_settings 必须传入落盘后的持久化字段，不得硬编码常量，实际函数体：{save_body}"
    );
}

#[test]
fn ot8_exit_path_releases_otel_provider() {
    let root = repo_root();
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rust_files(&root.join(TAURI_SRC_RELATIVE), &mut files);
    assert!(
        files.len() >= 5,
        "OT-8e 宿主源码扫描面过小（{} 个 .rs），静态断言不可信",
        files.len()
    );
    for anchor in ["lib.rs", "main.rs"] {
        assert!(
            files.iter().any(|file| file.file_name().and_then(|name| name.to_str()) == Some(anchor)),
            "OT-8e 扫描面必须包含宿主锚点文件 `{anchor}`（否则退出路径断言不可信）"
        );
    }

    let mut init_sites: Vec<String> = Vec::new();
    let mut shutdown_sites: Vec<String> = Vec::new();

    for file in files {
        let relative = repo_relative(&root, &file);
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("OT-8e 扫描面文件必须可读（{relative}）：{err}"));

        if source.contains("otel::init(") {
            init_sites.push(relative.clone());
        }

        // 退出路径：`RunEvent::Exit` 或 `RunEvent::ExitRequested` 分支内必须出现 shutdown。
        let exit_at = source
            .find("RunEvent::Exit")
            .map(|index| (index, "RunEvent::Exit"));
        let Some((exit_index, exit_marker)) = exit_at else {
            continue;
        };
        let search_from = exit_index + exit_marker.len();
        let shutdown_at = source[search_from..]
            .find("otel::shutdown()")
            .map(|offset| search_from + offset);
        let Some(shutdown_index) = shutdown_at else {
            continue;
        };

        let between = &source[search_from..shutdown_index];
        assert!(
            !between.contains("RunEvent::") && !between.contains("ExitRequested =>"),
            "OT-8e：`{relative}` 的 otel::shutdown() 必须落在同一个 Exit 分支内，不得跨分支放置"
        );
        shutdown_sites.push(relative);
    }

    assert!(
        !init_sites.is_empty(),
        "OT-8e：src-tauri/src 必须存在 otel::init( 启动装配点（provider 生命周期起点）"
    );
    assert!(
        !shutdown_sites.is_empty(),
        "OT-8e：src-tauri/src 必须在 RunEvent::Exit / ExitRequested 分支内调用 otel::shutdown()（NFR resource_release）"
    );
}
