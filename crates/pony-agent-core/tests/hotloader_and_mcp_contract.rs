//! hotloader 与 MCP transport 契约测试（W27 迁移：stdin 假 server 升级为 MCP 2024-11-05 规范会话）。
//!
//! 迁移背景：`StdioMcpTransport` 原先只做「抛一行 JSON-RPC、读一行响应」的一次性往返，
//! 假 server 也只 `read line` 一次就回 `resources`。W27 起 transport 升级为规范会话：
//!   `initialize` → （收到 result 后）`notifications/initialized` → `resources/list`。
//! 因此旧假 server（只读一行、不认 method）必然红，本文件改为按行读 JSON、按 method 回包的
//! Python3 假 MCP server，断言原意保持不变：
//!   `transport.execute(McpResourceOperation::ListResources)` 成功，且
//!   `content["resources"][0]["name"] == "test"`。
//!
//! 稳定性要点：
//! - 临时脚本落在唯一临时目录，测试结束（含 panic）由 Drop 清理；
//! - `source_id` 取唯一值：W27 客户端按 `(command, args, source_id)` 复用长连接并缓存会话，
//!   复用同一 source_id 会串台到上一个用例遗留的会话；
//! - 假 server 严格按 MCP 方法名应答，不保留 `list_resources` 旧别名 —— 否则会把
//!   「transport 未真正升级到规范会话」的回归掩盖成绿灯。

use pony_agent_core::agent::tool_runtime::{
    McpResourceOperation, McpTransport, McpTransportRequest, StdioMcpTransport,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 假 MCP resources server（MCP 2024-11-05 / JSON-RPC 2.0，按行分隔）。
const FAKE_RES_SERVER_PY: &str = r#"
import json
import sys


def emit(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def handle(msg):
    method = msg.get("method")
    msg_id = msg.get("id")

    if method == "initialize":
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"resources": {}},
                "serverInfo": {"name": "fake-res", "version": "0.1.0"},
            },
        })
        return

    if method == "notifications/initialized":
        return

    if method == "resources/list":
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"resources": [{"uri": "file:///test", "name": "test"}]},
        })
        return

    if method == "resources/templates/list":
        emit({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {"resourceTemplates": []},
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
    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except Exception:
            continue
        handle(msg)


if __name__ == "__main__":
    main()
"#;

/// 唯一临时目录，Drop 时清理（panic 路径也不会残留临时脚本）。
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "pony-agent-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("创建 MCP 契约测试临时目录失败");
        let root = dir.canonicalize().unwrap_or(dir);
        Self { root }
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, content).expect("写入 MCP 契约测试临时脚本失败");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// 唯一 source_id：避免命中 W27 客户端的 `(command, args, source_id)` 长连接缓存。
fn unique_source_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("test-src-{}-{nanos}", std::process::id())
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

#[test]
fn test_stdio_mcp_transport_roundtrip_with_mcp_handshake() {
    // Arrange：Python3 假 server（跨平台解释器命令 + <script>，无需 shebang/可执行位）
    let workspace = TempDir::new("mcp-stdio-contract");
    let script = workspace.write("fake_res_server.py", FAKE_RES_SERVER_PY);
    let script_arg = script
        .to_str()
        .expect("临时脚本路径必须是合法 UTF-8")
        .to_string();

    let (cmd, mut args) = resolve_python_command();
    args.push(script_arg);
    let transport = StdioMcpTransport::new(cmd, args);
    let request = McpTransportRequest {
        source_id: unique_source_id(),
        source_revision: "1".to_string(),
        operation: McpResourceOperation::ListResources,
        arguments: serde_json::json!({}),
    };

    // Act
    let outcome = transport.execute(&request);
    println!("[MCP][hotloader] execute(ListResources) = {outcome:?}");

    // Assert
    let response = outcome.unwrap_or_else(|error| {
        panic!(
            "StdioMcpTransport execute(ListResources) 必须成功（W27 规范会话：initialize → \
             notifications/initialized → resources/list），实际错误：{error}"
        )
    });
    let resources = response
        .content
        .get("resources")
        .and_then(|value| value.as_array())
        .unwrap_or_else(|| {
            panic!(
                "MCP resources/list 结果应含 resources 数组，实际 content={}",
                response.content
            )
        });
    assert_eq!(
        resources.len(),
        1,
        "MCP resources/list 应返回 1 条资源，实际 {} 条：{}",
        resources.len(),
        response.content
    );
    assert_eq!(
        resources[0].get("name").and_then(|value| value.as_str()),
        Some("test"),
        "resources[0].name 应为 \"test\"，实际 content={}",
        response.content
    );
    assert_eq!(
        resources[0].get("uri").and_then(|value| value.as_str()),
        Some("file:///test"),
        "resources[0].uri 应为 \"file:///test\"，实际 content={}",
        response.content
    );

    // 显式 drop：确保 transport 释放会话/子进程后再清理临时脚本
    drop(transport);
    let _ = fs::remove_file(&script);
}
