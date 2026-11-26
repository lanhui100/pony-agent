//! B176：`ponycli acp` 能力声明端到端（j2，POST-HOC 回归）。
//!
//! 契约：CLI 的 `acp` 子命令必须让 `AcpAgentSpec.capabilitiesHint` 真正生效 —— 当 hint.fs=true
//! 且宿主能提供 fs handler 时，真实 `initialize` 报文必须声明
//! `clientCapabilities.fs.{readTextFile,writeTextFile}=true`；hint.fs=false 时不得声明。
//!
//! 断言走**真实二进制** + fake agent 捕获的 initialize 原始报文（typed，不靠字符串包含）。
//! spec 只经冻结契约发现：`$XDG_CONFIG_HOME/pony-agent/acp-agents.json`（不新增 env 变量）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn unique_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "pony-acp-cli-caps-{}-{}-{tag}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("创建隔离根目录");
    root
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
    Duration::from_millis(nfr_acp_ms("request_timeout_ms") * 2)
}

fn fixture_script() -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script.to_string_lossy().into_owned()
}

struct IsolatedCli {
    root: PathBuf,
}

impl IsolatedCli {
    fn new(tag: &str, spec: Value) -> Self {
        let root = unique_root(tag);
        for child in ["home", "config", "data", "state", "cache", "workspace"] {
            std::fs::create_dir_all(root.join(child)).expect("创建隔离子目录");
        }
        let dir = root.join("config").join("pony-agent");
        std::fs::create_dir_all(&dir).expect("创建 pony-agent 配置目录");
        std::fs::write(
            dir.join("acp-agents.json"),
            serde_json::to_string_pretty(&json!([spec])).expect("清单可序列化"),
        )
        .expect("写入 acp-agents.json");
        Self { root }
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
    let label = format!("{command:?}");
    command.stdin(Stdio::null());
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

fn captured_client_capabilities(init_file: &Path) -> Value {
    let raw = std::fs::read_to_string(init_file)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获 initialize（{}：{error}）", init_file.display()));
    let line = raw.lines().next().expect("initialize 捕获为空");
    let frame: Value = serde_json::from_str(line).expect("initialize 必须是合法 JSON");
    assert_eq!(frame["method"], json!("initialize"));
    frame["params"]["clientCapabilities"].clone()
}

fn cli_caps_case(tag: &str, hint_fs: Value) -> Value {
    let root = unique_root(tag);
    let init_file = root.join("initialize.jsonl");
    let session_file = root.join("session-new.jsonl");
    let spec = json!({
        "id": "fake",
        "label": "Fake ACP agent",
        "command": "sh",
        "args": [
            fixture_script(),
            "init_capture",
            init_file.to_string_lossy(),
            "1",
            session_file.to_string_lossy()
        ],
        "capabilitiesHint": {"fs": hint_fs}
    });
    let env = IsolatedCli::new(tag, spec);
    let workspace = env.workspace();
    let output = run_cli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "-p",
            "hi",
            "--output-format",
            "jsonl",
            "--workspace",
            workspace.to_string_lossy().as_ref(),
            "--timeout-secs",
            "10",
        ]),
        watchdog(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "j2：CLI acp 单轮必须 EXIT 0\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let capabilities = captured_client_capabilities(&init_file);
    let _ = std::fs::remove_dir_all(&root);
    capabilities
}

/// j2：hint.fs=true 的 spec 经 CLI 启动时，真实 initialize 必须声明 clientCapabilities.fs。
///
/// [POST-HOC-GREEN: 实现（CLI 能力接线）先于用例，仅作回归守护]
#[test]
fn j2_cli_declares_fs_capability_from_hint() {
    let capabilities = cli_caps_case("hint-fs-true", json!(true));
    assert_eq!(
        capabilities["fs"]["readTextFile"],
        json!(true),
        "j2 违约：hint.fs=true 的 CLI 会话必须声明 readTextFile=true（不得走 spwan 一律不声明），实际 {capabilities}"
    );
    assert_eq!(
        capabilities["fs"]["writeTextFile"],
        json!(true),
        "j2 违约：hint.fs=true 的 CLI 会话必须声明 writeTextFile=true，实际 {capabilities}"
    );
}

/// j2 负相：hint.fs=false ⇒ 不得声明 fs（hint 是上限）。
#[test]
fn j2b_cli_does_not_declare_fs_when_hint_off() {
    let capabilities = cli_caps_case("hint-fs-false", json!(false));
    assert!(
        capabilities.get("fs").is_none(),
        "j2b 违约：hint.fs=false 时不得声明 fs，实际 {capabilities}"
    );
}
