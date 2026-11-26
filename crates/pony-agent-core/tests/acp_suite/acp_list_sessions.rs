//! B185：`ponycli acp --list-sessions` JSONL 契约（L1–L7）。
//!
//! 真源：`.dev-team/contract-matrix-acp-list-sessions.md`。
//! 断言全部走真实 `ponycli` 子进程 + fake agent 落盘的原始请求帧；
//! 输出面严格按冻结 JSONL：每会话一行、可选字段缺省不写 null、nextCursor 独立最后一行、空列表零行。
//!
//! 纪律：无 `#[ignore]`、无空断言；只碰 ACP 测试与夹具。

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

fn unique_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pony-acp-list-{}-{}-{tag}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("创建隔离根目录");
    root
}

fn fixture_script() -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script.to_string_lossy().into_owned()
}

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

fn watchdog() -> Duration {
    Duration::from_millis(nfr_acp_ms("request_timeout_ms") * 4)
}

struct IsolatedCli {
    root: PathBuf,
}

impl IsolatedCli {
    fn new(tag: &str) -> Self {
        let root = unique_root(tag);
        for child in ["home", "config", "data", "state", "cache", "workspace"] {
            std::fs::create_dir_all(root.join(child)).expect("创建隔离子目录");
        }
        Self { root }
    }

    fn with_agent(tag: &str, spec: Value) -> Self {
        let env = Self::new(tag);
        let dir = env.root.join("config").join("pony-agent");
        std::fs::create_dir_all(&dir).expect("创建 pony-agent 配置目录");
        std::fs::write(
            dir.join("acp-agents.json"),
            serde_json::to_string_pretty(&json!([spec])).expect("清单可序列化"),
        )
        .expect("写入 acp-agents.json");
        env
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(pony_test_support::ponycli_bin());
        command
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"));
        command
    }
}

impl Drop for IsolatedCli {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn run_cli(command: &mut Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    let label = format!("{command:?}");
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ponycli 必须可执行");
    let started = Instant::now();
    loop {
        match child.try_wait().expect("try_wait ponycli") {
            Some(_) => break,
            None => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let output = child.wait_with_output().expect("收集超时输出");
                    panic!(
                        "CLI 未在 {timeout:?} 内退出\ncmd: {label}\nstdout:\n{}\nstderr:\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    child.wait_with_output().expect("收集 ponycli 输出")
}

fn stdout_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn stdout_json_lines(output: &Output) -> Vec<Value> {
    stdout_lines(output)
        .into_iter()
        .map(|line| {
            serde_json::from_str::<Value>(&line)
                .unwrap_or_else(|error| panic!("stdout 必须每行合法 JSON（{error}）：{line}"))
        })
        .collect()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn captured_requests(path: &Path) -> Vec<Value> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("捕获行必须是合法 JSON"))
        .collect()
}

fn methods(path: &Path) -> Vec<String> {
    captured_requests(path)
        .into_iter()
        .filter_map(|frame| frame["method"].as_str().map(str::to_string))
        .collect()
}

fn list_sessions_spec(mode: &str, req_file: &Path, extra: Option<&str>) -> Value {
    let mut args = vec![
        fixture_script(),
        mode.to_string(),
        req_file.to_string_lossy().into_owned(),
    ];
    if let Some(extra) = extra {
        args.push(extra.to_string());
    }
    json!({"id": "fake", "label": "Fake ACP agent", "command": "sh", "args": args})
}

fn list_sessions_env(tag: &str, mode: &str, extra: Option<&str>) -> (IsolatedCli, PathBuf) {
    let req_file = std::path::Path::new(&std::env::temp_dir())
        .join(format!("pony-acp-list-req-{}-{tag}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&req_file);
    (
        IsolatedCli::with_agent(tag, list_sessions_spec(mode, &req_file, extra)),
        req_file,
    )
}

// ───────────────────────── L1 两行输出（缺省字段不写 null） ─────────────────────────

/// L1：2 个会话 ⇒ stdout 恰好 2 行 JSON，字段一一对应；可选字段缺省时**不出现该键**（不写 null）。
#[test]
fn l1_list_sessions_prints_one_json_line_per_session() {
    let (env, req_file) = list_sessions_env("l1", "list_sessions_ok", None);
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "--workspace",
            workspace_text.as_str(),
        ]),
        watchdog(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "L1 违约：--list-sessions 必须 Exit 0\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr_text(&output)
    );

    let lines = stdout_json_lines(&output);
    assert_eq!(lines.len(), 2, "L1 违约：2 个会话必须恰好 2 行，实际 {lines:?}");

    let first = &lines[0];
    assert_eq!(first["sessionId"], json!("sess-a"), "L1：sessionId 必含");
    assert_eq!(first["cwd"], json!("/tmp/ws"), "L1：cwd 必含");
    assert_eq!(first["title"], json!("Alpha"), "L1：title 必须写出");
    assert_eq!(
        first["updatedAt"],
        json!("2026-11-15T00:00:00Z"),
        "L1：updatedAt 必须写出"
    );
    assert_eq!(
        first["additionalDirectories"],
        json!(["/tmp/extra"]),
        "L1：additionalDirectories 必须写出"
    );

    let second = &lines[1];
    assert_eq!(second["sessionId"], json!("sess-b"), "L1：第二行 sessionId");
    assert_eq!(second["cwd"], json!("/tmp/ws2"), "L1：第二行 cwd");
    for optional in ["title", "updatedAt", "additionalDirectories"] {
        assert!(
            second.get(optional).is_none(),
            "L1 违约：缺省字段 `{optional}` 不得写 null/出现，实际 {second}"
        );
    }
    // 全量守卫：任何一行都不得出现显式 null 值。
    for line in &lines {
        for (key, value) in line.as_object().expect("行必须是对象") {
            assert!(
                !value.is_null(),
                "L1 违约：字段 `{key}` 不得写 null，实际 {line}"
            );
        }
    }

    let _ = std::fs::remove_file(&req_file);
}

// ───────────────────────── L2 nextCursor 行 ─────────────────────────

/// L2：响应带 `nextCursor` ⇒ 会话行之后**额外一行** `{"nextCursor":"..."}`（最后一行）。
#[test]
fn l2_next_cursor_is_an_extra_last_line() {
    let (env, req_file) = list_sessions_env("l2", "list_sessions_cursor", None);
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "--workspace",
            workspace_text.as_str(),
        ]),
        watchdog(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "L2 违约：带 nextCursor 的 --list-sessions 必须 Exit 0\nstderr:\n{}",
        stderr_text(&output)
    );
    let lines = stdout_json_lines(&output);
    assert_eq!(
        lines.len(),
        2,
        "L2 违约：1 个会话 + 1 行 nextCursor ⇒ 恰好 2 行，实际 {lines:?}"
    );
    assert_eq!(
        lines[0]["sessionId"],
        json!("sess-a"),
        "L2：第一行必须是会话行，实际 {lines:?}"
    );
    assert_eq!(
        lines[1],
        json!({"nextCursor": "cur-2"}),
        "L2 违约：nextCursor 必须是独立最后一行（且只含 nextCursor 键），实际 {lines:?}"
    );

    let _ = std::fs::remove_file(&req_file);
}

// ───────────────────────── L3 -32601 降级 ─────────────────────────

/// L3：fake agent 对 `session/list` 回 -32601 ⇒ Exit≠0、stderr 含 `acp_method_not_supported`、
/// **stdout 零行**（不得静默空列表、不得伪造行）。
#[test]
fn l3_method_not_supported_is_explicit_error_not_empty_list() {
    let (env, req_file) = list_sessions_env("l3", "list_sessions_unsupported", None);
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "--workspace",
            workspace_text.as_str(),
        ]),
        watchdog(),
    );
    let stderr = stderr_text(&output);
    assert_ne!(
        output.status.code(),
        Some(0),
        "L3 违约：-32601 必须非 0 退出（不得静默当空列表），实际 Exit=0\nstdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("acp_method_not_supported"),
        "L3 违约：stderr 必须含 acp_method_not_supported，实际 {stderr}"
    );
    assert!(
        stdout_lines(&output).is_empty(),
        "L3 违约：stdout 必须零行（不得伪造会话行），实际 {:?}",
        stdout_lines(&output)
    );

    let _ = std::fs::remove_file(&req_file);
}

// ───────────────────────── L4 不建会话 ─────────────────────────

/// L4：`--list-sessions` 全程 agent 侧**零** `session/new`（也零 `session/load`/`session/prompt`）。
#[test]
fn l4_list_sessions_never_creates_a_session() {
    let (env, req_file) = list_sessions_env("l4", "list_sessions_ok", None);
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "--workspace",
            workspace_text.as_str(),
        ]),
        watchdog(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "L4 前置：--list-sessions 必须 Exit 0\nstderr:\n{}",
        stderr_text(&output)
    );
    assert!(
        req_file.exists(),
        "L4 违约：agent 必须被真实连接并收到 session/list（捕获文件缺失：{}）",
        req_file.display()
    );
    let seen = methods(&req_file);
    assert!(
        seen.iter().any(|method| method == "session/list"),
        "L4 违约：agent 侧必须收到 session/list，实际 {seen:?}"
    );
    for forbidden in ["session/new", "session/load", "session/prompt"] {
        assert!(
            !seen.iter().any(|method| method == forbidden),
            "L4 违约：--list-sessions 绝不能出现 `{forbidden}` 帧，实际 {seen:?}"
        );
    }

    let _ = std::fs::remove_file(&req_file);
}

// ───────────────────────── L5 空列表 ─────────────────────────

/// L5：`sessions: []` ⇒ Exit 0 且 stdout **零行**（不打印 `[]`、不打印 nextCursor）。
#[test]
fn l5_empty_session_list_prints_nothing() {
    let (env, req_file) = list_sessions_env("l5", "list_sessions_ok", Some("empty"));
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "--workspace",
            workspace_text.as_str(),
        ]),
        watchdog(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "L5 违约：空列表必须 Exit 0\nstderr:\n{}",
        stderr_text(&output)
    );
    assert!(
        stdout_lines(&output).is_empty(),
        "L5 违约：空列表必须 stdout 零行（不得打印 [] 或 nextCursor），实际 {:?}",
        stdout_lines(&output)
    );

    let _ = std::fs::remove_file(&req_file);
}

// ───────────────────────── L6 语义不混淆 / 互斥 ─────────────────────────

/// L6：`--list` 仍是 agent 清单（结构不同）；`--list` + `--list-sessions`、缺 `--agent`、
/// 与 `-p` 混用 ⇒ Usage(2)。
#[test]
fn l6_list_and_list_sessions_semantics_do_not_mix() {
    let env = IsolatedCli::new("l6");
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();

    // (a) `--list` 输出 agent 清单结构（与 --list-sessions 的 session 结构不同）。
    let listed = run_cli(env.command().args(["acp", "--list"]), watchdog());
    assert_eq!(
        listed.status.code(),
        Some(0),
        "L6 违约：`acp --list` 必须 Exit 0\nstderr:\n{}",
        stderr_text(&listed)
    );
    let agents = stdout_json_lines(&listed);
    assert!(!agents.is_empty(), "L6：--list 至少列出内置 agent");
    for agent in &agents {
        for key in ["id", "label", "command", "args", "builtin"] {
            assert!(
                agent.get(key).is_some(),
                "L6 违约：--list 行必须含 `{key}`（agent 清单结构），实际 {agent}"
            );
        }
        assert!(
            agent.get("sessionId").is_none(),
            "L6 违约：--list 不得输出会话结构（出现 sessionId），实际 {agent}"
        );
    }

    // (b) `--list` + `--list-sessions` 互斥 ⇒ Usage(2)。
    let mixed = run_cli(
        env.command()
            .args(["acp", "--list", "--list-sessions"]),
        watchdog(),
    );
    assert_eq!(
        mixed.status.code(),
        Some(2),
        "L6 违约：--list 与 --list-sessions 混用必须 Usage(2)\nstderr:\n{}",
        stderr_text(&mixed)
    );

    // (c) `--list-sessions` 缺 `--agent` ⇒ Usage(2)。
    let missing_agent = run_cli(
        env.command()
            .args(["acp", "--list-sessions", "--workspace", workspace_text.as_str()]),
        watchdog(),
    );
    assert_eq!(
        missing_agent.status.code(),
        Some(2),
        "L6 违约：--list-sessions 缺 --agent 必须 Usage(2)\nstderr:\n{}",
        stderr_text(&missing_agent)
    );

    // (d) `--list-sessions` 与 `-p` 互斥 ⇒ Usage(2)。
    // 注意：CLI 先解析 `--agent`（spec 解析失败会先行 Exit 3），故此处必须配置 fake agent。
    let (prompt_env, _prompt_req) = list_sessions_env("l6-prompt", "list_sessions_ok", None);
    let workspace = prompt_env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let with_prompt = run_cli(
        prompt_env.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "-p",
            "hi",
            "--workspace",
            workspace_text.as_str(),
        ]),
        watchdog(),
    );
    assert_eq!(
        with_prompt.status.code(),
        Some(2),
        "L6 违约：--list-sessions 与 -p 混用必须 Usage(2)\nstderr:\n{}",
        stderr_text(&with_prompt)
    );
}

// ───────────────────────── L7 分页参数 ─────────────────────────

/// L7：`--session-limit 0` / 非整数 ⇒ Usage(2)；`--session-cursor abc --session-limit 2`
/// ⇒ 请求报文含对应 cursor/limit。
#[test]
fn l7_session_paging_parameters() {
    let env = IsolatedCli::new("l7-usage");
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();

    for bad in ["0", "abc", "-1"] {
        let output = run_cli(
            env.command().args([
                "acp",
                "--agent",
                "fake",
                "--list-sessions",
                "--session-limit",
                bad,
                "--workspace",
                workspace_text.as_str(),
            ]),
            watchdog(),
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "L7 违约：--session-limit {bad} 必须 Usage(2)，实际 Exit={:?}\nstderr:\n{}",
            output.status.code(),
            stderr_text(&output)
        );
    }

    // 合法分页参数：请求报文必须带 cursor/limit。
    let (env2, req_file) = list_sessions_env("l7-capture", "list_sessions_ok", None);
    let workspace2 = env2.workspace();
    let workspace2_text = workspace2.to_string_lossy().into_owned();
    let output = run_cli(
        env2.command().args([
            "acp",
            "--agent",
            "fake",
            "--list-sessions",
            "--session-cursor",
            "abc",
            "--session-limit",
            "2",
            "--workspace",
            workspace2_text.as_str(),
        ]),
        watchdog(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "L7 违约：合法分页参数必须 Exit 0\nstderr:\n{}",
        stderr_text(&output)
    );
    let list_frame = captured_requests(&req_file)
        .into_iter()
        .find(|frame| frame["method"] == json!("session/list"))
        .unwrap_or_else(|| {
            panic!(
                "L7 违约：agent 必须收到 session/list（捕获 {}）",
                req_file.display()
            )
        });
    assert_eq!(
        list_frame["params"]["cursor"],
        json!("abc"),
        "L7 违约：--session-cursor 必须原样透传，实际 {list_frame}"
    );
    assert_eq!(
        list_frame["params"]["limit"],
        json!(2),
        "L7 违约：--session-limit 必须原样透传，实际 {list_frame}"
    );

    let _ = std::fs::remove_file(&req_file);
}
