//! ACP P0 蓝军验收契约（B149-P0-T，冻结于 task-2）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2 / §4 P0 表 / §6 风险；
//! 契约矩阵：`.dev-team/contract-matrix-acp-p0.md`（A1~A9）。
//!
//! 本文件只读 SUT 的 P0 冻结面（`acp::{protocol, transport, config}`）：
//! - 蓝军断言：协议 round-trip（A1/A5/A6）、initialize 往返、入站请求分发（A4）、
//!   入站通知不串扰（A3）、stderr 环形缓冲有界、相对 cwd 拒绝、config 内置回退。
//! - 阈值唯一真源：`.dev-team/nfr-baseline.json` 的 `acp` 段（禁在本文件自选数值）。
//! - 对抗注入（超时/畸形/并发）在 `acp_p0_adversarial.rs`。
//!
//! 红相语义：transport/config 的函数体若仍为 `unimplemented!()`，对应用例必须真实 panic；
//! 协议类型 serde 契约（已冻结的 derive）属绿基线，不得被改写。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{
    agents_file_path, builtin_agent_specs, load_agents, resolve_agent, save_agents, AcpAgentSpec,
    AcpCapabilitiesHint, AcpConfigError, BUILTIN_AGENTS,
};
use pony_agent_core::acp::protocol::method;
use pony_agent_core::acp::protocol::{
    AuthMethodType, ClientCapabilities, ContentBlock, Implementation, InitializeRequest,
    InitializeResponse, PermissionOption, PermissionOptionKind, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SessionNewRequest,
    SessionNewResponse, SessionUpdate, StopReason, ToolCallStatus, ToolKind, PROTOCOL_VERSION,
};
use pony_agent_core::acp::transport::{
    AcpInboundRequest, AcpInboundRequestHandler, AcpInboundResponse, AcpNotificationHandler,
    AcpTransport, AcpTransportError, MALFORMED_LINE_STREAK_LIMIT, ACP_REQUEST_TIMEOUT,
    SPAWN_REAP_GRACE, STDERR_TAIL_LINES,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

// ───────────────────────────── 通用设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// 独立的临时工作目录（绝对路径 + canonicalize，符合 transport 对绝对 cwd 的要求）。
fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-p0-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// NFR 唯一真源：`.dev-team/nfr-baseline.json`（禁在测试里自选阈值）。
fn nfr_json() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.dev-team/nfr-baseline.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 NFR 真源失败 {}: {error}", path.display()));
    serde_json::from_str(&body).expect("nfr-baseline.json 必须是合法 JSON")
}

fn nfr_acp_ms(key: &str) -> u64 {
    nfr_json()
        .get("acp")
        .and_then(|acp| acp.get(key))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 acp.{key}（task-1 NFR 段未就绪）"))
}

/// fake ACP agent（POSIX sh，newline-delimited JSON-RPC）。
fn fake_spec(mode: &str, arg: Option<&str>) -> AcpAgentSpec {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    let mut args = vec![script.to_string_lossy().into_owned(), mode.to_string()];
    if let Some(extra) = arg {
        args.push(extra.to_string());
    }
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
    }
}

fn initialize_params() -> Value {
    serde_json::to_value(InitializeRequest {
        protocol_version: PROTOCOL_VERSION,
        client_capabilities: ClientCapabilities::default(),
        client_info: Some(Implementation {
            name: "pony-agent-tests".to_string(),
            title: None,
            version: "0.0.0".to_string(),
        }),
    })
    .expect("initialize 请求必须可序列化")
}

/// 有界条件轮询（非固定 sleep 作为同步手段：到达条件即返回，超时才算失败）。
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

/// 官方 v1 样例 JSON 的 typed round-trip：反序列化 → 再序列化 → 与原样例值等价。
fn assert_roundtrip<T>(sample: Value) where T: DeserializeOwned + Serialize {
    let typed: T = serde_json::from_value(sample.clone())
        .unwrap_or_else(|error| panic!("反序列化失败（样例与冻结字段名不符）: {error}\n样例: {sample}"));
    let reserialized = serde_json::to_value(&typed).expect("再序列化失败");
    assert_eq!(
        reserialized, sample,
        "A5/A6 违约：round-trip 不等价（字段名/camelCase/枚举值/skip 语义被破坏）"
    );
}

// ───────────────────────────── 协议契约（A1/A5/A6） ─────────────────────────────

/// A1：`protocolVersion` 是**整数** 1（不是 "1"、不是 "0.1"），且请求/响应两侧一致。
#[test]
fn contract_a1_protocol_version_is_integer_one() {
    let value = initialize_params();
    assert!(
        value["protocolVersion"].is_number(),
        "A1 违约：protocolVersion 必须是 JSON number，实际 {:?}",
        value["protocolVersion"]
    );
    assert_eq!(
        value["protocolVersion"].as_u64(),
        Some(1),
        "A1 违约：protocolVersion 必须等于整数 1"
    );
    assert_eq!(PROTOCOL_VERSION, 1, "A1：PROTOCOL_VERSION 冻结为 u32 = 1");

    let response = json!({
        "protocolVersion": 1,
        "agentCapabilities": {"loadSession": true, "promptCapabilities": {"image": false, "audio": false, "embeddedContext": false}, "mcpCapabilities": {"http": false, "sse": false}, "sessionCapabilities": {}},
        "authMethods": [{"id": "login", "name": "Login", "type": "terminal"}]
    });
    assert_roundtrip::<InitializeResponse>(response);
    assert_roundtrip::<InitializeRequest>(value);
}

/// A5：结构体字段 camelCase（`mimeType` / `toolCallId` / `optionId` / `sessionId` …）。
#[test]
fn contract_a5_camel_case_fields_roundtrip() {
    // ContentBlock 5 变体（判别键 type，变体内字段 camelCase）
    for block in [
        json!({"type": "text", "text": "hi"}),
        json!({"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png", "uri": "file:///tmp/a.png"}),
        json!({"type": "audio", "data": "UklGRg==", "mimeType": "audio/wav"}),
        json!({
            "type": "resource_link",
            "name": "notes.md",
            "uri": "file:///tmp/notes.md",
            "mimeType": "text/markdown",
            "size": 42,
            "title": "Notes",
            "description": "a note"
        }),
        json!({
            "type": "resource",
            "resource": {"uri": "file:///tmp/x.txt", "text": "body", "mimeType": "text/plain"}
        }),
    ] {
        assert_roundtrip::<ContentBlock>(block);
    }

    // session/new 与 session/prompt 的 camelCase
    assert_roundtrip::<SessionNewRequest>(json!({
        "cwd": "/tmp/workspace",
        "mcpServers": [],
        "additionalDirectories": ["/tmp/extra"]
    }));
    assert_roundtrip::<SessionNewResponse>(json!({"sessionId": "sess-1"}));

    // 权限请求：`toolCallId` / `optionId` + 选项 kind snake_case
    assert_roundtrip::<RequestPermissionRequest>(json!({
        "sessionId": "sess-1",
        "toolCall": {
            "toolCallId": "tc-1",
            "title": "edit a.rs",
            "kind": "edit",
            "status": "pending",
            "content": [{"type": "content", "content": {"type": "text", "text": "x"}}],
            "locations": [{"path": "a.rs", "line": 3}],
            "rawInput": {"path": "a.rs"}
        },
        "options": [
            {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
            {"optionId": "reject-always", "name": "Reject always", "kind": "reject_always"}
        ]
    }));

    // 权限应答两种形态（判别键 outcome）
    assert_roundtrip::<RequestPermissionResponse>(json!({
        "outcome": {"outcome": "selected", "optionId": "allow-once"}
    }));
    assert_roundtrip::<RequestPermissionResponse>(json!({"outcome": {"outcome": "cancelled"}}));
}

/// A3/A6：8 个 `sessionUpdate` 变体与 5 个 `stopReason` 逐一 round-trip，枚举值必须 snake_case。
#[test]
fn contract_a3_a6_session_update_and_stop_reason_variants() {
    for update in [
        json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "u"}, "messageId": "m-1"}),
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "a"}, "messageId": "m-2"}),
        json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "think"}}),
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tc-1",
            "title": "run tests",
            "kind": "execute",
            "status": "in_progress",
            "content": [{"type": "diff", "path": "a.rs", "newText": "new", "oldText": "old"}],
            "locations": [{"path": "a.rs", "line": 7}],
            "rawInput": {"cmd": "cargo test"}
        }),
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "tc-1",
            "status": "completed",
            "content": [{"type": "terminal", "terminalId": "term-1"}]
        }),
        json!({"sessionUpdate": "plan", "entries": [{"content": "step", "priority": "high", "status": "pending"}]}),
        json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "/init", "description": "init", "input": {"hint": "path"}}]}),
        json!({"sessionUpdate": "current_mode_update", "currentModeId": "code"}),
    ] {
        assert_roundtrip::<SessionUpdate>(update);
    }

    for reason in [
        "end_turn",
        "max_tokens",
        "max_turn_requests",
        "refusal",
        "cancelled",
    ] {
        assert_roundtrip::<PromptResponse>(json!({"stopReason": reason}));
    }

    // A6：枚举值必须 snake_case，不得出现驼峰/别名
    assert_eq!(serde_json::to_value(StopReason::MaxTurnRequests).unwrap(), json!("max_turn_requests"));
    assert_eq!(serde_json::to_value(ToolKind::SwitchMode).unwrap(), json!("switch_mode"));
    assert_eq!(serde_json::to_value(ToolKind::Other).unwrap(), json!("other"));
    assert_eq!(serde_json::to_value(ToolCallStatus::InProgress).unwrap(), json!("in_progress"));
    for (kind, expected) in [
        (PermissionOptionKind::AllowOnce, "allow_once"),
        (PermissionOptionKind::AllowAlways, "allow_always"),
        (PermissionOptionKind::RejectOnce, "reject_once"),
        (PermissionOptionKind::RejectAlways, "reject_always"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(expected));
    }
    assert_eq!(serde_json::to_value(AuthMethodType::Terminal).unwrap(), json!("terminal"));
    assert_eq!(
        serde_json::to_value(PermissionOption {
            option_id: "allow-once".to_string(),
            name: "Allow".to_string(),
            kind: PermissionOptionKind::AllowOnce,
        })
        .unwrap(),
        json!({"optionId": "allow-once", "name": "Allow", "kind": "allow_once"})
    );
    assert_eq!(
        serde_json::to_value(RequestPermissionOutcome::Cancelled).unwrap(),
        json!({"outcome": "cancelled"})
    );
}

// ───────────────────────────── NFR 常量对齐（A7/A8/A9 的数值面） ─────────────────────────────

/// 阈值唯一真源校验：代码常量 == `.dev-team/nfr-baseline.json` 的 `acp` 段。
#[test]
fn contract_nfr_constants_match_baseline() {
    assert_eq!(
        ACP_REQUEST_TIMEOUT,
        Duration::from_millis(nfr_acp_ms("request_timeout_ms")),
        "A7：ACP_REQUEST_TIMEOUT 必须等于 nfr.acp.request_timeout_ms"
    );
    assert_eq!(
        MALFORMED_LINE_STREAK_LIMIT as u64,
        nfr_acp_ms("malformed_line_streak_limit"),
        "A8：MALFORMED_LINE_STREAK_LIMIT 必须等于 nfr.acp.malformed_line_streak_limit"
    );
    assert_eq!(
        SPAWN_REAP_GRACE,
        Duration::from_millis(nfr_acp_ms("spawn_reap_grace_ms")),
        "A9：SPAWN_REAP_GRACE 必须等于 nfr.acp.spawn_reap_grace_ms"
    );
    assert_eq!(STDERR_TAIL_LINES, 32, "stderr 环形缓冲上限冻结为 32 行");
}

// ───────────────────────────── config 契约（内置回退 / 未知字段 / 错误码） ─────────────────────────────

/// 内置两条默认 agent 是**数据契约**（opencode → `opencode acp`、antigravity → `agy acp`）。
#[test]
fn contract_config_builtin_agents_data() {
    assert_eq!(BUILTIN_AGENTS.len(), 2, "内置 agent 必须恰好两条");
    assert_eq!(BUILTIN_AGENTS[0].id, "opencode");
    assert_eq!(BUILTIN_AGENTS[0].command, "opencode");
    assert_eq!(BUILTIN_AGENTS[0].args, &["acp"]);
    assert_eq!(BUILTIN_AGENTS[1].id, "antigravity");
    assert_eq!(BUILTIN_AGENTS[1].command, "agy");
    assert_eq!(BUILTIN_AGENTS[1].args, &["acp"]);

    let specs = builtin_agent_specs();
    assert_eq!(specs.len(), 2, "builtin_agent_specs 必须物化两条默认");
    assert_eq!(specs[0].id, "opencode");
    assert_eq!(specs[0].command, "opencode");
    assert_eq!(specs[0].args, vec!["acp".to_string()]);
    assert_eq!(specs[1].id, "antigravity");
    assert_eq!(specs[1].command, "agy");
}

/// 配置路径绝对且落在独立文件（与 providers.json 同目录、不同文件）。
#[test]
fn contract_config_agents_file_path() {
    let path = agents_file_path();
    assert!(path.is_absolute(), "acp-agents.json 路径必须是绝对路径: {}", path.display());
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("acp-agents.json")
    );
    assert!(
        path.to_string_lossy().contains("pony-agent"),
        "配置目录必须位于 pony-agent 下: {}",
        path.display()
    );
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 在隔离的用户配置目录下执行（XDG_CONFIG_HOME/HOME 指向临时空目录），结束后恢复。
fn with_isolated_config_dir<F: FnOnce(&Path)>(tag: &str, body: F) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_dir(tag);
    let old_xdg = std::env::var_os("XDG_CONFIG_HOME");
    let old_home = std::env::var_os("HOME");
    std::env::set_var("XDG_CONFIG_HOME", &root);
    std::env::set_var("HOME", &root);
    body(&root);
    match old_xdg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    match old_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    cleanup(&root);
}

fn sample_spec(id: &str, command: &str) -> AcpAgentSpec {
    AcpAgentSpec {
        id: id.to_string(),
        label: format!("{id} label"),
        command: command.to_string(),
        args: vec!["acp".to_string()],
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint { fs: false, terminal: false },
    }
}

/// 缺文件时回落到内置两条默认；未知 id 报 `NotFound`（不得 panic、不得静默返回空）。
#[test]
fn contract_config_missing_file_falls_back_to_builtin() {
    with_isolated_config_dir("config-fallback", |config_home| {
        let expected_file = config_home.join("pony-agent").join("acp-agents.json");
        assert!(!expected_file.exists(), "隔离目录初始必须无 acp-agents.json");
        assert_eq!(agents_file_path(), expected_file, "配置路径必须落在隔离的 XDG 配置目录");

        let agents = load_agents().expect("缺文件时必须回落到内置默认，而不是报错");
        assert_eq!(agents.len(), 2, "缺文件回落后必须恰好两条内置默认");
        assert!(agents.iter().any(|spec| spec.id == "opencode"));
        assert!(agents.iter().any(|spec| spec.id == "antigravity"));

        let opencode = resolve_agent("opencode").expect("内置 id 必须可解析");
        assert_eq!(opencode.command, "opencode");
        assert_eq!(opencode.args, vec!["acp".to_string()]);
        let antigravity = resolve_agent("antigravity").expect("内置 id 必须可解析");
        assert_eq!(antigravity.command, "agy");

        let unknown = resolve_agent("no-such-agent");
        match unknown {
            Ok(spec) => panic!("未知 agent 不得被静默解析: {spec:?}"),
            Err(error) => {
                assert!(
                    matches!(error, AcpConfigError::NotFound(_)),
                    "未知 agent 必须返回 NotFound，实际 {error:?}"
                );
                assert_eq!(error.code(), "acp_config_agent_not_found");
            }
        }
    });
}

/// save → load 往返等价；文件里多出的未知字段必须被忽略（不得让加载失败）。
#[test]
fn contract_config_roundtrip_and_unknown_fields_tolerated() {
    with_isolated_config_dir("config-roundtrip", |_config_home| {
        let specs = vec![sample_spec("opencode", "opencode"), sample_spec("my-agent", "/usr/bin/my-agent")];
        save_agents(&specs).expect("save_agents 必须成功写入");

        let path = agents_file_path();
        assert!(path.is_file(), "save_agents 后清单必须存在: {}", path.display());

        let loaded = load_agents().expect("save 后 load 必须成功");
        assert_eq!(loaded, specs, "save → load 必须无损往返");

        // 未知字段容错：在首个对象里注入一个官方 schema 之外的字段
        let raw = std::fs::read_to_string(&path).expect("读取已写入清单");
        let injected = raw.replacen("\"label\"", "\"unknownFutureField\":123,\"label\"", 1);
        assert_ne!(injected, raw, "夹具注入失败：清单里应有 label 字段");
        std::fs::write(&path, injected).expect("写回注入后的清单");

        let reloaded = load_agents().expect("未知字段不得导致加载失败");
        assert_eq!(reloaded, specs, "未知字段必须被忽略而非破坏已识别字段");
    });
}

/// `default_cwd` 非绝对路径必须在写入时被拒（fail-closed，`Invalid`）。
#[test]
fn contract_config_relative_default_cwd_rejected() {
    with_isolated_config_dir("config-relative-cwd", |_config_home| {
        let mut spec = sample_spec("bad-agent", "bad");
        spec.default_cwd = Some("relative/workspace".to_string());
        let result = save_agents(&[spec]);
        match result {
            Ok(()) => panic!("相对 default_cwd 必须被拒（fail-closed）"),
            Err(error) => {
                assert!(
                    matches!(error, AcpConfigError::Invalid(_)),
                    "相对 default_cwd 必须报 Invalid，实际 {error:?}"
                );
                assert_eq!(error.code(), "acp_config_invalid");
            }
        }
    });
}

/// 配置错误码是 4 个稳定字符串（§3.2.9 的 `acp_*` 前缀约束）。
#[test]
fn contract_config_error_code_mapping() {
    let cases = [
        (AcpConfigError::Io("x".to_string()), "acp_config_io"),
        (AcpConfigError::Parse("x".to_string()), "acp_config_parse"),
        (AcpConfigError::Invalid("x".to_string()), "acp_config_invalid"),
        (AcpConfigError::NotFound("x".to_string()), "acp_config_agent_not_found"),
    ];
    for (error, expected) in cases {
        assert_eq!(error.code(), expected, "配置错误码映射违约: {error:?}");
    }
}

// ───────────────────────────── transport 契约（A2/A3/A4 + 进程面） ─────────────────────────────

/// 入站请求 handler：记录调用次数，并把 `session/request_permission` 应答为 `selected`。
struct PermissionHandler {
    calls: Arc<AtomicUsize>,
}

impl AcpInboundRequestHandler for PermissionHandler {
    fn handle(&self, request: AcpInboundRequest) -> AcpInboundResponse {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            request.method,
            method::SESSION_REQUEST_PERMISSION,
            "handler 只应收到入站请求（agent→client）"
        );
        let parsed: RequestPermissionRequest = serde_json::from_value(request.params.clone())
            .expect("入站 session/request_permission params 必须符合 v1 schema");
        assert_eq!(parsed.session_id, "s-1");
        assert_eq!(parsed.tool_call.tool_call_id, "tc-1");
        assert_eq!(parsed.options.len(), 2);
        let option_id = parsed.options[0].option_id.clone();
        AcpInboundResponse::Result(
            serde_json::to_value(RequestPermissionResponse {
                outcome: RequestPermissionOutcome::Selected { option_id },
            })
            .expect("权限应答必须可序列化"),
        )
    }
}

/// 出站请求往返：initialize 拿到 result，且 id 关联正确（fake agent 用同一 id 回包）。
#[test]
fn contract_transport_initialize_roundtrip() {
    let cwd = temp_dir("transport-init");
    let spec = fake_spec("ok", None);
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };
    assert!(transport.pid() > 0, "spawn 后必须存在真实子进程 pid");

    let result = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("initialize 往返必须成功（id 关联正确）");
    let parsed: InitializeResponse =
        serde_json::from_value(result).expect("initialize result 必须符合 v1 InitializeResponse");
    assert_eq!(parsed.protocol_version, 1, "agent 回报的 protocolVersion 必须是 1");

    transport.close();
    cleanup(&cwd);
}

/// 入站请求（有 id + method）必须分发给 handler，并把 handler 的应答写回 agent。
#[test]
fn contract_transport_inbound_request_dispatched_to_handler() {
    let cwd = temp_dir("transport-inbound");
    let spec = fake_spec("permission", None);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };
    transport.set_handler(Arc::new(PermissionHandler {
        calls: Arc::clone(&calls),
    }));

    let result = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("入站请求被正确应答后，initialize 应能完成");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "A4：session/request_permission 必须恰好分发一次到 handler"
    );
    assert_eq!(
        result["recordedOutcome"].as_str(),
        Some("selected"),
        "handler 的 selected 应答必须原样写回 agent，实际 {result}"
    );

    transport.close();
    cleanup(&cwd);
}

/// A4 / §6 风险 1：未注入 handler 时权限请求必须 fail-closed（cancelled 或 error），**绝不** selected。
#[test]
fn contract_transport_permission_without_handler_is_fail_closed() {
    let cwd = temp_dir("transport-no-handler");
    let spec = fake_spec("permission", None);
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };
    // 刻意不注入 handler
    let result = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("无 handler 时应 fail-closed 应答，而不是挂死到 deadline");
    let recorded = result["recordedOutcome"].as_str().unwrap_or("<missing>");
    assert!(
        recorded == "cancelled" || recorded == "error",
        "A4 违约：无 handler 时必须回 cancelled 或 JSON-RPC error，实际 {recorded}"
    );
    assert_ne!(recorded, "selected", "A4 硬约束：无 handler 时绝不能默认 allow");

    transport.close();
    cleanup(&cwd);
}

/// A3/§3 三路分流：无 id 的 `session/update` 是通知，既不当响应也不当请求（handler 不得被调用）。
#[test]
fn contract_transport_notification_not_dispatched_as_request() {
    let cwd = temp_dir("transport-notification");
    let spec = fake_spec("notification", None);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };
    transport.set_handler(Arc::new(PermissionHandler {
        calls: Arc::clone(&calls),
    }));

    let result = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("通知与响应交错不得破坏 id 配对");
    let parsed: InitializeResponse =
        serde_json::from_value(result).expect("initialize result 必须符合 v1 schema");
    assert_eq!(parsed.protocol_version, 1);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "通知帧（无 id）绝不能被当作入站请求分发给 handler"
    );

    transport.close();
    cleanup(&cwd);
}

/// 入站通知 handler：用有界 mpsc 收集 `(method, params)`，供断言「恰好一次 + 载荷正确」。
struct CollectingNotificationHandler {
    sender: Mutex<mpsc::Sender<(String, Value)>>,
}

impl AcpNotificationHandler for CollectingNotificationHandler {
    fn handle_notification(&self, method: &str, params: Value) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 通知不应答、只观测：接收端已关闭时不得 panic（不得拖垮 reader 线程）
        let _ = sender.send((method.to_string(), params));
    }
}

fn drain_notifications(
    receiver: &mpsc::Receiver<(String, Value)>,
    sink: &mut Vec<(String, Value)>,
) {
    while let Ok(item) = receiver.try_recv() {
        sink.push(item);
    }
}

/// A10：agent 主动发的**无 id** `session/update` 必须分发到注入的 `AcpNotificationHandler`，
/// 恰好一次，且 `method` / `params.sessionId` / `update.sessionUpdate` 载荷正确。
#[test]
fn contract_a10_notification_handler_received_exactly_once() {
    let cwd = temp_dir("notification-handler");
    let spec = fake_spec("notification", None);
    let (sender, receiver) = mpsc::channel();
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };
    // 通知 handler 以 &self 注入（可热替换），与请求 handler 的 &mut 分离
    transport.set_notification_handler(Arc::new(CollectingNotificationHandler {
        sender: Mutex::new(sender),
    }));

    let result = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("注入通知 handler 不得影响 initialize 往返");
    let parsed: InitializeResponse =
        serde_json::from_value(result).expect("initialize result 必须符合 v1 schema");
    assert_eq!(parsed.protocol_version, 1);

    let mut collected: Vec<(String, Value)> = Vec::new();
    let arrived = wait_until(
        || {
            drain_notifications(&receiver, &mut collected);
            !collected.is_empty()
        },
        SPAWN_REAP_GRACE * 4,
    );
    assert!(
        arrived,
        "A10 违约：注入的通知 handler 未收到 session/update（通知被丢弃）"
    );
    assert_eq!(
        collected.len(),
        1,
        "A10 违约：同一条通知必须恰好分发一次，实际 {} 次: {collected:?}",
        collected.len()
    );

    let (notification_method, params) = &collected[0];
    assert_eq!(
        notification_method,
        "session/update",
        "A10：通知 method 必须原样上送"
    );
    assert_eq!(
        params["sessionId"],
        json!("s-1"),
        "A10 违约：params.sessionId 必须与 agent 发送的一致，实际 {params}"
    );
    assert_eq!(
        params["update"]["sessionUpdate"],
        json!("agent_message_chunk"),
        "A10：update 判别键必须保留（camelCase sessionUpdate）"
    );
    assert_eq!(
        params["update"]["content"]["text"],
        json!("hello"),
        "A10：update 载荷必须完整上送"
    );

    transport.close();
    cleanup(&cwd);
}

/// A10：未注入通知 handler 时，通知被丢弃但**不 panic**，后续请求/响应 id 配对仍正确。
#[test]
fn contract_a10_notification_without_handler_keeps_id_pairing() {
    let cwd = temp_dir("notification-no-handler");
    let spec = fake_spec("notification", None);
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };
    // 刻意不注入 AcpNotificationHandler

    let first = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("无通知 handler 时通知必须被丢弃，请求不得失败");
    let first_parsed: InitializeResponse =
        serde_json::from_value(first).expect("initialize result 必须符合 v1 schema");
    assert_eq!(first_parsed.protocol_version, 1);

    // fake agent 对每个请求都会先发一条通知：第二次请求成功即证明通知没有消耗挂起请求的 id
    let second = transport
        .request("fake/ping", json!({"ok": true}), ACP_REQUEST_TIMEOUT)
        .expect("丢弃通知后 transport 必须仍可继续请求（id 配对未被破坏）");
    let second_parsed: InitializeResponse =
        serde_json::from_value(second).expect("第二次响应仍须按 id 正确配对");
    assert_eq!(
        second_parsed.protocol_version, 1,
        "A10 违约：通知错配导致响应串包"
    );
    assert!(transport.pid() > 0, "丢通知不得影响子进程存活状态");

    transport.close();
    cleanup(&cwd);
}

/// stderr 环形缓冲有界：上限 = STDERR_TAIL_LINES，超限丢最旧、保留最新。
#[test]
fn contract_transport_stderr_tail_is_bounded_ring() {
    let cwd = temp_dir("transport-stderr");
    let spec = fake_spec("stderr", None);
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    };

    transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("stderr 灌入不影响 initialize 往返");

    // 有界条件轮询（stderr 由独立线程消费，不假设与响应同一时刻可见）
    let observed = wait_until(
        || transport.stderr_tail().len() == STDERR_TAIL_LINES,
        SPAWN_REAP_GRACE * 4,
    );
    let tail = transport.stderr_tail();
    assert!(
        observed,
        "stderr 尾部应稳定在 {STDERR_TAIL_LINES} 行，实际 {} 行: {:?}",
        tail.len(),
        tail
    );
    assert_eq!(tail.len(), STDERR_TAIL_LINES, "stderr 环形缓冲必须恰好保留上限行数");
    let line_count = tail.iter().filter(|line| line.contains("stderr-line-")).count();
    assert_eq!(
        line_count, STDERR_TAIL_LINES,
        "尾部 32 行必须全部是诊断行，实际 {tail:?}"
    );
    assert!(
        !tail.iter().any(|line| line.contains("stderr-line-00")),
        "最旧的 stderr 行必须被挤出环形缓冲，实际 {tail:?}"
    );
    assert!(
        tail.iter().any(|line| line.contains("stderr-line-95")),
        "最新的 stderr 行必须仍在缓冲内，实际 {tail:?}"
    );

    transport.close();
    cleanup(&cwd);
}

/// 相对 cwd 必须 fail-closed（§3.2.2：cwd 必须 absolute + canonicalize）。
#[test]
fn contract_transport_relative_cwd_rejected() {
    let spec = fake_spec("ok", None);
    let spawned = AcpTransport::spawn(&spec, Path::new("relative/workspace"), &[]);
    match spawned {
        Ok(mut transport) => {
            transport.close();
            panic!("相对 cwd 必须被拒绝（fail-closed），实际 spawn 成功");
        }
        Err(error) => assert!(
            matches!(
                error,
                AcpTransportError::Spawn(_)
                    | AcpTransportError::Protocol(_)
                    | AcpTransportError::Unavailable(_)
            ),
            "相对 cwd 必须以传输层错误拒绝，实际 {error:?}"
        ),
    }
}

/// 8 个 `acp_*` 错误码是稳定机器可读契约（§3.2.9）。
#[test]
fn contract_transport_error_code_mapping() {
    let cases = [
        (AcpTransportError::Spawn("x".into()), "acp_spawn_failed"),
        (AcpTransportError::Initialize("x".into()), "acp_initialize_failed"),
        (AcpTransportError::Protocol("x".into()), "acp_protocol_error"),
        (AcpTransportError::Agent("x".into()), "acp_agent_error"),
        (AcpTransportError::Refused("x".into()), "acp_agent_refused"),
        (AcpTransportError::AuthRequired("x".into()), "acp_auth_required"),
        (AcpTransportError::Timeout("x".into()), "acp_timeout"),
        (AcpTransportError::Unavailable("x".into()), "acp_unavailable"),
    ];
    for (error, expected) in cases {
        assert_eq!(error.code(), expected, "传输错误码映射违约: {error:?}");
    }
}
