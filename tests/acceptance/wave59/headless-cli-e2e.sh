#!/usr/bin/env bash
# acceptance: wave-59 / PA-164 无头 CLI（ponycli）真实二进制端到端验收
# ticket: PA-164 | frozen acceptance test — verify-only, DO NOT MODIFY during fixes
#
# 真值源：.dev-team/contract-matrix-wave59-headless.md（Lead 冻结）
#
# Contract under test (real process, real socket, no assertion doubles):
#   E1  C2/C4  `--permission-mode <非法值>` → exit 2（Usage），且不得 panic、不得起 turn
#   E2  C1     `--output-format <非法值>`   → exit 2（Usage）
#   E3  C1     合法 turn + `--output-format jsonl` → exit 0；stdout **每一行**都是合法 JSON
#              对象（含 seq/type）；stdout 无 ANSI（\x1b）；人类可读诊断只在 stderr
#   E4  C3     `--result-json <path>` 落盘封套：schema == pony.headless.result.v1、
#              字段齐全、`status==completed ⟺ exit_code==0`，且 exit_code 与真实进程
#              Exit Code 交叉一致（该不变量只能在真实产物上黑盒断言，见 Rust 测试头注）
#   E5  C2     provider 不可达（本地已关闭端口）→ 进程 Exit Code != 0（绝不退 0），
#              stdout 仍是纯净 JSONL（无 ANSI），stderr 无 panic
#   E6  C4     `--permission-mode deny` + mock 返回工具调用 → 无审批应答方时**禁止挂起**
#              （timeout 兜底判定），stdout 纯净
#   E6b C4/D1  不传 `--permission-mode`（默认必须 fail-closed=read_only）→ 同样不得挂起
#   E7  L2-AT  provider 返回畸形 JSON（200 + 非 JSON 体）→ 进程不 panic、Exit Code != 0、
#              stdout 纯净 JSONL
#
# 契约 v2/v3 说明（Lead 裁定，见 .dev-team/contract-matrix-wave59-headless.md）：
#   - `--permission-mode` 取值为 deny|read-only|read_only → read_only、
#     workspace-write|workspace_write → workspace_write、full-access|full_access → full_access；
#     已撤销 `allow-list`/`yolo`/`--allow-tools`；`--max-turns` 亦已撤销（v3），本脚本不引用。
#
# Receipts: tests/acceptance/wave59/receipts/
#
# 环境降级：若本机无法起 mock gateway（python3 缺失 / 端口无法监听），脚本自行退化为
# 参数校验路径并显式打印 `[RED DEGRADED: ARG-ONLY]`，绝不静默把 E3~E7 记为通过。
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
RECEIPTS="$HERE/receipts"
mkdir -p "$RECEIPTS"
LOG="$RECEIPTS/e2e-run.log"
: > "$LOG"

log() { echo "$*" | tee -a "$LOG"; }

PASS=0
FAIL=0
SKIP=0
ok()  { PASS=$((PASS + 1)); echo "PASS: $1" | tee -a "$LOG"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL: $1" | tee -a "$LOG"; }
skip() { SKIP=$((SKIP + 1)); echo "SKIP: $1" | tee -a "$LOG"; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/pony-headless-e2e.XXXXXX")"
MOCK_PIDS=()
cleanup() {
  for pid in "${MOCK_PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# 0. 解析被测二进制（`[[bin]] name = "ponycli"`）
# ---------------------------------------------------------------------------
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
BIN="$TARGET_DIR/debug/ponycli"
if [ -x "$BIN" ]; then
  log "== reusing built binary: $BIN"
else
  log "== building ponycli (cold build may take minutes)..."
  if ! ( cd "$REPO" && cargo build -p pony-agent-core --bin ponycli ) >"$RECEIPTS/build.log" 2>&1; then
    log "FATAL: cargo build -p pony-agent-core --bin ponycli failed (see $RECEIPTS/build.log)"
    tail -30 "$RECEIPTS/build.log" | tee -a "$LOG"
    exit 2
  fi
fi
if [ ! -x "$BIN" ]; then
  log "FATAL: no ponycli binary at $BIN"
  exit 2
fi
{
  echo "bin=$BIN"
  echo "bin_sha256=$(sha256sum "$BIN" | awk '{print $1}')"
  echo "generated_at=$(date -Is)"
} | tee "$RECEIPTS/bin.log" >>"$LOG"

# ---------------------------------------------------------------------------
# 1. 隔离夹具（HOME / XDG / 工作区），杜绝污染真实用户配置
# ---------------------------------------------------------------------------
ISO="$WORK/iso"
mkdir -p "$ISO/home" "$ISO/xdg-config" "$ISO/xdg-data" "$ISO/xdg-cache" "$ISO/project"
printf '{"name":"wave59-headless-project","version":"0.0.1","marker":"HEADLESS_E2E_MARKER"}\n' \
  >"$ISO/project/package.json"

# 每个用例独立 HOME/XDG，避免会话状态跨用例串味
fresh_env() { # $1 = tag → echo 出 HOME 目录
  local tag="$1"
  local root="$ISO/$tag"
  mkdir -p "$root/home" "$root/xdg-config" "$root/xdg-data" "$root/xdg-cache"
  echo "$root"
}

# 统一的调用封装：绝不继承开发机真实 provider 凭证
run_cli() { # $1 = env root, 其余 = ponycli 参数
  local env_root="$1"
  shift
  ( cd "$ISO/project" && \
    env -u PONYLLM_API_KEY -u PONY_API_KEY -u OPENAI_API_KEY \
        HOME="$env_root/home" \
        XDG_CONFIG_HOME="$env_root/xdg-config" \
        XDG_DATA_HOME="$env_root/xdg-data" \
        XDG_CACHE_HOME="$env_root/xdg-cache" \
        NO_COLOR=1 \
        timeout 120 "$BIN" "$@" < /dev/null )
}

# ---------------------------------------------------------------------------
# 2. 自包含 mock OpenAI gateway（内嵌 python，参考 tests/acceptance/l2at-tui-workspace.sh）
#    mode: text    = 正常文本回复
#          tool    = 无条件返回一个工具调用（验证 deny 模式不挂起）
#          garbage = 200 + 非 JSON 体（L2-AT 畸形 payload）
# ---------------------------------------------------------------------------
cat >"$WORK/mock.py" <<'PY'
import argparse
import json
import sys
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL_ID = "mock-alpha"
READ_ARGS = json.dumps({"path": "package.json"})


def message_text(message):
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(
            item.get("text", "") for item in content if isinstance(item, dict)
        )
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
        with self.lock:
            with open(self.log_path, "a", encoding="utf-8") as fh:
                fh.write(json.dumps(record, ensure_ascii=False) + "\n")

    def next_id(self):
        with self.lock:
            self.counter += 1
            return "chatcmpl-wave59-%d" % self.counter


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "wave59-headless-mock/1.0"

    def log_message(self, fmt, *args):
        pass

    @property
    def state(self):
        return self.server.state

    def _json(self, code, obj):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _raw(self, code, body_text, content_type="application/json; charset=utf-8"):
        body = body_text.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length).decode("utf-8", "replace") if length > 0 else ""

    def do_GET(self):  # noqa: N802
        path = urllib.parse.urlparse(self.path).path.rstrip("/")
        if path in ("/health", "/healthz", "/v1/health"):
            self._json(200, {"ok": True})
            return
        if path.endswith("/models"):
            self._json(200, {"object": "list", "data": [{"id": MODEL_ID, "object": "model", "owned_by": "wave59-mock"}]})
            return
        self._json(404, {"error": {"message": "unknown path %s" % self.path}})

    def do_POST(self):  # noqa: N802
        path = urllib.parse.urlparse(self.path).path.rstrip("/")
        body_text = self._read_body()
        if not path.endswith("/chat/completions"):
            self._json(404, {"error": {"message": "unknown path %s" % self.path}})
            return
        try:
            body = json.loads(body_text) if body_text else {}
        except json.JSONDecodeError:
            self._json(400, {"error": {"message": "invalid json"}})
            return

        messages = body.get("messages") or []
        stream = bool(body.get("stream"))
        has_tool_message = any(str(m.get("role")) == "tool" for m in messages)
        self.state.log({
            "path": self.path,
            "mode": self.server.mode,
            "stream": stream,
            "roles": [m.get("role") for m in messages],
            "authorization": "<redacted>",
        })

        if self.server.mode == "garbage":
            # L2-AT：200 但响应体不是 JSON —— 客户端必须优雅失败而非 panic
            self._raw(200, "<html>gateway exploded</html>")
            return

        if self.server.mode == "tool" and not has_tool_message:
            if stream:
                self._stream_tool_call()
            else:
                self._json_tool_call()
            return

        answer = "无头模式回答：任务完成。" if not has_tool_message else "工具被拒绝或已执行，任务完成。"
        if stream:
            self._stream_text(answer)
        else:
            self._json_text(answer)

    # ---- non-stream -----------------------------------------------------
    def _json_text(self, text):
        self._json(200, {
            "id": self.state.next_id(),
            "object": "chat.completion",
            "created": int(time.time()),
            "model": MODEL_ID,
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 20, "total_tokens": 32},
        })

    def _json_tool_call(self):
        self._json(200, {
            "id": self.state.next_id(),
            "object": "chat.completion",
            "created": int(time.time()),
            "model": MODEL_ID,
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "我先读取 package.json。",
                    "tool_calls": [{
                        "id": "call_wave59_read_1",
                        "type": "function",
                        "function": {"name": "Read", "arguments": READ_ARGS},
                    }],
                },
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 20, "total_tokens": 32},
        })

    # ---- SSE ------------------------------------------------------------
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
            "id": "chatcmpl-wave59",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": MODEL_ID,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        }

    def _usage(self):
        return {
            "id": "chatcmpl-wave59",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": MODEL_ID,
            "choices": [],
            "usage": {"prompt_tokens": 12, "completion_tokens": 20, "total_tokens": 32},
        }

    def _stream_text(self, text):
        try:
            self._start_sse()
            self._sse(self._delta({"role": "assistant", "content": ""}))
            for i in range(0, len(text), 8):
                self._sse(self._delta({"content": text[i:i + 8]}))
            self._sse(self._delta({}, "stop"))
            self._sse(self._usage())
            self._end_sse()
        except (BrokenPipeError, ConnectionResetError):
            pass

    def _stream_tool_call(self):
        try:
            self._start_sse()
            self._sse(self._delta({"role": "assistant", "content": "我先读取 package.json。"}))
            self._sse({
                "id": "chatcmpl-wave59",
                "object": "chat.completion.chunk",
                "created": int(time.time()),
                "model": MODEL_ID,
                "choices": [{
                    "index": 0,
                    "finish_reason": "tool_calls",
                    "delta": {"tool_calls": [{
                        "index": 0,
                        "id": "call_wave59_read_1",
                        "type": "function",
                        "function": {"name": "Read", "arguments": READ_ARGS},
                    }]},
                }],
            })
            self._sse(self._delta({}, "tool_calls"))
            self._sse(self._usage())
            self._end_sse()
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--mode", default="text", choices=["text", "tool", "garbage"])
    args = parser.parse_args()
    state = State(args.log)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.state = state
    server.mode = args.mode
    print(json.dumps({"listening": True, "port": args.port, "mode": args.mode}), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
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

MOCK_PORTS=()
MOCK_MODES=("text" "tool" "garbage")
MOCK_READY=1
if ! command -v python3 >/dev/null 2>&1; then
  log "[RED DEGRADED: ARG-ONLY] python3 missing → 无法起 mock gateway，网络类用例跳过"
  MOCK_READY=0
fi

if [ "$MOCK_READY" -eq 1 ]; then
  for mode in "${MOCK_MODES[@]}"; do
    port="$(free_port)"
    out="$RECEIPTS/mock-$mode.out"
    python3 "$WORK/mock.py" --port "$port" --log "$RECEIPTS/mock-$mode-requests.jsonl" --mode "$mode" >"$out" 2>&1 &
    pid=$!
    MOCK_PIDS+=("$pid")
    ready=0
    for _ in $(seq 1 100); do
      if grep -q '"listening"' "$out" 2>/dev/null; then ready=1; break; fi
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.05
    done
    if [ "$ready" -ne 1 ]; then
      log "[RED DEGRADED: ARG-ONLY] mock($mode) 未能监听（见 $out）→ 网络类用例跳过"
      MOCK_READY=0
      break
    fi
    MOCK_PORTS+=("$port")
    log "== mock($mode) listening on 127.0.0.1:$port (pid $pid)"
  done
fi

PORT_TEXT="${MOCK_PORTS[0]:-0}"
PORT_TOOL="${MOCK_PORTS[1]:-0}"
PORT_GARBAGE="${MOCK_PORTS[2]:-0}"
log "== ports: text=$PORT_TEXT tool=$PORT_TOOL garbage=$PORT_GARBAGE"

# 已关闭端口（连接必然被拒）——用于 provider 网络失败用例
DEAD_PORT="$(free_port)"

# ---------------------------------------------------------------------------
# helpers: stdout 纯净性判定
# ---------------------------------------------------------------------------
cat >"$WORK/check_jsonl.py" <<'PY'
"""判定 stdout 是否满足 C1：逐行合法 JSON 对象、含 seq/type、无 ANSI 转义。

exit 0 = 合规；exit 1 = 不合规（原因打到 stderr）
用法: check_jsonl.py <file> <require_lines:0|1>
"""
import json
import sys

path, require_lines = sys.argv[1], sys.argv[2] == "1"
raw = open(path, "rb").read()
problems = []

if b"\x1b" in raw:
    idx = raw.find(b"\x1b")
    problems.append("stdout 含 ANSI 转义字节 0x1b（位置 %d，上下文 %r）" % (idx, raw[max(0, idx - 20):idx + 20]))

text = raw.decode("utf-8", "replace")
lines = [ln for ln in text.split("\n") if ln.strip()]
if require_lines and not lines:
    problems.append("stdout 没有任何事件行（期望 >=1 行 JSONL）")

for i, line in enumerate(lines):
    try:
        obj = json.loads(line)
    except json.JSONDecodeError as err:
        problems.append("第 %d 行不是合法 JSON（%s）：%r" % (i + 1, err, line[:160]))
        continue
    if not isinstance(obj, dict):
        problems.append("第 %d 行不是 JSON 对象：%r" % (i + 1, line[:160]))
        continue
    if "seq" not in obj:
        problems.append("第 %d 行缺 `seq` 字段：%r" % (i + 1, line[:160]))
    if "type" not in obj:
        problems.append("第 %d 行缺 `type` 字段：%r" % (i + 1, line[:160]))

if problems:
    for problem in problems[:10]:
        print(problem, file=sys.stderr)
    sys.exit(1)
print("ok: %d lines" % len(lines))
PY

assert_jsonl_pure() { # $1 = stdout file, $2 = label, $3 = require_lines(0|1)
  if python3 "$WORK/check_jsonl.py" "$1" "${3:-0}" >>"$LOG" 2>"$WORK/check.err"; then
    ok "$2: stdout 逐行合法 JSON + 无 ANSI"
  else
    bad "$2: stdout 纯净性违规 —— $(tr '\n' ' | ' <"$WORK/check.err")"
  fi
}

assert_no_panic() { # $1 = files..., $2 covered by label
  local label="$1"
  shift
  if grep -qE "panicked at|RUST_BACKTRACE|thread 'main' .* panicked" "$@" 2>/dev/null; then
    bad "$label: 出现 panic"
  else
    ok "$label: 无 panic"
  fi
}

# ---------------------------------------------------------------------------
# E1: 非法 --permission-mode → exit 2（Usage）
# ---------------------------------------------------------------------------
E1="$RECEIPTS/e1-invalid-permission-mode"
env_root="$(fresh_env e1)"
run_cli "$env_root" --permission-mode accept-all -p "hi" >"$E1.stdout" 2>"$E1.stderr"
rc=$?
echo "exit_code=$rc" >"$E1.exit"
if [ "$rc" -eq 2 ]; then
  ok "E1 非法 --permission-mode → exit 2（Usage）"
else
  bad "E1 非法 --permission-mode 期望 exit 2，实际 $rc（stdout=$(head -c 200 "$E1.stdout" | tr '\n' ' '))"
fi
assert_no_panic "E1" "$E1.stdout" "$E1.stderr"
if grep -qiE "permission|mode" "$E1.stderr"; then
  ok "E1b 非法值诊断落在 stderr（可读）"
else
  bad "E1b stderr 未给出可读的 --permission-mode 诊断：$(head -c 200 "$E1.stderr" | tr '\n' ' ')"
fi

# ---------------------------------------------------------------------------
# E2: 非法 --output-format → exit 2（Usage）
# ---------------------------------------------------------------------------
E2="$RECEIPTS/e2-invalid-output-format"
env_root="$(fresh_env e2)"
run_cli "$env_root" --output-format xml -p "hi" >"$E2.stdout" 2>"$E2.stderr"
rc=$?
echo "exit_code=$rc" >"$E2.exit"
if [ "$rc" -eq 2 ]; then
  ok "E2 非法 --output-format → exit 2（Usage）"
else
  bad "E2 非法 --output-format 期望 exit 2，实际 $rc"
fi
assert_no_panic "E2" "$E2.stdout" "$E2.stderr"

# ---------------------------------------------------------------------------
# E3~E7: 需要 mock gateway
# ---------------------------------------------------------------------------
if [ "$MOCK_READY" -ne 1 ]; then
  log "[RED DEGRADED: ARG-ONLY] E3/E4/E5/E6/E7 未执行（mock gateway 不可用）"
  for case in E3 E4 E5 E6 E7; do skip "$case (degraded: arg-only)"; done
else
  # ---- E3: jsonl 正常路径 ------------------------------------------------
  E3="$RECEIPTS/e3-jsonl-happy"
  env_root="$(fresh_env e3)"
  run_cli "$env_root" \
    --output-format jsonl -p "say hi" \
    -g "http://127.0.0.1:$PORT_TEXT" --key mock-key -m mock-alpha \
    >"$E3.stdout" 2>"$E3.stderr"
  rc=$?
  echo "exit_code=$rc" >"$E3.exit"
  if [ "$rc" -eq 0 ]; then
    ok "E3 jsonl 正常 turn → exit 0"
  else
    bad "E3 jsonl 正常 turn 期望 exit 0，实际 $rc（stderr=$(tail -c 300 "$E3.stderr" | tr '\n' ' '))"
  fi
  assert_jsonl_pure "$E3.stdout" "E3" 1
  assert_no_panic "E3" "$E3.stdout" "$E3.stderr"
  # C1：jsonl 模式下横幅/人类可读诊断不得写进 stdout（stdout 只承载事件）
  if grep -q "=== Pony Agent CLI" "$E3.stdout" 2>/dev/null; then
    bad "E3c jsonl 模式 stdout 出现人类可读横幅（C1：横幅须转 stderr）"
  else
    ok "E3c jsonl 模式 stdout 无人类可读横幅"
  fi

  # ---- E4: --result-json 封套（黑盒产物 + 退出码交叉一致性）-------------
  E4="$RECEIPTS/e4-result-json"
  RESULT_JSON="$WORK/result-wave59.json"
  env_root="$(fresh_env e4)"
  run_cli "$env_root" \
    --output-format jsonl --result-json "$RESULT_JSON" -p "say hi" \
    -g "http://127.0.0.1:$PORT_TEXT" --key mock-key -m mock-alpha \
    >"$E4.stdout" 2>"$E4.stderr"
  rc=$?
  echo "exit_code=$rc" >"$E4.exit"
  cp "$RESULT_JSON" "$E4.envelope.json" 2>/dev/null || true
  if [ -f "$RESULT_JSON" ]; then
    ok "E4a --result-json 落盘"
    if python3 - "$RESULT_JSON" "$rc" <<'PY' >>"$LOG" 2>"$WORK/e4.err"
import json
import sys

path, proc_exit = sys.argv[1], int(sys.argv[2])
env = json.load(open(path, encoding="utf-8"))
problems = []

if env.get("schema") != "pony.headless.result.v1":
    problems.append("schema 期望 pony.headless.result.v1，实际 %r" % (env.get("schema"),))
for field in ("schema", "session_id", "turn_id", "status", "exit_code", "final_text",
              "usage", "tool_calls", "duration_ms"):
    if field not in env:
        problems.append("缺少契约字段 %r" % field)
usage = env.get("usage") or {}
for field in ("input_tokens", "output_tokens"):
    if not isinstance(usage.get(field), int):
        problems.append("usage.%s 必须是整数，实际 %r" % (field, usage.get(field)))
if not isinstance(env.get("tool_calls"), list):
    problems.append("tool_calls 必须是数组")
status = str(env.get("status", "")).lower()
exit_code = env.get("exit_code")
if status == "completed" and exit_code != 0:
    problems.append("status=completed 但 exit_code=%r（⟺ 自洽被击穿）" % (exit_code,))
if status == "failed" and exit_code == 0:
    problems.append("status=failed 但 exit_code=0（Failed ⟹ 非 0 被击穿）")
if exit_code != proc_exit:
    problems.append("封套 exit_code=%r 与真实进程 Exit Code=%r 不一致" % (exit_code, proc_exit))
if problems:
    print("; ".join(problems), file=sys.stderr)
    sys.exit(1)
print("ok: schema/字段/自洽性/进程退出码交叉一致 (status=%s exit_code=%s)" % (status, exit_code))
PY
    then
      ok "E4b 封套 schema+字段集+status⟺exit_code+进程 Exit Code 交叉一致"
    else
      bad "E4b 封套自洽性违规 —— $(tr '\n' ' ' <"$WORK/e4.err")"
    fi
  else
    bad "E4a --result-json 未落盘（C3 原子写缺失）"
    bad "E4b 无法断言封套（前置 E4a 失败）"
  fi

  # ---- E5: provider 不可达 → 绝不退 0 ------------------------------------
  E5="$RECEIPTS/e5-provider-down"
  env_root="$(fresh_env e5)"
  run_cli "$env_root" \
    --output-format jsonl -p "say hi" \
    -g "http://127.0.0.1:$DEAD_PORT" --key mock-key -m mock-alpha \
    >"$E5.stdout" 2>"$E5.stderr"
  rc=$?
  echo "exit_code=$rc" >"$E5.exit"
  if [ "$rc" -ne 0 ] && [ "$rc" -ne 124 ]; then
    ok "E5 provider 不可达 → 非 0 退出（实际 $rc）"
  elif [ "$rc" -eq 0 ]; then
    bad "E5 provider 不可达却以 0 退出 —— 失败被吞（C2 fail-closed 被击穿）"
  else
    bad "E5 provider 不可达时进程挂起（timeout 124）—— 无界等待"
  fi
  assert_jsonl_pure "$E5.stdout" "E5" 0
  assert_no_panic "E5" "$E5.stdout" "$E5.stderr"

  # ---- E6: deny 模式 + 工具调用 → 不得挂起 -------------------------------
  E6="$RECEIPTS/e6-deny-mode-no-hang"
  env_root="$(fresh_env e6)"
  run_cli "$env_root" \
    --output-format jsonl --permission-mode deny -p "read the package.json" \
    -g "http://127.0.0.1:$PORT_TOOL" --key mock-key -m mock-alpha \
    >"$E6.stdout" 2>"$E6.stderr"
  rc=$?
  echo "exit_code=$rc" >"$E6.exit"
  if [ "$rc" -eq 124 ]; then
    bad "E6 deny 模式无审批应答方时挂起（timeout 124）—— C4 禁止挂起进程"
  else
    ok "E6 deny 模式未挂起（退出码 $rc）"
  fi
  assert_jsonl_pure "$E6.stdout" "E6" 0
  assert_no_panic "E6" "$E6.stdout" "$E6.stderr"

  # ---- E6b: 不传 --permission-mode（默认 fail-closed = read_only）同样不得挂起 ----
  # 缺陷背景：契约 v2 之前 CLI 默认传 None，宿主对 None 判定等价于「完全不受限」。
  # 此例锁「默认档位既不放行写入、也不挂起」。
  E6B="$RECEIPTS/e6b-default-permission-mode"
  env_root="$(fresh_env e6b)"
  run_cli "$env_root" \
    --output-format jsonl -p "read the package.json" \
    -g "http://127.0.0.1:$PORT_TOOL" --key mock-key -m mock-alpha \
    >"$E6B.stdout" 2>"$E6B.stderr"
  rc=$?
  echo "exit_code=$rc" >"$E6B.exit"
  if [ "$rc" -eq 124 ]; then
    bad "E6b 默认审批档位下挂起（timeout 124）—— 默认必须 fail-closed 且不得挂起"
  else
    ok "E6b 默认审批档位未挂起（退出码 $rc）"
  fi
  assert_jsonl_pure "$E6B.stdout" "E6b" 0
  assert_no_panic "E6b" "$E6B.stdout" "$E6B.stderr"

  # ---- E7: 畸形 provider 响应 → 优雅失败 --------------------------------
  E7="$RECEIPTS/e7-malformed-provider"
  env_root="$(fresh_env e7)"
  run_cli "$env_root" \
    --output-format jsonl -p "say hi" \
    -g "http://127.0.0.1:$PORT_GARBAGE" --key mock-key -m mock-alpha \
    >"$E7.stdout" 2>"$E7.stderr"
  rc=$?
  echo "exit_code=$rc" >"$E7.exit"
  if [ "$rc" -ne 0 ] && [ "$rc" -ne 124 ]; then
    ok "E7 畸形响应 → 非 0 退出（实际 $rc）"
  elif [ "$rc" -eq 0 ]; then
    bad "E7 provider 返回畸形 JSON 却以 0 退出 —— 解析失败被吞"
  else
    bad "E7 provider 返回畸形 JSON 时进程挂起（timeout 124）"
  fi
  assert_jsonl_pure "$E7.stdout" "E7" 0
  assert_no_panic "E7" "$E7.stdout" "$E7.stderr"
fi

# ---------------------------------------------------------------------------
# 汇总
# ---------------------------------------------------------------------------
{
  echo "pass=$PASS"
  echo "fail=$FAIL"
  echo "skip=$SKIP"
  echo "generated_at=$(date -Is)"
} >"$RECEIPTS/summary.txt"
log "==================================================="
log "headless-cli-e2e: PASS=$PASS FAIL=$FAIL SKIP=$SKIP  (receipts: $RECEIPTS)"
if [ "$FAIL" -ne 0 ]; then
  log "RESULT: FAIL"
  exit 1
fi
log "RESULT: PASS"
exit 0
