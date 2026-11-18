//! ACP 会话生命周期 + `additionalDirectories` 契约（B179/B181 / task-23）。
//!
//! 真源：`.dev-team/contract-matrix-acp-session-lifecycle.md`（L1–L7）。
//! 断言全部走**真实请求/响应报文**：fake agent 捕获原始请求行、测试反序列化后 typed 比较。
//!
//! 覆盖：
//! - L1 `session/list` 分页（cursor/limit 透传、nextCursor；agy 空列表合法）；
//! - L2 `session/load` 两形态（无 sessionId ⇒ `None` + 会话标识沿用请求 id + 结构化 JSON 诊断；有 sessionId ⇒ `Some`）；
//! - L3 `session/close` 幂等（存在/不存在/重复均 Ok）；
//! - L4 `-32601` ⇒ `Unsupported`/`acp_method_not_supported`（不得 agent/auth）；
//! - L5 `additionalDirectories` 本地 fail-closed（相对/glob/不存在，且**不发** session/new）+ 合法目录 canonical 去重保序；
//! - L6 既有入口不传 dirs ⇒ 报文无 `additionalDirectories` 键。
//!
//! 纪律：无 `#[ignore]`、无空断言；夹具旧模式零改动。

#[path = "support/paths.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use pony_agent_core::acp::client::AcpAgentClient;
use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::{
    method, CloseSessionRequest, ListSessionsRequest, LoadSessionRequest,
};
use pony_agent_core::acp::session::AcpAgentSession;
use pony_agent_core::acp::transport::{
    AcpTransport, AcpTransportError, ACP_REQUEST_TIMEOUT,
};
use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-life-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    support::deverbatim(&path.canonicalize().expect("canonicalize 临时工作目录"))
}

fn fixture_script() -> PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script
}

fn spec(mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let mut args = vec![fixture_script().to_string_lossy().into_owned(), mode.to_string()];
    args.extend(extra.iter().map(|value| (*value).to_string()));
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
        inherit_proxy_env: true,
        mcp_servers: Some(Vec::new()),
    }
}

fn captured_requests(path: &Path) -> Vec<Value> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("捕获行必须是合法 JSON"))
        .collect()
}

fn captured_method(path: &Path, method_name: &str) -> Value {
    captured_requests(path)
        .into_iter()
        .find(|frame| frame["method"] == json!(method_name))
        .unwrap_or_else(|| panic!("fake agent 必须收到 `{method_name}` 请求（{}）", path.display()))
}

fn spawn_session(spec: &AcpAgentSpec, cwd: &Path) -> AcpAgentSession {
    match AcpAgentSession::spawn_with_mcp(spec, cwd, Vec::new()) {
        Ok(session) => session,
        Err(error) => panic!("spawn 必须成功（fake agent 可用）：{error:?}"),
    }
}

fn assert_additional_dirs_invalid(error: AcpTransportError, case: &str) {
    match error {
        AcpTransportError::Spawn(message) => assert!(
            message.contains("acp_additional_directories_invalid"),
            "L5 违约（{case}）：错误消息必须含 acp_additional_directories_invalid，实际 {message}"
        ),
        other => panic!("L5 违约（{case}）：必须是 Spawn 错误（fail-closed），实际 {other:?}"),
    }
}

// ───────────────────────── L1 session/list 分页 ─────────────────────────

/// L1（opencode 形态）：`cursor`/`limit` 必须透传到 agent；响应 2 条会话 + `nextCursor` 原样返回。
#[test]
fn l1_list_sessions_passes_paging_and_returns_sessions() {
    let cwd = temp_dir("list-opencode");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec("session_list", &[capture_text.as_str(), "opencode"]);
    let session = spawn_session(&spec, &cwd);

    let request = ListSessionsRequest {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        cursor: Some("cur-1".to_string()),
        limit: Some(2),
        meta: None,
    };
    let response = match session.list_sessions(request) {
        Ok(response) => response,
        Err(error) => panic!("L1 违约：session/list 必须成功，实际 {error:?}"),
    };
    assert_eq!(response.sessions.len(), 2, "L1 违约：必须解析出 2 条会话");
    assert_eq!(response.sessions[0].session_id, "sess-a", "L1：sessionId");
    assert_eq!(response.sessions[0].cwd, "/tmp/ws", "L1：cwd");
    assert_eq!(response.sessions[0].title.as_deref(), Some("A"), "L1：title");
    assert_eq!(
        response.sessions[0].updated_at.as_deref(),
        Some("2026-11-15T00:00:00Z"),
        "L1：updatedAt"
    );
    assert_eq!(response.sessions[1].session_id, "sess-b");
    assert_eq!(
        response.next_cursor.as_deref(),
        Some("cur-2"),
        "L1 违约：nextCursor 必须原样透传"
    );

    // 分页参数真实透传（断言 agent 侧收到 request 报文，而非只看函数返回值）。
    let wire = captured_method(&capture, "session/list");
    assert_eq!(wire["params"]["cursor"], json!("cur-1"), "L1 违约：cursor 未透传");
    assert_eq!(wire["params"]["limit"], json!(2), "L1 违约：limit 未透传");
    assert_eq!(
        wire["params"]["cwd"],
        json!(cwd.to_string_lossy().into_owned()),
        "L1 违约：cwd 过滤条件未透传"
    );

    let mut session = session;
    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

/// L1（agy 形态）：仅落盘会话 ⇒ 空列表合法；无 `nextCursor` 键 ⇒ `None`。
#[test]
fn l1b_list_sessions_agy_empty_list_is_valid() {
    let cwd = temp_dir("list-agy");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec("session_list", &[capture_text.as_str(), "agy"]);
    let mut session = spawn_session(&spec, &cwd);

    let response = match session.list_sessions(ListSessionsRequest::default()) {
        Ok(response) => response,
        Err(error) => panic!("L1b 违约：agy 空列表必须成功（不得报错），实际 {error:?}"),
    };
    assert!(
        response.sessions.is_empty(),
        "L1b：agy 只列已落盘会话，此处应为空"
    );
    assert_eq!(
        response.next_cursor, None,
        "L1b：响应无 nextCursor 时必须为 None（表示无更多页）"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── L2 session/load 两形态 ─────────────────────────

/// L2（opencode 形态）：响应无 `sessionId` ⇒ `session_id == None`；
/// session 侧会话标识必须沿用**请求 id**，并留下一条结构化 JSON 诊断（C3.1：stdout，不要求文案）。
#[test]
fn l2_load_without_session_id_keeps_request_id() {
    let cwd = temp_dir("load-opencode");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec("session_load", &[capture_text.as_str(), "without-id"]);
    let requested_id = "sess-from-request-xyz";
    let mut session = spawn_session(&spec, &cwd);

    let request = LoadSessionRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        session_id: requested_id.to_string(),
        mcp_servers: Vec::new(),
        additional_directories: None,
        meta: None,
    };
    let response = match session.load_session(request) {
        Ok(response) => response,
        Err(error) => panic!("L2 违约：opencode 形态的 session/load 必须成功，实际 {error:?}"),
    };
    assert_eq!(
        response.session_id, None,
        "L2 违约：opencode 响应不含 sessionId ⇒ 必须解析为 None（不得伪造）"
    );
    assert!(
        response.config_options.as_ref().is_some_and(|options| options.len() == 1),
        "L2：configOptions 必须被保留，实际 {:?}",
        response.config_options
    );
    assert!(
        response.modes.as_ref().is_some_and(|modes| modes.current_mode_id == "code"),
        "L2：modes 必须被保留，实际 {:?}",
        response.modes
    );
    assert_eq!(
        session.session_id(),
        requested_id,
        "L2 违约：响应缺 sessionId 时会话标识必须沿用请求 id"
    );

    // 诊断面（C3.1 迁移）：诊断介质由 stderr 文本 `[acp]` 改为 `agent::otel::log` 结构化 JSON 行
    // （缺省 sink = stdout；核心键 level/message/ts/trace_id）。helper 子进程是**测试二进制**
    // （非 CLI，不调用 `set_stderr_output`）⇒ 诊断落在 stdout，按行 parse JSON 取证。
    let exe = std::env::current_exe().expect("current_exe");
    let output = std::process::Command::new(exe)
        .arg("--exact")
        .arg("l2_helper_load_without_session_id_subprocess")
        .arg("--nocapture")
        .output()
        .expect("必须能启动 helper 子进程");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "L2 helper 子进程必须通过，stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let diagnostic = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|record| {
            record
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.to_lowercase().contains("session"))
                && record.get("trace_id").is_some()
        });
    assert!(
        diagnostic.is_some(),
        "L2 违约：响应缺 sessionId 时 session 侧必须留下结构化 JSON 诊断（`message` 含 session + `trace_id` 键），实际 stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

/// L2 helper：与上面的场景等价的独立用例子进程（其 stderr 由父用例断言）。
#[test]
fn l2_helper_load_without_session_id_subprocess() {
    let cwd = temp_dir("load-opencode-helper");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec("session_load", &[capture_text.as_str(), "without-id"]);
    let requested_id = "sess-from-request-xyz";
    let mut session = spawn_session(&spec, &cwd);
    let response = match session.load_session(LoadSessionRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        session_id: requested_id.to_string(),
        mcp_servers: Vec::new(),
        additional_directories: None,
        meta: None,
    }) {
        Ok(response) => response,
        Err(error) => panic!("L2 helper：load 必须成功，实际 {error:?}"),
    };
    assert_eq!(response.session_id, None, "L2 helper：sessionId 必须是 None");
    assert_eq!(
        session.session_id(),
        requested_id,
        "L2 helper：会话标识必须沿用请求 id"
    );
    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

/// L2（agy 形态）：响应含 `sessionId` + `models`（未知字段）+ `modes` ⇒ 成功且 `Some(id)`。
#[test]
fn l2b_load_with_session_id_and_unknown_fields() {
    let cwd = temp_dir("load-agy");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec("session_load", &[capture_text.as_str(), "with-id"]);
    let mut session = spawn_session(&spec, &cwd);

    let response = match session.load_session(LoadSessionRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        session_id: "sess-requested".to_string(),
        mcp_servers: Vec::new(),
        additional_directories: None,
        meta: None,
    }) {
        Ok(response) => response,
        Err(error) => panic!("L2b 违约：agy 形态必须成功，实际 {error:?}"),
    };
    assert_eq!(
        response.session_id.as_deref(),
        Some("sess-loaded"),
        "L2b 违约：响应含 sessionId 时必须解析为 Some(id)"
    );
    assert!(
        response.modes.as_ref().is_some_and(|modes| modes.current_mode_id == "code"),
        "L2b：modes 必须保留"
    );
    // 未知字段 models 必须被容忍（反序列化未失败即证明）。

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── L3 session/close 幂等 ─────────────────────────

/// L3：对存在 / 不存在 / 重复 close 均必须 `Ok(())`（agy/opencode 回 `{}`）。
#[test]
fn l3_close_session_is_idempotent() {
    let cwd = temp_dir("close-idempotent");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec("session_close", &[capture_text.as_str()]);
    let session = spawn_session(&spec, &cwd);

    for (case, session_id) in [
        ("已存在 id", "sess-a"),
        ("不存在 id", "sess-does-not-exist"),
        ("重复 close", "sess-a"),
    ] {
        match session.close_session(CloseSessionRequest {
            session_id: session_id.to_string(),
            meta: None,
        }) {
            Ok(()) => {}
            Err(error) => panic!("L3 违约：close（{case}）必须幂等成功，实际 {error:?}"),
        }
    }
    // 真实报文：三次 close 都到达 agent。
    let closers = captured_requests(&capture)
        .into_iter()
        .filter(|frame| frame["method"] == json!("session/close"))
        .count();
    assert_eq!(closers, 3, "L3：三次 close 必须都发出请求，实际 {closers}");

    let mut session = session;
    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── L4 -32601 能力降级 ─────────────────────────

/// L4：agent 对 `session/delete` 回 -32601 ⇒ 必须 `Unsupported` + `acp_method_not_supported`
///（**不得** acp_agent_error / acp_auth_required）。
#[test]
fn l4_method_not_found_maps_to_unsupported() {
    let cwd = temp_dir("delete-unsupported");
    let capture = cwd.join("requests.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = spec(
        "session_delete_unsupported",
        &[capture_text.as_str()],
    );
    let mut transport = match AcpTransport::spawn(&spec, &cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("L4：fake agent 必须可启动，实际 {error:?}"),
    };

    let error = match transport.request(
        method::SESSION_DELETE,
        json!({"sessionId": "sess-a"}),
        ACP_REQUEST_TIMEOUT,
    ) {
        Ok(result) => panic!("L4 违约：-32601 必须返回错误，实际成功 {result}"),
        Err(error) => error,
    };
    assert!(
        matches!(error, AcpTransportError::Unsupported(_)),
        "L4 违约：-32601 必须归因 Unsupported，实际 {error:?}"
    );
    assert_eq!(
        error.code(),
        "acp_method_not_supported",
        "L4 违约：错误码必须是 acp_method_not_supported"
    );
    assert_ne!(error.code(), "acp_agent_error", "L4 违约：不得落在 acp_agent_error");
    assert_ne!(error.code(), "acp_auth_required", "L4 违约：不得落在 acp_auth_required");

    // 报文证据：agent 确实收到的是 session/delete 请求。
    let wire = captured_method(&capture, "session/delete");
    assert_eq!(wire["params"]["sessionId"], json!("sess-a"));

    transport.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── L5 additionalDirectories 校验 ─────────────────────────

/// L5：相对 / glob / 不存在目录 ⇒ **本地** fail-closed（`acp_additional_directories_invalid`），
/// 且 agent 侧**未收到任何 session/new 帧**；合法目录 ⇒ 报文含 canonical 值、去重保序。
#[test]
fn l5_additional_directories_fail_closed_and_canonicalized() {
    let cwd = temp_dir("dirs-validation");
    let init_file = cwd.join("initialize.jsonl");
    let session_file = cwd.join("session-new.jsonl");
    let spec = spec(
        "init_capture",
        &[
            init_file.to_string_lossy().as_ref(),
            "1",
            session_file.to_string_lossy().as_ref(),
        ],
    );
    let mut client = match AcpAgentClient::connect(&spec, &cwd) {
        Ok(client) => client,
        Err(error) => panic!("L5：connect 必须成功：{error:?}"),
    };

    // (a) 相对路径
    let relative = PathBuf::from("relative-dir");
    assert_additional_dirs_invalid(
        client
            .new_session_with_dirs(&cwd, Vec::new(), &[relative])
            .expect_err("L5 违约：相对路径必须在本地被拒"),
        "相对路径",
    );
    // (b) glob 元字符（绝对路径但包含 `*`）
    let glob = cwd.join("glob-*");
    assert_additional_dirs_invalid(
        client
            .new_session_with_dirs(&cwd, Vec::new(), &[glob])
            .expect_err("L5 违约：glob 路径必须在本地被拒"),
        "glob",
    );
    // (c) 不存在目录
    let missing = cwd.join("does-not-exist");
    assert_additional_dirs_invalid(
        client
            .new_session_with_dirs(&cwd, Vec::new(), &[missing])
            .expect_err("L5 违约：不存在目录必须在本地被拒"),
        "不存在目录",
    );
    // 本地 fail-closed 证据：三次失败都未发出 session/new。
    assert!(
        !session_file.exists() || std::fs::read_to_string(&session_file).unwrap_or_default().trim().is_empty(),
        "L5 违约：非法 additionalDirectories 必须在**发请求之前**失败，agent 侧不得收到 session/new"
    );

    // (d) 合法绝对目录（含重复项）：canonical + 去重 + 保序。
    let dir_a = cwd.join("extra-a");
    let dir_b = cwd.join("extra-b");
    std::fs::create_dir_all(&dir_a).expect("创建 extra-a");
    std::fs::create_dir_all(&dir_b).expect("创建 extra-b");
    client
        .new_session_with_dirs(&cwd, Vec::new(), &[dir_a.clone(), dir_a.clone(), dir_b.clone()])
        .expect("L5：合法目录必须成功");

    let wire = captured_method(&session_file, "session/new");
    let dirs = wire["params"]["additionalDirectories"]
        .as_array()
        .unwrap_or_else(|| panic!("L5 违约：session/new 必须携带 additionalDirectories，实际 {wire}"));
    let expected = vec![
        dir_a.canonicalize().expect("canonicalize a").to_string_lossy().into_owned(),
        dir_b.canonicalize().expect("canonicalize b").to_string_lossy().into_owned(),
    ];
    assert_eq!(
        dirs,
        &expected
            .iter()
            .map(|value| json!(value))
            .collect::<Vec<_>>(),
        "L5 违约：additionalDirectories 必须 canonical 化、去重且保序"
    );

    client.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── L6 回归锚：无 dirs ⇒ 报文无该键 ─────────────────────────

/// L6：既有 `spawn` / `spawn_with_mcp` / `spawn_for_cli` 不传 dirs 时，session/new 报文
/// **不得**出现 `additionalDirectories` 键（与 B181 前逐字节一致）。
#[test]
fn l6_legacy_entries_omit_additional_directories_key() {
    for entry in ["spawn", "spawn_with_mcp", "spawn_for_cli"] {
        let cwd = temp_dir(&format!("legacy-{entry}"));
        let init_file = cwd.join("initialize.jsonl");
        let session_file = cwd.join("session-new.jsonl");
        let spec = spec(
            "init_capture",
            &[
                init_file.to_string_lossy().as_ref(),
                "1",
                session_file.to_string_lossy().as_ref(),
            ],
        );
        let mut session = match entry {
            "spawn" => match AcpAgentSession::spawn(&spec, &cwd) {
                Ok(session) => session,
                Err(error) => panic!("L6：{entry} 必须成功：{error:?}"),
            },
            "spawn_with_mcp" => match AcpAgentSession::spawn_with_mcp(&spec, &cwd, Vec::new()) {
                Ok(session) => session,
                Err(error) => panic!("L6：{entry} 必须成功：{error:?}"),
            },
            _ => match AcpAgentSession::spawn_for_cli(&spec, &cwd) {
                Ok(session) => session,
                Err(error) => panic!("L6：{entry} 必须成功：{error:?}"),
            },
        };

        let wire = captured_method(&session_file, "session/new");
        assert!(
            wire["params"].get("additionalDirectories").is_none(),
            "L6 违约：{entry}（无 dirs）的 session/new 不得出现 additionalDirectories 键，实际 {wire}"
        );
        session.close();
        let _ = std::fs::remove_dir_all(&cwd);
    }
}
