#!/usr/bin/env bash
# start-tauri-dev.sh —— Linux/macOS 侧的 `dev:tauri` 启动器（Windows 走 start-tauri-dev.ps1）。
#
# 与 PowerShell 版的语义对齐（WebView2 / Job Object 相关逻辑仅 Windows 需要，这里不复制）：
#   1. 清掉上一次 dev 会话的 pid 与 Vite 端口残留，避免 beforeDevCommand 起不来；
#   2. 直接以 node 启动 Vite（不经 npm 管道），等 127.0.0.1:4176 就绪；
#   3. `cargo run` 走 scripts/cargo-lane.sh，遵守并发预算与负载熔断；
#   4. 退出时回收 Vite 与端口（trap）。
# 退出码：0 正常 / 1 dev 启动失败 / 2 前置缺失（node / cargo / vite）。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${PONY_DEV_PORT:-4176}"
PID_FILE="$REPO_ROOT/.tauri-dev.pid"
VITE_BIN="$REPO_ROOT/node_modules/vite/bin/vite.js"

die() { echo "[dev] $*" >&2; exit "${2:-1}"; }

command -v node >/dev/null 2>&1 || die "找不到 node" 2
command -v cargo >/dev/null 2>&1 || die "找不到 cargo" 2
[ -f "$VITE_BIN" ] || die "找不到 Vite：$VITE_BIN（先 npm install）" 2

free_port() {
  if command -v fuser >/dev/null 2>&1; then
    fuser -k "${PORT}/tcp" >/dev/null 2>&1 && echo "[dev] 已释放端口 $PORT"
  elif command -v lsof >/dev/null 2>&1; then
    local pids
    pids="$(lsof -ti "tcp:$PORT" 2>/dev/null || true)"
    [ -n "$pids" ] && kill $pids 2>/dev/null && echo "[dev] 已释放端口 $PORT"
  fi
}

if [ -f "$PID_FILE" ]; then
  prev="$(cat "$PID_FILE" 2>/dev/null || true)"
  if [ -n "${prev:-}" ] && kill -0 "$prev" 2>/dev/null; then
    kill "$prev" 2>/dev/null && echo "[dev] 已停止上一次 dev 进程（pid $prev）"
    sleep 1
  fi
  rm -f "$PID_FILE"
fi
free_port

export CARGO_TARGET_DIR="$REPO_ROOT/target"
export CARGO_PROFILE_DEV_DEBUG=0

echo "[dev] 启动 Vite（node $VITE_BIN --configLoader native）"
node "$VITE_BIN" --configLoader native &
VITE_PID=$!
echo $$ > "$PID_FILE"

cleanup() {
  kill "$VITE_PID" 2>/dev/null || true
  wait "$VITE_PID" 2>/dev/null || true
  rm -f "$PID_FILE"
  free_port
}
trap cleanup EXIT INT TERM

ready=0
for _ in $(seq 1 100); do
  if (echo > "/dev/tcp/127.0.0.1/$PORT") >/dev/null 2>&1; then ready=1; break; fi
  sleep 0.3
done
[ "$ready" -eq 1 ] || die "Vite 30s 内未就绪（端口 $PORT）"
echo "[dev] Vite 就绪：http://127.0.0.1:$PORT"

# 桌面端编译走 lane：并发预算 + 负载熔断与其他 lane 共用同一套真源。
# 但**编译完必须释放构建锁**：`cargo-lane.sh run` 会让长驻桌面进程一直持有全局 flock，
# 其他会话的 cargo:check / test / check:fast 会无限期等锁（审核 P0）。故 build → 直接跑产物。
echo "[dev] 编译桌面端（lane：并发预算 + 负载熔断）"
bash "$REPO_ROOT/scripts/cargo-lane.sh" build --manifest-path "$REPO_ROOT/src-tauri/Cargo.toml" --no-default-features \
  || die "桌面端编译失败"
BIN="$REPO_ROOT/target/debug/pony-agent"
[ -x "$BIN" ] || die "找不到产物：$BIN"
echo "[dev] 启动桌面端（$BIN，已脱离构建锁）"
"$BIN"
