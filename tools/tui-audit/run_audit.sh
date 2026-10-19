#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# TUI audit runner (baseline vs after)
#
# Drives the real pony TUI inside an isolated tmux server (unique socket), talks
# to the mock OpenAI server in tools/tui-audit/mock_llm.py, captures every screen
# as text + ANSI + PNG, runs strict text assertions and writes:
#
#   .dev-team/evidence/tui-polish/<phase>/<scenario>/{*.txt,*.ansi,*.png,
#                                                      meta.json,extra.json}
#   .dev-team/evidence/tui-polish/<phase>/report.json
#   .dev-team/evidence/tui-polish/<phase>/report.md
#
# Usage:
#   tools/tui-audit/run_audit.sh --phase baseline --bin tools/tui-audit/bin/pony-baseline
#   tools/tui-audit/run_audit.sh --phase after    --bin target/debug/pony
#
# Options:
#   --phase baseline|after   evidence directory + report label (required)
#   --bin <path>             binary under test (required)
#   --scale <n>              PNG scale passed to render_ansi.py (default 2)
#   --only a,b,c             run only these scenarios (debugging)
#
# Exit status: 0 = every assertion passed, 1 = at least one FAIL, 2 = harness
# error.  Baseline is expected to exit 1: its failures are the contrast evidence.
#
# Isolation: every scenario gets its own HOME + XDG dirs and its own mock LLM on
# a free port.  The tmux server is `tmux -L tui-audit-<pid>` and only that server
# is ever killed.
# ---------------------------------------------------------------------------
set -uo pipefail

PHASE=""
BIN=""
SCALE=2
ONLY=""

# wave-39 R6/R7：底部状态短语（就绪/完成/失败/写入中）已删除，footer 恒为指标行。
# 「本轮落定」的当前真实信号 = 指标行出现非零 token（↑N），由 turn:output_end 的
# usage 回执经 apply_turn_usage 写入；纯「上下文」二字在任何时刻都存在，不能代表落定。
TURN_SETTLED_RE='↑[1-9]'

die() { echo "FATAL: $*" >&2; exit 2; }
warn() { echo "WARN: $*" >&2; }

usage() {
  sed -n '2,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --phase) PHASE="${2:-}"; shift 2 ;;
    --bin) BIN="${2:-}"; shift 2 ;;
    --scale) SCALE="${2:-2}"; shift 2 ;;
    --only) ONLY="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

case "$PHASE" in
  baseline|after) ;;
  *) die "--phase must be baseline or after" ;;
esac
[ -n "$BIN" ] || die "--bin is required"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLS="$ROOT/tools/tui-audit"
case "$BIN" in /*) ;; *) BIN="$ROOT/$BIN" ;; esac
[ -x "$BIN" ] || die "binary not executable: $BIN"

EVID="$ROOT/.dev-team/evidence/tui-polish/$PHASE"
mkdir -p "$EVID"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/tui-audit.XXXXXX")"
SOCK="tui-audit-$$"
MOCK_PID=""
SCEN_INDEX=0
SCEN_FAILED=0
SESS=""

cleanup() {
  if [ -n "${MOCK_PID:-}" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
    MOCK_PID=""
  fi
  tmux -L "$SOCK" kill-server >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT TERM

# tmux server: no status bar, deterministic terminfo, deep history.
tmux -L "$SOCK" -f /dev/null start-server >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g status off >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g default-terminal "screen-256color" >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g history-limit 20000 >/dev/null 2>&1 || true

# ---------------------------------------------------------------------------
# binary identity (physical receipt)
# ---------------------------------------------------------------------------
BIN_VERSION="$("$BIN" --version </dev/null 2>&1 | head -1)"
BIN_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
BIN_MTIME="$(date -Is -r "$BIN")"
BIN_SIZE="$(stat -c %s "$BIN")"
python3 - "$EVID/bin-info.json" <<PY
import json, sys
json.dump({
    "phase": "$PHASE",
    "bin": "$BIN",
    "sha256": "$BIN_SHA",
    "mtime": "$BIN_MTIME",
    "size": $BIN_SIZE,
    "version": """$BIN_VERSION""",
    "generated_at": "$(date -Is)",
    "repo": "$ROOT",
}, open(sys.argv[1], "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY
echo "== phase=$PHASE bin=$BIN"
echo "== version=$BIN_VERSION sha256=${BIN_SHA:0:16}… mtime=$BIN_MTIME"
echo "== evidence=$EVID"

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------
free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

make_fixture() { # $1 iso root, $2 mock port
  local iso="$1" port="$2"
  mkdir -p "$iso/xdg-config/pony-agent" "$iso/xdg-data" "$iso/xdg-cache" "$iso/home"
  python3 - "$iso" "$port" <<'PY'
import json
import sys

root, port = sys.argv[1], int(sys.argv[2])
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
    "base_url": f"http://127.0.0.1:{port}/v1",
    "auth_type": "bearer",
    "api_key_env_var": "MOCK_API_KEY",
    "secret_ref": "",
    "api_key_value": "mock-key",
    "supported_protocols": ["openai-completions"],
    "endpoints": [],
    "models": [model],
    "selected_model_id": "mock-alpha",
}
registry = {"providers": [provider], "selected_provider_id": "fakep"}
with open(f"{root}/xdg-config/pony-agent/providers.json", "w", encoding="utf-8") as fh:
    json.dump(registry, fh, ensure_ascii=False, indent=2)
PY
}

start_mock() { # $1 scenario dir
  local dir="$1"
  PORT="$(free_port)"
  MOCK_LOG="$dir/mock-requests.jsonl"
  : > "$MOCK_LOG"
  python3 "$TOOLS/mock_llm.py" \
    --port "$PORT" \
    --log "$MOCK_LOG" \
    --models-count 20 \
    --chunk-delay-ms 15 \
    > "$dir/mock.out" 2>&1 &
  MOCK_PID=$!
  local i
  for i in $(seq 1 100); do
    if grep -q '"listening"' "$dir/mock.out" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$MOCK_PID" 2>/dev/null; then
      die "mock_llm.py died on startup (see $dir/mock.out)"
    fi
    sleep 0.05
  done
  die "mock_llm.py did not become ready (see $dir/mock.out)"
}

stop_mock() {
  if [ -n "${MOCK_PID:-}" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
    MOCK_PID=""
  fi
}

env_prefix() { # writes env flags into ENV_ARGS array
  ENV_ARGS=(
    env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY
        -u http_proxy -u https_proxy -u all_proxy
        -u NODE_USE_ENV_PROXY -u NO_COLOR
        "XDG_CONFIG_HOME=$ISO/xdg-config"
        "XDG_DATA_HOME=$ISO/xdg-data"
        "XDG_CACHE_HOME=$ISO/xdg-cache"
        "HOME=$ISO/home"
        "LANG=${LANG:-C.UTF-8}"
  )
}

begin_scenario() { # $1 name, $2 cols, $3 rows
  SCEN_NAME="$1"; COLS="$2"; ROWS="$3"
  SCEN_INDEX=$((SCEN_INDEX + 1))
  SCEN_DIR="$EVID/$SCEN_NAME"
  rm -rf "$SCEN_DIR"
  mkdir -p "$SCEN_DIR"
  ISO="$WORK/iso-$SCEN_NAME"
  SESS="audit-$SCEN_INDEX-$SCEN_NAME"
  SCEN_STARTED="$(date -Is)"
  SCEN_T0=$SECONDS
  echo
  echo "== [$SCEN_INDEX] $SCEN_NAME (${COLS}x${ROWS})"
}

prepare_iso() {
  # Hard isolation per scenario: no HOME/XDG state and no session history can
  # leak in from a previous scenario (Lead hit 401 pollution from a warm
  # sessions.db during manual verification).
  rm -rf "$ISO"
  mkdir -p "$ISO/xdg-config/pony-agent" "$ISO/xdg-data" "$ISO/xdg-cache" "$ISO/home"
  rm -f "$ISO/xdg-data/PonyAgent/sessions.db" \
        "$ISO/xdg-data/PonyAgent/sessions.db-shm" \
        "$ISO/xdg-data/PonyAgent/sessions.db-wal"
}

begin_tui_scenario() { # name cols rows
  begin_scenario "$@"
  prepare_iso
  start_mock "$SCEN_DIR"
  make_fixture "$ISO" "$PORT"
  env_prefix
  tmux -L "$SOCK" new-session -d -s "$SESS" -x "$COLS" -y "$ROWS" -c "$ROOT" \
    "${ENV_ARGS[@]}" "$BIN" >/dev/null 2>&1 || die "tmux new-session failed ($SESS)"
  wait_ready || warn "$SCEN_NAME: TUI did not show composer prompt in 20s"
  read_pane_size
  sleep 0.4
}

read_pane_size() {
  local size
  size="$(tmux -L "$SOCK" display -p -t "$SESS" '#{pane_width}x#{pane_height}' 2>/dev/null)"
  if [[ "$size" =~ ^([0-9]+)x([0-9]+)$ ]]; then
    COLS="${BASH_REMATCH[1]}"
    ROWS="${BASH_REMATCH[2]}"
  fi
}

wait_ready() {
  local i out
  for i in $(seq 1 100); do
    out="$(tmux -L "$SOCK" capture-pane -t "$SESS" -p 2>/dev/null || true)"
    if grep -q "›" <<<"$out"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

wait_for() { # $1 session, $2 extended regex, $3 timeout seconds
  local sess="$1" pattern="$2" timeout="${3:-20}" i out
  local end=$((SECONDS + timeout))
  while [ "$SECONDS" -lt "$end" ]; do
    out="$(tmux -L "$SOCK" capture-pane -t "$sess" -p 2>/dev/null || true)"
    if grep -qE "$pattern" <<<"$out"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

send_literal() { tmux -L "$SOCK" send-keys -t "$SESS" -l "$1"; }
send_line() { tmux -L "$SOCK" send-keys -t "$SESS" -l "$1"; tmux -L "$SOCK" send-keys -t "$SESS" Enter; }
send_key() { tmux -L "$SOCK" send-keys -t "$SESS" "$1"; }

capture() { # $1 session, $2 dir, $3 name
  local sess="$1" dir="$2" name="$3"
  tmux -L "$SOCK" capture-pane -t "$sess" -p > "$dir/$name.txt" 2>/dev/null || true
  tmux -L "$SOCK" capture-pane -t "$sess" -p -e > "$dir/$name.ansi" 2>/dev/null || true
  if ! python3 "$TOOLS/render_ansi.py" \
      --input "$dir/$name.ansi" \
      --output "$dir/$name.png" \
      --cols "$COLS" --rows "$ROWS" --scale "$SCALE" \
      >> "$dir/render.log" 2>&1; then
    warn "$SCEN_NAME: render failed for $name (see $dir/render.log)"
  fi
}

write_extra() { # $1 json string
  python3 - "$SCEN_DIR/extra.json" "$1" <<'PY'
import json
import sys
json.dump(json.loads(sys.argv[2]), open(sys.argv[1], "w", encoding="utf-8"),
          ensure_ascii=False, indent=2)
PY
}

write_meta() {
  SCEN_NAME="$SCEN_NAME" SCEN_INDEX="$SCEN_INDEX" SCEN_DIR="$SCEN_DIR" \
  ACT_COLS="$COLS" ACT_ROWS="$ROWS" PHASE="$PHASE" BIN="$BIN" \
  SCEN_STARTED="$SCEN_STARTED" \
  SCEN_DURATION_MS=$(( (SECONDS - SCEN_T0) * 1000 )) \
  python3 - "$SCEN_DIR/meta.json" <<'PY'
import json
import os
import sys
json.dump(
    {
        "phase": os.environ["PHASE"],
        "scenario": os.environ["SCEN_NAME"],
        "index": int(os.environ["SCEN_INDEX"]),
        "cols": int(os.environ["ACT_COLS"]),
        "rows": int(os.environ["ACT_ROWS"]),
        "bin": os.environ["BIN"],
        "started_at": os.environ["SCEN_STARTED"],
        "duration_ms": int(os.environ["SCEN_DURATION_MS"]),
    },
    open(sys.argv[1], "w", encoding="utf-8"),
    ensure_ascii=False,
    indent=2,
)
PY
}

end_scenario() {
  stop_mock
  tmux -L "$SOCK" kill-session -t "$SESS" >/dev/null 2>&1 || true
  write_meta
  local result
  result="$(python3 "$TOOLS/checks.py" check --dir "$SCEN_DIR" 2>&1)"
  echo "$result"
  if ! grep -q "fail=0" <<<"$result"; then
    SCEN_FAILED=$((SCEN_FAILED + 1))
  fi
}

should_run() {
  [ -z "$ONLY" ] && return 0
  case ",$ONLY," in *",$1,"*) return 0 ;; esac
  return 1
}

# ---------------------------------------------------------------------------
# scenarios
# ---------------------------------------------------------------------------
scn_empty() {
  begin_tui_scenario empty 120 34
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" main
  write_extra '{}'
  end_scenario
}

scn_conversation() {
  begin_tui_scenario conversation 120 34
  send_line "读取 package.json 并总结"
  wait_for "$SESS" "结论" 25 || warn "conversation: final answer not seen"
  wait_for "$SESS" "$TURN_SETTLED_RE" 15 || warn "conversation: turn usage metrics not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_slash_menu() {
  begin_tui_scenario slash-menu 120 34
  send_literal "/"
  wait_for "$SESS" "/quit" 8 || warn "slash-menu: command table not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra '{}'
  end_scenario
}

scn_models_panel() {
  begin_tui_scenario models-panel 120 34
  send_line "/models"
  wait_for "$SESS" "模型 · 1/20" 25 || warn "models-panel: counter/title not seen (mock catalogue may still be loading)"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_provider_panel() {
  begin_tui_scenario provider-panel 120 34
  send_line "/provider"
  wait_for "$SESS" "a 新增" 15 || warn "provider-panel: panel not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra '{}'
  end_scenario
}

scn_sessions_panel() {
  begin_tui_scenario sessions-panel 120 34
  send_line "读取 package.json 并总结"
  wait_for "$SESS" "结论" 25 || warn "sessions-panel: seed conversation did not finish"
  wait_for "$SESS" "$TURN_SETTLED_RE" 15 || true
  send_line "/resume"
  wait_for "$SESS" "读取 package" 15 || warn "sessions-panel: prior session row not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_add_provider() {
  begin_tui_scenario add-provider 120 34
  send_line "/provider"
  wait_for "$SESS" "a 新增" 15 || warn "add-provider: provider panel not seen"
  send_literal "a"
  wait_for "$SESS" "新增提供商" 10 || warn "add-provider: modal not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra '{}'
  end_scenario
}

scn_streaming() {
  begin_tui_scenario streaming 120 34
  send_line "慢速输出测试"
  wait_for "$SESS" "第一部分" 15 || warn "streaming: partial body not seen"
  capture "$SESS" "$SCEN_DIR" mid
  wait_for "$SESS" "最后一行：流式结束" 25 || warn "streaming: final line not seen"
  wait_for "$SESS" "$TURN_SETTLED_RE" 15 || true
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" final
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_error() {
  begin_tui_scenario error 120 34
  send_line "401 认证测试"
  wait_for "$SESS" "Error:" 20 || warn "error: error row not seen"
  sleep 0.5
  capture "$SESS" "$SCEN_DIR" main
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_narrow() {
  begin_tui_scenario narrow 60 20
  capture "$SESS" "$SCEN_DIR" empty
  send_line "读取 package.json 并总结"
  wait_for "$SESS" "结论" 25 || warn "narrow: final answer not seen"
  wait_for "$SESS" "$TURN_SETTLED_RE" 15 || true
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" conversation
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_wide() {
  begin_tui_scenario wide 160 40
  capture "$SESS" "$SCEN_DIR" empty
  send_line "读取 package.json 并总结"
  wait_for "$SESS" "结论" 25 || warn "wide: final answer not seen"
  wait_for "$SESS" "$TURN_SETTLED_RE" 15 || true
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" conversation
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_scrolled() {
  begin_tui_scenario scrolled 120 34
  send_line "长文滚动测试"
  wait_for "$SESS" "这是最后一行" 30 || warn "scrolled: long answer tail not seen"
  wait_for "$SESS" "$TURN_SETTLED_RE" 15 || true
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" before
  send_key PageUp
  # wave-39 R6：footer 的「已上翻」提示已删除；PageUp 生效的真实信号 = 更早的正文行
  # （第 14 行）进入视口，与 checks.py `scrolled.after_shows_earlier_rows` 的断言同一信号。
  wait_for "$SESS" "第 14 行" 8 || warn "scrolled: scrolled-up row not seen"
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" after
  write_extra "{\"mock_log\": \"$MOCK_LOG\"}"
  end_scenario
}

scn_help() {
  begin_tui_scenario help 120 34
  send_line "/help"
  wait_for "$SESS" "命令列表|Commands:" 10 || warn "help: command reference not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra '{}'
  end_scenario
}

scn_cost() {
  begin_tui_scenario cost 120 34
  send_line "/cost"
  wait_for "$SESS" "网关|Gateway:" 10 || warn "cost: cost line not seen"
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" main
  write_extra '{}'
  end_scenario
}

scn_ctrl_c() {
  begin_scenario ctrl-c 120 34
  prepare_iso
  start_mock "$SCEN_DIR"
  make_fixture "$ISO" "$PORT"
  env_prefix
  tmux -L "$SOCK" new-session -d -s "$SESS" -x "$COLS" -y "$ROWS" -c "$ROOT" \
    /bin/bash --noprofile --norc -i >/dev/null 2>&1 || die "tmux new-session failed ($SESS)"
  sleep 0.8
  local cmd
  cmd="$(printf '%q ' "${ENV_ARGS[@]}")$BIN"
  read_pane_size
  tmux -L "$SOCK" send-keys -t "$SESS" "$cmd; echo AUDIT_EXIT=\$?" Enter
  wait_ready || warn "ctrl-c: TUI did not show composer prompt"
  sleep 0.4
  tmux -L "$SOCK" send-keys -t "$SESS" C-c
  wait_for "$SESS" "AUDIT_EXIT=" 20 || warn "ctrl-c: process exit marker not seen"
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" exit
  tmux -L "$SOCK" send-keys -t "$SESS" "echo AUDIT_SHELL_ALIVE=ok" Enter
  wait_for "$SESS" "AUDIT_SHELL_ALIVE=ok" 8 || warn "ctrl-c: shell echo not seen"
  sleep 0.2
  capture "$SESS" "$SCEN_DIR" shell
  local pane_cmd
  pane_cmd="$(tmux -L "$SOCK" display -p -t "$SESS" '#{pane_current_command}' 2>/dev/null || true)"
  write_extra "{\"pane_command_after\": \"$pane_cmd\"}"
  end_scenario
}

scn_input_edit() {
  begin_scenario input-edit 120 34
  prepare_iso
  start_mock "$SCEN_DIR"
  make_fixture "$ISO" "$PORT"
  env_prefix
  tmux -L "$SOCK" new-session -d -s "$SESS" -x "$COLS" -y "$ROWS" -c "$ROOT" \
    /bin/bash --noprofile --norc -i >/dev/null 2>&1 || die "tmux new-session failed ($SESS)"
  sleep 0.8
  local cmd
  cmd="$(printf '%q ' "${ENV_ARGS[@]}")$BIN"
  read_pane_size
  tmux -L "$SOCK" send-keys -t "$SESS" "$cmd; echo AUDIT_EXIT=\$?" Enter
  wait_ready || warn "input-edit: TUI did not show composer prompt"
  sleep 0.4

  send_literal "abc"
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" abc

  send_key Left
  send_literal "X"
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" inserted

  send_key Right
  send_key BSpace
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" backspace

  send_key C-w
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" word_cleared

  send_literal "/mo"
  sleep 0.3
  send_key Tab
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" tab_complete

  send_key C-u
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" ctrl_u_cleared

  send_literal "line1"
  send_key C-j
  send_literal "line2"
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" multiline

  send_key Escape
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" esc_cleared

  send_literal "draft"
  sleep 0.25
  capture "$SESS" "$SCEN_DIR" draft

  send_key C-c
  sleep 0.4
  capture "$SESS" "$SCEN_DIR" draft_cleared

  send_key C-c
  wait_for "$SESS" "AUDIT_EXIT=" 15 || warn "input-edit: process exit marker not seen"
  sleep 0.3
  capture "$SESS" "$SCEN_DIR" exit
  tmux -L "$SOCK" send-keys -t "$SESS" "echo AUDIT_SHELL_ALIVE=ok" Enter
  wait_for "$SESS" "AUDIT_SHELL_ALIVE=ok" 8 || warn "input-edit: shell echo not seen"
  sleep 0.2
  capture "$SESS" "$SCEN_DIR" shell
  local pane_cmd
  pane_cmd="$(tmux -L "$SOCK" display -p -t "$SESS" '#{pane_current_command}' 2>/dev/null || true)"
  write_extra "{\"pane_command_after\": \"$pane_cmd\"}"
  end_scenario
}

scn_non_tty() {
  begin_scenario non-tty 80 24
  prepare_iso
  env_prefix
  local code
  timeout 25 "${ENV_ARGS[@]}" "$BIN" </dev/null > "$SCEN_DIR/nontty.out" 2>&1
  code=$?
  echo "-- non-tty exit_code=$code"
  write_extra "{\"output_file\": \"nontty.out\", \"exit_code\": $code}"
  end_scenario
}

scn_cli_modes() {
  begin_scenario cli-modes 80 24
  prepare_iso
  env_prefix
  local names=(version help doctor)
  local args=(--version --help --doctor)
  local runs=""
  local i code
  for i in 0 1 2; do
    timeout 30 "${ENV_ARGS[@]}" "$BIN" "${args[$i]}" </dev/null \
      > "$SCEN_DIR/${names[$i]}.out" 2>&1
    code=$?
    echo "-- --${names[$i]} exit_code=$code"
    [ -n "$runs" ] && runs="$runs,"
    runs="$runs{\"name\": \"${names[$i]}\", \"exit_code\": $code, \"output_file\": \"${names[$i]}.out\"}"
  done
  write_extra "{\"cmd_runs\": [$runs]}"
  end_scenario
}

# ---------------------------------------------------------------------------
# run
# ---------------------------------------------------------------------------
for scenario in empty conversation slash-menu models-panel provider-panel \
                sessions-panel add-provider streaming error narrow wide \
                scrolled help cost ctrl-c input-edit non-tty cli-modes; do
  if ! should_run "$scenario"; then
    continue
  fi
  case "$scenario" in
    empty) scn_empty ;;
    conversation) scn_conversation ;;
    slash-menu) scn_slash_menu ;;
    models-panel) scn_models_panel ;;
    provider-panel) scn_provider_panel ;;
    sessions-panel) scn_sessions_panel ;;
    add-provider) scn_add_provider ;;
    streaming) scn_streaming ;;
    error) scn_error ;;
    narrow) scn_narrow ;;
    wide) scn_wide ;;
    scrolled) scn_scrolled ;;
    help) scn_help ;;
    cost) scn_cost ;;
    ctrl-c) scn_ctrl_c ;;
    input-edit) scn_input_edit ;;
    non-tty) scn_non_tty ;;
    cli-modes) scn_cli_modes ;;
  esac
done

# ---------------------------------------------------------------------------
# aggregate report
# ---------------------------------------------------------------------------
echo
AGG_ARGS=(--phase-dir "$EVID")
BASELINE_DIR="$(cd "$EVID/.." && pwd)/baseline"
if [ "$PHASE" = "after" ] && [ -f "$BASELINE_DIR/report.json" ]; then
  AGG_ARGS+=(--compare "$BASELINE_DIR")
fi
AGG_JSON="$(python3 "$TOOLS/checks.py" aggregate "${AGG_ARGS[@]}")"
AGG_CODE=$?
if [ "$AGG_CODE" -ne 0 ]; then
  die "report aggregation failed"
fi
echo "$AGG_JSON"
SUMMARY="$(python3 - "$EVID/report.json" <<'SUM'
import json
import sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
s = report["summary"]
print(f"SUMMARY: {s['pass']} pass / {s['fail']} fail across {s['scenarios']} scenarios")
print("failed_scenarios=" + ",".join(s["failed_scenarios"]))
SUM
)"
echo "$SUMMARY"
echo "report: $EVID/report.json"
echo "report: $EVID/report.md"
FAIL_COUNT="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["summary"]["fail"])' "$EVID/report.json")"
if [ "$FAIL_COUNT" -eq 0 ]; then
  exit 0
fi
exit 1
