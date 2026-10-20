#!/usr/bin/env python3
"""Stub OpenAI-compatible gateway for TUI end-to-end verification.

First model call returns a tool call for a chosen tool (default `Ask`); once a
tool result is present in the transcript it streams a plain text completion.
This lets the real pony runtime execute a real host-mediated tool so the TUI's
pending-request dialog path can be exercised without depending on a live model.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

TOOL = sys.argv[1] if len(sys.argv) > 1 else "Ask"
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 18731

ASK_ARGS = {
    "question": "本次验证要跑哪种测试环境？",
    "description": "TUI 端到端验证：Ask 交互面板",
}

TOOL_ARGS = {
    "Ask": ASK_ARGS,
    "Run": {
        "command": "echo tui-permission-probe",
        "purpose": "TUI 权限审批面板验证",
        "timeoutMs": 5000,
    },
    "Write": {
        "path": "tui-permission-probe.txt",
        "content": "probe",
    },
}

ARGS = TOOL_ARGS.get(TOOL, {"question": "fallback question"})


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

    def log_message(self, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        messages = body.get("messages", [])
        has_tool_result = any(m.get("role") == "tool" for m in messages)
        print(f"[mock] POST tools_present={bool(body.get('tools'))} tool_result={has_tool_result}", flush=True)

        if has_tool_result:
            text = "已收到你的回答，谢谢。"
            sse(self, [
                chunk({"role": "assistant", "content": ""}),
                chunk({"content": text}),
                chunk({}, "stop"),
            ])
            return

        sse(self, [
            chunk({"role": "assistant", "content": ""}),
            chunk({
                "tool_calls": [{
                    "index": 0,
                    "id": "call_mock_1",
                    "type": "function",
                    "function": {"name": TOOL, "arguments": json.dumps(ARGS, ensure_ascii=False)},
                }]
            }),
            chunk({}, "tool_calls"),
        ])


if __name__ == "__main__":
    print(f"[mock] listening on 127.0.0.1:{PORT} tool={TOOL}", flush=True)
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
