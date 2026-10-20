#!/usr/bin/env python3
"""Mock server that returns Assistant message with markdown bold emphasis and code blocks."""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18830

RICH_TEXT = """在这里我们针对系统进行了 **深度重构与架构升级**，请重点关注以下核心逻辑：

这是一段关键配置与示例函数：
```rust
// 核心状态机调度器实现
#[derive(Debug, Clone)]
pub struct EngineDispatcher {
    pub session_id: String,
    pub max_retry_limit: u32,
}

impl EngineDispatcher {
    pub async fn dispatch(&mut self, request_id: &str) -> Result<Option<String>, Error> {
        let active = true;
        if active && self.max_retry_limit > 0 {
            let path = "/var/log/pony.log";
            println!("Dispatching request: {}", request_id);
            return Ok(Some(path.to_string()));
        }
        Ok(None)
    }
}
```

注意：上述 **错误重试次数** 与 **网络超时拦截** 均已由网关强制守卫。
"""

def sse(handler, chunks):
    handler.send_response(200)
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.end_headers()
    for chunk in chunks:
        handler.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
        handler.wfile.flush()
    handler.wfile.write(b"data: [DONE]\n\n")
    handler.wfile.flush()

def chunk(delta, finish=None):
    return {
        "id": "chatcmpl-mock",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "mock",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    }

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    def log_message(self, *args): pass

    def do_POST(self):
        sse(self, [
            chunk({"role": "assistant", "content": ""}),
            chunk({"content": RICH_TEXT}),
            chunk({}, "stop"),
        ])

if __name__ == "__main__":
    print(f"[mock-rich] listening on 127.0.0.1:{PORT}", flush=True)
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
