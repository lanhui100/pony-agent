#!/usr/bin/env bash
# //! acceptance: TUI auth/model passthrough + mouse-toggle regressions (L2-T) — frozen suite
# //! ticket: l2at-tui-models-auth | verify-only, DO NOT MODIFY during fixes
# //!
# //! Contracts under test:
# //!   A) Strict auth & model passthrough (reproduces "selected model then
# //!      unauthorized" user report): fake server returns 200 ONLY when
# //!      Authorization: Bearer test-dummy-key AND chat body model == discovered
# //!      id (alpha-1); everything else -> 401 {"error":"unauthorized"}.
# //!      Flow: isolated HOME (models empty) -> /models fetch -> select alpha-1
# //!      (Enter) -> Esc closes panel -> type "hi" + Enter.
# //!      Assert: no unauthorized/error on screen; server receipt shows
# //!      model=alpha-1 + Authorization Bearer test-dummy-key.
# //!   B) Mouse-capture toggle:
# //!      default startup emits NO mouse-enable sequences (?1000h/?1002h/?1003h);
# //!      F2 toggles enable sequences on; F2 again toggles off (?1000l etc.).
# //!
# //! Receipts: tests/acceptance/receipts/l2at-tui-models-auth/
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RECEIPTS="$HERE/receipts/l2at-tui-models-auth"
mkdir -p "$RECEIPTS"
SRC="$REPO/crates/pony-agent-core/src/bin/tui.rs"

PONY=""
newer_than_src() {
  [ -x "$1" ] || return 1
  python3 -c "import os,sys;sys.exit(0 if os.stat('$1').st_mtime_ns>os.stat('$SRC').st_mtime_ns else 1)"
}
if newer_than_src "$REPO/target/debug/pony"; then
  PONY="$REPO/target/debug/pony"
elif newer_than_src "$REPO/target/release/pony"; then
  PONY="$REPO/target/release/pony"
elif command -v pony >/dev/null 2>&1; then
  PONY="$(command -v pony)"
elif [ -x "$REPO/target/debug/pony" ]; then
  PONY="$REPO/target/debug/pony"
elif [ -x "$REPO/target/release/pony" ]; then
  PONY="$REPO/target/release/pony"
fi
if [ -z "$PONY" ]; then
  echo "FATAL: no pony binary found" | tee "$RECEIPTS/fatal.log"
  exit 2
fi
echo "BIN under test: $PONY" | tee "$RECEIPTS/bin.log"

PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "PASS: $1"; }
bad() { FAIL=$((FAIL+1)); echo "FAIL: $1"; }
tmux_cleanup() { tmux kill-server >/dev/null 2>&1 || true; }
trap tmux_cleanup EXIT

# ---------------------------------------------------------------
# Strict-mode fake server: logs every request (headers+body) to a receipt,
# /v1/models serves 3 ids; chat completions 200 only when auth+model match.
# ---------------------------------------------------------------
make_server() { # $1 port, $2 logfile
cat > "$RECEIPTS/strict_server.py" <<EOF
import http.server, json, sys
PORT = int(sys.argv[1]); LOG = "$2"
EXPECT_KEY = "test-dummy-key"
EXPECT_MODEL = "alpha-1"
def log(obj):
    with open(LOG, "a") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")
class H(http.server.BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def _record(self):
        length = int(self.headers.get("Content-Length", 0) or 0)
        body = self.rfile.read(length).decode("utf-8", "replace") if length else ""
        rec = {"path": self.path, "authorization": self.headers.get("Authorization", ""),
               "body": body}
        try:
            rec["model"] = json.loads(body).get("model") if body else None
        except Exception:
            rec["model"] = None
        log(rec)
        return rec
    def do_POST(self):
        rec = self._record()
        p = self.path.split("?")[0].rstrip("/")
        if p.endswith("/chat/completions") or p.endswith("/messages"):
            auth_ok = rec["authorization"] == f"Bearer {EXPECT_KEY}"
            model_ok = rec["model"] == EXPECT_MODEL
            if auth_ok and model_ok:
                # minimal OpenAI-style JSON response (content not asserted)
                self._send(200, {"id": "chatcmpl-1", "object": "chat.completion",
                                 "model": rec["model"], "choices": [
                                     {"index": 0, "finish_reason": "stop",
                                      "message": {"role": "assistant", "content": "ok"}}]})
            else:
                self._send(401, {"error": "unauthorized"})
        else:
            self._send(404)
    def do_GET(self):
        p = self.path.split("?")[0].rstrip("/")
        if p.endswith("/models"):
            log({"path": self.path, "authorization": self.headers.get("Authorization", ""), "body": ""})
            body = json.dumps({"data": [{"id": "alpha-1"}, {"id": "beta-2"}, {"id": "gamma-3"}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self._send(404)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", PORT), H).serve_forever()
EOF
}

pick_port() { python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()"; }
start_server() { # $1 port, $2 logfile
  python3 "$RECEIPTS/strict_server.py" "$1" "$2" >/dev/null 2>&1 &
  echo $! > "$RECEIPTS/strict-server.pid"
  for _ in $(seq 1 10); do
    sleep 0.5
    curl -sf -o /dev/null "http://127.0.0.1:$1/v1/models" && return 0
  done
  return 1
}

make_fixture() { # $1 root, $2 port
  mkdir -p "$1/xdg-config/pony-agent" "$1/xdg-data" "$1/home"
  python3 - "$1" "$2" <<'EOF'
import json, sys
root, port = sys.argv[1], int(sys.argv[2])
prov = {
    "id": "fakep", "name": "FakeProvider", "protocol": "openai-completions",
    "base_url": f"http://127.0.0.1:{port}/v1", "auth_type": "bearer",
    "api_key_env_var": "FAKE_API_KEY", "secret_ref": "",
    "api_key_value": "test-dummy-key", "supported_protocols": ["openai-completions"],
    "endpoints": [], "models": [], "selected_model_id": None,
}
reg = {"providers": [prov], "selected_provider_id": "fakep"}
open(f"{root}/xdg-config/pony-agent/providers.json", "w").write(
    json.dumps(reg, ensure_ascii=False, indent=2))
EOF
}

launch_tui() { # $1 session, $2 root, $3 extra-env-string (optional)
  tmux_cleanup
  tmux new-session -d -s "$1" -x 100 -y 30 2>/dev/null
  sleep 0.5
  local EXTRA="${3:-}"
  tmux send-keys -t "$1" \
    "env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy -u NODE_USE_ENV_PROXY XDG_CONFIG_HOME=$2/xdg-config XDG_DATA_HOME=$2/xdg-data HOME=$2/home $EXTRA $PONY" Enter
  for _ in $(seq 1 20); do
    sleep 0.5
    tmux capture-pane -t "$1" -p 2>/dev/null | grep -qE "就绪|写入中|思考中|连接中" && break
  done
  sleep 1
}

# ---------------------------------------------------------------
# A) auth/model passthrough
# ---------------------------------------------------------------
PORT=$(pick_port)
SRVLOG="$RECEIPTS/a-server-requests.log"
: > "$SRVLOG"
make_server "$PORT" "$SRVLOG"
start_server "$PORT" "$SRVLOG" && ok "A0 strict fake server up (401 unless auth+model match)" \
  || bad "A0 strict fake server failed to start"
ISO="$RECEIPTS/iso-auth"
rm -rf "$ISO"; make_fixture "$ISO" "$PORT"

launch_tui auth "$ISO"
# open panel, wait for it, select alpha-1 (first row, already highlighted) via Enter
tmux send-keys -t auth -l "/models"
tmux send-keys -t auth Enter
for _ in $(seq 1 20); do
  tmux capture-pane -t auth -p 2>/dev/null | grep -q "r 刷新" && break
  sleep 0.5
done
for _ in $(seq 1 10); do
  tmux capture-pane -t auth -p 2>/dev/null | grep -q "已加载" && break
  sleep 0.5
done
sleep 1
tmux send-keys -t auth Enter        # select alpha-1 (first item)
sleep 1
tmux send-keys -t auth Escape       # close panel (constant close semantics)
sleep 1
tmux send-keys -t auth -l "hi"
tmux send-keys -t auth Enter
sleep 4
tmux capture-pane -t auth -p > "$RECEIPTS/a-screen-after-send.txt" 2>/dev/null
tmux capture-pane -t auth -p -e > "$RECEIPTS/a-screen-after-send-esc.txt" 2>/dev/null
quit_tui() {
  tmux send-keys -t "$1" Escape
  sleep 0.5
  tmux send-keys -t "$1" -l "/quit"
  tmux send-keys -t "$1" Enter
  sleep 2
}
quit_tui auth
tmux_cleanup

# A1: no unauthorized / auth error visible on screen
if grep -qE "unauthorized|401|鉴权失败|认证失败" "$RECEIPTS/a-screen-after-send.txt" "$RECEIPTS/a-screen-after-send-esc.txt" 2>/dev/null; then
  bad "A1 unauthorized/error visible on screen (see $RECEIPTS/a-screen-after-send.txt)"
else
  ok "A1 no unauthorized/error on screen"
fi

# A2: server saw chat request with model=alpha-1 and correct Bearer key
a2_verdict=$(python3 - "$SRVLOG" <<'EOF'
import json, sys
chat = []
for line in open(sys.argv[1]):
    line = line.strip()
    if not line:
        continue
    rec = json.loads(line)
    if rec["path"].rstrip("/").endswith(("/chat/completions", "/messages")):
        chat.append(rec)
if not chat:
    print("FAIL no chat request reached server")
    raise SystemExit
ok_rec = [r for r in chat if r["model"] == "alpha-1" and r["authorization"] == "Bearer test-dummy-key"]
if ok_rec:
    print("PASS")
else:
    print(f"FAIL chat_requests={json.dumps(chat, ensure_ascii=False)[:600]}")
EOF
)
echo "A2: $a2_verdict" | tee "$RECEIPTS/a2-server-verdict.txt"
if echo "$a2_verdict" | grep -q "PASS"; then
  ok "A2 server receipt: chat body model=alpha-1 + Authorization Bearer test-dummy-key"
else
  # diagnostic: distinguish header-missing vs model-passthrough error
  diag=$(python3 - "$SRVLOG" <<'EOF'
import json, sys
chat = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
chat = [r for r in chat if r["path"].rstrip("/").endswith(("/chat/completions", "/messages"))]
if not chat:
    print("no chat request reached server")
else:
    r = chat[-1]
    print(f"last chat request: authorization={r['authorization']!r} model={r['model']!r}")
EOF
)
  echo "A2 diagnosis: $diag" | tee "$RECEIPTS/a2-diagnosis.txt"
  bad "A2 server receipt mismatch ($diag)"
fi
cp "$SRVLOG" "$RECEIPTS/a-server-requests-copy.log" 2>/dev/null || true

# ---------------------------------------------------------------
# A3) ponyllm hardcoded branch: registry entry with id "ponyllm" must take
# precedence over the gateway fallback (no PONYLLM_API_KEY env set)
# ---------------------------------------------------------------
PORTA3=$(pick_port)
SRVLOG3="$RECEIPTS/a3-server-requests.log"
: > "$SRVLOG3"
make_server "$PORTA3" "$SRVLOG3"
start_server "$PORTA3" "$SRVLOG3" || bad "A3 strict fake server failed to start"
ISO3="$RECEIPTS/iso-ponyllm"
rm -rf "$ISO3"
mkdir -p "$ISO3/xdg-config/pony-agent" "$ISO3/xdg-data" "$ISO3/home"
python3 - "$ISO3" "$PORTA3" <<'EOF3'
import json, sys
root, port = sys.argv[1], int(sys.argv[2])
prov = {
    "id": "ponyllm", "name": "PonyLlm Registry", "protocol": "openai-completions",
    "base_url": f"http://127.0.0.1:{port}/v1", "auth_type": "bearer",
    "api_key_env_var": "FAKE_API_KEY", "secret_ref": "",
    "api_key_value": "test-dummy-key", "supported_protocols": ["openai-completions"],
    "endpoints": [], "models": [], "selected_model_id": None,
}
reg = {"providers": [prov], "selected_provider_id": "ponyllm"}
open(f"{root}/xdg-config/pony-agent/providers.json", "w").write(
    json.dumps(reg, ensure_ascii=False, indent=2))
EOF3
# launch with PONYLLM_API_KEY explicitly UNSET (env -u)
tmux_cleanup
tmux new-session -d -s a3 -x 100 -y 30 2>/dev/null
sleep 0.5
tmux send-keys -t a3 \
  "env -u PONYLLM_API_KEY -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy -u NODE_USE_ENV_PROXY XDG_CONFIG_HOME=$ISO3/xdg-config XDG_DATA_HOME=$ISO3/xdg-data HOME=$ISO3/home $PONY" Enter
for _ in $(seq 1 20); do
  sleep 0.5
  tmux capture-pane -t a3 -p 2>/dev/null | grep -qE "就绪|写入中|思考中|连接中" && break
done
sleep 1
tmux send-keys -t a3 -l "/models"
tmux send-keys -t a3 Enter
for _ in $(seq 1 20); do
  tmux capture-pane -t a3 -p 2>/dev/null | grep -q "r 刷新" && break
  sleep 0.5
done
for _ in $(seq 1 10); do
  tmux capture-pane -t a3 -p 2>/dev/null | grep -q "已加载" && break
  sleep 0.5
done
sleep 1
tmux send-keys -t a3 Enter
sleep 1
tmux send-keys -t a3 Escape
sleep 1
tmux send-keys -t a3 -l "hi"
tmux send-keys -t a3 Enter
sleep 4
tmux capture-pane -t a3 -p > "$RECEIPTS/a3-screen-after-send.txt" 2>/dev/null
quit_tui a3
tmux_cleanup

if grep -qE "unauthorized|401|鉴权失败|认证失败" "$RECEIPTS/a3-screen-after-send.txt" 2>/dev/null; then
  bad "A3a unauthorized/error visible on screen (ponyllm fallback hijacked)"
else
  ok "A3a no unauthorized/error on screen (ponyllm registry-first)"
fi
a3_verdict=$(python3 - "$SRVLOG3" <<'EOF4'
import json, sys
chat = []
for line in open(sys.argv[1]):
    line = line.strip()
    if not line:
        continue
    rec = json.loads(line)
    if rec["path"].rstrip("/").endswith(("/chat/completions", "/messages")):
        chat.append(rec)
if not chat:
    print("FAIL no chat request reached server")
else:
    ok_rec = [r for r in chat if r["model"] == "alpha-1" and r["authorization"] == "Bearer test-dummy-key"]
    print("PASS" if ok_rec else f"FAIL {json.dumps(chat, ensure_ascii=False)[:500]}")
EOF4
)
echo "A3: $a3_verdict" | tee "$RECEIPTS/a3-server-verdict.txt"
cp "$SRVLOG3" "$RECEIPTS/a3-server-requests-copy.log" 2>/dev/null || true
if echo "$a3_verdict" | grep -q "PASS"; then
  ok "A3b server receipt: model=alpha-1 + Bearer test-dummy-key (registry beats gateway fallback)"
else
  bad "A3b server receipt mismatch ($a3_verdict)"
fi

# ---------------------------------------------------------------
# B) mouse-capture toggle
# ---------------------------------------------------------------
ISO2="$RECEIPTS/iso-mouse"
rm -rf "$ISO2"; make_fixture "$ISO2" "$PORT"
tmux_cleanup
tmux new-session -d -s mouse -x 100 -y 30 2>/dev/null
sleep 0.5
tmux pipe-pane -t mouse "cat > '$RECEIPTS/b-raw-full.bin'"
tmux send-keys -t mouse \
  "env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy -u NODE_USE_ENV_PROXY XDG_CONFIG_HOME=$ISO2/xdg-config XDG_DATA_HOME=$ISO2/xdg-data HOME=$ISO2/home $PONY" Enter
for _ in $(seq 1 20); do
  sleep 0.5
  tmux capture-pane -t mouse -p 2>/dev/null | grep -qE "就绪|写入中|思考中|连接中" && break
done
sleep 1
tmux pipe-pane -t mouse "" 2>/dev/null
tmux capture-pane -t mouse -p > "$RECEIPTS/b-default-screen.txt" 2>/dev/null

# B1: raw startup stream must NOT contain mouse-enable sequences
# (strip the initial shell echo: only inspect bytes emitted after the TUI's
#  alternate-screen enter; simpler: check the whole file — the interactive shell
#  does not emit mouse sequences anyway)
if LC_ALL=C grep -aq $'\x1b\[?1000h' "$RECEIPTS/b-raw-full.bin" 2>/dev/null \
   || LC_ALL=C grep -aq $'\x1b\[?1002h' "$RECEIPTS/b-raw-full.bin" 2>/dev/null \
   || LC_ALL=C grep -aq $'\x1b\[?1003h' "$RECEIPTS/b-raw-full.bin" 2>/dev/null; then
  bad "B1 mouse-enable sequence (?1000h/?1002h/?1003h) present at default startup"
else
  ok "B1 no mouse-enable sequences at default startup"
fi

# B2: F2 enables mouse capture; F2 again disables
tmux pipe-pane -t mouse "cat > '$RECEIPTS/b-raw-after-f2on.bin'"
tmux send-keys -t mouse F2
sleep 1.5
tmux pipe-pane -t mouse "" 2>/dev/null
if LC_ALL=C grep -aq $'\x1b\[?1000h' "$RECEIPTS/b-raw-after-f2on.bin" 2>/dev/null \
   || LC_ALL=C grep -aq $'\x1b\[?1002h' "$RECEIPTS/b-raw-after-f2on.bin" 2>/dev/null; then
  ok "B2a F2 emits mouse-enable sequence"
else
  bad "B2a F2 did not emit mouse-enable sequence (see $RECEIPTS/b-raw-after-f2on.bin)"
fi

tmux pipe-pane -t mouse "cat > '$RECEIPTS/b-raw-after-f2off.bin'"
tmux send-keys -t mouse F2
sleep 1.5
tmux pipe-pane -t mouse "" 2>/dev/null
if LC_ALL=C grep -aq $'\x1b\[?1000l' "$RECEIPTS/b-raw-after-f2off.bin" 2>/dev/null \
   || LC_ALL=C grep -aq $'\x1b\[?1002l' "$RECEIPTS/b-raw-after-f2off.bin" 2>/dev/null \
   || LC_ALL=C grep -aq $'\x1b\[?1003l' "$RECEIPTS/b-raw-after-f2off.bin" 2>/dev/null; then
  ok "B2b second F2 emits mouse-disable sequence"
else
  bad "B2b second F2 did not emit mouse-disable sequence (see $RECEIPTS/b-raw-after-f2off.bin)"
fi
tmux send-keys -t mouse Escape
sleep 0.5
tmux send-keys -t mouse -l "/quit"
tmux send-keys -t mouse Enter
sleep 2
tmux_cleanup

# ---------------------------------------------------------------
echo
echo "==== l2at-tui-models-auth summary: PASS=$PASS FAIL=$FAIL ===="
[ "$FAIL" -eq 0 ]
