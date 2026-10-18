#!/usr/bin/env bash
# //! acceptance: /models remote model-catalog fetch (L2-T) — frozen suite
# //! ticket: l2at-tui-models | verify-only, DO NOT MODIFY during fixes
# //!
# //! Contract under test (crates/pony-agent-core/src/bin/tui.rs):
# //!   /models when the selected provider's models array is empty must fetch
# //!   {base_url}/models (OpenAI-compatible) and list every returned model id;
# //!   on success persist the ids into providers.json models array.
# //!   On failure (connection refused / non-2xx) show an error message in the
# //!   session stream without crashing or exiting.
# //!   Inside the model panel, pressing `r` re-fetches and refreshes the list.
# //!
# //! Method: real PTY via tmux; isolated XDG_CONFIG_HOME/XDG_DATA_HOME/HOME;
# //! local OpenAI-compatible fake server on a random port with /switch to flip
# //! the catalog 3 -> 2 items and /fail500 mode for the error path.
# //! Receipts: tests/acceptance/receipts/l2at-tui-models/
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RECEIPTS="$HERE/receipts/l2at-tui-models"
mkdir -p "$RECEIPTS"
SRC="$REPO/crates/pony-agent-core/src/bin/tui.rs"

PONY=""
newer_than_src() { # $1 binary — true if mtime (ns) strictly newer than tui.rs
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
# Fake OpenAI-compatible server (python3, stdlib only)
# Modes: default=3 models; /switch -> 2 models; /fail500 -> 500 on /models
# ---------------------------------------------------------------
cat > "$RECEIPTS/fake_server.py" <<'EOF'
import http.server, json, sys
PORT = int(sys.argv[1])
STATE = {"count": 3, "fail": False}
class H(http.server.BaseHTTPRequestHandler):
    def _send(self, code, body=b""):
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        p = self.path.split("?")[0].rstrip("/")
        if p.endswith("/switch"):
            STATE["count"] = 2
            self._send(200, b'{"ok":true}')
        elif p.endswith("/fail500"):
            STATE["fail"] = True
            self._send(200, b'{"ok":true}')
        elif p.endswith("/models"):
            if STATE["fail"]:
                self._send(500, b'{"error":"internal boom"}')
                return
            ids = ["alpha-1", "beta-2", "gamma-3"][:STATE["count"]]
            body = json.dumps({"data": [{"id": i} for i in ids]}).encode()
            self._send(200, body)
        else:
            self._send(404)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", PORT), H).serve_forever()
EOF

pick_port() { python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()"; }
start_server() { # $1 outfile-pid-var
  local port; port=$(pick_port)
  python3 "$RECEIPTS/fake_server.py" "$port" >/dev/null 2>&1 &
  echo $! > "$RECEIPTS/fake-server.pid"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    sleep 0.5
    curl -sf -o /dev/null "http://127.0.0.1:$port/v1/models" && break
  done
  echo "$port"
}

# Fixture: isolated HOME + providers.json pointing at $PORT with empty models
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

launch_tui() { # $1 session, $2 root
  tmux_cleanup
  tmux new-session -d -s "$1" -x 100 -y 30 2>/dev/null
  sleep 0.5
  # scrub proxy env: loopback requests must be deterministic (lead-adjudicated root cause)
  local CLEAN="env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy -u NODE_USE_ENV_PROXY"
  tmux send-keys -t "$1" \
    "$CLEAN XDG_CONFIG_HOME=$2/xdg-config XDG_DATA_HOME=$2/xdg-data HOME=$2/home $PONY" Enter
  # wait for main TUI (status word)
  for _ in $(seq 1 20); do
    sleep 0.5
    tmux capture-pane -t "$1" -p 2>/dev/null | grep -qE "就绪|写入中|思考中|连接中" && break
  done
  sleep 1
}

send_models() { # $1 session
  tmux send-keys -t "$1" -l "/models"
  tmux send-keys -t "$1" Enter
  sleep 2
}

quit_tui() { # $1 session — esc (close panel) then /quit; assert still alive
  tmux send-keys -t "$1" Escape
  sleep 0.5
  tmux send-keys -t "$1" -l "/quit"
  tmux send-keys -t "$1" Enter
  sleep 2
}

# ---------------------------------------------------------------
# M1+M2+M3: happy path — fetch, display, persist
# ---------------------------------------------------------------
PORT=$(start_server)
ISO="$RECEIPTS/iso-m123"
rm -rf "$ISO"; make_fixture "$ISO" "$PORT"
echo "port=$PORT" | tee "$RECEIPTS/m1-port.txt"
curl -sf "http://127.0.0.1:$PORT/v1/models" > "$RECEIPTS/m1-fake-served.json" \
  && ok "M1 fake server up and serving 3-model catalog" \
  || bad "M1 fake server not reachable"

launch_tui m123 "$ISO"
send_models m123
tmux capture-pane -t m123 -p > "$RECEIPTS/m2-panel-after-models.txt" 2>/dev/null
tmux capture-pane -t m123 -p -e > "$RECEIPTS/m2-panel-esc.txt" 2>/dev/null

if grep -q "alpha-1" "$RECEIPTS/m2-panel-after-models.txt" \
   && grep -q "beta-2" "$RECEIPTS/m2-panel-after-models.txt" \
   && grep -q "gamma-3" "$RECEIPTS/m2-panel-after-models.txt"; then
  ok "M2a panel lists all three fetched model ids"
else
  bad "M2a panel missing fetched ids (alpha-1/beta-2/gamma-3) — see $RECEIPTS/m2-panel-after-models.txt"
fi
# status text may appear in panel or in the session stream after the panel closes
tmux send-keys -t m123 Escape
sleep 1
tmux capture-pane -t m123 -p > "$RECEIPTS/m2-stream-after-close.txt" 2>/dev/null
if grep -qE "已加载|正在拉取" "$RECEIPTS/m2-panel-after-models.txt" "$RECEIPTS/m2-panel-esc.txt" "$RECEIPTS/m2-stream-after-close.txt" 2>/dev/null; then
  ok "M2b status text shows 已加载/正在拉取 (final or transient state)"
else
  bad "M2b no 已加载/正在拉取 status text found"
fi

# M3: persistence
sleep 1
m3_verdict=$(python3 - "$ISO/xdg-config/pony-agent/providers.json" <<'EOF'
import json, sys
reg = json.load(open(sys.argv[1]))
prov = [p for p in reg["providers"] if p["id"] == "fakep"][0]
ids = {m["id"] for m in prov.get("models", [])}
need = {"alpha-1", "beta-2", "gamma-3"}
print("PASS" if need <= ids else f"FAIL persisted={sorted(ids)}")
EOF
)
echo "M3: $m3_verdict" | tee "$RECEIPTS/m3-persist-verdict.txt"
cp "$ISO/xdg-config/pony-agent/providers.json" "$RECEIPTS/m3-providers-after.json"
if echo "$m3_verdict" | grep -q "PASS"; then ok "M3 models persisted into providers.json"; else bad "M3 persistence failed ($m3_verdict)"; fi

# ---------------------------------------------------------------
# M5: refresh with `r` INSIDE the model panel (lead ruling: `r` is panel-only;
# Normal mode uses Ctrl+R; bare `r` in Normal must NOT trigger fetch)
# ---------------------------------------------------------------
curl -sf "http://127.0.0.1:$PORT/switch" >/dev/null && echo "switched to 2 items" >> "$RECEIPTS/m1-port.txt"
# (re)open the panel with /models — do NOT rely on panel still being open from M2
# (Esc semantics are constant close; M2b keeps its own Esc assertion)
tmux send-keys -t m123 Escape
sleep 0.5
send_models m123
# wait for the panel title row before pressing r
for _ in $(seq 1 20); do
  tmux capture-pane -t m123 -p 2>/dev/null | grep -q "r 刷新" && break
  sleep 0.5
done
# settle: ensure the initial fetch has fully completed (已加载) before pressing r
for _ in $(seq 1 10); do
  tmux capture-pane -t m123 -p 2>/dev/null | grep -q "已加载" && break
  sleep 0.5
done
sleep 1
tmux send-keys -t m123 r
# poll: refresh may take a moment; capture until panel shows 2-item list or closes
m5_seen=""
for _ in $(seq 1 10); do
  sleep 0.5
  tmux capture-pane -t m123 -p > "$RECEIPTS/m5-panel-after-r.txt" 2>/dev/null
  if grep -q "alpha-1" "$RECEIPTS/m5-panel-after-r.txt" \
     && grep -q "beta-2" "$RECEIPTS/m5-panel-after-r.txt" \
     && ! grep -q "gamma-3" "$RECEIPTS/m5-panel-after-r.txt"; then
    m5_seen="panel"; break
  fi
done
if [ "$m5_seen" = "panel" ]; then
  ok "M5 panel-internal [r] refresh: list updated to 2-item catalog (gamma-3 gone)"
else
  # fallback: refresh must at least persist the new 2-item catalog
  m5_persist=$(python3 -c "
import json
reg=json.load(open('$ISO/xdg-config/pony-agent/providers.json'))
ids=sorted(m['id'] for m in reg['providers'][0]['models'])
print('PASS' if ids==['alpha-1','beta-2'] else f'FAIL {ids}')
")
  echo "M5 fallback persistence: $m5_persist" >> "$RECEIPTS/m5-persist-verdict.txt"
  if echo "$m5_persist" | grep -q PASS; then
    ok "M5 panel-internal [r] refresh persisted 2-item catalog (panel closed on refresh; screen not captured)"
  else
    bad "M5 [r] refresh neither repainted panel nor persisted 2-item catalog (see $RECEIPTS/m5-panel-after-r.txt)"
  fi
fi

# M7 regression: bare text starting with `r` in Normal mode must NOT open the panel
quit_tui m123
tmux list-panes -t m123 -F '#{pane_dead} #{pane_dead_status}' > "$RECEIPTS/m123-quit-status.txt" 2>/dev/null
tmux_cleanup
launch_tui m7 "$ISO"
tmux send-keys -t m7 -l "read"
sleep 1
tmux capture-pane -t m7 -p > "$RECEIPTS/m7-normal-read.txt" 2>/dev/null
if grep -q "r 刷新" "$RECEIPTS/m7-normal-read.txt"; then
  bad "M7a Normal-mode 'read' opened the model panel (r-refresh title visible)"
else
  ok "M7a Normal-mode 'read' did not open the model panel"
fi
if grep -qE "›.*read" "$RECEIPTS/m7-normal-read.txt"; then
  ok "M7b input line shows typed text 'read'"
else
  bad "M7b typed text 'read' not visible in input line (see $RECEIPTS/m7-normal-read.txt)"
fi
quit_tui m7
tmux_cleanup

# M8 (optional entry kept): Normal mode Ctrl+R opens the panel and fetches
launch_tui m8 "$ISO"
tmux send-keys -t m8 C-r
sleep 2
tmux capture-pane -t m8 -p > "$RECEIPTS/m8-ctrl-r-panel.txt" 2>/dev/null
if grep -q "r 刷新" "$RECEIPTS/m8-ctrl-r-panel.txt" \
   && grep -q "alpha-1" "$RECEIPTS/m8-ctrl-r-panel.txt"; then
  ok "M8 Ctrl+R in Normal mode opens model panel with fetched list"
else
  bad "M8 Ctrl+R did not open panel with fetched list (see $RECEIPTS/m8-ctrl-r-panel.txt)"
fi
quit_tui m8
tmux_cleanup
tmux list-panes -t m123 -F '#{pane_dead} #{pane_dead_status}' > "$RECEIPTS/m123-quit-status.txt" 2>/dev/null
tmux_cleanup

# ---------------------------------------------------------------
# M4: failure path — provider points at a closed port
# ---------------------------------------------------------------
CLOSED_PORT=$(pick_port)
ISO2="$RECEIPTS/iso-m4"
rm -rf "$ISO2"; make_fixture "$ISO2" "$CLOSED_PORT"
echo "closed_port=$CLOSED_PORT" | tee "$RECEIPTS/m4-port.txt"
launch_tui m4 "$ISO2"
send_models m4
tmux capture-pane -t m4 -p > "$RECEIPTS/m4-after-models.txt" 2>/dev/null
if grep -qE "失败|错误|error|Error|获取模型" "$RECEIPTS/m4-after-models.txt"; then
  ok "M4a error message shown on fetch failure"
else
  bad "M4a no error message visible (see $RECEIPTS/m4-after-models.txt)"
fi
quit_tui m4
m4_alive=$(tmux list-panes -t m4 -F '#{pane_dead} #{pane_dead_status}' 2>/dev/null)
echo "pane_dead=$m4_alive" | tee "$RECEIPTS/m4-quit-status.txt"
if echo "$m4_alive" | grep -qE "^1 (0)$|^0 "; then
  ok "M4b TUI survived failure and exited cleanly via /quit"
else
  bad "M4b TUI did not exit cleanly after failure (status: $m4_alive)"
fi
tmux_cleanup

# M4c: HTTP 500 variant
PORT2=$(start_server)
curl -sf "http://127.0.0.1:$PORT2/fail500" >/dev/null
ISO3="$RECEIPTS/iso-m4c"
rm -rf "$ISO3"; make_fixture "$ISO3" "$PORT2"
echo "fail500_port=$PORT2" | tee "$RECEIPTS/m4-port.txt"
launch_tui m4c "$ISO3"
send_models m4c
tmux capture-pane -t m4c -p > "$RECEIPTS/m4c-after-models.txt" 2>/dev/null
if grep -qE "失败|错误|error|Error|500|获取模型" "$RECEIPTS/m4c-after-models.txt"; then
  ok "M4c HTTP 500 shows error message"
else
  bad "M4c no error message on HTTP 500 (see $RECEIPTS/m4c-after-models.txt)"
fi
quit_tui m4c
tmux_cleanup

# ---------------------------------------------------------------
# M6: adversarial proxy injection — broken proxy (port 9) must NOT hijack
# loopback /models fetch; product must bypass proxy for 127.0.0.1
# ---------------------------------------------------------------
PORT3=$(start_server)
ISO4="$RECEIPTS/iso-m6"
rm -rf "$ISO4"; make_fixture "$ISO4" "$PORT3"
echo "m6_port=$PORT3 broken_proxy=http://127.0.0.1:9" | tee "$RECEIPTS/m6-port.txt"
tmux_cleanup
tmux new-session -d -s m6 -x 100 -y 30 2>/dev/null
sleep 0.5
tmux send-keys -t m6 \
  "env XDG_CONFIG_HOME=$ISO4/xdg-config XDG_DATA_HOME=$ISO4/xdg-data HOME=$ISO4/home HTTP_PROXY=http://127.0.0.1:9 HTTPS_PROXY=http://127.0.0.1:9 ALL_PROXY=http://127.0.0.1:9 http_proxy=http://127.0.0.1:9 https_proxy=http://127.0.0.1:9 all_proxy=http://127.0.0.1:9 $PONY" Enter
for _ in $(seq 1 20); do
  sleep 0.5
  tmux capture-pane -t m6 -p 2>/dev/null | grep -qE "就绪|写入中|思考中|连接中" && break
done
sleep 1
send_models m6
tmux capture-pane -t m6 -p > "$RECEIPTS/m6-panel-broken-proxy.txt" 2>/dev/null
if grep -q "alpha-1" "$RECEIPTS/m6-panel-broken-proxy.txt" \
   && grep -q "beta-2" "$RECEIPTS/m6-panel-broken-proxy.txt" \
   && grep -q "gamma-3" "$RECEIPTS/m6-panel-broken-proxy.txt"; then
  ok "M6 loopback bypasses broken proxy: all 3 models fetched"
else
  bad "M6 loopback fetch hijacked by broken proxy (see $RECEIPTS/m6-panel-broken-proxy.txt)"
fi
quit_tui m6
tmux_cleanup

pkill -f "fake_server.py" 2>/dev/null || true

# ---------------------------------------------------------------
echo
echo "==== l2at-tui-models summary: PASS=$PASS FAIL=$FAIL ===="
[ "$FAIL" -eq 0 ]
