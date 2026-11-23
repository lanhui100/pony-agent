//! ACP fs/terminal 能力声明与 handler 装配契约（backlog B175 / task-12）。
//!
//! 契约真源：`.dev-team/contract-matrix-acp-capabilities.md`（C1~C9）+ Lead 2026-11-14 裁决。
//! 断言全部走**真实 initialize / 入站请求报文**：fake agent 捕获原始行，反序列化后做 typed 比较。
//!
//! 绿/红分组（红相报告必须分开，不把「已绿」当红相）：
//! - **已实现面（预期绿）**：`capabilities_for` 纯公式、`connect_with_capabilities` 的 wire 声明、
//!   `spawn_with_mcp`/`spawn` 全 false、未声明能力时入站请求回 JSON-RPC error；
//! - **handler 骨架面（预期红）**：`WorkspaceFsHandler::new` / `ManagedTerminalHandler::new` 与
//!   各方法体仍是 `unimplemented!()` ⇒ fs/terminal 用例必须真实 panic。
//!
//! 纪律：无 `#[ignore]`、无空断言；只碰 ACP 测试文件与夹具。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pony_agent_core::acp::client::{AcpAgentClient, AcpClientCapabilities, TerminalHandler};
use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::handlers::{ManagedTerminalHandler, WorkspaceFsHandler};
use pony_agent_core::acp::protocol::{CreateTerminalRequest, ReadTextFileResponse};
use pony_agent_core::acp::session::{capabilities_for, AcpAgentSession};
use pony_agent_core::agent::path_permission::{AuthorizeStore, PathAuthorizationScope, PathPurpose};
use pony_agent_core::agent::process::ProcessManager;
use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-caps-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn fixture_script() -> PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script
}

/// B230：Windows 的 `std::fs::canonicalize` 产出 `\\?\` verbatim 前缀，POSIX shell（Git Bash）
/// 无法按该形态打开文件（capture 落盘静默失败）。传给 fake agent 的路径统一去前缀 + 正斜杠；
/// Unix 下为恒等变换。
fn shell_path(path: &Path) -> String {
    let raw = path.to_string_lossy().into_owned();
    let normalized = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw
    };
    normalized.replace('\\', "/")
}

/// 基础 spec（hint 由调用方指定；`mcp_servers: Some(vec![])` 关闭 inherit 干扰）。
fn spec_with_hint(hint: AcpCapabilitiesHint, mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let mut args = vec![shell_path(&fixture_script()), mode.to_string()];
    args.extend(extra.iter().map(|value| (*value).to_string()));
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: hint,
        inherit_proxy_env: true,
        mcp_servers: Some(Vec::new()),
    }
}

/// `init_capture` 模式：把 initialize 原始请求行落到 `<cwd>/initialize.jsonl`。
fn spec_capturing_initialize(hint: AcpCapabilitiesHint, cwd: &Path) -> AcpAgentSpec {
    let init_file = cwd.join("initialize.jsonl");
    let session_file = cwd.join("session-new.jsonl");
    spec_with_hint(
        hint,
        "init_capture",
        &[
            shell_path(&init_file).as_str(),
            "1",
            shell_path(&session_file).as_str(),
        ],
    )
}

/// 读取被捕获的 initialize 请求的 `clientCapabilities`（原始 JSON，断言「是否声明」用）。
fn captured_client_capabilities(cwd: &Path) -> Value {
    let path = cwd.join("initialize.jsonl");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获 initialize 报文（{}：{error}）", path.display()));
    let line = raw.lines().next().expect("initialize 捕获为空");
    let frame: Value = serde_json::from_str(line).expect("initialize 必须是合法 JSON");
    assert_eq!(frame["method"], json!("initialize"));
    frame["params"]["clientCapabilities"].clone()
}

/// 读取 probe_* 模式落盘的入站应答（按 `id` 建索引）。
fn captured_responses(cwd: &Path) -> Vec<Value> {
    let path = cwd.join("responses.jsonl");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获入站应答（{}：{error}）", path.display()));
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("应答行必须是合法 JSON"))
        .collect()
}

fn response_for(responses: &[Value], id: u64) -> Value {
    responses
        .iter()
        .find(|response| response["id"].as_u64() == Some(id))
        .unwrap_or_else(|| panic!("缺少 id={id} 的应答，实际 {responses:?}"))
        .clone()
}

fn write_paths_file(cwd: &Path, paths: &[&str]) -> PathBuf {
    let path = cwd.join("probe-paths.jsonl");
    let body: String = paths
        .iter()
        .map(|value| format!("{value}\n"))
        .collect::<Vec<_>>()
        .concat();
    std::fs::write(&path, body).expect("写 paths 文件");
    path
}

fn spec_probing_fs(hint: AcpCapabilitiesHint, cwd: &Path, paths: &[&str]) -> AcpAgentSpec {
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let paths_file = write_paths_file(cwd, paths);
    spec_with_hint(
        hint,
        "probe_fs",
        &[
            shell_path(&init_file).as_str(),
            shell_path(&resp_file).as_str(),
            shell_path(&paths_file).as_str(),
        ],
    )
}

/// marker 经**文件**传递：fake agent argv 只含文件路径，marker 只拼进终端子进程命令行，
/// 因此 `pgrep -f <marker>` 不会命中 fake agent 自身。
fn spec_probing_terminal(hint: AcpCapabilitiesHint, cwd: &Path, marker_file: &Path) -> AcpAgentSpec {
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    spec_with_hint(
        hint,
        "probe_terminal",
        &[
            shell_path(&init_file).as_str(),
            shell_path(&resp_file).as_str(),
            shell_path(&marker_file).as_str(),
        ],
    )
}

fn hint(fs: bool, terminal: bool) -> AcpCapabilitiesHint {
    AcpCapabilitiesHint { fs, terminal }
}

fn run_prompt(session: &AcpAgentSession) {
    struct NoopSink;
    impl pony_agent_core::agent::turn_flow::TurnEventSink for NoopSink {
        fn emit(&self, _name: &str, _payload: pony_agent_core::agent::runtime::TurnStreamEvent) {}
    }
    let result = session.prompt(
        vec![pony_agent_core::acp::protocol::ContentBlock::Text {
            text: "probe".to_string(),
        }],
        &NoopSink,
    );
    assert_eq!(result.status, "ok", "probe 轮必须正常结束（end_turn），实际 {result:?}");
}

/// NFR 唯一真源（残留断言的有界等待上界由 `acp.spawn_reap_grace_ms` 派生）。
fn nfr_acp_ms(key: &str) -> u64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.dev-team/nfr-baseline.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 NFR 真源失败 {}: {error}", path.display()));
    let parsed: Value = serde_json::from_str(&body).expect("nfr-baseline.json 必须是合法 JSON");
    parsed
        .get("acp")
        .and_then(|acp| acp.get(key))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 acp.{key}"))
}

fn error_text(response: &Value) -> String {
    response
        .get("error")
        .map(|error| error.to_string())
        .unwrap_or_else(|| format!("<无 error 字段：{response}>"))
}

// ───────────────────────── G1. 声明公式（C1/C2 纯函数，预期绿） ─────────────────────────

/// G1：`capabilities_for(spec, has_fs, has_terminal)` = hint ∧ handler 存在（hint 是上限）。
#[test]
fn g1_capabilities_for_is_hint_and_handler() {
    for hint_fs in [false, true] {
        for hint_terminal in [false, true] {
            for has_fs in [false, true] {
                for has_terminal in [false, true] {
                    let spec = spec_with_hint(hint(hint_fs, hint_terminal), "flow", &["end_turn", "s"]);
                    let caps = capabilities_for(&spec, has_fs, has_terminal);
                    assert_eq!(
                        caps.fs,
                        hint_fs && has_fs,
                        "C1/C2 违约：fs(fs={hint_fs}∧has_fs={has_fs}) 期望 {} 实际 {}",
                        hint_fs && has_fs,
                        caps.fs
                    );
                    assert_eq!(
                        caps.terminal,
                        hint_terminal && has_terminal,
                        "C1/C2 违约：terminal(terminal={hint_terminal}∧has_terminal={has_terminal})"
                    );
                }
            }
        }
    }
    // 纯函数 `capabilities_for` 的 Default 语义：无 hint 无 handler ⇒ NONE。
    let empty = spec_with_hint(hint(false, false), "flow", &["end_turn", "s"]);
    assert_eq!(
        capabilities_for(&empty, false, false),
        AcpClientCapabilities::NONE
    );
}

// ───────────────────────── G2. 真实 initialize wire 声明（预期绿） ─────────────────────────

/// G2：`connect_with_capabilities` 必须把 caps 精确映射到 initialize 的 `clientCapabilities`；
/// 全 false 时 fs/terminal 键必须缺席（既有 connect 行为逐字节兼容）。
#[test]
fn g2_connect_with_capabilities_declares_exactly_caps() {
    // (a) caps 全 true ⇒ fs{readTextFile:true,writeTextFile:true} + terminal:true
    {
        let cwd = temp_dir("caps-true");
        let spec = spec_capturing_initialize(hint(false, false), &cwd);
        let mut client = match AcpAgentClient::connect_with_capabilities(
            &spec,
            &cwd,
            AcpClientCapabilities { fs: true, terminal: true },
        ) {
            Ok(client) => client,
            Err(error) => panic!("connect_with_capabilities 必须成功：{error:?}"),
        };
        let capabilities = captured_client_capabilities(&cwd);
        assert_eq!(
            capabilities["fs"]["readTextFile"],
            json!(true),
            "C1 违约：caps.fs=true 必须声明 readTextFile:true，实际 {capabilities}"
        );
        assert_eq!(capabilities["fs"]["writeTextFile"], json!(true));
        assert_eq!(
            capabilities["terminal"],
            json!(true),
            "C1 违约：caps.terminal=true 必须声明 terminal:true，实际 {capabilities}"
        );
        client.close();
        let _ = std::fs::remove_dir_all(&cwd);
    }

    // (b) NONE ⇒ fs/terminal 键缺席（不得出现 null 或 false 占位漂移）。
    {
        let cwd = temp_dir("caps-none");
        let spec = spec_capturing_initialize(hint(false, false), &cwd);
        let mut client = match AcpAgentClient::connect_with_capabilities(
            &spec,
            &cwd,
            AcpClientCapabilities::NONE,
        ) {
            Ok(client) => client,
            Err(error) => panic!("connect_with_capabilities(NONE) 必须成功：{error:?}"),
        };
        let capabilities = captured_client_capabilities(&cwd);
        assert!(
            capabilities.get("fs").is_none(),
            "C7 违约：caps.fs=false 时不得声明 fs，实际 {capabilities}"
        );
        assert!(
            capabilities.get("terminal").is_none(),
            "C7 违约：caps.terminal=false 时不得声明 terminal，实际 {capabilities}"
        );
        client.close();
        let _ = std::fs::remove_dir_all(&cwd);
    }
}

/// G2b：`spawn` / `spawn_with_mcp` 既有入口必须全 false（hint=true 也不得声明）。
#[test]
fn g2b_legacy_session_entries_never_declare_capabilities() {
    let cwd = temp_dir("legacy-entries");
    let spec = spec_capturing_initialize(hint(true, true), &cwd);
    let mut session = match AcpAgentSession::spawn_with_mcp(&spec, &cwd, Vec::new()) {
        Ok(session) => session,
        Err(error) => panic!("spawn_with_mcp 必须成功：{error:?}"),
    };
    let capabilities = captured_client_capabilities(&cwd);
    assert!(
        capabilities.get("fs").is_none() && capabilities.get("terminal").is_none(),
        "C7/C9 违约：既有入口（handler 全 None）不得声明任何能力，实际 {capabilities}"
    );
    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G3. hint 上限（无 handler 时，预期绿） ─────────────────────────

/// G3/C9：hint=true 但 handler 未提供 ⇒ 不声明（hint 只是上限，不是授权）。
#[test]
fn g3_hint_without_handler_declares_nothing() {
    let cwd = temp_dir("hint-no-handler");
    let spec = spec_capturing_initialize(hint(true, true), &cwd);
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(&spec, &cwd, Vec::new(), None, None)
    {
        Ok(session) => session,
        Err(error) => panic!("hint=true 但无 handler 必须成功（不声明），实际 {error:?}"),
    };
    let capabilities = captured_client_capabilities(&cwd);
    assert!(
        capabilities.get("fs").is_none(),
        "C2/C9 违约：hint.fs=true 但无 FsHandler 时不得声明 fs，实际 {capabilities}"
    );
    assert!(
        capabilities.get("terminal").is_none(),
        "C2/C9 违约：hint.terminal=true 但无 TerminalHandler 时不得声明 terminal，实际 {capabilities}"
    );
    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G4. 声明↔装配一致（handler 骨架，预期红） ─────────────────────────

fn workspace_fs_handler(root: &Path) -> Arc<WorkspaceFsHandler> {
    Arc::new(WorkspaceFsHandler::new(
        root.to_path_buf(),
        Arc::new(AuthorizeStore::new()),
        PathPurpose::Read,
    ))
}

fn managed_terminal_handler() -> Arc<ManagedTerminalHandler> {
    Arc::new(ManagedTerminalHandler::new(Arc::new(ProcessManager::new())))
}

/// G7c（B177 ADVISORY ⑨）：**未绑定 workspace** 的 handler 不得接受自定义 cwd。
///
/// 复审实测：`ManagedTerminalHandler::new()`（`workspace_root = None`）在旧实现下会让
/// `terminal/create` 带任意绝对 `cwd`（如 `/etc`）成功执行——等于把终端开到任意位置。生产路径
/// 都走 `with_workspace_root`，但 `new()` 是公开构造口，必须自身 fail-closed。
///
/// 用例用 `std::env::temp_dir()` 而非写死 `/etc`：写死 unix 路径会在 Windows 上被更早的
/// 「cwd 必须是绝对路径」判定拦下（`/etc` 在 Windows 非绝对），于是本用例断言的
/// 「未绑定 workspace」理由永远到不了，门禁在 Windows 上变成假红。
///
/// 红相：若取消该兜底，本用例会拿到 `Ok`（终端真的被创建），第一条断言即红。
#[test]
fn g7c_unbound_handler_rejects_explicit_cwd() {
    let handler = ManagedTerminalHandler::new(Arc::new(ProcessManager::new()));
    // 平台无关的绝对路径：Windows 的 `/etc` 不是绝对路径，会在更早的绝对性判定处被拒。
    let unbound_cwd = std::env::temp_dir();
    let request = CreateTerminalRequest {
        session_id: "sess-unbound".to_string(),
        command: "sh".to_string(),
        args: None,
        env: None,
        cwd: Some(unbound_cwd.to_string_lossy().into_owned()),
        output_byte_limit: None,
    };
    match handler.create(&request) {
        Ok(created) => panic!(
            "G7c 违约：未绑定 workspace 时不得用自定义 cwd 创建终端，实际返回 terminalId={:?}",
            created.terminal_id
        ),
        Err(error) => {
            // `AcpHandlerError` 是 crate 内类型（未导出），用 Debug 形态断言归类与文案。
            let text = format!("{error:?}");
            assert!(
                text.contains("Denied") && text.contains("未绑定 workspace"),
                "G7c：应为 Denied 且理由说明「未绑定 workspace」，实际 `{text}`"
            );
        }
    }

/// G10（B177 ADVISORY ③④）：**慢入站请求不得饿死其它请求**。
///
/// 夹具 `probe_terminal_interleave` 在发完 `terminal/wait_for_exit`（9402，长命令 sleep 3s）后
/// **立即**发出 `terminal/output`（9404）而**不等待** 9402 的应答：
/// - 单线程串行分发 ⇒ 9404 只能排在 3s 的 wait 之后，应答顺序是 9402 → 9404；
/// - worker 池（`ACP_INBOUND_WORKERS`）⇒ 9404 由另一个 worker 处理，应答 9404 先落盘。
///
/// **红相**：把分发改回单线程串行，`fast < slow` 立即失败。这就是复审 ④ 说的
/// 「旧夹具严格同步、永远观测不到并发」被真正补上的地方。
#[test]
fn g10_slow_inbound_request_does_not_starve_others() {
    let cwd = temp_dir("inbound-interleave");
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let spec = spec_with_hint(
        hint(false, true),
        "probe_terminal_interleave",
        &[
            init_file.to_string_lossy().as_ref(),
            resp_file.to_string_lossy().as_ref(),
            "3",
        ],
    );
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        Some(managed_terminal_handler()),
    ) {
        Ok(session) => session,
        Err(error) => panic!("G10：spawn 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    let ids: Vec<Value> = responses.iter().map(|r| r["id"].clone()).collect();
    let fast = responses.iter().position(|r| r["id"] == 9404);
    let slow = responses.iter().position(|r| r["id"] == 9402);
    let (Some(fast), Some(slow)) = (fast, slow) else {
        panic!("G10 违约：9402 与 9404 的应答都必须落盘，实际 id 序列 {ids:?}");
    };
    assert!(
        fast < slow,
        "G10 违约：terminal/output(9404) 被 wait_for_exit(9402) 饿死——入站分发仍是串行（9404@{fast} vs 9402@{slow}，全序列 {ids:?}）"
    );
}

/// G4/C8：hint=true + 两个真实 handler ⇒ wire 声明 true（声明与装配一致；同一路径）。
#[test]
fn g4_declared_capabilities_match_installed_handlers() {
    let cwd = temp_dir("caps-with-handlers");
    let spec = spec_capturing_initialize(hint(true, true), &cwd);
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        Some(workspace_fs_handler(&cwd)),
        Some(managed_terminal_handler()),
    ) {
        Ok(session) => session,
        Err(error) => panic!("C8 违约：hint=true + handler 齐备必须成功，实际 {error:?}"),
    };
    let capabilities = captured_client_capabilities(&cwd);
    assert_eq!(
        capabilities["fs"]["readTextFile"],
        json!(true),
        "C8 违约：声明/装配矛盾（fs），实际 {capabilities}"
    );
    assert_eq!(capabilities["terminal"], json!(true), "C8：terminal 同上");
    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G5. fs 正向（handler 骨架，预期红） ─────────────────────────

/// G5/C4：workspace 内 read 逐字节相等；write 落盘内容可验证。
#[test]
fn g5_fs_handler_reads_and_writes_inside_workspace() {
    let cwd = temp_dir("fs-positive");
    let target = cwd.join("notes.txt");
    std::fs::write(&target, "WIRE-CONTENT").expect("写 workspace 文件");
    let target_json = format!("{:?}", target.to_string_lossy().into_owned());
    let spec = spec_probing_fs(hint(true, false), &cwd, &[target_json.as_str()]);

    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        Some(workspace_fs_handler(&cwd)),
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("fs 正向：spawn_with_mcp_and_handlers 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    let read = response_for(&responses, 9100);
    let parsed: ReadTextFileResponse = serde_json::from_value(read["result"].clone())
        .unwrap_or_else(|error| panic!("C4 违约：workspace 内 read 必须成功（{error}）：{read}"));
    assert_eq!(parsed.content, "WIRE-CONTENT", "C4 违约：read 内容必须逐字节相等");

    let write = response_for(&responses, 9500);
    assert!(
        write.get("result").is_some() && write.get("error").is_none(),
        "C4 违约：workspace 内 write 必须成功，实际 {write}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("读回写入文件"),
        "PROBE-WRITE-0",
        "C4 违约：write 必须真实落盘（内容 == 请求 content）"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

/// G5b/C4：workspace 外路径经 `AuthorizeStore::grant_with_scope(Read)` 授权后可读。
#[test]
fn g5b_fs_handler_reads_authorized_external_path() {
    let cwd = temp_dir("fs-authorized");
    let external = temp_dir("fs-external");
    let external_file = external.join("outside.txt");
    std::fs::write(&external_file, "AUTHORIZED").expect("写外部文件");
    let external_json = format!("{:?}", external_file.to_string_lossy().into_owned());
    let spec = spec_probing_fs(hint(true, false), &cwd, &[external_json.as_str()]);

    let store = Arc::new(AuthorizeStore::new());
    let canonical = external_file.canonicalize().expect("canonicalize 外部文件");
    store
        .grant_with_scope(canonical, PathAuthorizationScope::Read)
        .expect("显式授权外部读");
    let handler = Arc::new(WorkspaceFsHandler::new(
        cwd.clone(),
        Arc::clone(&store),
        PathPurpose::Read,
    ));

    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        Some(handler),
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("授权外部读：spawn 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let read = response_for(&captured_responses(&cwd), 9100);
    let parsed: ReadTextFileResponse = serde_json::from_value(read["result"].clone())
        .unwrap_or_else(|error| panic!("C4 违约：已授权外部读必须成功（{error}）：{read}"));
    assert_eq!(parsed.content, "AUTHORIZED");

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
    let _ = std::fs::remove_dir_all(&external);
}

// ───────────────────────── G6. fs 反向 fail-closed（handler 骨架，预期红） ─────────────────────────

/// G6/C4：workspace 外未授权、`../` 逃逸、空 path ⇒ 全部 JSON-RPC error，
/// 不得返回空 content、不得 panic、不得落盘到 workspace 外。
#[test]
fn g6_fs_handler_fails_closed_outside_workspace() {
    let cwd = temp_dir("fs-denied");
    let escape_target = cwd.join("..").join("pony-acp-escape-target.txt");
    let escape_json = format!("{:?}", escape_target.to_string_lossy().into_owned());
    let absolute_outside = temp_dir("fs-outside");
    let outside_json = format!(
        "{:?}",
        absolute_outside.join("secret.txt").to_string_lossy().into_owned()
    );
    std::fs::write(absolute_outside.join("secret.txt"), "SECRET").expect("写外部文件");

    let spec = spec_probing_fs(
        hint(true, false),
        &cwd,
        &[escape_json.as_str(), outside_json.as_str(), "\"\""],
    );
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        Some(workspace_fs_handler(&cwd)),
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("fs 反向：spawn 必须成功（越界在请求期拒绝）：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    // read 与 write 各 3 条（id 9100..9102 / 9500..9502），必须全为 error。
    for index in 0..3u64 {
        for base in [9100u64, 9500u64] {
            let response = response_for(&responses, base + index);
            assert!(
                response.get("result").is_none(),
                "C4 违约：越界请求必须失败，实际返回 result：{response}"
            );
            let text = error_text(&response);
            assert!(
                text.contains("permission_denied")
                    || text.contains("requires_authorization")
                    || text.contains("outside_workspace_write_denied"),
                "C4 违约：越界错误的 code 语义必须保留（permission_denied/requires_authorization/outside_workspace_write_denied），实际 {text}"
            );
            assert!(
                !text.contains("SECRET"),
                "C4 违约：错误消息不得泄漏文件内容，实际 {text}"
            );
        }
    }
    assert!(
        !escape_target.exists(),
        "C4 违约：`../` 逃逸写不得落盘到 workspace 外：{}",
        escape_target.display()
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
    let _ = std::fs::remove_dir_all(&absolute_outside);
}

// ───────────────────────── G7. terminal 全链 + 无残留（handler 骨架，预期红） ─────────────────────────

/// G7/C5：create→output(含 hi)→wait_for_exit(0)→kill(幂等)→release；
/// release 后 `pgrep -f <唯一 marker>` 必须无匹配（无残留子进程）。
#[test]
#[cfg_attr(
    windows,
    ignore = "进程残留检查依赖 POSIX pgrep（Windows 无等价命令，B230/B246）"
)]
fn g7_terminal_handler_full_chain_and_no_residue() {
    let cwd = temp_dir("terminal-chain");
    let marker = format!("pony-acp-term-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos());
    let marker_file = cwd.join("marker.txt");
    std::fs::write(&marker_file, &marker).expect("写 marker 文件");
    let spec = spec_probing_terminal(hint(false, true), &cwd, &marker_file);

    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        Some(managed_terminal_handler()),
    ) {
        Ok(session) => session,
        Err(error) => panic!("terminal 链：spawn 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    let created = response_for(&responses, 9201);
    let terminal_id = created["result"]["terminalId"]
        .as_str()
        .unwrap_or_else(|| panic!("C5 违约：terminal/create 必须返回 terminalId，实际 {created}"));
    assert!(!terminal_id.is_empty(), "C5：terminalId 不得为空");

    let output = response_for(&responses, 9202);
    assert!(
        output["result"]["output"]
            .as_str()
            .is_some_and(|text| text.contains("hi")),
        "C5 违约：terminal/output 必须包含子进程 stdout `hi`，实际 {output}"
    );

    let waited = response_for(&responses, 9203);
    assert_eq!(
        waited["result"]["exitCode"],
        json!(0),
        "C5 违约：wait_for_exit 必须归约 exitCode=0，实际 {waited}"
    );

    for id in [9204u64, 9205u64] {
        let killed = response_for(&responses, id);
        assert!(
            killed.get("error").is_none(),
            "C5 违约：terminal/kill（含幂等第二次）必须成功，实际 {killed}"
        );
    }
    let released = response_for(&responses, 9206);
    assert!(
        released.get("error").is_none(),
        "C5 违约：terminal/release 必须成功，实际 {released}"
    );

    // 无残留断言必须在会话关闭**之后**（fake agent argv 不含 marker，故不会自命中；
    // 关闭会话保证外部 agent 本身与终端子进程都已回收），并有 NFR 派生的有界等待。
    session.close();
    let deadline = Duration::from_millis(nfr_acp_ms("spawn_reap_grace_ms") * 4);
    let started = Instant::now();
    loop {
        let status = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(&marker)
            .output()
            .expect("pgrep 可执行");
        if status.status.code() == Some(1) {
            break;
        }
        assert!(
            started.elapsed() < deadline,
            "C5 违约：release+close 后不得残留子进程（pgrep 期望无匹配，实际 exit={:?}），stdout:{}",
            status.status.code(),
            String::from_utf8_lossy(&status.stdout)
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G8. 未声明能力收到反向请求（预期绿） ─────────────────────────

/// G8/C3：caps 全 false + 未装 handler 时 agent 主动发 fs/terminal 请求 ⇒ JSON-RPC error，
/// 不得执行 I/O、不得 panic，且本轮 prompt 仍能 end_turn。
#[test]
fn g8_undeclared_capability_requests_are_rejected() {
    let cwd = temp_dir("undeclared");
    let spec = spec_probing_fs(hint(false, false), &cwd, &["\"/tmp/should-not-be-read.txt\""]);
    let mut session = match AcpAgentSession::spawn_with_mcp(&spec, &cwd, Vec::new()) {
        Ok(session) => session,
        Err(error) => panic!("未声明能力：spawn_with_mcp 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    for id in [9100u64, 9500u64] {
        let response = response_for(&responses, id);
        assert!(
            response.get("error").is_some(),
            "C3 违约：未声明能力的入站请求必须回 JSON-RPC error，实际 {response}"
        );
        assert!(
            response.get("result").is_none(),
            "C3 违约：不得对未声明能力返回 result，实际 {response}"
        );
        let text = error_text(&response);
        assert!(
            !text.is_empty(),
            "C3 违约：错误必须带可读原因，实际 {response}"
        );
    }

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── G9. 畸形 path 参数（handler 骨架，预期红） ─────────────────────────

/// G9/C4：`path` 为 number / null / 空串 ⇒ 明确错误码（AcpHandlerError::Denied/Internal →
/// JSON-RPC error），非 panic；绝不返回空 content 假装成功。
#[test]
fn g9_malformed_path_parameters_yield_errors() {
    let cwd = temp_dir("malformed-path");
    let spec = spec_probing_fs(hint(true, false), &cwd, &["123", "null", "\"\""]);
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        Some(workspace_fs_handler(&cwd)),
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("畸形 path：spawn 必须成功（请求期拒绝）：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    for index in 0..3u64 {
        for base in [9100u64, 9500u64] {
            let response = response_for(&responses, base + index);
            assert!(
                response.get("error").is_some(),
                "C4 违约：畸形 path 必须回 JSON-RPC error，实际 {response}"
            );
            assert!(
                response.get("result").is_none(),
                "C4 违约：畸形 path 不得返回 result（不得假装成功），实际 {response}"
            );
        }
    }

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}
