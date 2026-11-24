//! ACP `mcpServers` 接线（backlog B174 / task-10）红相契约。
//!
//! 契约真源：`.dev-team/contract-matrix-acp-mcp.md`（M1~M8）+ Lead 三态裁决。
//! 断言纪律：
//! - 三态（inherit / 显式空 / 显式条目）必须可区分（M1/M4）；
//! - inherit 只产 stdio，并逐字段等价 Pony loader 结果（M2/M3）；
//! - env 命中 `is_denied_spec_env_key` ⇒ fail-closed（M5，不是静默过滤）；
//! - 致命配置诊断 ⇒ fail-closed（M6），不降级为空列表；
//! - 走**真实 `session/new` 报文**：fake agent 捕获原始行后反序列化为 `SessionNewRequest`
//!   做 typed 比较，不靠字符串包含。
//!
//! 阈值：本文件无超时类断言（纯配置/报文等价），等待只用于子进程退出后的文件落地。

use super::support;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::MutexGuard;
use std::time::Instant;

use pony_agent_core::acp::config::{
    resolve_mcp_servers, resolve_mcp_servers_in, AcpAgentSpec, AcpCapabilitiesHint, AcpConfigError,
    AcpMcpServerSpec, AcpMcpTransportSpec,
};
use pony_agent_core::acp::protocol::{
    EnvVariable, HttpHeader, McpServer, SessionNewRequest,
};
use pony_agent_core::acp::session::AcpAgentSession;
use serde_json::{json, Value};

// ───────────────────────────── 通用设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-mcp-{}-{}-{tag}-{seq}",
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

/// 隔离环境（HOME / XDG_* / PONY_AGENT_HOME），进程级串行（env 是全局状态）。

struct IsolatedEnv {
    _guard: MutexGuard<'static, ()>,
    root: PathBuf,
    restore: Vec<(&'static str, Option<OsString>)>,
}

impl IsolatedEnv {
    /// `home_mcp_json`: `Some(text)` 表示写入 `$PONY_AGENT_HOME/mcp.json`（全局层，可信）。
    fn new(tag: &str, home_mcp_json: Option<&str>) -> Self {
        let guard = crate::env_lock::lock();
        let root = temp_dir(tag);
        for child in ["home", "config", "data", "state", "cache", "workspace", "pony-agent-home"] {
            std::fs::create_dir_all(root.join(child)).expect("创建隔离子目录");
        }
        if let Some(text) = home_mcp_json {
            std::fs::write(root.join("pony-agent-home").join("mcp.json"), text)
                .expect("写入全局 MCP 配置");
        }

        let mut restore = Vec::new();
        let assignments: Vec<(&'static str, PathBuf)> = vec![
            ("HOME", root.join("home")),
            ("XDG_CONFIG_HOME", root.join("config")),
            ("XDG_DATA_HOME", root.join("data")),
            ("XDG_STATE_HOME", root.join("state")),
            ("XDG_CACHE_HOME", root.join("cache")),
            ("PONY_AGENT_HOME", root.join("pony-agent-home")),
            ("PONY_MCP_TEST_TOKEN", PathBuf::from("resolved-token")),
        ];
        for (key, value) in assignments {
            restore.push((key, std::env::var_os(key)));
            std::env::set_var(key, value);
        }
        Self {
            _guard: guard,
            root,
            restore,
        }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        for (key, old) in self.restore.drain(..).rev() {
            match old {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 基础 spec：`mcp_servers` 由调用方显式给出（三态可区分）。
fn base_spec(mcp_servers: Option<Vec<AcpMcpServerSpec>>) -> AcpAgentSpec {
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args: vec![
            fixture_script().to_string_lossy().into_owned(),
            "init_capture".to_string(),
        ],
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
        inherit_proxy_env: true,
        mcp_servers,
    }
}

fn stdio_entry(name: &str, command: &str, args: &[&str], env: &[(&str, &str)]) -> AcpMcpServerSpec {
    let mut env_map = BTreeMap::new();
    for (key, value) in env {
        env_map.insert((*key).to_string(), (*value).to_string());
    }
    AcpMcpServerSpec {
        name: name.to_string(),
        transport: AcpMcpTransportSpec::Stdio {
            command: command.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            env: env_map,
        },
        auto_approve: None,
    }
}

fn http_entry(name: &str, url: &str, headers: &[(&str, &str)]) -> AcpMcpServerSpec {
    AcpMcpServerSpec {
        name: name.to_string(),
        transport: AcpMcpTransportSpec::Http {
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(key, value)| HttpHeader {
                    name: (*key).to_string(),
                    value: (*value).to_string(),
                })
                .collect(),
        },
        auto_approve: None,
    }
}

fn sse_entry(name: &str, url: &str) -> AcpMcpServerSpec {
    AcpMcpServerSpec {
        name: name.to_string(),
        transport: AcpMcpTransportSpec::Sse {
            url: url.to_string(),
            headers: Vec::new(),
        },
        auto_approve: None,
    }
}

/// 让 spec 走 init_capture 模式并把 `session/new` 原始行落到 `<cwd>/session-new.jsonl`。
fn spec_with_capture(mut spec: AcpAgentSpec, cwd: &Path) -> AcpAgentSpec {
    let init_capture = cwd.join("initialize.jsonl");
    let session_capture = cwd.join("session-new.jsonl");
    spec.args = vec![
        fixture_script().to_string_lossy().into_owned(),
        "init_capture".to_string(),
        init_capture.to_string_lossy().into_owned(),
        "1".to_string(),
        session_capture.to_string_lossy().into_owned(),
    ];
    spec
}

/// 读取并反序列化被捕获的 `session/new` 报文（typed）。
fn read_session_new(cwd: &Path) -> SessionNewRequest {
    let path = cwd.join("session-new.jsonl");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获 session/new 报文（{}：{error}）", path.display()));
    let line = raw.lines().next().unwrap_or_else(|| panic!("session/new 捕获为空"));
    let frame: Value = serde_json::from_str(line).expect("session/new 必须是合法 JSON");
    assert_eq!(frame["method"], json!("session/new"), "捕获的必须是 session/new 报文");
    serde_json::from_value(frame["params"].clone()).expect("session/new params 必须符合 v1 SessionNewRequest")
}

fn code_of(error: &AcpConfigError) -> &'static str {
    error.code()
}

// ───────────────────────── A. 三态（M1/M4） ─────────────────────────

/// A1：`mcpServers` 三态在 serde 层可区分；`None` 序列化时不落键（旧清单逐字节兼容）。
#[test]
fn a1_three_states_are_distinguishable_in_serde() {
    let no_field: AcpAgentSpec =
        serde_json::from_value(json!({"id": "a", "label": "A", "command": "sh", "args": []}))
            .expect("缺 mcpServers 必须可反序列化");
    assert_eq!(no_field.mcp_servers, None, "M1：缺省必须解析为 None（inherit）");

    let explicit_empty: AcpAgentSpec = serde_json::from_value(json!({
        "id": "a", "label": "A", "command": "sh", "args": [], "mcpServers": []
    }))
    .expect("显式空数组必须可反序列化");
    assert_eq!(
        explicit_empty.mcp_servers,
        Some(Vec::new()),
        "M1：显式 [] 必须解析为 Some(vec![])，与缺省 None 可区分"
    );

    let explicit_list: AcpAgentSpec = serde_json::from_value(json!({
        "id": "a", "label": "A", "command": "sh", "args": [],
        "mcpServers": [{"name": "files", "transport": {"type": "stdio", "command": "npx", "args": ["-y", "srv"]}}]
    }))
    .expect("显式条目必须可反序列化");
    match explicit_list.mcp_servers.as_deref() {
        Some([entry]) => {
            assert_eq!(entry.name, "files");
            assert!(matches!(entry.transport, AcpMcpTransportSpec::Stdio { .. }));
        }
        other => panic!("M1：显式条目必须解析为 Some(list)，实际 {other:?}"),
    }

    // None 不落键（旧清单兼容）；Some([]) 落 "mcpServers":[]。
    let none_json = serde_json::to_value(base_spec(None)).expect("serialize");
    assert!(
        none_json.get("mcpServers").is_none(),
        "M1：None 时不得落 `mcpServers` 键，实际 {none_json}"
    );
    let empty_json = serde_json::to_value(base_spec(Some(Vec::new()))).expect("serialize");
    assert_eq!(
        empty_json["mcpServers"],
        json!([]),
        "M1：Some([]) 必须序列化为显式空数组"
    );
}

/// A2：resolve 三态 —— inherit 取 Pony 配置、显式 [] 禁用、显式条目仅用条目（不叠加）。
#[test]
fn a2_resolve_three_states() {
    let env = IsolatedEnv::new(
        "three-states",
        Some(
            &json!({
                "schemaVersion": "mcp-config-v1",
                "mcpServers": {
                    "alpha": {"command": "echo", "args": ["hi"]},
                    "beta": {"command": "cat", "args": ["x"], "env": {"TOKEN": "${env:PONY_MCP_TEST_TOKEN}"}}
                }
            })
            .to_string(),
        ),
    );
    let workspace = env.workspace();

    // inherit：缺省 ⇒ Pony 全局配置（M2/M3：只产 stdio，逐字段等价，丢弃 Pony 专属字段）。
    let inherit = resolve_mcp_servers_in(&base_spec(None), &workspace)
        .expect("inherit 必须成功（全局 mcp.json 合法）");
    assert_eq!(inherit.len(), 2, "M2：inherit 必须带上 Pony 配置的两条 server，实际 {inherit:?}");
    for (server, expected) in inherit.iter().zip([
        ("alpha", "echo", vec!["hi"], Vec::new()),
        ("beta", "cat", vec!["x"], vec![EnvVariable {
            name: "TOKEN".to_string(),
            value: "resolved-token".to_string(),
        }]),
    ]) {
        match server {
            McpServer::Stdio(stdio) => {
                assert_eq!(stdio.name, expected.0, "M2：id → wire name");
                assert_eq!(stdio.command, expected.1, "M2：command 逐字段等价");
                assert_eq!(stdio.args, expected.2, "M2：args 逐字段等价");
                assert_eq!(stdio.env, expected.3, "M3：env 为 loader 已解析值（${{env:VAR}} 已展开）");
            }
            other => panic!("M2 违约：inherit 只允许 stdio，实际 {other:?}"),
        }
    }

    // 显式 []：禁用，绝不回落 inherit。
    let disabled = resolve_mcp_servers_in(&base_spec(Some(Vec::new())), &workspace)
        .expect("显式 [] 必须成功");
    assert!(
        disabled.is_empty(),
        "M4 违约：显式 [] 必须得到空列表（不得回落 inherit），实际 {disabled:?}"
    );

    // 显式条目：仅用条目（不叠加 inherit）。
    let explicit = resolve_mcp_servers_in(
        &base_spec(Some(vec![stdio_entry("only", "node", &["srv.js"], &[])])),
        &workspace,
    )
    .expect("显式条目必须成功");
    assert_eq!(explicit.len(), 1, "M4 违约：显式条目不得叠加 inherit，实际 {explicit:?}");
    match &explicit[0] {
        McpServer::Stdio(stdio) => assert_eq!(stdio.name, "only"),
        other => panic!("M4：显式 stdio 条目类型错误：{other:?}"),
    }
}

// ───────────────────────── B. 真实 session/new 报文逐字段等价（M2/M4） ─────────────────────────

/// B：显式 stdio/http/sse 三变体必须原样出现在真实 `session/new.mcpServers`。
#[test]
fn b1_explicit_entries_reach_real_session_new_wire() {
    let cwd = temp_dir("wire-three-variants");
    let spec = spec_with_capture(
        base_spec(Some(vec![
            stdio_entry("files", "npx", &["-y", "@mcp/files"], &[("TOKEN", "abc")]),
            http_entry("docs", "https://example.test/mcp", &[("X-Tenant", "t1")]),
            sse_entry("events", "https://example.test/sse"),
        ])),
        &cwd,
    );

    let resolved = resolve_mcp_servers_in(&spec, &cwd).expect("三变体 resolve 必须成功");
    assert_eq!(resolved.len(), 3, "显式三变体必须逐条解析");

    let mut session = match AcpAgentSession::spawn_with_mcp(&spec, &cwd, resolved.clone()) {
        Ok(session) => session,
        Err(error) => panic!("spawn_with_mcp 必须成功（fake agent 可用）：{error:?}"),
    };
    let wire = read_session_new(&cwd);

    // typed 等价：wire 的 mcpServers 必须与 resolve 结果逐字段一致。
    assert_eq!(
        wire.mcp_servers, resolved,
        "M4 违约：session/new 的 mcpServers 必须与解析结果 typed 等价，实际 {:?}",
        wire.mcp_servers
    );

    // 判别键与字段面（防 camelCase/枚举值漂移）。
    let raw = std::fs::read_to_string(cwd.join("session-new.jsonl")).expect("读取捕获报文");
    let frame: Value = serde_json::from_str(raw.lines().next().expect("行缺失")).expect("合法 JSON");
    let params = frame["params"].clone();
    let entries = params["mcpServers"].as_array().expect("mcpServers 必须是数组");
    assert_eq!(entries.len(), 3);
    assert!(
        entries[0].get("type").is_none(),
        "M4：stdio 变体无 `type` 判别键（http/sse 才有），实际 {}",
        entries[0]
    );
    assert_eq!(entries[0]["command"], json!("npx"));
    assert_eq!(entries[0]["env"][0]["name"], json!("TOKEN"));
    assert_eq!(entries[1]["type"], json!("http"), "http 判别键缺失/错误：{}", entries[1]);
    assert_eq!(entries[1]["url"], json!("https://example.test/mcp"));
    assert_eq!(entries[1]["headers"][0]["name"], json!("X-Tenant"));
    assert_eq!(entries[2]["type"], json!("sse"), "sse 判别键缺失/错误：{}", entries[2]);
    assert_eq!(entries[2]["url"], json!("https://example.test/sse"));
    for entry in entries {
        assert!(
            entry.get("autoApprove").is_none(),
            "M4：autoApprove 是 Pony 策略提示，不得进 wire，实际 {entry}"
        );
    }

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── C. env deny fail-closed（M5） ─────────────────────────

/// C：显式条目 env 命中 deny 表 ⇒ `acp_config_invalid` 整单拒绝（不得静默过滤），
/// 且错误消息只含键名、不含值；session 边界保真映射。
#[test]
fn c1_denied_env_key_fails_closed_without_leaking_value() {
    let cwd = temp_dir("env-deny");
    for (denied_key, secret) in [
        ("PATH", "SECRET-VALUE-PATH"),
        ("LD_PRELOAD", "SECRET-VALUE-LD"),
        ("NODE_OPTIONS", "SECRET-VALUE-NODE"),
    ] {
        let spec = base_spec(Some(vec![stdio_entry(
            "evil",
            "node",
            &["srv.js"],
            &[("SAFE_KEY", "ok"), (denied_key, secret)],
        )]));
        let error = match resolve_mcp_servers_in(&spec, &cwd) {
            Ok(servers) => panic!(
                "M5 违约：env 键 `{denied_key}` 必须整单 fail-closed，实际返回 {servers:?}"
            ),
            Err(error) => error,
        };
        assert_eq!(
            code_of(&error),
            "acp_config_invalid",
            "M5/M7 违约：deny 命中必须报 acp_config_invalid，实际 {error:?}"
        );
        assert!(matches!(error, AcpConfigError::Invalid(_)));
        let message = format!("{error:?}");
        assert!(
            message.contains(denied_key),
            "M5：错误消息必须含命中的键名 `{denied_key}`，实际 {message}"
        );
        assert!(
            !message.contains(secret),
            "M5 违约：错误消息绝不回显 env 值，实际 {message}"
        );

        // session 边界：spawn 必须 fail-closed，且消息保留 acp_config_invalid（M7）。
        let spawn_error = match AcpAgentSession::spawn(&spec, &cwd) {
            Ok(mut session) => {
                session.close();
                panic!("M7 违约：deny 配置下 spawn 必须失败，实际成功")
            }
            Err(error) => format!("{error:?}"),
        };
        assert!(
            spawn_error.contains("acp_config_invalid"),
            "M7 违约：session 边界必须保留 acp_config_invalid，实际 {spawn_error}"
        );
    }
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── D. 非法配置明确报错（M6/M7） ─────────────────────────

/// D：畸形 `mcpServers` 在 serde 层被拒（非 panic）；inherit 的致命诊断 fail-closed，
/// 不静默降级为空列表。
#[test]
fn d1_invalid_config_fails_closed_not_silent_empty() {
    // (i) serde：mcpServers 非数组 / http 缺 url ⇒ 明确反序列化错误。
    for bad in [
        json!({"id": "a", "label": "A", "command": "sh", "mcpServers": {"a": {}}}),
        json!({"id": "a", "label": "A", "command": "sh", "mcpServers": [{"name": "h", "type": "http"}]}),
    ] {
        let parsed: Result<AcpAgentSpec, _> = serde_json::from_value(bad.clone());
        assert!(
            parsed.is_err(),
            "D 违约：非法 mcpServers 必须被拒，实际接受 {bad}"
        );
    }

    // (ii) 致命诊断：全局 mcp.json 非法 JSON ⇒ acp_config_parse（不得降级为空）。
    {
        let _env = IsolatedEnv::new("inherit-bad-json", Some("{not-json"));
        let error = match resolve_mcp_servers_in(&base_spec(None), &temp_dir("ws-bad-json")) {
            Ok(servers) => panic!("M6 违约：非法 JSON 的 inherit 配置必须 fail-closed，实际 {servers:?}"),
            Err(error) => error,
        };
        assert_eq!(
            code_of(&error),
            "acp_config_parse",
            "M6/M7 违约：invalid_json 必须报 acp_config_parse，实际 {error:?}"
        );
    }

    // (iii) 不支持的 schemaVersion ⇒ acp_config_invalid（文件级致命，不降级）。
    {
        let _env = IsolatedEnv::new(
            "inherit-bad-schema",
            Some(&json!({"schemaVersion": "mcp-config-v9", "mcpServers": {}}).to_string()),
        );
        let error = match resolve_mcp_servers_in(&base_spec(None), &temp_dir("ws-bad-schema")) {
            Ok(servers) => panic!("M6 违约：不支持的 schemaVersion 必须 fail-closed，实际 {servers:?}"),
            Err(error) => error,
        };
        assert_eq!(
            code_of(&error),
            "acp_config_invalid",
            "M6/M7 违约：unsupported_schema_version 必须报 acp_config_invalid，实际 {error:?}"
        );
    }

    // (iv) 显式条目缺 command / 空 name ⇒ Invalid（非 panic）。
    for bad_entry in [
        stdio_entry("no-command", "", &[], &[]),
        stdio_entry("", "node", &[], &[]),
        http_entry("bad-http", "", &[]),
    ] {
        let error = match resolve_mcp_servers_in(&base_spec(Some(vec![bad_entry])), &temp_dir("ws-bad-entry")) {
            Ok(servers) => panic!("D 违约：非法显式条目必须报错，实际 {servers:?}"),
            Err(error) => error,
        };
        assert_eq!(
            code_of(&error),
            "acp_config_invalid",
            "D/M7 违约：非法显式条目必须报 acp_config_invalid，实际 {error:?}"
        );
    }
}

// ───────────────────────── E. inherit 无配置 ⇒ 空列表（不得 panic） ─────────────────────────

/// E：无 Pony MCP 配置（无全局 mcp.json、无工作区 .mcp.json）时，inherit 得到空列表；
/// 真实 `session/new` 报文中 `mcpServers` 必须是空数组，且 spawn 不 panic。
#[test]
fn e1_inherit_without_pony_config_yields_empty_list() {
    let env = IsolatedEnv::new("inherit-empty", None);
    let cwd = env.workspace();
    std::fs::create_dir_all(env.root().join("pony-agent-home")).expect("home dir");

    let resolved = resolve_mcp_servers(&base_spec(None)).expect("无配置的 inherit 必须成功");
    assert!(
        resolved.is_empty(),
        "E 违约：无 Pony MCP 配置时 inherit 必须为空列表，实际 {resolved:?}"
    );

    // 真实 session/new：spawn 内部走 inherit 路径（签名不变）。
    let spec = spec_with_capture(base_spec(None), &cwd);
    let mut session = match AcpAgentSession::spawn(&spec, &cwd) {
        Ok(session) => session,
        Err(error) => panic!("E 违约：无 MCP 配置时 spawn 不得失败/panic，实际 {error:?}"),
    };
    let wire = read_session_new(&cwd);
    assert!(
        wire.mcp_servers.is_empty(),
        "E 违约：session/new.mcpServers 必须是空数组，实际 {:?}",
        wire.mcp_servers
    );

    let raw = std::fs::read_to_string(cwd.join("session-new.jsonl")).expect("读取捕获报文");
    let frame: Value = serde_json::from_str(raw.lines().next().expect("行缺失")).expect("合法 JSON");
    assert!(
        frame["params"]["mcpServers"].is_array(),
        "E 违约：wire 必须显式携带 mcpServers 数组（即使为空），实际 {frame}"
    );

    session.close();
}
