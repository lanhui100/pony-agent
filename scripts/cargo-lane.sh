#!/usr/bin/env bash
# cargo-lane.sh —— 本仓**唯一**的并发构建入口（并发预算 + 跨输出目录全局锁 + nice 降权）。
#
# 为什么需要它：cargo 的锁按 target 目录加锁（target/debug、target-test、target-check、
# target/release 各一把），多 lane 并行时互不阻塞；每条 cargo 又默认占满整机核数，
# 于是 N lane × 核数 个 rustc 同时跑，形成 2–3 倍 CPU 超订（2026-11-17 实测 load 45.86 / 16 核，
# release 构建 90min 未完成，单跑 3m07s–5m32s）。
#
# 真源：`.dev-team/build-budget.json`（形状由 dev-team 的 dev-build-budget-lint.py 校验）。
# 用法：
#   bash scripts/cargo-lane.sh test -p pony-agent-core --lib        # lane 构建/测试
#   bash scripts/cargo-lane.sh --heavy build --release --bin pony   # 重活（仅 Lead、独跑）
#   bash scripts/cargo-lane.sh --dry-run test -p pony-agent-core    # 打印最终命令，不执行
# 环境覆盖（排障用，不改变默认预算）：PONY_BUILD_JOBS / PONY_BUILD_LOCK / PONY_BUILD_NICE
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="$REPO_ROOT/.dev-team/build-budget.json"

HEAVY=0
DRY_RUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --heavy) HEAVY=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --) shift; break ;;
    *) break ;;
  esac
done

if [ ! -f "$CONFIG" ]; then
  echo "[cargo-lane] fail-closed：缺少 $CONFIG（并发预算未声明，拒绝无界构建）" >&2
  exit 2
fi

read -r JOBS LOCK NICE_VALUE < <(python3 - "$CONFIG" <<'PY'
import json, sys
config = json.load(open(sys.argv[1], encoding="utf-8"))
print(config["max_jobs_per_lane"], config["serialize_lock_path"], config["heavy_nice"])
PY
) || { echo "[cargo-lane] fail-closed：无法解析 $CONFIG" >&2; exit 2; }

JOBS="${PONY_BUILD_JOBS:-$JOBS}"
LOCK="${PONY_BUILD_LOCK:-$LOCK}"
if [ "$HEAVY" -eq 1 ]; then
  NICE_VALUE="${PONY_BUILD_NICE:-$NICE_VALUE}"
else
  NICE_VALUE="${PONY_BUILD_NICE:-5}"
fi

if ! command -v flock >/dev/null 2>&1; then
  echo "[cargo-lane] fail-closed：缺少 flock（跨输出目录串行化不可用，拒绝无界并发）" >&2
  exit 2
fi

CMD=(flock "$LOCK" nice -n "$NICE_VALUE" env "CARGO_BUILD_JOBS=$JOBS" cargo "$@")

if [ "$DRY_RUN" -eq 1 ]; then
  printf '%s\n' "${CMD[*]}"
  exit 0
fi

exec "${CMD[@]}"
