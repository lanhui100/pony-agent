//! ACP P2 宿主形态（`ponycli acp`）黑盒契约（B151-P2-T，冻结于 task-6）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §4 P2 表 / §5 P2 阶段；
//! CLI 面由 Lead 2026-11-14 冻结：
//! `ponycli acp [--agent <id>] [--list] [-p|--prompt <text>] [--output-format jsonl|sse]
//!              [--workspace <dir>] [--timeout-secs <n>] [--resume <session-id>]`
//!
//! 黑盒纪律：
//! - 走**真实二进制** `env!("CARGO_BIN_EXE_ponycli")`（与 `session_bundle_export_import.rs` 同惯例）；
//! - 配置发现只走冻结契约：`$XDG_CONFIG_HOME/pony-agent/acp-agents.json`（`acp/config.rs:128`），
//!   同时隔离 HOME / XDG_DATA_HOME / XDG_STATE_HOME / XDG_CACHE_HOME，避免污染真实用户配置；
//! - 每个子进程都有看门狗（`request_timeout_ms * 2`，NFR 派生）：挂起即失败，而非让测试套件卡死；
//! - 无 `#[ignore]`、无空断言；红相必须因 CLI 尚未实现而真实失败。

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ───────────────────────────── 隔离与看门狗 ─────────────────────────────

fn unique_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pony-acp-p2-{}-{}-{tag}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("创建隔离根目录");
    root
}

/// NFR 唯一真源：看门狗上界 = `acp.request_timeout_ms * 2`（派生，不自选数值）。
fn cli_timeout() -> Duration {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.dev-team/nfr-baseline.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 NFR 真源失败 {}: {error}", path.display()));
    let parsed: Value = serde_json::from_str(&body).expect("nfr-baseline.json 必须是合法 JSON");
    let ms = parsed
        .get("acp")
        .and_then(|acp| acp.get("request_timeout_ms"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 acp.request_timeout_ms"));
    Duration::from_millis(ms * 2)
}

/// 隔离的 CLI 运行环境（独立 HOME / XDG_* / workspace；Drop 时整目录回收）。
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

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }

    fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    /// 按冻结契约注入 ACP agent 清单：`<XDG_CONFIG_HOME>/pony-agent/acp-agents.json`（裸数组）。
    fn write_agents_file(&self, specs: Value) {
        let dir = self.config_dir().join("pony-agent");
        std::fs::create_dir_all(&dir).expect("创建 pony-agent 配置目录");
        std::fs::write(
            dir.join("acp-agents.json"),
            serde_json::to_string_pretty(&specs).expect("清单必须可序列化"),
        )
        .expect("写入 acp-agents.json");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ponycli"));
        command
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.config_dir())
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

/// 有界执行：stdin 关闭（防 REPL 等输入挂死），超时 kill 并附上原始输出作为失败证据。
fn run_cli(command: &mut Command, timeout: Duration) -> Output {
    let label = format!("{command:?}");
    command.stdin(Stdio::null());
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ponycli 必须可执行（CARGO_BIN_EXE_ponycli）");
    let started = Instant::now();
    loop {
        match child.try_wait().expect("try_wait ponycli") {
            Some(_) => break,
            None => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let output = child.wait_with_output().expect("收集超时 CLI 输出");
                    panic!(
                        "CLI 未在 {timeout:?} 内退出（不得挂起/等输入）\ncmd: {label}\nstdout:\n{}\nstderr:\n{}",
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

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_no_panic(output: &Output, case: &str) {
    let combined = format!("{}{}", stdout_text(output), stderr_text(output));
    assert!(
        !combined.contains("panicked at") && !combined.contains("RUST_BACKTRACE"),
        "{case}：CLI 不得 panic，实际输出：\n{combined}"
    );
}

fn assert_exit_nonzero(output: &Output, case: &str) -> i32 {
    let code = output.status.code().unwrap_or_else(|| {
        panic!("{case}：进程被信号终止而非以退出码结束，stdout:\n{}\nstderr:\n{}",
            stdout_text(output), stderr_text(output))
    });
    assert_ne!(code, 0, "{case}：必须非 0 退出，实际 Exit=0，stdout:\n{}\nstderr:\n{}",
        stdout_text(output), stderr_text(output));
    code
}

fn fixture_script() -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script.to_string_lossy().into_owned()
}

/// `updates` 模式：initialize/session/new 正常应答，prompt 期间发
/// agent_message_chunk(text="hello") / agent_thought_chunk / tool_call，最后 `end_turn`；
/// initialize 响应固定声明 `loadSession=false`（用于 P2 恢复降级用例）。
fn fake_updates_spec() -> Value {
    json!([{
        "id": "fake",
        "label": "Fake ACP agent",
        "command": "sh",
        "args": [fixture_script(), "updates", "-", "acp-sess-1"]
    }])
}

// ───────────────────────── P2-1 `acp --list` ─────────────────────────

/// P2-1：`acp --list` 必须 Exit 0，stdout **每行一个 JSON 对象**
/// （`{id,label,command,args,builtin}`），含内置 opencode / antigravity 且 `builtin=true`。
#[test]
fn p2_01_acp_list_prints_builtin_specs_as_json_lines() {
    let env = IsolatedCli::new("list");
    // 不写 acp-agents.json：隔离环境等价于「缺文件」，必须回落到内置两条默认。
    let output = run_cli(env.command().args(["acp", "--list"]), cli_timeout());
    assert_eq!(
        output.status.code(),
        Some(0),
        "P2-1：`acp --list` 必须 Exit 0\nstdout:\n{}\nstderr:\n{}",
        stdout_text(&output),
        stderr_text(&output)
    );

    let stdout = stdout_text(&output);
    let mut specs: Vec<Value> = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("P2-1 违约：--list stdout 每行必须是合法 JSON（{error}）：{line}")
        });
        assert!(
            value.is_object(),
            "P2-1 违约：--list 每行必须是 JSON 对象，实际 {value}"
        );
        for key in ["id", "label", "command", "args", "builtin"] {
            assert!(
                value.get(key).is_some(),
                "P2-1 违约：--list 行缺字段 `{key}`，实际 {value}"
            );
        }
        assert!(
            value["builtin"].is_boolean(),
            "P2-1 违约：builtin 必须是 bool，实际 {value}"
        );
        specs.push(value);
    }

    let opencode = specs
        .iter()
        .find(|spec| spec["id"] == json!("opencode"))
        .unwrap_or_else(|| panic!("P2-1 违约：--list 必须含 opencode，实际 {specs:?}"));
    assert_eq!(opencode["builtin"], json!(true), "P2-1：opencode 是内置项");
    assert_eq!(opencode["command"], json!("opencode"), "P2-1：opencode 启动命令");
    assert_eq!(opencode["args"], json!(["acp"]), "P2-1：opencode 启动参数");
    assert!(opencode["label"].is_string(), "P2-1：label 必须是字符串");

    let antigravity = specs
        .iter()
        .find(|spec| spec["id"] == json!("antigravity"))
        .unwrap_or_else(|| panic!("P2-1 违约：--list 必须含 antigravity，实际 {specs:?}"));
    assert_eq!(antigravity["builtin"], json!(true));
    assert_eq!(antigravity["command"], json!("agy"), "P2-1：antigravity 启动命令");
    assert_eq!(antigravity["args"], json!(["acp"]));
}

// ───────────────────────── P2-2 未知 agent / 缺 --agent ─────────────────────────

/// P2-2：`--agent` 不存在必须 Exit≠0 且 stderr 含 acp_agent_not_found（或 acp_unavailable），
/// 且不得 panic；非 `--list` 时缺 `--agent` 同样必须显式失败（不得静默进入默认流程）。
#[test]
fn p2_02_unknown_or_missing_agent_fails_with_error_code() {
    let env = IsolatedCli::new("unknown-agent");
    let timeout = cli_timeout();

    let unknown = run_cli(env.command().args(["acp", "--agent", "nope"]), timeout);
    let code = assert_exit_nonzero(&unknown, "P2-2 未知 agent");
    assert_no_panic(&unknown, "P2-2 未知 agent");
    let stderr = stderr_text(&unknown);
    assert!(
        stderr.contains("acp_agent_not_found")
            || stderr.contains("acp_unavailable")
            || stderr.contains("acp_config_agent_not_found"),
        "P2-2 违约：未知 agent 的 stderr 必须含 acp_agent_not_found / acp_unavailable（Exit={code}），实际:\n{stderr}"
    );

    // 缺 --agent 且非 --list：必须显式失败（不许静默跑默认流程）。
    let missing = run_cli(env.command().arg("acp"), timeout);
    assert_exit_nonzero(&missing, "P2-2 缺 --agent");
    assert_no_panic(&missing, "P2-2 缺 --agent");
}

// ───────────────────────── P2-3 单轮 prompt + JsonlSink ─────────────────────────

/// P2-3：`--agent fake -p <text> --output-format jsonl` 必须 Exit 0，
/// stdout **每行合法 JSON**，且含 `"type":"turn:delta"`（载荷 text="hello" 来自 fake agent）。
#[test]
fn p2_03_single_turn_emits_jsonl_turn_delta() {
    let env = IsolatedCli::new("single-turn");
    env.write_agents_file(fake_updates_spec());
    let workspace = env.workspace();

    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "-p",
            "run-the-task",
            "--output-format",
            "jsonl",
            "--workspace",
            workspace.to_string_lossy().as_ref(),
            "--timeout-secs",
            "10",
        ]),
        cli_timeout(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "P2-3：单轮 ACP prompt 必须 Exit 0\nstdout:\n{}\nstderr:\n{}",
        stdout_text(&output),
        stderr_text(&output)
    );
    assert_no_panic(&output, "P2-3 单轮");

    let stdout = stdout_text(&output);
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "P2-3 违约：jsonl 模式 stdout 必须承载事件行，实际为空；stderr:\n{}",
        stderr_text(&output)
    );

    let mut events: Vec<Value> = Vec::new();
    for line in &lines {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("P2-3 违约：jsonl stdout 每行必须是合法 JSON（{error}）：{line}")
        });
        events.push(value);
    }

    let delta = events
        .iter()
        .find(|event| {
            event["type"] == json!("turn:delta") && event["text"] == json!("hello")
        })
        .unwrap_or_else(|| {
            panic!(
                "P2-3 违约：必须出现 turn:delta 且 text=\"hello\"（fake agent 的 agent_message_chunk），实际事件：{events:?}"
            )
        });
    assert!(
        delta["type"] == json!("turn:delta"),
        "P2-3：delta 事件类型判定错误"
    );
}

// ───────────────────────── P2-4 `--help` 含 acp 段 ─────────────────────────

/// P2-4：`acp --help` 必须 Exit 0，且 usage 文本含 acp 段（`--agent` / `--list` 等冻结拼写）。
#[test]
fn p2_04_acp_help_documents_acp_surface() {
    let env = IsolatedCli::new("help");
    let output = run_cli(env.command().args(["acp", "--help"]), cli_timeout());
    assert_eq!(
        output.status.code(),
        Some(0),
        "P2-4：`acp --help` 必须 Exit 0\nstdout:\n{}\nstderr:\n{}",
        stdout_text(&output),
        stderr_text(&output)
    );
    let combined = format!("{}{}", stdout_text(&output), stderr_text(&output));
    assert!(
        combined.contains("acp"),
        "P2-4 违约：usage 必须包含 acp 子命令段，实际:\n{combined}"
    );
    for flag in ["--agent", "--list", "--resume"] {
        assert!(
            combined.contains(flag),
            "P2-4 违约：usage 的 acp 段必须列出 `{flag}`（Lead 冻结的 CLI 面），实际:\n{combined}"
        );
    }
}

// ───────────────────────── P2-5 session/load 降级 ─────────────────────────

/// P2-5：agent 声明 `loadSession=false` 时，`--resume` 必须显式降级（Exit≠0 + `acp_unavailable`），
/// 不得静默把它当新会话继续跑。
#[test]
fn p2_05_resume_without_load_session_fails_closed() {
    let env = IsolatedCli::new("resume");
    env.write_agents_file(fake_updates_spec());
    let workspace = env.workspace();

    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "--resume",
            "acp-sess-1",
            "-p",
            "continue-the-task",
            "--output-format",
            "jsonl",
            "--workspace",
            workspace.to_string_lossy().as_ref(),
            "--timeout-secs",
            "10",
        ]),
        cli_timeout(),
    );
    let code = assert_exit_nonzero(&output, "P2-5 恢复降级");
    assert_no_panic(&output, "P2-5 恢复降级");
    let stderr = stderr_text(&output);
    assert!(
        stderr.contains("acp_unavailable"),
        "P2-5 违约：loadSession=false 时必须显式报 acp_unavailable（Exit={code}），实际 stderr:\n{stderr}"
    );
    // 不得静默当新会话：jsonl 事件流不得出现 turn:delta（那意味着它悄悄跑了一轮新会话）。
    let stdout = stdout_text(&output);
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            assert_ne!(
                value["type"],
                json!("turn:delta"),
                "P2-5 违约：不得静默当新会话继续执行（stdout 出现 turn:delta）：{line}"
            );
        }
    }
}
