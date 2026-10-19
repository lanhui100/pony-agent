#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# wave-38 L2-AT：TUI「启动目录即工作区」真实链路 E2E（Test Agent 冻结，Executor 只读）
#
# 契约来源：.dev-team/contract-matrix-tui-workspace.md §2「真实 TUI E2E」。
#
# 场景（物理链路，无 mock 断言替身）：
#   1. 隔离 HOME / XDG_CONFIG_HOME / XDG_DATA_HOME / XDG_CACHE_HOME。
#   2. 自包含 mock OpenAI SSE server（本脚本内嵌 python heredoc，不依赖 tools/ 未跟踪文件）：
#        第 1 次 POST /v1/chat/completions → 返回一个 `Read` 工具调用（arguments={"path":"package.json"}）；
#        第 2 次（带 role=tool）           → 返回最终中文回答，回显工具结果里读到的标记。
#   3. $ISO/project/package.json         写入唯一标记 TUI_WORKSPACE_MARKER_<uuid>；
#      $ISO/home/pony_agent/package.json 写入 DEFAULT_WS_MARKER_<uuid>。
#   4. tmux（唯一 socket tui-ws-$$，status off，remain-on-exit）以 -c "$ISO/project" 启动 pony，
#      发送触发词；捕屏断言：
#      注：触发词刻意不含路径/检索关键词——否则 LocalTurnPlanner 的 local preflight 会先行
#      本地决定工具调用，绕过 provider；本用例要覆盖「模型返回 Read → 工具执行 → 二次请求」的
#      真实网络链路（mock 第 1 次即返回 Read 工具调用）。
#         A1：屏幕含 TUI_WORKSPACE_MARKER_（工具确实读的是启动目录）
#         A2：屏幕不含 DEFAULT_WS_MARKER_（证明没落到注册表默认工作区 ~/pony_agent）
#         A3：屏幕无 panic；进程退出码存在且 != 101
#         A4：mock 请求流水（JSONL）第 2 次请求的 role=tool 内容含 TUI_WORKSPACE_MARKER_
#
# 退出码：
#   0 = 全绿（绿相）；1 = 断言失败（红相）；2 = Harness/构建错误；3 = [RED DEGRADED: STATIC-ONLY]（环境缺失）
#
# 物理收据：tests/acceptance/receipts/l2at-tui-workspace/
#   run.log / bin.log / build.log / mock-requests.jsonl / screen-final.txt / screen-final.ansi
#   screen-exit.txt / tui-exit-code.txt / assertions.json / summary.json / project-package.json /
#   default-ws-package.json / tmux-meta.txt
# ---------------------------------------------------------------------------
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RECEIPTS="$HERE/receipts/l2at-tui-workspace"
mkdir -p "$RECEIPTS"

LOG="$RECEIPTS/run.log"
: > "$LOG"
log() { echo "$*" | tee -a "$LOG"; }

PASS=0
FAIL=0
FAILED_NAMES=()
ok() { PASS=$((PASS + 1)); log "PASS: $1"; }
bad() { FAIL=$((FAIL + 1)); FAILED_NAMES+=("$1"); log "FAIL: $1"; }

# ---------------------------------------------------------------------------
# 0. 环境门禁（缺失 → 显式降级，不静默放行）
# ---------------------------------------------------------------------------
MISSING=""
command -v tmux >/dev/null 2>&1 || MISSING="$MISSING tmux"
command -v python3 >/dev/null 2>&1 || MISSING="$MISSING python3"
if [ -n "$MISSING" ]; then
  echo "[RED DEGRADED: STATIC-ONLY] missing:$MISSING" | tee "$RECEIPTS/degraded.log" | tee -a "$LOG"
  exit 3
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/wave38-tui-ws.XXXXXX")"
SOCK="tui-ws-$$"
SESS="wave38"
MOCK_PID=""
ISO="$WORK/iso"
TMUX_STARTED=0

cleanup() {
  if [ -n "${MOCK_PID:-}" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
    MOCK_PID=""
  fi
  if [ "$TMUX_STARTED" -eq 1 ]; then
    tmux -L "$SOCK" kill-server >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# 1. 二进制：优先 target/debug/pony；不存在或旧于 tui.rs → 构建并落盘 stdout/EXIT
# ---------------------------------------------------------------------------
TUI_SRC="$REPO/crates/pony-agent-core/src/bin/tui.rs"
BIN="$REPO/target/debug/pony"
NEED_BUILD=0
if [ ! -x "$BIN" ]; then
  NEED_BUILD=1
elif [ -e "$TUI_SRC" ] && [ "$TUI_SRC" -nt "$BIN" ]; then
  NEED_BUILD=1
fi
if [ "$NEED_BUILD" -eq 1 ]; then
  log "== building bin (stale or missing): cargo build -p pony-agent-core --bin pony"
  ( cd "$REPO" && cargo build -p pony-agent-core --bin pony ) > "$RECEIPTS/build.log" 2>&1
  BUILD_EXIT=$?
  echo "build_exit=$BUILD_EXIT" >> "$RECEIPTS/build.log"
  tail -3 "$RECEIPTS/build.log" | tee -a "$LOG"
  if [ "$BUILD_EXIT" -ne 0 ] || [ ! -x "$BIN" ]; then
    log "FATAL: build failed (exit $BUILD_EXIT), see $RECEIPTS/build.log"
    exit 2
  fi
else
  log "== using fresh bin: $BIN"
fi
{
  echo "bin=$BIN"
  echo "bin_mtime=$(date -Is -r "$BIN" 2>/dev/null || stat -c %y "$BIN")"
  echo "bin_sha256=$(sha256sum "$BIN" | awk '{print $1}')"
  echo "tui_src_mtime=$(date -Is -r "$TUI_SRC" 2>/dev/null || stat -c %y "$TUI_SRC")"
  echo "generated_at=$(date -Is)"
} > "$RECEIPTS/bin.log"
cat "$RECEIPTS/bin.log" | tee -a "$LOG"

# ---------------------------------------------------------------------------
# 2. 自包含 mock OpenAI SSE server
# ---------------------------------------------------------------------------
MOCK_LOG="$RECEIPTS/mock-requests.jsonl"
: > "$MOCK_LOG"
cat > "$WORK/mock.py" <<'PY'
import argparse
import json
import os
import re
import sys
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL_IDS = ["mock-alpha"]
MARKER_RE = re.compile(r"(?:TUI_WORKSPACE_MARKER_|DEFAULT_WS_MARKER_)[A-Za-z0-9-]+")
READ_ARGS = json.dumps({"path": "package.json", "description": "读取 package.json 确认工作区"}, ensure_ascii=False)


def message_text(message):
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict):
                text = item.get("text")
                if isinstance(text, str):
                    parts.append(text)
            elif isinstance(item, str):
                parts.append(item)
        return "".join(parts)
    if content is None:
        return ""
    return json.dumps(content, ensure_ascii=False)


class State:
    def __init__(self, log_path):
        self.log_path = log_path
        self.lock = threading.Lock()
        self.counter = 0

    def log(self, record):
        record = dict(record)
        record.setdefault("ts", time.time())
        line = json.dumps(record, ensure_ascii=False)
        with self.lock:
            with open(self.log_path, "a", encoding="utf-8") as fh:
                fh.write(line + "\n")

    def next_id(self):
        with self.lock:
            self.counter += 1
            return "chatcmpl-wave38-%d" % self.counter


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "wave38-mock/1.0"

    def log_message(self, fmt, *args):
        pass

    @property
    def state(self):
        return self.server.state

    # ---- HTTP plumbing -------------------------------------------------
    def _json(self, code, obj):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0:
            return ""
        return self.rfile.read(length).decode("utf-8", "replace")

    # ---- GET -----------------------------------------------------------
    def do_GET(self):  # noqa: N802
        path = urllib.parse.urlparse(self.path).path.rstrip("/")
        if path in ("/health", "/healthz"):
            self._json(200, {"ok": True})
            return
        if path.endswith("/models"):
            self.state.log({"method": "GET", "path": self.path})
            self._json(200, {"object": "list", "data": [{"id": mid, "object": "model", "owned_by": "wave38-mock"} for mid in MODEL_IDS]})
            return
        self._json(404, {"error": {"message": "unknown path %s" % self.path}})

    # ---- POST ----------------------------------------------------------
    def do_POST(self):  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path.rstrip("/")
        body_text = self._read_body()
        try:
            body = json.loads(body_text) if body_text else {}
        except json.JSONDecodeError:
            self._json(400, {"error": {"message": "invalid json"}})
            return
        if not path.endswith("/chat/completions"):
            self._json(404, {"error": {"message": "unknown path %s" % self.path}})
            return

        messages = body.get("messages") or []
        stream = bool(body.get("stream"))
        tool_texts = [message_text(m) for m in messages if str(m.get("role")) == "tool"]
        assistant_texts = [message_text(m) for m in messages if str(m.get("role")) == "assistant"]
        last_user = ""
        for m in reversed(messages):
            if str(m.get("role")) == "user":
                last_user = message_text(m)
                break
        auth = self.headers.get("Authorization") or ""
        self.state.log({
            "method": "POST",
            "path": self.path,
            "model": body.get("model"),
            "stream": stream,
            "msg_roles": [m.get("role") for m in messages],
            "last_user": last_user[:300],
            "tool_message_count": len(tool_texts),
            "tool_message_contents": tool_texts,
            "assistant_contents": [a[:300] for a in assistant_texts],
            "authorization": "<redacted>" if auth else "",
        })
        # 实时可观测（receipts/mock-server.out），便于区分「请求落错 server」类 harness 缺陷。
        print(json.dumps({
            "served": "chat_completions",
            "roles": [m.get("role") for m in messages],
            "tool_messages": len(tool_texts),
        }, ensure_ascii=False), flush=True)

        if tool_texts:
            markers = []
            for text in tool_texts:
                markers.extend(MARKER_RE.findall(text))
            if markers:
                answer = "已读取工作区文件，文件中的标记是：%s 。任务完成。" % markers[0]
            else:
                answer = "工具读取结果中未找到任何工作区标记，任务完成。"
            if stream:
                self._stream_text(answer)
            else:
                self._json_text(answer)
            return

        if stream:
            self._stream_tool_call()
        else:
            self._json_tool_call()

    # ---- JSON (non-stream) --------------------------------------------
    def _json_text(self, text):
        self._json(200, {
            "id": self.state.next_id(),
            "object": "chat.completion",
            "created": int(time.time()),
            "model": MODEL_IDS[0],
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
            "usage": {"prompt_tokens": 32, "completion_tokens": 64, "total_tokens": 96},
        })

    def _json_tool_call(self):
        self._json(200, {
            "id": self.state.next_id(),
            "object": "chat.completion",
            "created": int(time.time()),
            "model": MODEL_IDS[0],
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "我先读取 package.json 确认启动目录。",
                    "tool_calls": [{
                        "id": "call_wave38_read_1",
                        "type": "function",
                        "function": {"name": "Read", "arguments": READ_ARGS},
                    }],
                },
            }],
            "usage": {"prompt_tokens": 24, "completion_tokens": 32, "total_tokens": 56},
        })

    # ---- SSE (stream) --------------------------------------------------
    def _start_sse(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "keep-alive")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()

    def _chunk_write(self, payload):
        self.wfile.write(("%X\r\n" % len(payload)).encode("ascii"))
        self.wfile.write(payload)
        self.wfile.write(b"\r\n")
        self.wfile.flush()

    def _sse(self, obj):
        payload = obj if isinstance(obj, str) else json.dumps(obj, ensure_ascii=False)
        self._chunk_write(("data: %s\n\n" % payload).encode("utf-8"))

    def _end_sse(self):
        self._sse("[DONE]")
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()

    def _delta(self, delta, finish=None):
        return {
            "id": "chatcmpl-wave38",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": MODEL_IDS[0],
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        }

    def _usage(self):
        return {
            "id": "chatcmpl-wave38",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": MODEL_IDS[0],
            "choices": [],
            "usage": {"prompt_tokens": 32, "completion_tokens": 64, "total_tokens": 96},
        }

    def _stream_text(self, text):
        try:
            self._start_sse()
            self._sse(self._delta({"role": "assistant", "content": ""}))
            for i in range(0, len(text), 8):
                self._sse(self._delta({"content": text[i:i + 8]}))
                time.sleep(0.01)
            self._sse(self._delta({}, "stop"))
            self._sse(self._usage())
            self._end_sse()
        except (BrokenPipeError, ConnectionResetError):
            pass

    def _stream_tool_call(self):
        try:
            self._start_sse()
            self._sse(self._delta({"role": "assistant", "content": ""}))
            first = "我先读取 package.json 确认启动目录。"
            for i in range(0, len(first), 8):
                self._sse(self._delta({"content": first[i:i + 8]}))
                time.sleep(0.01)
            self._sse(self._delta({"tool_calls": [{
                "index": 0,
                "id": "call_wave38_read_1",
                "type": "function",
                "function": {"name": "Read", "arguments": ""},
            }]}))
            for start in (0, 8, 16, 24):
                fragment = READ_ARGS[start:start + 8]
                if not fragment:
                    continue
                self._sse(self._delta({"tool_calls": [{"index": 0, "function": {"arguments": fragment}}]}))
                time.sleep(0.01)
            if len(READ_ARGS) > 32:
                self._sse(self._delta({"tool_calls": [{"index": 0, "function": {"arguments": READ_ARGS[32:]}}]}))
            self._sse(self._delta({}, "tool_calls"))
            self._sse(self._usage())
            self._end_sse()
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    httpd = ThreadingHTTPServer((args.host, args.port), Handler)
    httpd.daemon_threads = True
    httpd.state = State(args.log)
    print(json.dumps({"event": "listening", "port": httpd.server_address[1]}), flush=True)
    try:
        httpd.serve_forever(poll_interval=0.1)
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
PY

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

MOCK_PORT="$(free_port)"
MOCK_OUT="$RECEIPTS/mock-server.out"
python3 "$WORK/mock.py" --port "$MOCK_PORT" --log "$MOCK_LOG" > "$MOCK_OUT" 2>&1 &
MOCK_PID=$!
MOCK_READY=0
for _ in $(seq 1 100); do
  if grep -q '"listening"' "$MOCK_OUT" 2>/dev/null; then MOCK_READY=1; break; fi
  kill -0 "$MOCK_PID" 2>/dev/null || break
  sleep 0.05
done
if [ "$MOCK_READY" -ne 1 ]; then
  log "FATAL: mock server did not start (see $MOCK_OUT)"
  exit 2
fi
log "== mock listening on 127.0.0.1:$MOCK_PORT (pid $MOCK_PID)"

# ---------------------------------------------------------------------------
# 3. 隔离夹具：HOME / XDG / project / 默认工作区
# ---------------------------------------------------------------------------
UUID="$(python3 -c 'import uuid; print(uuid.uuid4())')"
MARKER="TUI_WORKSPACE_MARKER_${UUID}"
DEFAULT_MARKER="DEFAULT_WS_MARKER_${UUID}"

mkdir -p "$ISO/home/pony_agent" "$ISO/project" "$ISO/xdg-config/pony-agent" "$ISO/xdg-data" "$ISO/xdg-cache"

python3 - "$ISO/project/package.json" "$MARKER" <<'PY'
import json, sys
json.dump({"name": "wave38-project", "version": "0.0.1", "private": True, "marker": sys.argv[2]},
          open(sys.argv[1], "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY
python3 - "$ISO/home/pony_agent/package.json" "$DEFAULT_MARKER" <<'PY'
import json, sys
json.dump({"name": "pony_agent-default", "version": "0.0.0", "marker": sys.argv[2]},
          open(sys.argv[1], "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY
cp "$ISO/project/package.json" "$RECEIPTS/project-package.json"
cp "$ISO/home/pony_agent/package.json" "$RECEIPTS/default-ws-package.json"

# provider 注册表（fakep → 本脚本 mock；参考 tools/tui-audit/run_audit.sh 的 fakep 形态，但自包含）
python3 - "$ISO/xdg-config/pony-agent/providers.json" "$MOCK_PORT" <<'PY'
import json, sys
path, port = sys.argv[1], int(sys.argv[2])
model = {
    "id": "mock-alpha",
    "name": "mock-alpha",
    "model": "mock-alpha",
    "temperature": 0.2,
    "maxOutputTokens": 64000,
    "capabilityPreset": "open-ai-chat",
    "reasoningEffort": None,
    "reasoningBudgetTokens": None,
    "protocol": "openai-completions",
    "capabilities": {
        "contextWindowTokens": 256000,
        "supportsTools": True,
        "supportsStreaming": True,
        "supportsImageInput": True,
        "supportsReasoning": False,
        "supportsVideoInput": False,
        "supportsAudioInput": False,
        "supportsImageOutput": False,
        "supportsVideoOutput": False,
        "supportsAudioOutput": False,
    },
}
provider = {
    "id": "fakep",
    "name": "FakeProvider",
    "protocol": "openai-completions",
    "base_url": "http://127.0.0.1:%d/v1" % port,
    "auth_type": "bearer",
    "api_key_env_var": "MOCK_API_KEY",
    "secret_ref": "",
    "api_key_value": "mock-key",
    "supported_protocols": ["openai-completions"],
    "endpoints": [],
    "models": [model],
    "selected_model_id": "mock-alpha",
}
with open(path, "w", encoding="utf-8") as fh:
    json.dump({"providers": [provider], "selected_provider_id": "fakep"}, fh, ensure_ascii=False, indent=2)
PY
log "== marker(project)=$MARKER"
log "== marker(default)=$DEFAULT_MARKER"

# ---------------------------------------------------------------------------
# 4. tmux 启动真实 TUI（-c 启动目录 = $ISO/project）
# ---------------------------------------------------------------------------
cat > "$WORK/run_tui.sh" <<RUNNER
#!/usr/bin/env bash
cd "$ISO/project" || exit 90
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \\
  -u NODE_USE_ENV_PROXY -u NO_COLOR \\
  HOME="$ISO/home" \\
  XDG_CONFIG_HOME="$ISO/xdg-config" \\
  XDG_DATA_HOME="$ISO/xdg-data" \\
  XDG_CACHE_HOME="$ISO/xdg-cache" \\
  PONYLLM_GATEWAY_URL="http://127.0.0.1:$MOCK_PORT" \\
  PONYLLM_API_KEY="mock-key" \\
  LANG="\${LANG:-C.UTF-8}" TERM=screen-256color \\
  "$BIN"
rc=\$?
echo "\$rc" > "$RECEIPTS/tui-exit-code.txt"
exit \$rc
RUNNER
chmod +x "$WORK/run_tui.sh"

tmux -L "$SOCK" -f /dev/null start-server >/dev/null 2>&1 || true
TMUX_STARTED=1
tmux -L "$SOCK" set-option -g status off >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g remain-on-exit on >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g history-limit 20000 >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g default-terminal "screen-256color" >/dev/null 2>&1 || true
tmux -L "$SOCK" new-session -d -s "$SESS" -x 120 -y 36 -c "$ISO/project" "$WORK/run_tui.sh" >/dev/null 2>&1
NEW_SESSION_EXIT=$?
if [ "$NEW_SESSION_EXIT" -ne 0 ]; then
  log "FATAL: tmux new-session failed (exit $NEW_SESSION_EXIT)"
  exit 2
fi

capture_pane() { tmux -L "$SOCK" capture-pane -t "$SESS" -p 2>/dev/null || true; }
capture_pane_ansi() { tmux -L "$SOCK" capture-pane -t "$SESS" -p -e 2>/dev/null || true; }

READY=0
for _ in $(seq 1 150); do
  if capture_pane | grep -q '›'; then READY=1; break; fi
  sleep 0.2
done
if [ "$READY" -ne 1 ]; then
  log "FATAL: TUI composer prompt not rendered within 30s"
  capture_pane > "$RECEIPTS/screen-not-ready.txt"
  exit 2
fi
log "== TUI ready (composer prompt visible), sending trigger"
sleep 0.3

tmux -L "$SOCK" send-keys -t "$SESS" -l "请帮我对这个项目做一个简短的评估"
tmux -L "$SOCK" send-keys -t "$SESS" Enter

FINAL=0
for _ in $(seq 1 450); do
  out="$(capture_pane)"
  if grep -q "$MARKER" <<<"$out" || grep -q "$DEFAULT_MARKER" <<<"$out"; then FINAL=1; break; fi
  sleep 0.2
done
if [ "$FINAL" -ne 1 ]; then
  log "WARN: marker not observed on screen within 90s (continuing to assertions)"
fi
sleep 0.5
capture_pane > "$RECEIPTS/screen-final.txt"
capture_pane_ansi > "$RECEIPTS/screen-final.ansi"

# 让 TUI 走 /quit 正常退出，落盘真实退出码（无 panic 的可判定证据）
tmux -L "$SOCK" send-keys -t "$SESS" -l "/quit"
tmux -L "$SOCK" send-keys -t "$SESS" Enter
EXITED=0
for _ in $(seq 1 100); do
  if [ -s "$RECEIPTS/tui-exit-code.txt" ]; then EXITED=1; break; fi
  sleep 0.2
done
if [ "$EXITED" -ne 1 ]; then
  log "WARN: TUI did not exit via /quit within 20s; killing session"
  tmux -L "$SOCK" kill-session -t "$SESS" >/dev/null 2>&1 || true
  echo "killed" > "$RECEIPTS/tui-exit-code.txt"
fi
sleep 0.3
capture_pane > "$RECEIPTS/screen-exit.txt"
{
  tmux -L "$SOCK" list-panes -a -F 'dead=#{pane_dead} status=#{pane_dead_status} size=#{pane_width}x#{pane_height} cmd=#{pane_current_command}' 2>&1 || true
  tmux -L "$SOCK" list-sessions -F 'session=#{session_name} windows=#{session_windows}' 2>&1 || true
} > "$RECEIPTS/tmux-meta.txt"

# TUI 自身把 provider/runtime 日志 dup2 到 $HOME/.pony-agent/tui-logs/tui-runtime.log；
# 在隔离 HOME 被清理前落到 receipts，作为「请求去了哪里」的物理证据。
if [ -f "$ISO/home/.pony-agent/tui-logs/tui-runtime.log" ]; then
  cp "$ISO/home/.pony-agent/tui-logs/tui-runtime.log" "$RECEIPTS/tui-runtime.log"
else
  echo "(no tui-runtime.log found under isolated HOME)" > "$RECEIPTS/tui-runtime.log"
fi

# ---------------------------------------------------------------------------
# 5. 断言（内容级、可复核；每条都引用物理证据）
# ---------------------------------------------------------------------------
SCREEN="$RECEIPTS/screen-final.txt"
SCREEN_ALL="$(cat "$SCREEN" "$RECEIPTS/screen-exit.txt" 2>/dev/null)"

if grep -q "$MARKER" "$SCREEN"; then
  ok "A1 屏幕含 $MARKER（工具读到的是启动目录 $ISO/project）"
else
  bad "A1 屏幕未出现 $MARKER（工具未读取启动目录的 package.json）"
fi

if grep -q "$DEFAULT_MARKER" "$SCREEN"; then
  bad "A2 屏幕出现 $DEFAULT_MARKER（工具落到了注册表默认工作区 ~/pony_agent）"
else
  ok "A2 屏幕不含 $DEFAULT_MARKER（未落到默认工作区）"
fi

if grep -qE "panicked at|thread 'main' panicked|RUST_BACKTRACE" <<<"$SCREEN_ALL"; then
  bad "A3 屏幕出现 panic 痕迹（见 screen-final.txt / screen-exit.txt）"
else
  ok "A3 屏幕无 panic 痕迹"
fi

EXIT_CODE="$(cat "$RECEIPTS/tui-exit-code.txt" 2>/dev/null || echo "missing")"
if [ "$EXIT_CODE" = "0" ]; then
  ok "A3b TUI 正常退出（exit code 0）"
elif [ "$EXIT_CODE" = "101" ]; then
  bad "A3b TUI panic 退出（exit code 101）"
elif [ "$EXIT_CODE" = "missing" ]; then
  bad "A3b 未取得 TUI 真实退出码（收据缺失）"
elif [ "$EXIT_CODE" = "killed" ]; then
  bad "A3b TUI 未在 20s 内响应 /quit（被 kill，无真实退出码）"
else
  bad "A3b TUI 非零退出（exit code $EXIT_CODE）"
fi

# A4：mock 流水断言（最后一次携带 role=tool 的请求，其工具结果必须含项目标记）
python3 - "$MOCK_LOG" "$MARKER" "$DEFAULT_MARKER" "$RECEIPTS/assertions.json" <<'PY'
import json, sys
log_path, marker, default_marker, out_path = sys.argv[1:5]
records = []
with open(log_path, "r", encoding="utf-8") as fh:
    for line in fh:
        line = line.strip()
        if line:
            records.append(json.loads(line))
posts = [r for r in records if r.get("method") == "POST" and r.get("path", "").endswith("/chat/completions")]
tool_posts = [r for r in posts if (r.get("tool_message_count") or 0) > 0]
result = {
    "chat_completions_requests": len(posts),
    "tool_bearing_requests": len(tool_posts),
    "request_roles": [r.get("msg_roles") for r in posts],
    "tool_message_counts": [r.get("tool_message_count") for r in posts],
    "last_tool_request_contains_project_marker": False,
    "last_tool_request_contains_default_marker": False,
}
if tool_posts:
    contents = "\n".join(tool_posts[-1].get("tool_message_contents") or [])
    result["last_tool_request_contains_project_marker"] = marker in contents
    result["last_tool_request_contains_default_marker"] = default_marker in contents
    result["last_tool_request_tool_preview"] = contents[:800]
json.dump(result, open(out_path, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
print(json.dumps(result, ensure_ascii=False))
PY
A4_EXIT=$?
if [ "$A4_EXIT" -ne 0 ]; then
  bad "A4 mock 流水断言脚本执行失败（exit $A4_EXIT）"
else
  A4_TOOL_POSTS="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["tool_bearing_requests"])' "$RECEIPTS/assertions.json" 2>/dev/null)"
  A4_PROJECT="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["last_tool_request_contains_project_marker"])' "$RECEIPTS/assertions.json" 2>/dev/null)"
  A4_DEFAULT="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["last_tool_request_contains_default_marker"])' "$RECEIPTS/assertions.json" 2>/dev/null)"
  if [ "${A4_TOOL_POSTS:-0}" = "0" ]; then
    bad "A4pre mock 未收到任何携带 role=tool 的请求（工具链未执行，见 assertions.json）"
  else
    ok "A4pre mock 收到携带 role=tool 的请求 $A4_TOOL_POSTS 次（工具链已真实执行）"
  fi
  if [ "$A4_PROJECT" = "True" ]; then
    ok "A4 最后一次 tool 请求的工具结果含 $MARKER（工具执行根 = 启动目录）"
  else
    bad "A4 最后一次 tool 请求的工具结果不含 $MARKER（工具执行根不是启动目录，见 assertions.json）"
  fi
  if [ "$A4_DEFAULT" = "True" ]; then
    bad "A4b 最后一次 tool 请求的工具结果含 $DEFAULT_MARKER（落到默认工作区 ~/pony_agent）"
  else
    ok "A4b 最后一次 tool 请求的工具结果不含 $DEFAULT_MARKER"
  fi
fi

TOTAL=$((PASS + FAIL))
VERDICT="GREEN"
[ "$FAIL" -ne 0 ] && VERDICT="RED"
FAILED_JOINED="$(printf '%s\n' "${FAILED_NAMES[@]:-}")"
python3 - "$RECEIPTS/summary.json" "$VERDICT" "$PASS" "$FAIL" "$TOTAL" "$FAILED_JOINED" <<'PY'
import json, os, sys
out, verdict, passed, failed, total, failed_names = sys.argv[1:7]
json.dump({
    "wave": "wave-38",
    "suite": "l2at-tui-workspace",
    "verdict": verdict,
    "pass": int(passed),
    "fail": int(failed),
    "total": int(total),
    "failed_assertions": [line for line in failed_names.splitlines() if line.strip()],
    "generated_at": __import__("datetime").datetime.now().isoformat(timespec="seconds"),
    "receipts": os.path.dirname(out),
}, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY

log "== RESULT: $VERDICT (pass=$PASS fail=$FAIL total=$TOTAL)"
if [ "$FAIL" -ne 0 ]; then
  log "== receipts: $RECEIPTS"
  exit 1
fi
exit 0
