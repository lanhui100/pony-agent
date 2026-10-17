//! W31 验收测试：LspClient 真实进程通信、协议握手与 AST/符号诊断通道
//!
//! 契约要求：
//! 1. `LspServerSpec` 支持配置启动命令、参数与工作目录。
//! 2. `LspClient` 启动子进程后发送 `initialize` 与 `initialized` 完成生命周期握手。
//! 3. 支持 `textDocument/definition` 取回符号定义位置（支持多 Location）。
//! 4. 支持 `textDocument/hover` 取回 Markdown 格式文档。
//! 5. 支持 `textDocument/documentSymbol` 或 `workspace/symbol` 符号导航列表。
//! 6. 支持 `textDocument/publishDiagnostics` 通知或 `textDocument/diagnostic` 查询真实诊断。
//! 7. 优雅降级与错误处理：当进程退出或超时后，返回结构化错误而不是死锁或 panic。

use pony_agent_core::agent::lsp::{
    LspClient, LspPosition, LspRange, LspServerSpec,
};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::time::Duration;

/// 生成轻量可控的 fake lsp server 脚本（Python），模拟标准 LSP JSON-RPC Content-Length 协议
fn create_mock_lsp_server(dir: &std::path::Path) -> std::path::PathBuf {
    let script_path = dir.join("mock_lsp_server.py");
    let script = r#"
import sys, json

def send_response(resp):
    body = json.dumps(resp).encode('utf-8')
    header = f"Content-Length: {len(body)}\r\n\r\n".encode('ascii')
    sys.stdout.buffer.write(header + body)
    sys.stdout.buffer.flush()

def read_message():
    line = sys.stdin.buffer.readline().decode('ascii', errors='ignore')
    if not line: return None
    length = 0
    while line.strip():
        if line.lower().startswith("content-length:"):
            length = int(line.split(":")[1].strip())
        line = sys.stdin.buffer.readline().decode('ascii', errors='ignore')
    if length == 0: return None
    body = sys.stdin.buffer.read(length)
    return json.loads(body.decode('utf-8'))

while True:
    msg = read_message()
    if not msg: break
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        send_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "capabilities": {
                    "definitionProvider": True,
                    "hoverProvider": True,
                    "documentSymbolProvider": True
                },
                "serverInfo": {"name": "mock-lsp", "version": "1.0.0"}
            }
        })
    elif method == "initialized":
        pass
    elif method == "textDocument/definition":
        send_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": [
                {
                    "uri": "file:///workspace/src/lib.rs",
                    "range": {
                        "start": {"line": 10, "character": 4},
                        "end": {"line": 10, "character": 12}
                    }
                }
            ]
        })
    elif method == "textDocument/hover":
        send_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "contents": {"kind": "markdown", "value": "```rust\nfn calculate()\n```\nDocs for calculate."}
            }
        })
    elif method == "textDocument/documentSymbol":
        send_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": [
                {
                    "name": "calculate",
                    "kind": 12,
                    "range": {
                        "start": {"line": 10, "character": 0},
                        "end": {"line": 15, "character": 1}
                    }
                }
            ]
        })
    elif method == "shutdown":
        send_response({"jsonrpc": "2.0", "id": req_id, "result": None})
        break
"#;
    let mut file = fs::File::create(&script_path).expect("create mock lsp script");
    file.write_all(script.trim().as_bytes()).expect("write mock lsp script");
    script_path
}

#[test]
fn test_lsp_client_handshake_and_definition() {
    let temp = std::env::temp_dir().join(format!("test_lsp_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp).expect("create temp dir");
    let script_path = create_mock_lsp_server(&temp);

    let spec = LspServerSpec {
        language_id: "rust".to_string(),
        command: "python3".to_string(),
        args: vec![script_path.to_str().unwrap().to_string()],
        cwd: Some(temp.to_str().unwrap().to_string()),
        env: vec![],
    };

    let mut client = LspClient::connect(&spec, Duration::from_secs(5)).expect("connect lsp client");
    assert_eq!(client.server_name(), "mock-lsp");
    assert_eq!(client.server_version(), "1.0.0");

    let def_locations = client
        .goto_definition("file:///workspace/src/main.rs", LspPosition { line: 5, character: 10 })
        .expect("goto_definition");
    assert_eq!(def_locations.len(), 1);
    assert_eq!(def_locations[0].uri, "file:///workspace/src/lib.rs");
    assert_eq!(def_locations[0].range.start.line, 10);

    let hover = client
        .hover("file:///workspace/src/main.rs", LspPosition { line: 5, character: 10 })
        .expect("hover");
    assert!(hover.contains("Docs for calculate."));

    let symbols = client
        .document_symbols("file:///workspace/src/lib.rs")
        .expect("document_symbols");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "calculate");

    client.shutdown().expect("shutdown");
    let _ = fs::remove_dir_all(temp);
}
