//! W27「MCP 客户端握手与工具/资源读取」红相验收测试。
//!
//! 背景：`agent/mcp_resources.rs` 只做静态资源目录登记，真正按 MCP spec 2024-11-05 起
//! 子进程、走 JSON-RPC 2.0 行分隔握手并完成 `tools/list` / `tools/call` /
//! `resources/list` / `resources/read` 的客户端**不存在**，因此无法验证任何 MCP 真实链路。
//!
//! W27 冻结契约（`docs/audit/agent-core-improvement-plan.md` §W27）：
//! - `pony_agent_core::agent::mcp_client::{McpServerSpec, McpToolSpec, McpClient, McpClientError}`；
//! - `connect` 发 `initialize`（params 含 `protocolVersion:"2024-11-05"`、`capabilities:{}`、
//!   `clientInfo:{name,version}`），收到 result 后再发 **无 id** 的 `notifications/initialized`；
//! - `list_tools`→`tools/list`；`call_tool`→`tools/call`（params `{name,arguments}`，返回 result 对象）；
//!   `list_resources`→`resources/list`（result.resources）；`read_resource`→`resources/read`（返回 result 对象）；
//! - 请求 id 从 1 递增、按 id 匹配响应并跳过服务端通知；超时→`Timeout`；服务端 `error`→`Server(message)`；
//!   非法 JSON→`Protocol`；`Drop` 必须终止子进程（不残留）。
//!
//! 测试手法：用 Python3 写一个**临时假 MCP server**（本文件内 `fs::write` 落到唯一临时目录，
//! `spec.command = "/usr/bin/env"`、`args = ["python3", <script>, ...]`），逐行读 stdin 的
//! JSON-RPC 并按 method 回包；它同时把收到的每条报文追加到 `MCP_FAKE_RECORD` 记录的 JSONL
//! 文件（t1 断言握手顺序/无 id 通知），并在启动时把自身 pid 写入 `MCP_FAKE_PID`（t7 断言进程已终止）。
//!
//! 红相说明：`agent::mcp_client` 模块当前不存在，本文件因此**编译失败**（这是预期的第一层红）。
//! 为了在实现落地后立即变为有效断言，这里只引用上述冻结的公开路径与签名，不绕过任何 API。
//!
//! 契约歧义（已回执提请 Lead 裁决）：
//! 1) `McpServerSpec::new` / `with_arg` / `with_env` 的入参类型未冻结（`String` vs `&str` vs
//!    `impl Into<String>`）；本测试按结构体字段类型（`pub id: String`）以 `String` 调用，
//!    若实现选用 `&str` 形参将编译失败。
//! 2) `McpClientError` 未冻结 `Display`/`Debug` 派生；t6 的消息包含断言使用 `Debug` 格式化
//!    （枚举几乎必然 derive Debug），不要求 Display 实现。
//! 3) t7 的「300ms 内退出」按可确定性方式实现：10ms 轮询 `/proc/<pid>` 判定进程已终止，
//!    断言观测耗时 < 1000ms（比 300ms 更宽以抗 CI 抖动），不使用固定 sleep。

use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::mcp_client::{McpClient, McpClientError, McpServerSpec};
use serde_json::{json, Value};

// ─────────────────────────────────────────────────────────────────────────────
// 假 MCP server 脚本（MCP spec 2024-11-05 / JSON-RPC 2.0，按行分隔）
//
// 环境变量：
//   MCP_FAKE_RECORD  —— 收到的每条报文以 JSONL 追加写入
//   MCP_FAKE_PID     —— 启动时写入自身 pid
// 参数：
//   --never-reply    —— 对 tools/list 不回复（超时用例）
// ─────────────────────────────────────────────────────────────────────────────
const FAKE_MCP_SERVER_PY: &str = r#"
import json
import os
import sys

NEVER_REPLY = "--never-reply" in sys.argv[1:]
RECORD = os.environ.get("MCP_FAKE_RECORD", "")
PID_FILE = os.environ.get("MCP_FAKE_PID", "")


def emit(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def record(msg):
    if not RECORD:
        return
    with open(RECORD, "a", encoding="utf-8") as record_handle:
        record_handle.write(json.dumps(msg, separators=(",", ":")) + "\n")
        record_handle.flush()


def handle(msg):
    method = msg.get("method")
    msg_id = msg.get("id")

    if method == "initialize":
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "fake-mcp", "version": "0.1.0"},
            },
        })
        return

    if method == "notifications/initialized":
        return

    if method == "tools/list":
        if NEVER_REPLY:
            return
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo the received arguments back as text",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"x": {"type": "number"}},
                        },
                    },
                    {
                        "name": "fail",
                        "description": "Always answers with a JSON-RPC error",
                        "inputSchema": {"type": "object"},
                    },
                ]
            },
        })
        return

    if method == "tools/call":
        params = msg.get("params") or {}
        if params.get("name") == "echo":
            text = json.dumps(params.get("arguments"), separators=(",", ":"))
            emit({
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {"content": [{"type": "text", "text": text}]},
            })
        else:
            emit({
                "jsonrpc": "2.0",
                "id": msg_id,
                "error": {"code": -32000, "message": "boom"},
            })
        return

    if method == "resources/list":
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"resources": [{"uri": "file:///a.txt", "name": "a"}]},
        })
        return

    if method == "resources/read":
        uri = (msg.get("params") or {}).get("uri")
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"contents": [{"uri": uri, "text": "hello"}]},
        })
        return

    if msg_id is not None:
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "error": {"code": -32601, "message": "method not found: " + str(method)},
        })


def main():
    if PID_FILE:
        with open(PID_FILE, "w", encoding="utf-8") as pid_handle:
            pid_handle.write(str(os.getpid()))
            pid_handle.flush()
    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except Exception:
            continue
        record(msg)
        handle(msg)


if __name__ == "__main__":
    main()
"#;

// ─────────────────────────────────────────────────────────────────────────────
// 夹具
// ─────────────────────────────────────────────────────────────────────────────

/// 唯一临时工作区（`std::env::temp_dir()` 下唯一子目录，drop 时清理）。
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "pony-agent-w27-{tag}-{}-{nanos}-{sequence}",
            process::id()
        ));
        fs::create_dir_all(&dir).expect("创建 W27 临时工作区失败");
        let root = dir.canonicalize().unwrap_or(dir);
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn script_path(&self) -> PathBuf {
        let path = self.path("fake_mcp_server.py");
        fs::write(&path, FAKE_MCP_SERVER_PY).expect("写入 W27 假 MCP server 脚本失败");
        path
    }

    fn record_path(&self) -> PathBuf {
        self.path("received.jsonl")
    }

    fn pid_path(&self) -> PathBuf {
        self.path("server.pid")
    }

    /// 构造假 server 的 spec：跨平台 python 命令 `<script> [extra...]`，并注入记录/pid 路径。
    fn spec(&self, extra_args: &[&str]) -> McpServerSpec {
        let script = self.script_path();
        let (cmd, base_args) = resolve_python_command();
        let mut spec = McpServerSpec::new(
            "fake-mcp-server".to_string(),
            cmd.to_string(),
        );
        for base in base_args {
            spec = spec.with_arg(base);
        }
        spec = spec.with_arg(
            script
                .to_str()
                .expect("W27 假 server 脚本路径必须是合法 UTF-8")
                .to_string(),
        );
        for extra in extra_args {
            spec = spec.with_arg((*extra).to_string());
        }
        spec.with_env(
            "MCP_FAKE_RECORD".to_string(),
            self.record_path()
                .to_str()
                .expect("W27 记录路径必须是合法 UTF-8")
                .to_string(),
        )
        .with_env(
            "MCP_FAKE_PID".to_string(),
            self.pid_path()
                .to_str()
                .expect("W27 pid 路径必须是合法 UTF-8")
                .to_string(),
        )
    }
}

/// 解析跨平台 Python 解释器命令与基础参数。
///
/// 为什么 Windows 不能用 `/usr/bin/env`：Windows 没有 Unix 虚拟路径 `/usr/bin/env`，
/// 直接调用会触发 os error 3（系统找不到指定路径）。
/// Windows 下按序探测：python --version → python3 --version → py -3 --version，
/// 规避 Microsoft Store 的 python 别名存根；
/// Unix 下保持既有 `/usr/bin/env python3`。
fn resolve_python_command() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        let candidates: &[(&str, &[&str])] = &[
            ("python", &[]),
            ("python3", &[]),
            ("py", &["-3"]),
        ];
        for &(program, extra_args) in candidates {
            if let Ok(output) = std::process::Command::new(program)
                .args(extra_args)
                .arg("--version")
                .output()
            {
                if output.status.success() {
                    return (
                        program,
                        extra_args.iter().map(|s| (*s).to_string()).collect(),
                    );
                }
            }
        }
        panic!("CI 环境缺少可用 python 解释器（已尝试 python, python3, py -3），MCP stdio 契约测试无法运行");
    }

    #[cfg(not(windows))]
    {
        ("/usr/bin/env", vec!["python3".to_string()])
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn connect(workspace: &TempWorkspace, extra_args: &[&str], timeout: Duration) -> McpClient {
    let spec = workspace.spec(extra_args);
    McpClient::connect(&spec, timeout).unwrap_or_else(|error| {
        panic!(
            "W27 前置失败：连接假 MCP server 失败（code={:?}，error={:?}）。\
             期望：握手成功（initialize → notifications/initialized）。",
            error.code(),
            error
        )
    })
}

fn expect_error<T>(outcome: Result<T, McpClientError>) -> McpClientError {
    match outcome {
        Ok(_) => panic!(
            "W27 断言失败：期望返回 Err，实际返回 Ok —— 错误路径没有生效"
        ),
        Err(error) => error,
    }
}

fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if predicate() {
            return true;
        }
        if started.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 读取假 server 的 JSONL 记录（文件可能尚不存在/仍在写入）。
fn read_record(path: &PathBuf) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

/// `/proc/<pid>` 存活判定：`Some(true)` 运行中、`Some(false)` 已终止（含僵尸/已回收）、
/// `None` 表示当前平台不支持确定性探测。
#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> Option<bool> {
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            // 格式：`pid (comm) S ...`；comm 可能含空格/括号，取最后一个 ')' 之后的状态字符。
            let state = stat
                .rsplit(')')
                .next()
                .and_then(|rest| rest.trim_start().chars().next());
            Some(!matches!(state, Some('Z') | Some('X') | Some('x')))
        }
        Err(_) => Some(false),
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn pid_alive(pid: u32) -> Option<bool> {
    let status = process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .ok()?;
    Some(status.success())
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> Option<bool> {
    None
}

/// 从 pid 文件读出子进程 pid（轮询等待假 server 启动落盘）。
fn read_server_pid(path: &PathBuf) -> u32 {
    let mut pid = None;
    wait_until(Duration::from_secs(2), || {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(value) = text.trim().parse::<u32>() {
                pid = Some(value);
                return true;
            }
        }
        false
    });
    pid.unwrap_or_else(|| {
        panic!(
            "W27 前置失败：假 server 未在 2s 内写出 pid 文件 {}（子进程可能未真正启动）",
            path.display()
        )
    })
}

fn preview(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        format!(
            "{}…(truncated, total {} chars)",
            text.chars().take(limit).collect::<String>(),
            text.chars().count()
        )
    }
}

fn method_of(message: &Value) -> &str {
    message.get("method").and_then(Value::as_str).unwrap_or("")
}

// ─────────────────────────────────────────────────────────────────────────────
// t1：握手 —— serverInfo 与 initialize → notifications/initialized 顺序/无 id
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t1_connect_handshakes_with_initialize_then_initialized_notification() {
    // Arrange
    let workspace = TempWorkspace::new("t1-handshake");
    let record_path = workspace.record_path();

    // Act
    let client = connect(&workspace, &[], Duration::from_secs(5));

    // Assert 1：serverInfo 透传
    assert_eq!(
        client.server_name(),
        "fake-mcp",
        "t1 失败：server_name 应来自 initialize.result.serverInfo.name，期望 \"fake-mcp\"，实际 {:?}",
        client.server_name()
    );
    assert_eq!(
        client.server_version(),
        "0.1.0",
        "t1 失败：server_version 应来自 initialize.result.serverInfo.version，期望 \"0.1.0\"，实际 {:?}",
        client.server_version()
    );

    // Assert 2：假 server 侧记录到两条握手报文，且顺序为 initialize → notifications/initialized
    let recorded = {
        let mut lines = Vec::new();
        let ok = wait_until(Duration::from_secs(2), || {
            lines = read_record(&record_path);
            lines.len() >= 2
        });
        assert!(
            ok,
            "t1 失败：2s 内假 server 只记录到 {} 条报文（期望 ≥2：initialize 与 notifications/initialized）。\
             记录文件={}，实际内容={:?}",
            lines.len(),
            record_path.display(),
            lines
        );
        lines
    };

    let methods: Vec<&str> = recorded.iter().map(method_of).collect();
    println!("[W27][t1] server 收到的 method 序列 = {methods:?}");

    assert_eq!(
        methods.first().copied(),
        Some("initialize"),
        "t1 失败：握手第一条报文必须是 `initialize`，实际第一条={:?}，完整序列={methods:?}",
        methods.first()
    );
    assert_eq!(
        methods.get(1).copied(),
        Some("notifications/initialized"),
        "t1 失败：握手第二条报文必须是 `notifications/initialized`（收到 initialize.result 之后发送），\
         实际第二条={:?}，完整序列={methods:?}",
        methods.get(1)
    );

    // Assert 3：initialize 的 params 契约（protocolVersion / capabilities / clientInfo）
    let initialize = &recorded[0];
    let params = initialize.get("params").unwrap_or_else(|| {
        panic!(
            "t1 失败：`initialize` 请求缺少 params。实际报文={}",
            preview(&initialize.to_string(), 600)
        )
    });
    assert_eq!(
        params.get("protocolVersion").and_then(Value::as_str),
        Some("2024-11-05"),
        "t1 失败：initialize.params.protocolVersion 应为 \"2024-11-05\"，实际={:?}，完整报文={}",
        params.get("protocolVersion"),
        preview(&initialize.to_string(), 600)
    );
    assert!(
        params.get("capabilities").map(Value::is_object).unwrap_or(false),
        "t1 失败：initialize.params.capabilities 应为对象（当前契约要求空对象 {{}}），实际={:?}",
        params.get("capabilities")
    );
    let client_info = params.get("clientInfo").unwrap_or_else(|| {
        panic!(
            "t1 失败：initialize.params.clientInfo 缺失。实际 params={}",
            preview(&params.to_string(), 600)
        )
    });
    for field in ["name", "version"] {
        let value = client_info.get(field).and_then(Value::as_str).unwrap_or("");
        assert!(
            !value.trim().is_empty(),
            "t1 失败：initialize.params.clientInfo.{field} 缺失或为空字符串。实际 clientInfo={:?}",
            client_info
        );
    }

    // Assert 4：initialized 是通知 —— 不得携带 id
    let initialized = &recorded[1];
    assert!(
        initialized.get("id").is_none(),
        "t1 失败：`notifications/initialized` 不得携带 id（它是 JSON-RPC 通知），实际报文={}",
        preview(&initialized.to_string(), 600)
    );
    assert_eq!(
        initialized.get("jsonrpc").and_then(Value::as_str),
        Some("2.0"),
        "t1 失败：握手报文必须是 JSON-RPC 2.0，实际={:?}",
        initialized.get("jsonrpc")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t2：tools/list 解析
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t2_list_tools_parses_tool_specs_and_input_schema() {
    // Arrange
    let workspace = TempWorkspace::new("t2-tools-list");
    let client = connect(&workspace, &[], Duration::from_secs(5));

    // Act
    let tools = client.list_tools().unwrap_or_else(|error| {
        panic!(
            "t2 失败：`tools/list` 应成功，实际返回 Err（code={:?}，error={:?}）",
            error.code(),
            error
        )
    });
    println!(
        "[W27][t2] tools = {:?}",
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
    );

    // Assert 1：数量与名字集合
    assert_eq!(
        tools.len(),
        2,
        "t2 失败：应解析出 2 个工具，实际 {} 个（名字={:?}）",
        tools.len(),
        tools.iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>()
    );
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec!["echo", "fail"],
        "t2 失败：工具名集合应为 [\"echo\", \"fail\"]，实际={names:?}"
    );

    // Assert 2：echo 的 description 与 input_schema（必须保留为 JSON 对象）
    let echo = tools
        .iter()
        .find(|tool| tool.name == "echo")
        .expect("t2 失败：工具列表中缺少 `echo`");
    assert!(
        echo.description.as_deref().map(|text| !text.trim().is_empty()).unwrap_or(false),
        "t2 失败：echo.description 应非空，实际={:?}",
        echo.description
    );
    assert!(
        echo.input_schema.is_object(),
        "t2 失败：echo.input_schema 必须是 JSON 对象（保留服务端原始 schema），实际={}",
        preview(&echo.input_schema.to_string(), 600)
    );
    assert_eq!(
        echo.input_schema.get("type").and_then(Value::as_str),
        Some("object"),
        "t2 失败：echo.input_schema.type 应为 \"object\"，实际 schema={}",
        preview(&echo.input_schema.to_string(), 600)
    );
    assert!(
        echo.input_schema
            .pointer("/properties/x")
            .is_some(),
        "t2 失败：echo.input_schema 应保留 properties.x，实际 schema={}",
        preview(&echo.input_schema.to_string(), 600)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t3：tools/call 往返（返回 result 对象）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t3_call_tool_returns_result_content() {
    // Arrange
    let workspace = TempWorkspace::new("t3-call-tool");
    let client = connect(&workspace, &[], Duration::from_secs(5));

    // Act
    let result = client.call_tool("echo", json!({ "x": 1 })).unwrap_or_else(|error| {
        panic!(
            "t3 失败：`tools/call` name=echo 应成功，实际返回 Err（code={:?}，error={:?}）",
            error.code(),
            error
        )
    });
    println!(
        "[W27][t3] call_tool result = {}",
        preview(&result.to_string(), 600)
    );

    // Assert：返回的是 result 对象（不是整个 JSON-RPC 信封），且正文含参数字面量
    assert!(
        result.is_object(),
        "t3 失败：`call_tool` 应返回 result 对象，实际类型={}",
        preview(&result.to_string(), 600)
    );
    assert!(
        result.get("jsonrpc").is_none() && result.get("result").is_none(),
        "t3 失败：`call_tool` 返回的应是 result 对象本身，而不是完整 JSON-RPC 信封，实际={}",
        preview(&result.to_string(), 600)
    );
    let text = result
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!(
                "t3 失败：result.content[0].text 缺失或不是字符串，实际 result={}",
                preview(&result.to_string(), 600)
            )
        });
    assert!(
        text.contains("\"x\":1"),
        "t3 失败：echo 回显的 text 应含参数字面量 `\"x\":1`，实际 text={text:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t4：resources/list 与 resources/read
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t4_list_and_read_resources() {
    // Arrange
    let workspace = TempWorkspace::new("t4-resources");
    let client = connect(&workspace, &[], Duration::from_secs(5));

    // Act 1：list_resources
    let resources = client.list_resources().unwrap_or_else(|error| {
        panic!(
            "t4 失败：`resources/list` 应成功，实际返回 Err（code={:?}，error={:?}）",
            error.code(),
            error
        )
    });
    println!("[W27][t4] resources = {resources:?}");

    assert_eq!(
        resources.len(),
        1,
        "t4 失败：应解析出 1 条 resource，实际 {} 条={:?}",
        resources.len(),
        resources
    );
    assert_eq!(
        resources[0].get("uri").and_then(Value::as_str),
        Some("file:///a.txt"),
        "t4 失败：resources[0].uri 应为 \"file:///a.txt\"，实际={:?}，完整值={}",
        resources[0].get("uri"),
        preview(&resources[0].to_string(), 600)
    );

    // Act 2：read_resource
    let read = client
        .read_resource("file:///a.txt")
        .unwrap_or_else(|error| {
            panic!(
                "t4 失败：`resources/read` uri=file:///a.txt 应成功，实际返回 Err（code={:?}，error={:?}）",
                error.code(),
                error
            )
        });
    println!(
        "[W27][t4] read_resource = {}",
        preview(&read.to_string(), 600)
    );

    // Assert：返回 result 对象，contents[0] 的 uri/text 正确
    assert!(
        read.is_object(),
        "t4 失败：`read_resource` 应返回 result 对象，实际={}",
        preview(&read.to_string(), 600)
    );
    assert!(
        read.get("jsonrpc").is_none() && read.get("result").is_none(),
        "t4 失败：`read_resource` 返回的应是 result 对象本身，而不是完整 JSON-RPC 信封，实际={}",
        preview(&read.to_string(), 600)
    );
    assert_eq!(
        read.pointer("/contents/0/text").and_then(Value::as_str),
        Some("hello"),
        "t4 失败：contents[0].text 应为 \"hello\"，实际 result={}",
        preview(&read.to_string(), 600)
    );
    assert_eq!(
        read.pointer("/contents/0/uri").and_then(Value::as_str),
        Some("file:///a.txt"),
        "t4 失败：contents[0].uri 应为 \"file:///a.txt\"，实际 result={}",
        preview(&read.to_string(), 600)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t5：超时 —— server 不回复时按 timeout 返回 Timeout，且不得挂死
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t5_list_tools_times_out_when_server_never_replies() {
    // Arrange：--never-reply 让 server 对 tools/list 保持沉默；connect 仍正常握手
    let workspace = TempWorkspace::new("t5-timeout");
    let client = connect(&workspace, &["--never-reply"], Duration::from_millis(500));

    // Act
    let started = Instant::now();
    let outcome = client.list_tools();
    let elapsed = started.elapsed();
    println!("[W27][t5] list_tools 耗时 = {elapsed:?}，结果 = {:?}", outcome.is_err());

    // Assert 1：必须在 2s 内返回（不得无限阻塞）
    assert!(
        elapsed < Duration::from_millis(2_000),
        "t5 失败：server 不回复时 `tools/list` 应在超时后立即返回，实际耗时 {elapsed:?}（>2s，疑似未实现超时/挂死）"
    );

    // Assert 2：错误类型为 Timeout
    let error = expect_error(outcome);
    assert_eq!(
        error.code(),
        "timeout",
        "t5 失败：超时错误码应为 \"timeout\"，实际 code={:?}，error={:?}，耗时={elapsed:?}",
        error.code(),
        error
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t6：服务端 JSON-RPC error → Server(message)
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t6_call_tool_maps_server_error_to_server_error_code() {
    // Arrange
    let workspace = TempWorkspace::new("t6-server-error");
    let client = connect(&workspace, &[], Duration::from_secs(5));

    // Act：name=fail，假 server 回 {"error":{"code":-32000,"message":"boom"}}
    let error = expect_error(client.call_tool("fail", json!({})));

    // Assert 1：错误码映射
    assert_eq!(
        error.code(),
        "server_error",
        "t6 失败：服务端 error 对象应映射为 \"server_error\"，实际 code={:?}，error={:?}",
        error.code(),
        error
    );

    // Assert 2：原始 message 必须透传（至少出现在 Debug 表示中）
    let debug = format!("{error:?}");
    assert!(
        debug.contains("boom"),
        "t6 失败：错误表示中应保留服务端原始 message \"boom\"，实际 Debug={debug:?}（期望 error=Server(\"boom\")）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t7：Drop 必须终止子进程（不残留）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t7_drop_terminates_child_process() {
    // Arrange
    let workspace = TempWorkspace::new("t7-drop");
    let pid_path = workspace.pid_path();
    let client = connect(&workspace, &[], Duration::from_secs(5));

    // 假 server 启动时写入自身 pid（`/usr/bin/env` 会 exec 成 python，pid 不变）
    let pid = read_server_pid(&pid_path);
    println!("[W27][t7] 假 server pid = {pid}");

    if pid_alive(pid).is_none() {
        println!("[W27][t7] 当前平台不支持确定性进程探测，跳过 Drop 终止断言");
        drop(client);
        return;
    }
    assert_eq!(
        pid_alive(pid),
        Some(true),
        "t7 前置失败：连接期间假 server（pid={pid}）应处于运行态，实际已退出"
    );

    // Act
    let started = Instant::now();
    drop(client);

    // Assert：Drop 后子进程应在短时间内终止（10ms 轮询，不 sleep 盲等）
    let terminated = wait_until(Duration::from_secs(1), || pid_alive(pid) == Some(false));
    let elapsed = started.elapsed();
    println!("[W27][t7] drop 后进程终止耗时 = {elapsed:?}，terminated={terminated}");
    assert!(
        terminated,
        "t7 失败：`drop(client)` 后 1s 内假 server 子进程（pid={pid}）仍在运行 —— Drop 未终止子进程（残留）。\
         观测耗时={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1_000),
        "t7 失败：Drop 终止子进程耗时 {elapsed:?} 超过 1000ms（期望尽快终止，契约参考值 300ms）"
    );
}
