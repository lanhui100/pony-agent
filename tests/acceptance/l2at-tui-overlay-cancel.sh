#!/usr/bin/env bash
# //! acceptance: l2at-tui-overlay-cancel —— TUI Ask 覆盖层与取消反馈的**真实链路**验收
# //! ticket: l2at-tui-overlay-cancel | frozen acceptance — verify-only, DO NOT MODIFY during fixes
# //!
#! 契约真值源：`.dev-team/contract-matrix-tui-overlay-cancel.md` §2（O1/O4/O5/O6/O7/J1/C9）
#! 与 `docs/decisions/0063-tui-ask-user-overlay-and-cancel-feedback.md`。
#!
#! 三个真实场景（每个场景独占 HOME/XDG/日志/mock 端口；不碰真实 OTel/PonySentry 外发）：
#!   S1 stop-esc（O4/O5/C9 真实帧）：mock_llm.py 的 slow 场景 → 发送消息 → 等「正在分析」
#!      → Esc 两次 → 抓「正在停止…」帧 → 等「已停止」终态帧。
#!   S2 ask-overlay（O1 面板几何 + O7 真实帧）：以既有 `PONY_AUDIT_ASK_BOOTSTRAP`（生产 API
#!      造真实 pending Ask）拉起面板 → 抓「面板与 header/Composer/指标行共存」帧 → Esc 取消
#!      → 抓「面板关闭且无取消残留」帧。「历史与问题同帧」的判据由 Rust 渲染层用例承担
#!      （见下方 NOTE：该引导的 pending Ask 归属启动期临时 session id，无法与 resume 历史复现于同一进程）。
#!   S3 resume-summary（J1 真实帧）：真实跑一轮带工具的会话 → `pony resume` → 抓恢复帧，
#!      断言用户可见摘要是 description 而非内部 arguments JSON。
#!
#! 退出码：0 = 全绿；1 = 断言失败（红相）；2 = Harness/构建错误；3 = [RED DEGRADED: STATIC-ONLY]
#! （缺 tmux/python3 等环境）。
#!
#! 物理收据（tests/acceptance/receipts/l2at-tui-overlay-cancel/）：
#!   run.log / bin.log / build.log / s1-*.{txt,ansi,png} / s2-*.{txt,ansi,png} /
#!   s3-*.{txt,ansi,png} / mock-requests-*.jsonl / summary.json
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RECEIPTS="$HERE/receipts/l2at-tui-overlay-cancel"
TOOLS="$REPO/tools/tui-audit"
mkdir -p "$RECEIPTS"

LOG="$RECEIPTS/run.log"
: > "$LOG"
log() { echo "$*" | tee -a "$LOG"; }

PASS=0
FAIL=0
FAILED_NAMES=()
: > "$RECEIPTS/failed-assertions.txt"
ok() { PASS=$((PASS + 1)); log "PASS: $1"; }
bad() { FAIL=$((FAIL + 1)); FAILED_NAMES+=("$1"); printf '%s\n' "$1" >> "$RECEIPTS/failed-assertions.txt"; log "FAIL: $1"; }

# ---------------------------------------------------------------------------
# 0. 环境门禁（缺失 ⇒ 显式降级，不静默放行）
# ---------------------------------------------------------------------------
MISSING=""
command -v tmux >/dev/null 2>&1 || MISSING="$MISSING tmux"
command -v python3 >/dev/null 2>&1 || MISSING="$MISSING python3"
[ -f "$TOOLS/mock_llm.py" ] || MISSING="$MISSING tools/tui-audit/mock_llm.py"
if [ -n "$MISSING" ]; then
  echo "[RED DEGRADED: STATIC-ONLY] missing:$MISSING" | tee "$RECEIPTS/degraded.log" | tee -a "$LOG"
  exit 3
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/l2at-overlay.XXXXXX")"
SOCK="l2at-overlay-$$"
SESS=""
SCEN_DIR=""
ISO=""
MOCK_PID=""
COLS=120
ROWS=34
TMUX_STARTED=0
CURRENT_RUNTIME_LOG=""
CURRENT_RUNTIME_LOG_OFFSET=0

cleanup() {
  stop_mock
  if [ "$TMUX_STARTED" -eq 1 ]; then
    tmux -L "$SOCK" kill-server >/dev/null 2>&1 || true
  fi
  chmod -R u+w "$WORK" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# 1. 二进制：优先本次构建的 target/debug/pony；陈旧则重建并留 build.log/EXIT
# ---------------------------------------------------------------------------
TUI_SRC="$REPO/crates/pony-agent-core/src/bin/tui.rs"
BIN="$REPO/target/debug/pony"
NEED_BUILD=0
if [ ! -x "$BIN" ]; then
  NEED_BUILD=1
elif [ "$TUI_SRC" -nt "$BIN" ]; then
  NEED_BUILD=1
fi
if [ "$NEED_BUILD" -eq 1 ]; then
  log "== rebuilding bin (stale or missing): cargo build -p pony-agent-core --bin pony"
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
  echo "tui_src_sha256=$(sha256sum "$TUI_SRC" | awk '{print $1}')"
  echo "generated_at=$(date -Is)"
  echo "git_commit=$(cd "$REPO" && git rev-parse HEAD 2>/dev/null)"
} > "$RECEIPTS/bin.log"
cat "$RECEIPTS/bin.log" | tee -a "$LOG"

tmux -L "$SOCK" -f /dev/null start-server >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g status off >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g default-terminal "screen-256color" >/dev/null 2>&1 || true
tmux -L "$SOCK" set-option -g history-limit 20000 >/dev/null 2>&1 || true
TMUX_STARTED=1

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

# 全屏 diff 渲染会在字素之间插入光标定位序列；断言前统一去掉空白（与 Rust 驱动同口径）。
squeeze() { python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\s+", "", sys.stdin.read()))'; }

prepare_iso() { # $1 iso root, $2 mock port
  local iso="$1" port="$2"
  rm -rf "$iso"
  mkdir -p "$iso/xdg-config/pony-agent" "$iso/xdg-data" "$iso/xdg-cache" "$iso/home"
  python3 - "$iso" "$port" <<'PY'
import json
import sys

root, port = sys.argv[1], int(sys.argv[2])
model = {
    "id": "mock-alpha", "name": "mock-alpha", "model": "mock-alpha",
    "temperature": 0.2, "maxOutputTokens": 64000, "capabilityPreset": "open-ai-chat",
    "reasoningEffort": None, "reasoningBudgetTokens": None, "protocol": "openai-completions",
    "capabilities": {
        "contextWindowTokens": 256000, "supportsTools": True, "supportsStreaming": True,
        "supportsImageInput": True, "supportsReasoning": False, "supportsVideoInput": False,
        "supportsAudioInput": False, "supportsImageOutput": False, "supportsAudioOutput": False,
    },
}
provider = {
    "id": "fakep", "name": "FakeProvider", "protocol": "openai-completions",
    "base_url": "http://127.0.0.1:%d/v1" % port, "auth_type": "bearer",
    "api_key_env_var": "MOCK_API_KEY", "secret_ref": "", "api_key_value": "mock-key",
    "supported_protocols": ["openai-completions"], "endpoints": [],
    "models": [model], "selected_model_id": "mock-alpha",
}
with open("%s/xdg-config/pony-agent/providers.json" % root, "w", encoding="utf-8") as fh:
    json.dump({"providers": [provider], "selected_provider_id": "fakep"}, fh, ensure_ascii=False, indent=2)
PY
}

start_mock() { # $1 scenario dir, $2 first-token-delay-ms
  local dir="$1" delay="${2:-0}"
  PORT="$(free_port)"
  MOCK_LOG="$dir/mock-requests.jsonl"
  : > "$MOCK_LOG"
  python3 "$TOOLS/mock_llm.py" \
    --port "$PORT" --log "$MOCK_LOG" --chunk-delay-ms 15 \
    --first-token-delay-ms "$delay" \
    > "$dir/mock.out" 2>&1 &
  MOCK_PID=$!
  local i
  for i in $(seq 1 100); do
    grep -q '"listening"' "$dir/mock.out" 2>/dev/null && return 0
    kill -0 "$MOCK_PID" 2>/dev/null || { log "FATAL: mock_llm.py died (see $dir/mock.out)"; exit 2; }
    sleep 0.05
  done
  log "FATAL: mock_llm.py did not become ready (see $dir/mock.out)"
  exit 2
}

stop_mock() {
  if [ -n "${MOCK_PID:-}" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
    MOCK_PID=""
  fi
}

BASE_ENV_ARGS=()
build_env_args() {
  BASE_ENV_ARGS=(
    env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY
        -u http_proxy -u https_proxy -u all_proxy -u NO_COLOR
        "XDG_CONFIG_HOME=$ISO/xdg-config"
        "XDG_DATA_HOME=$ISO/xdg-data"
        "XDG_CACHE_HOME=$ISO/xdg-cache"
        "HOME=$ISO/home"
        "TERM=screen-256color"
        "LANG=${LANG:-C.UTF-8}"
        # 不外发真实可观测性后端：Sentry 关闭、OTel 采样关到 0 且端点指向本机不可达端口。
        "PONYSENTRY_INGEST_URL="
        "PONYSENTRY_CLIENT_TOKEN="
        "OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:9/v1/traces"
        "OTEL_TRACES_SAMPLER=always_off"
        "OTEL_TRACES_SAMPLER_ARG=0"
  )
}

begin_scenario() { # $1 name, $2 first-token-delay-ms
  local name="$1" delay="${2:-0}"
  SCEN_DIR="$RECEIPTS/$name"
  rm -rf "$SCEN_DIR"
  mkdir -p "$SCEN_DIR"
  ISO="$WORK/iso-$name"
  SESS="l2at-$name"
  start_mock "$SCEN_DIR" "$delay"
  prepare_iso "$ISO" "$PORT"
  build_env_args
  CURRENT_RUNTIME_LOG="$SCEN_DIR/tui-runtime.log"
  : > "$CURRENT_RUNTIME_LOG"
  CURRENT_RUNTIME_LOG_OFFSET="$(stat -c%s "$CURRENT_RUNTIME_LOG" 2>/dev/null || echo 0)"
  BASE_ENV_ARGS+=("PONY_TUI_RUNTIME_LOG=$CURRENT_RUNTIME_LOG")
  echo "== scenario $name (mock port $PORT)" | tee -a "$LOG"
}

launch_tui() { # $1 extra env assignment (optional), rest = args
  local extra_env="$1"; shift
  local env_args=("${BASE_ENV_ARGS[@]}")
  if [ -n "$extra_env" ]; then
    env_args+=("$extra_env")
  fi
  tmux -L "$SOCK" new-session -d -s "$SESS" -x "$COLS" -y "$ROWS" -c "$REPO" \
    "${env_args[@]}" "$BIN" "$@" >/dev/null 2>&1 || { log "FATAL: tmux new-session failed ($SESS)"; exit 2; }
  tmux -L "$SOCK" pipe-pane -O -t "$SESS" "cat >> '$SCEN_DIR/stream.raw'" >/dev/null 2>&1 || true
  wait_for "›" 40
}

end_scenario() {
  tmux -L "$SOCK" kill-session -t "$SESS" >/dev/null 2>&1 || true
  SESS=""
}

wait_for() { # $1 extended regex, $2 timeout seconds
  local pattern timeout="${2:-20}" out
  pattern="$(printf '%s' "$1" | squeeze)"
  local end=$((SECONDS + timeout))
  while [ "$SECONDS" -lt "$end" ]; do
    out="$(tmux -L "$SOCK" capture-pane -t "$SESS" -p 2>/dev/null | squeeze)"
    if grep -qE "$pattern" <<<"$out"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

wait_gone() { # $1 extended regex, $2 timeout seconds
  local pattern timeout="${2:-20}" out
  pattern="$(printf '%s' "$1" | squeeze)"
  local end=$((SECONDS + timeout))
  while [ "$SECONDS" -lt "$end" ]; do
    out="$(tmux -L "$SOCK" capture-pane -t "$SESS" -p 2>/dev/null | squeeze)"
    if ! grep -qE "$pattern" <<<"$out"; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

send_line() { tmux -L "$SOCK" send-keys -t "$SESS" -l "$1"; tmux -L "$SOCK" send-keys -t "$SESS" Enter; }
send_esc() { tmux -L "$SOCK" send-keys -t "$SESS" Escape; }

capture() { # $1 name
  local name="$1"
  tmux -L "$SOCK" capture-pane -t "$SESS" -p > "$SCEN_DIR/$name.txt" 2>/dev/null || true
  tmux -L "$SOCK" capture-pane -t "$SESS" -p -e > "$SCEN_DIR/$name.ansi" 2>/dev/null || true
  python3 "$TOOLS/render_ansi.py" --input "$SCEN_DIR/$name.ansi" --output "$SCEN_DIR/$name.png" \
    --cols "$COLS" --rows "$ROWS" --scale 2 >> "$SCEN_DIR/render.log" 2>&1 || \
    log "WARN: render failed for $name (see $SCEN_DIR/render.log)"
  squeeze < "$SCEN_DIR/$name.txt" > "$SCEN_DIR/$name.squeezed.txt"
}

pane_has() { # $1 needle (会被同样去空白后做定宽匹配), $2 capture 名
  local needle
  needle="$(printf '%s' "$1" | squeeze)"
  grep -qF -- "$needle" "$SCEN_DIR/$2.squeezed.txt"
}

capture_when() { # $1 extended regex, $2 capture 名, $3 timeout 秒 —— 命中即抓帧（20ms 轮询）
  local pattern name="$2" timeout="${3:-5}" out
  pattern="$(printf '%s' "$1" | squeeze)"
  local end=$((SECONDS + timeout))
  while [ "$SECONDS" -lt "$end" ]; do
    out="$(tmux -L "$SOCK" capture-pane -t "$SESS" -p 2>/dev/null | squeeze)"
    if grep -qE "$pattern" <<<"$out"; then
      printf '%s' "$out" > "$SCEN_DIR/$name.squeezed.txt"
      tmux -L "$SOCK" capture-pane -t "$SESS" -p > "$SCEN_DIR/$name.txt" 2>/dev/null || true
      tmux -L "$SOCK" capture-pane -t "$SESS" -p -e > "$SCEN_DIR/$name.ansi" 2>/dev/null || true
      python3 "$TOOLS/render_ansi.py" --input "$SCEN_DIR/$name.ansi" --output "$SCEN_DIR/$name.png" \
        --cols "$COLS" --rows "$ROWS" --scale 2 >> "$SCEN_DIR/render.log" 2>&1 || true
      return 0
    fi
    sleep 0.02
  done
  return 1
}

# ---------------------------------------------------------------------------
# S1：O4 / O5 / C9 —— 真实流式双击 Esc：正在停止 → 已停止（用户中断）
# ---------------------------------------------------------------------------
scenario_stop_esc() {
  begin_scenario "s1-stop-esc" 1500
  launch_tui "" || { bad "S1-prelude TUI 未在 40s 内就绪"; end_scenario; return; }

  send_line "慢速 请给我一个流式回答"
  if ! wait_for "正在分析" 40; then
    bad "S1-prelude mock 慢速流未开始"
    end_scenario
    return
  fi
  capture "s1-0-streaming"

  # O4：第一击只上膛、第二击命中（500ms 窗口内）——真实按键。
  send_esc
  sleep 0.12
  send_esc
  if capture_when "正在停止" "s1-1-stopping" 4; then
    ok "O4 受理后状态行显示「正在停止…」"
  else
    capture "s1-1-stopping"
    bad "O4 双击 Esc 受理后未显示「正在停止…」"
  fi
  if pane_has "已停止" "s1-1-stopping" || pane_has "已终止" "s1-1-stopping"; then
    bad "O4 未收到终态事件前不得显示已停止/已终止"
  else
    ok "O4 未收到终态事件前未显示已停止/已终止"
  fi
  if pane_has "已中断本轮生成。" "s1-1-stopping"; then
    bad "O4 出现被禁止的本地伪造终态文案「已中断本轮生成。」"
  else
    ok "O4 无本地伪造终态文案"
  fi

  # O5：终态事件驱动「已停止（用户中断）」，且不新增取消消息。
  if wait_for "已停止" 25; then
    sleep 0.4
    capture "s1-2-stopped"
    if pane_has "已停止（用户中断）" "s1-2-stopped"; then
      ok "O5 终态显示「已停止（用户中断）」"
    else
      bad "O5 终态未显示「已停止（用户中断）」"
    fi
    if pane_has "慢速 请给我一个流式回答" "s1-2-stopped"; then
      ok "O5 取消终态保留用户消息与已渲染正文"
    else
      bad "O5 取消终态丢失用户消息/正文"
    fi
  else
    bad "O5 25s 内未观察到取消终态「已停止」"
    capture "s1-2-stopped"
  fi

  # O8（用户所述第二缺陷）：`otel::log` 的 JSON 缺省写 **stdout**，而 TUI 只 dup2 stderr ⇒
  # warn 级结构化日志（含 provider_stream_cancelled / trace_id / level）会直接混进真实屏幕与
  # 原始 pane 流。这里对**终态帧 + 累计原始流**同时取证：任何一条出现即判失败。
  local leak=0 marker red_frame="$RECEIPTS/s1-prefix-structured-log-screen-red-phase.txt"
  for marker in '"level":"warn"' '"provider_stream_cancelled"' '"trace_id":"'; do
    if grep -qF -- "$marker" "$SCEN_DIR/s1-2-stopped.txt" 2>/dev/null; then
      leak=1
      log "  leak-marker=$marker file=s1-2-stopped.txt"
    fi
    if grep -qF -- "$marker" "$SCEN_DIR/stream.raw" 2>/dev/null; then
      leak=1
      log "  leak-marker=$marker file=stream.raw"
    fi
  done
  if [ "$leak" -eq 1 ]; then
    bad "O8 真实屏幕/原始 pane 流出现结构化日志 JSON（level / provider_stream_cancelled / trace_id）"
  else
    ok "O8 真实屏幕与原始 pane 流无结构化日志 JSON 泄漏，且「已停止（用户中断）」可见"
  fi
  # 判别力自检：同一组判据必须能命中**红相归档帧**，否则说明该断言在红绿两相都成立（空断言）。
  if [ -f "$red_frame" ] && grep -qF -- '"level":"warn"' "$red_frame"; then
    ok "O8 判据判别力自检：红相归档屏幕可被同一判据命中（修复前必红）"
  else
    bad "O8 判据判别力自检失败：红相归档屏幕未被命中（断言可能形同虚设，证据文件缺失或被改写）"
  fi

  # C9：真实终端里停止链路必须「请求可见 → 终态可见」（sink 未注册时 core 会向 stdout 打印
  # `stop accepted but no turn event sink registered; turn:stop_requested not dispatched`，
  # 一旦出现在任何抓帧里即判失败）。强判据（累计原始 stdout 中该告警必须缺席）由
  # `cargo test --test tui_overlay_cancel_acceptance c9_...` 承担，本脚本只给物理帧。
  if grep -q "no turn event sink registered" "$SCEN_DIR/stream.raw" 2>/dev/null; then
    bad "C9 控制面 stop_turn 未命中已注册 sink（原始输出出现：stop accepted but no turn event sink registered）"
  else
    ok "C9 控制面 stop_turn 命中已注册 sink（原始 pane 流无未注册告警）"
  fi

  local pane_text
  pane_text="$(cat "$SCEN_DIR"/s1-*.txt 2>/dev/null)"
  if grep -qE "panicked at|RUST_BACKTRACE" <<<"$pane_text"; then
    bad "S1 真实 TUI 出现 panic"
  else
    ok "S1 真实 TUI 无 panic"
  fi
  end_scenario
}

# ---------------------------------------------------------------------------
# S2：O1 / O7 —— 恢复的历史 + Ask 覆盖层同帧；Esc 取消关闭面板且无取消残留
# ---------------------------------------------------------------------------
seed_one_turn() { # 复用同一 ISO 资产跑完一轮后 /quit
  send_line "读取 package.json 并总结"
  wait_for "结论" 45 || return 1
  wait_for "↑[1-9]" 30 || return 1
  send_line "/quit"
  sleep 1.5
  return 0
}

scenario_ask_overlay() {
  local question="请选择本次审计的 Ask 闭环分支："
  begin_scenario "s2-ask-overlay" 0
  # 既有 debug 审计引导：用**生产 API**（dispatch_governed → WaitingHost → persist_pending_request）
  # 造一条真实 pending Ask，面板拉起/键盘/CAS 全部走既有链路；不是伪造面板。
  local bootstrap="PONY_AUDIT_ASK_BOOTSTRAP={\"prompt\":\"${question}\",\"options\":[\"分支 A · 冒烟\",\"分支 B · 全量\"]}"
  launch_tui "$bootstrap" || { bad "S2-prelude TUI 未在 40s 内就绪"; end_scenario; return; }
  if ! wait_for "$question" 45; then
    bad "S2-prelude Ask 面板未在真实终端拉起"
    capture "s2-0-no-panel"
    end_scenario
    return
  fi
  sleep 0.5
  capture "s2-1-ask-open"

  if pane_has "$question" "s2-1-ask-open"; then
    ok "O1 Ask 问题在真实终端可见"
  else
    bad "O1 Ask 问题不可见"
  fi
  # 说明（能力边界，非放水）：`PONY_AUDIT_ASK_BOOTSTRAP` 在 resume 之前创建 pending Ask，
  # 归属启动时的临时 session id，故「恢复的历史 + Ask 同帧」无法经该引导在同一进程复现；
  # O1 的「历史与问题同帧」判据由 Rust 渲染层用例
  # `cargo test --test tui_overlay_cancel_acceptance o1_...`（TestBackend 真实 `ui()` 渲染）承担。
  log "NOTE: O1 历史/问题同帧由 Rust TestBackend 用例判定；本帧只证明面板、header、Composer 不互相遮挡"
  # header（row0）/ Composer 色带（Ask 态显示「等待你的选择…」替身）/ 指标行（last row 的工作区标签）
  # 三者都必须仍在真实终端上可见 —— 这是「覆盖层不得遮挡固定基座」的可观测判据。
  local last_row
  last_row="$(tail -1 "$SCEN_DIR/s2-1-ask-open.txt" 2>/dev/null | squeeze)"
  if pane_has "pony" "s2-1-ask-open" && pane_has "等待你的选择" "s2-1-ask-open" && grep -qF "pony-agent" <<<"$last_row"; then
    ok "O1 Ask 未遮挡 header / Composer / 指标行"
  else
    bad "O1 Ask 遮挡了 header / Composer / 指标行之一（last_row=$last_row）"
  fi

  # O7：Ask 内 Esc 走既有取消语义（单次 Esc 即关面板；不继承流式双击停止）。
  send_esc
  if wait_gone "$question" 15; then
    sleep 0.5
    capture "s2-2-ask-cancelled"
    ok "O7 Ask 内 Esc 关闭面板"
  else
    capture "s2-2-ask-cancelled"
    bad "O7 Ask 内 Esc 未关闭面板"
  fi
  if pane_has "正在停止" "s2-2-ask-cancelled" || pane_has "已停止" "s2-2-ask-cancelled"; then
    bad "O7 Ask 内 Esc 误触发普通流式停止语义"
  else
    ok "O7 Ask 内 Esc 未继承流式双击停止语义"
  fi
  if pane_has "取消提问失败" "s2-2-ask-cancelled"; then
    bad "O7 Ask 取消报错（CAS 失败被暴露为用户可见错误）"
  else
    ok "O7 Ask 取消无 CAS 失败可见文案"
  fi
  end_scenario
}

# ---------------------------------------------------------------------------
# S3：J1 —— resume 的用户可见摘要必须是 description，而非内部 arguments JSON
# ---------------------------------------------------------------------------
scenario_resume_summary() {
  begin_scenario "s3-resume-summary" 0
  launch_tui "" || { bad "S3-prelude TUI 未在 40s 内就绪"; end_scenario; return; }
  if ! seed_one_turn; then
    bad "S3-prelude 历史种子轮未完成"
    end_scenario
    return
  fi
  end_scenario

  SESS="l2at-s3-resume"
  tmux -L "$SOCK" new-session -d -s "$SESS" -x "$COLS" -y "$ROWS" -c "$REPO" \
    "${BASE_ENV_ARGS[@]}" "$BIN" resume >/dev/null 2>&1 || { log "FATAL: tmux resume failed"; exit 2; }
  if ! wait_for "结论" 45; then
    bad "J1 `pony resume` 未恢复出历史"
    capture "s3-0-resume-failed"
    end_scenario
    return
  fi
  sleep 1.0
  capture "s3-1-resume"

  # 单一判据（避免同一事实两行结论互相矛盾）：
  # 恢复摘要必须是 description（用户可读），且不得出现结构化 arguments JSON 字段。
  local j1_json_leak=0 j1_desc=0
  if pane_has "\"path\"" "s3-1-resume" || pane_has "\"lineCount\"" "s3-1-resume"; then
    j1_json_leak=1
  fi
  if pane_has "收集路径package.json的上下文" "s3-1-resume"; then
    j1_desc=1
  fi
  if [ "$j1_json_leak" -eq 1 ]; then
    bad "J1 恢复视图把结构化 arguments JSON 反显为消息摘要（description 优先未生效）"
  elif [ "$j1_desc" -eq 0 ]; then
    bad "J1 恢复视图既无 JSON 泄漏也未显示 description（摘要缺失）"
  else
    ok "J1 恢复视图优先显示 description 且无结构化 JSON"
  fi
  end_scenario
}

# ---------------------------------------------------------------------------
# run
# ---------------------------------------------------------------------------
scenario_stop_esc
scenario_ask_overlay
scenario_resume_summary

python3 - "$RECEIPTS/summary.json" "$PASS" "$FAIL" "$RECEIPTS/failed-assertions.txt" <<'PY'
import json
import sys

path, passed, failed, names_path = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
try:
    with open(names_path, encoding="utf-8") as fh:
        names = [line.strip() for line in fh if line.strip()]
except FileNotFoundError:
    names = []
json.dump({
    "ticket": "l2at-tui-overlay-cancel",
    "pass": passed,
    "fail": failed,
    "failed_assertions": names,
}, open(path, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
PY

echo | tee -a "$LOG"
echo "==== l2at-tui-overlay-cancel summary: PASS=$PASS FAIL=$FAIL ====" | tee -a "$LOG"
[ "$FAIL" -eq 0 ]
