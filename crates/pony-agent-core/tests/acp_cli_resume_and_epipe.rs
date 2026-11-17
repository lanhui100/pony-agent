//! B184：SIGPIPE（E1）与 CLI `--resume`（R1–R4）端到端契约。
//!
//! 真源：`.dev-team/contract-matrix-acp-cli-resume.md`。
//!
//! EPIPE 口径（Lead 2026-11-15 裁决）：实现选择「写入忽略 BrokenPipe、安静退出 0」，因此
//! E1 断言退出码 **0**（修复前为 panic 101），不要求按信号终止（避免 141 语义歧义）。
//!
//! 断言全部走真实 `ponycli` 子进程 + fake agent 落盘的原始请求帧；
//! R1/R2 额外断言 agent 侧**没有** `session/new` 帧（防「先建后载」假绿）。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

fn unique_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pony-acp-resume-{}-{}-{tag}",
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

/// 隔离的 ponycli 运行环境（独立 HOME/XDG；可注入 fake agent 清单）。
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_ponycli"));
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

// ───────────────────────── E1 SIGPIPE（`| head -1`） ─────────────────────────

/// 模拟 `ponycli <args> | head -1`：读一行后关闭读端（drop stdout），子进程必须安静退出 0。
fn early_close_pipe_exit_code(env: &IsolatedCli, args: &[&str]) -> Option<i32> {
    let mut child = env
        .command()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ponycli 必须可执行");
    let stdout = child.stdout.take().expect("stdout pipe");
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    let _ = reader.read_line(&mut first_line);
    // 关键：读一行即关闭读端 —— 后续写入触发 EPIPE（等价 `| head -1`）。
    drop(reader);
    // stderr 也要读掉并关闭，避免子进程因 stderr 满而阻塞（影响退出码判定）。
    if let Some(stderr) = child.stderr.take() {
        let mut stderr_reader = BufReader::new(stderr);
        let mut sink = String::new();
        let _ = stderr_reader.read_to_string(&mut sink);
    }
    let status = child.wait().expect("wait ponycli");
    status.code()
}

/// E1：`ponycli models list | head -1` 与 `ponycli acp --list | head -1` 必须安静退出 0
///（修复前：写 EPIPE 触发 panic ⇒ 退出码 101）。
#[test]
fn e1_early_pipe_close_exits_zero_not_panic() {
    let env = IsolatedCli::new("epipe");

    let models_code = early_close_pipe_exit_code(&env, &["models", "list"]);
    assert_eq!(
        models_code,
        Some(0),
        "E1 违约：`ponycli models list | head -1` 在 consumer 提前关管道时必须安静退出 0（修复前是 panic 101），实际 {models_code:?}"
    );

    let acp_list_code = early_close_pipe_exit_code(&env, &["acp", "--list"]);
    assert_eq!(
        acp_list_code,
        Some(0),
        "E1 违约：`ponycli acp --list | head -1` 必须安静退出 0（修复前是 panic 101），实际 {acp_list_code:?}"
    );
}

// ───────────────────────── R1 resume 成功（不新建） ─────────────────────────

fn resume_spec(mode: &str, req_file: &Path, session_id: &str) -> Value {
    json!({
        "id": "fake",
        "label": "Fake ACP agent",
        "command": "sh",
        "args": [
            fixture_script(),
            mode,
            req_file.to_string_lossy(),
            session_id
        ]
    })
}

fn resume_cli_args<'a>(workspace: &'a str, session_id: &'a str) -> Vec<&'a str> {
    vec![
        "acp",
        "--agent",
        "fake",
        "-p",
        "hi",
        "--resume",
        session_id,
        "--output-format",
        "jsonl",
        "--workspace",
        workspace,
        "--timeout-secs",
        "10",
    ]
}

/// R1：agent 支持 loadSession=true ⇒ CLI 必须走 `initialize → session/load → session/prompt`，
/// **绝不**发 `session/new`；stdout 含 turn:delta 且 Exit 0。
#[test]
fn r1_resume_loads_session_without_creating_new_one() {
    let root = unique_root("resume-ok");
    let req_file = root.join("requests.jsonl");
    let env = IsolatedCli::with_agent(
        "resume-ok-agent",
        resume_spec("resume_ok", &req_file, "acp-sess-1"),
    );
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();

    let output = run_cli(
        env.command().args(resume_cli_args(&workspace_text, "acp-sess-1")),
        watchdog(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "R1 违约：loadSession=true 的 --resume 必须 Exit 0\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let seen = methods(&req_file);
    assert!(
        seen.iter().any(|method| method == "session/load"),
        "R1 违约：agent 侧必须收到 session/load，实际 {seen:?}"
    );
    assert!(
        !seen.iter().any(|method| method == "session/new"),
        "R1 违约：--resume 绝不发 session/new（防「先建后载」），实际 {seen:?}"
    );
    let load = captured_requests(&req_file)
        .into_iter()
        .find(|frame| frame["method"] == json!("session/load"))
        .expect("session/load 帧缺失");
    assert_eq!(
        load["params"]["sessionId"],
        json!("acp-sess-1"),
        "R1 违约：session/load 必须携带请求的 sessionId，实际 {load}"
    );
    assert_eq!(
        load["params"]["cwd"],
        json!(workspace_text),
        "R1 违约：session/load 的 cwd 必须是 canonical 绝对路径，实际 {load}"
    );
    assert!(
        load["params"]["mcpServers"].is_array(),
        "R1 违约：session/load 必须携带 mcpServers 数组，实际 {load}"
    );

    // stdout：jsonl 事件流必须含 turn:delta（证明 resume 后真的跑了一轮）。
    let mut saw_delta = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("R1 违约：jsonl 每行必须合法（{error}）：{line}"));
        if value["type"] == json!("turn:delta") && value["text"] == json!("hello") {
            saw_delta = true;
        }
    }
    assert!(
        saw_delta,
        "R1 违约：resume 后 stdout 必须含 turn:delta(text=hello)\nstdout:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ───────────────────────── R2 不支持降级（绝不静默新建） ─────────────────────────

/// R2：`initialize.agentCapabilities.loadSession=false` ⇒ Exit≠0 + stderr `acp_unavailable`，
/// 且 agent 侧零 `session/load`、零 `session/new`。
#[test]
fn r2_resume_unsupported_degrades_without_new_session() {
    let root = unique_root("resume-disabled");
    let req_file = root.join("requests.jsonl");
    let env = IsolatedCli::with_agent(
        "resume-disabled-agent",
        resume_spec("resume_disabled", &req_file, "acp-sess-1"),
    );
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();

    let output = run_cli(
        env.command().args(resume_cli_args(&workspace_text, "acp-sess-1")),
        watchdog(),
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_ne!(
        output.status.code(),
        Some(0),
        "R2 违约：loadSession=false 时 --resume 必须非 0 退出（不得静默新建），实际 Exit=0\nstdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("acp_unavailable"),
        "R2 违约：stderr 必须含 acp_unavailable，实际 {stderr}"
    );
    let seen = methods(&req_file);
    assert!(
        !seen.iter().any(|method| method == "session/load"),
        "R2 违约：loadSession=false 时不得尝试 session/load，实际 {seen:?}"
    );
    assert!(
        !seen.iter().any(|method| method == "session/new"),
        "R2 硬约束：绝不静默降级为新建会话（不得出现 session/new），实际 {seen:?}"
    );

    // R2②：loadSession=true 但 agent 对 session/load 回 -32601 ⇒ 同样 acp_unavailable，
    // 且「尝试过 load」但不回落 session/new。
    let root2 = unique_root("resume-method-unsupported");
    let req_file2 = root2.join("requests.jsonl");
    let env2 = IsolatedCli::with_agent(
        "resume-unsupported-agent",
        resume_spec("resume_method_unsupported", &req_file2, "acp-sess-1"),
    );
    let workspace2 = env2.workspace();
    let workspace2_text = workspace2.to_string_lossy().into_owned();
    let output2 = run_cli(
        env2.command().args(resume_cli_args(&workspace2_text, "acp-sess-1")),
        watchdog(),
    );
    let stderr2 = String::from_utf8_lossy(&output2.stderr).into_owned();
    assert_ne!(
        output2.status.code(),
        Some(0),
        "R2 违约：agent 回 -32601 时 --resume 必须非 0 退出，实际 Exit=0\nstderr:\n{stderr2}"
    );
    assert!(
        stderr2.contains("acp_unavailable"),
        "R2 违约：-32601 降级必须报 acp_unavailable，实际 {stderr2}"
    );
    let seen2 = methods(&req_file2);
    assert!(
        seen2.iter().any(|method| method == "session/load"),
        "R2② 违约：loadSession=true 时必须先尝试 session/load（再由 -32601 降级），实际 {seen2:?}"
    );
    assert!(
        !seen2.iter().any(|method| method == "session/new"),
        "R2② 硬约束：-32601 降级不得回落 session/new，实际 {seen2:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&root2);
}

// ───────────────────────── R3 缺 sessionId 形态 ─────────────────────────

/// R3：opencode 形态（session/load 响应不含 sessionId）⇒ CLI 仍 Exit 0 并跑完一轮 prompt；
/// 会话标识沿用请求 id，并留下一条结构化 JSON 诊断（C3.1）。
#[test]
fn r3_resume_without_response_session_id_still_runs_turn() {
    let root = unique_root("resume-no-id");
    let req_file = root.join("requests.jsonl");
    let env = IsolatedCli::with_agent(
        "resume-no-id-agent",
        resume_spec("resume_no_id", &req_file, "acp-sess-1"),
    );
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();

    let output = run_cli(
        env.command().args(resume_cli_args(&workspace_text, "acp-sess-1")),
        watchdog(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "R3 违约：响应缺 sessionId 时 --resume 必须 Exit 0（沿用请求 id）\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let seen = methods(&req_file);
    assert!(
        seen.iter().any(|method| method == "session/prompt"),
        "R3 违约：resume 后必须完成一轮 prompt，实际 {seen:?}"
    );
    assert!(
        !seen.iter().any(|method| method == "session/new"),
        "R3：缺 sessionId 形态也不得新建会话，实际 {seen:?}"
    );
    let mut saw_delta = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value["type"] == json!("turn:delta") {
                saw_delta = true;
            }
        }
    }
    assert!(saw_delta, "R3 违约：stdout 必须含 turn:delta，实际:\n{stdout}");
    // 诊断面（C3.1 迁移）：介质由 stderr 文本 `[acp]` 改为 `agent::otel::log` 结构化 JSON 行。
    // wave-59 C1：`--output-format jsonl|sse` 下 stdout 只承载结构化事件，JSON 诊断改道 **stderr**
    // （`set_stderr_output(true)`，src/bin/cli.rs `ObservabilityBootstrap::install`），故按行 parse stderr。
    let diagnostic = stderr
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
        "R3 违约：响应缺 sessionId 时必须留下结构化 JSON 诊断（`message` 含 session + `trace_id` 键），实际 stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ───────────────────────── R4 additionalDirectories 同口径 ─────────────────────────

/// R4：CLI `acp` 子命令**未暴露** `additionalDirectories` 参数（冻结 CLI 面只有
/// `--agent/--list/-p/--output-format/--workspace/--timeout-secs/--resume`），
/// 因此本用例按 task 描述跳过运行期校验，只断言 help 面确实没有该参数（防止未来悄悄加参数却不校验）。
#[test]
fn r4_resume_dirs_surface_absent_in_frozen_cli() {
    let env = IsolatedCli::new("resume-dirs-surface");
    let output = run_cli(env.command().args(["acp", "--help"]), watchdog());
    assert_eq!(
        output.status.code(),
        Some(0),
        "R4 前置：`acp --help` 必须 Exit 0，stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for absent in ["additional-directories", "additionalDirectories", "--dirs"] {
        assert!(
            !help.contains(absent),
            "R4 违约：CLI 未暴露 dirs 参数（若不暴露则不应出现 `{absent}`），实际 help:\n{help}"
        );
    }
}
