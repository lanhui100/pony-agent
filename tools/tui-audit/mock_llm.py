#!/usr/bin/env python3
"""OpenAI-compatible mock LLM server for the pony TUI audit harness.

Endpoints
---------
GET  /v1/models              -> {"object":"list","data":[{"id":...}, ...]}
POST /v1/chat/completions    -> streaming (SSE) and non-streaming responses

Behaviour is selected per request (no state needed):
  * messages contain a role=tool message        -> final Chinese markdown answer
  * last user text contains "401" / "认证失败"   -> HTTP 401 JSON error
  * last user text contains "慢速"               -> slow SSE stream (~6s)
  * last user text contains "长文"               -> long SSE stream (~60 lines)
  * otherwise (first turn)                       -> Chinese markdown + one
                                                    tool_call (Read package.json)
                                                    finish_reason=tool_calls

Overrides:
  * env MOCK_SCENARIO=ok|401|slow|long forces behaviour for every request
  * query parameter ?scenario=ok|401|slow|long on /v1/chat/completions

Streaming format follows the real provider parser
(crates/pony-agent-core/src/agent/provider/openai_sse.rs):
`data: {json}\n\n` events terminated by `data: [DONE]\n\n`, with
choices[0].delta.content / choices[0].delta.tool_calls[].function.{name,arguments}.

Every request is appended to the --log JSONL file (physical receipt).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL_IDS = ["mock-alpha", "mock-beta", "mock-gamma"]


def model_ids(count: int) -> list[str]:
    """Deterministic catalogue of `count` ids (first three stay the historic ones)."""
    count = max(1, count)
    ids = list(MODEL_IDS)
    i = len(ids) + 1
    while len(ids) < count:
        ids.append(f"mock-model-{i:02d}")
        i += 1
    return ids[:count]

FIRST_TURN_TEXT = """## 先看一下项目

我先读取 `package.json`，确认项目名称和版本号，再给你结论。

- 工作区：当前目录
- 目标文件：`package.json`
"""

FINAL_TEXT = """## 结论

`package.json` 显示这是 **pony-agent**，版本 `0.1.125`。

```json
{
  "name": "pony-agent",
  "version": "0.1.125"
}
```

- 包管理器：npm
- 关键信息：私有项目、使用 Vite 构建

如果需要，我可以继续列出依赖清单。"""

LONG_TEXT = "\n".join(
    ["## 长内容滚动测试", ""]
    + [f"- 第 {i:02d} 行：这是一段用于验证滚动与折行的中文正文，包含 `inline code` 与 **加粗** 片段。" for i in range(1, 41)]
    + ["", "```rust", "fn main() {", '    println!("scroll test");', "}", "```", "", "## 结束", "这是最后一行，滚动到底部时应完整可见。"]
)

SLOW_CHUNKS = [
    "## 正在分析",
    "\n\n",
    "我先把问题拆成几个部分：",
    "\n\n- 第一部分：读取上下文",
    "\n- 第二部分：整理结论",
    "\n- 第三部分：输出建议",
    "\n\n这部分内容会**逐字**到达，用于截取流式进行中的画面。",
    "\n\n`streaming` 状态应该稳定显示在底部。",
    "\n\n最后一行：流式结束。",
]

# Chinese markdown split into small pieces to exercise incremental rendering.
def split_text(text: str, size: int = 6) -> list[str]:
    return [text[i : i + size] for i in range(0, len(text), size)]


class State:
    def __init__(
        self,
        log_path: str,
        chunk_delay_ms: int,
        first_token_delay_ms: int,
        models: list[str] | None = None,
    ):
        self.log_path = log_path
        self.chunk_delay_ms = chunk_delay_ms
        self.first_token_delay_ms = first_token_delay_ms
        self.models = models or list(MODEL_IDS)
        self.lock = threading.Lock()
        self.counter = 0

    def log(self, record: dict) -> None:
        record = dict(record)
        record.setdefault("ts", time.time())
        line = json.dumps(record, ensure_ascii=False)
        with self.lock:
            with open(self.log_path, "a", encoding="utf-8") as fh:
                fh.write(line + "\n")


def message_text(message: dict) -> str:
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict):
                if item.get("type") in ("text", "input_text") and isinstance(item.get("text"), str):
                    parts.append(item["text"])
            elif isinstance(item, str):
                parts.append(item)
        return "".join(parts)
    return ""


def scenario_for(messages: list[dict], forced: str | None) -> str:
    if forced:
        return forced
    has_tool = any(str(m.get("role")) == "tool" for m in messages)
    last_user = ""
    for m in reversed(messages):
        if str(m.get("role")) == "user":
            last_user = message_text(m)
            break
    low = last_user.lower()
    if "401" in low or "unauthorized" in low or "认证失败" in last_user:
        return "401"
    if "慢速" in last_user:
        return "slow"
    if "长文" in last_user:
        return "long"
    if has_tool:
        return "final"
    return "tool"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "pony-audit-mock/1.0"

    # -- plumbing ---------------------------------------------------------
    def log_message(self, fmt, *args):  # silence default stderr logging
        pass

    @property
    def state(self) -> State:
        return self.server.state  # type: ignore[attr-defined]

    def _json(self, code: int, obj: dict) -> None:
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_body(self) -> str:
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0:
            return ""
        return self.rfile.read(length).decode("utf-8", "replace")

    # -- HTTP verbs -------------------------------------------------------
    def do_GET(self):  # noqa: N802
        path = urllib.parse.urlparse(self.path).path.rstrip("/")
        if path in ("/health", "/healthz"):
            self._json(200, {"ok": True})
            return
        if path.endswith("/models"):
            self.state.log({"path": self.path, "authorization": self.headers.get("Authorization", ""), "response": "models"})
            self._json(
                200,
                {
                    "object": "list",
                    "data": [
                        {"id": mid, "object": "model", "owned_by": "audit-mock"}
                        for mid in self.state.models
                    ],
                },
            )
            return
        self._json(404, {"error": {"message": f"unknown path {self.path}"}})

    def do_POST(self):  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path.rstrip("/")
        query = urllib.parse.parse_qs(parsed.query)
        body_text = self._read_body()

        try:
            body = json.loads(body_text) if body_text else {}
        except json.JSONDecodeError:
            self._json(400, {"error": {"message": "invalid json body"}})
            return

        if not path.endswith("/chat/completions"):
            self._json(404, {"error": {"message": f"unknown path {self.path}"}})
            return

        messages = body.get("messages") or []
        model = body.get("model") or "mock-alpha"
        stream = bool(body.get("stream"))
        forced = os.environ.get("MOCK_SCENARIO") or (query.get("scenario", [None])[0])
        scenario = scenario_for(messages, forced if forced in ("ok", "401", "slow", "long") else None)
        if forced == "ok":
            scenario = "tool"

        self.state.log(
            {
                "path": self.path,
                "query": parsed.query,
                "model": model,
                "stream": stream,
                "authorization": self.headers.get("Authorization", ""),
                "msg_roles": [m.get("role") for m in messages],
                "last_user": next(
                    (message_text(m) for m in reversed(messages) if m.get("role") == "user"), ""
                )[:200],
                "n_tool_messages": sum(1 for m in messages if m.get("role") == "tool"),
                "n_tools_declared": len(body.get("tools") or []),
                "scenario": scenario,
            }
        )

        if scenario == "401":
            self._json(
                401,
                {
                    "error": {
                        "message": "mock 401 unauthorized: invalid api key (audit scenario)",
                        "type": "invalid_request_error",
                        "code": "invalid_api_key",
                    }
                },
            )
            return

        if stream:
            self._stream_chat(model, scenario)
        else:
            self._json_chat(model, scenario)

    # -- response builders ------------------------------------------------
    def _chat_id(self) -> str:
        with self.state.lock:
            self.state.counter += 1
            return f"chatcmpl-mock-{self.state.counter}"

    def _json_chat(self, model: str, scenario: str) -> None:
        if scenario in ("final", "slow", "long"):
            text = LONG_TEXT if scenario == "long" else FINAL_TEXT
            self._json(
                200,
                {
                    "id": self._chat_id(),
                    "object": "chat.completion",
                    "created": int(time.time()),
                    "model": model,
                    "choices": [
                        {
                            "index": 0,
                            "finish_reason": "stop",
                            "message": {"role": "assistant", "content": text},
                        }
                    ],
                    "usage": {"prompt_tokens": 32, "completion_tokens": 64, "total_tokens": 96},
                },
            )
            return
        self._json(
            200,
            {
                "id": self._chat_id(),
                "object": "chat.completion",
                "created": int(time.time()),
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "finish_reason": "tool_calls",
                        "message": {
                            "role": "assistant",
                            "content": FIRST_TURN_TEXT,
                            "tool_calls": [
                                {
                                    "id": "call_mock_read_1",
                                    "type": "function",
                                    "function": {
                                        "name": "Read",
                                        "arguments": json.dumps({"path": "package.json", "description": "读取 package.json 确认项目信息"}),
                                    },
                                }
                            ],
                        },
                    }
                ],
                "usage": {"prompt_tokens": 24, "completion_tokens": 32, "total_tokens": 56},
            },
        )

    # -- SSE --------------------------------------------------------------
    def _start_sse(self) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "keep-alive")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()

    def _chunk_write(self, payload: bytes) -> None:
        self.wfile.write(f"{len(payload):X}\r\n".encode("ascii"))
        self.wfile.write(payload)
        self.wfile.write(b"\r\n")
        self.wfile.flush()

    def _sse(self, obj: dict | str) -> None:
        if isinstance(obj, str):
            data = f"data: {obj}\n\n"
        else:
            data = "data: " + json.dumps(obj, ensure_ascii=False) + "\n\n"
        self._chunk_write(data.encode("utf-8"))

    def _end_sse(self) -> None:
        self._sse("[DONE]")
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()

    def _delta(self, model: str, delta: dict, finish: str | None = None) -> dict:
        return {
            "id": "chatcmpl-mock",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        }

    def _stream_chat(self, model: str, scenario: str) -> None:
        try:
            self._start_sse()
            if scenario == "slow":
                delay = 0.22
                texts = SLOW_CHUNKS
            elif scenario == "long":
                delay = 0.0
                texts = split_text(LONG_TEXT, 12)
            elif scenario == "tool":
                delay = self.state.chunk_delay_ms / 1000.0
                texts = split_text(FIRST_TURN_TEXT, 8)
            else:
                delay = self.state.chunk_delay_ms / 1000.0
                texts = split_text(FINAL_TEXT, 8)

            if self.state.first_token_delay_ms:
                time.sleep(self.state.first_token_delay_ms / 1000.0)

            self._sse(self._delta(model, {"role": "assistant", "content": ""}))
            for piece in texts:
                if not piece:
                    continue
                self._sse(self._delta(model, {"content": piece}))
                if delay:
                    time.sleep(delay)

            if scenario == "tool":
                args = json.dumps({"path": "package.json", "description": "读取 package.json 确认项目信息"})
                self._sse(
                    self._delta(
                        model,
                        {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_mock_read_1",
                                    "type": "function",
                                    "function": {"name": "Read", "arguments": ""},
                                }
                            ]
                        },
                    )
                )
                # Split arguments across chunks, like real providers do.
                for fragment in (args[:6], args[6:14], args[14:]):
                    self._sse(
                        self._delta(
                            model,
                            {"tool_calls": [{"index": 0, "function": {"arguments": fragment}}]},
                        )
                    )
                self._sse(self._delta(model, {}, "tool_calls"))
            else:
                self._sse(self._delta(model, {}, "stop"))

            self._sse(
                {
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "created": int(time.time()),
                    "model": model,
                    "choices": [],
                    "usage": {"prompt_tokens": 32, "completion_tokens": 64, "total_tokens": 96},
                }
            )
            self._end_sse()
        except (BrokenPipeError, ConnectionResetError):
            pass


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--log", required=True, help="JSONL request receipt path")
    ap.add_argument("--chunk-delay-ms", type=int, default=0)
    ap.add_argument("--first-token-delay-ms", type=int, default=0)
    ap.add_argument(
        "--models-count",
        type=int,
        default=len(MODEL_IDS),
        help="number of ids advertised by GET /v1/models (default: 3)",
    )
    args = ap.parse_args(argv)

    state = State(
        args.log,
        args.chunk_delay_ms,
        args.first_token_delay_ms,
        model_ids(args.models_count),
    )
    httpd = ThreadingHTTPServer((args.host, args.port), Handler)
    httpd.daemon_threads = True
    httpd.state = state  # type: ignore[attr-defined]
    port = httpd.server_address[1]
    print(json.dumps({"event": "listening", "host": args.host, "port": port}), flush=True)
    try:
        httpd.serve_forever(poll_interval=0.1)
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
