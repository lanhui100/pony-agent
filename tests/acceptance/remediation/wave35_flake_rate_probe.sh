#!/usr/bin/env bash
# wave35_flake_rate_probe.sh — 独立测量 freeze-2 全量并行下的残留 flake 率
#
# 用法: bash tests/acceptance/remediation/wave35_flake_rate_probe.sh [轮数,默认 5]
# 每轮: RUSTFLAGS="-D warnings" timeout 900 cargo test -p pony-agent-core --lib
# 记录: 每轮 通过/失败、耗时、失败用例名;末尾给出 flake 率点估计(带 Wilson 95% 上界)。
set -uo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT"
N="${1:-5}"
OUT_DIR="$REPO_ROOT/tests/acceptance/receipts/wave35"
STAMP="$(date +%Y%m%dT%H%M%S)"
LOG="$OUT_DIR/flake_rate_probe_${STAMP}.log"
RUNS_DIR="$OUT_DIR/flake_rate_runs_${STAMP}"
mkdir -p "$RUNS_DIR"

{
  echo "# flake rate probe @ $(date -Is) rounds=$N"
  echo "# HEAD=$(git rev-parse HEAD) retry.rs_sha256=$(sha256sum crates/pony-agent-core/src/agent/retry.rs | cut -d' ' -f1)"
} > "$LOG"

FAILS=0
for i in $(seq 1 "$N"); do
  f="$RUNS_DIR/run_${i}.log"
  t0=$(date +%s)
  RUSTFLAGS="-D warnings" timeout 900 cargo test -p pony-agent-core --lib -- --test-threads=$(nproc) > "$f" 2>&1
  rc=$?
  t1=$(date +%s)
  res=$(grep -E '^test result:' "$f" | tail -1 | sed 's/; finished in.*//')
  bad=$(grep -E '^test .* \.\.\. FAILED$' "$f" | sed 's/^test /    FAILED: /' | head -5)
  echo "---- run $i: exit=$rc wall=$((t1-t0))s :: $res" | tee -a "$LOG"
  [ -n "$bad" ] && printf '%s\n' "$bad" | tee -a "$LOG"
  if [ "$rc" -ne 0 ]; then FAILS=$((FAILS + 1)); fi
done

python3 - "$N" "$FAILS" "$LOG" <<'PY'
import math, sys
n, k, log = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
p = k / n if n else 0.0
# Wilson 95% 上界
z = 1.96
den = 1 + z*z/n
centre = (p + z*z/(2*n)) / den
half = z * math.sqrt(p*(1-p)/n + z*z/(4*n*n)) / den
line = f"SUMMARY rounds={n} failed_runs={k} point_estimate={p:.3f} ({k}/{n}) wilson95_upper={(centre+half):.3f}"
print(line)
open(log, "a", encoding="utf-8").write("\n" + line + "\n")
PY
echo "raw: $LOG"; echo "per-run logs: $RUNS_DIR"