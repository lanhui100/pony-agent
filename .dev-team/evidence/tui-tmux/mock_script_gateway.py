#!/usr/bin/env python3
"""Stub OpenAI-compatible gateway that replays a scripted tool-call plan.

Each assistant turn returns the next scripted tool call; once the script is
exhausted (or a tool result is the only new content) it streams a plain text
completion. Used to drive real runtime tool executions (TodoWrite, create_goal,
Plan, Ask) so the TUI's real-data panels can be verified end-to-end in tmux.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PLAN_PATH = sys.argv[1]
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 18800

with open(PLAN_PATH, encoding="utf-8") as handle:
    STEPS = json.load(handle)["steps"]

FINAL_TEXT = "已按脚本完成真实工具调用。"


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
        tool_results = sum(1 for m in messages if m.get("role") == "tool")
        print(f"[mock] POST tool_results={tool_results} steps={len(STEPS)}", flush=True)

        if tool_results < len(STEPS):
            step = STEPS[tool_results]
            sse(self, [
                chunk({"role": "assistant", "content": ""}),
                chunk({
                    "tool_calls": [{
                        "index": 0,
                        "id": f"call_mock_{tool_results + 1}",
                        "type": "function",
                        "function": {
                            "name": step["tool"],
                            "arguments": json.dumps(step.get("args", {}), ensure_ascii=False),
                        },
                    }]
                }),
                chunk({}, "tool_calls"),
            ])
            return

        sse(self, [
            chunk({"role": "assistant", "content": ""}),
            chunk({"content": FINAL_TEXT}),
            chunk({}, "stop"),
        ])


if __name__ == "__main__":
    print(f"[mock] listening on 127.0.0.1:{PORT} steps={len(STEPS)}", flush=True)
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
