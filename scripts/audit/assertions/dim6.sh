#!/usr/bin/env bash
# scripts/audit/assertions/dim6.sh —— PA-169 C3：维度 6（上报链路自检）阈值断言
#
# 阈值口径：r1-dim6 提案（docs/audit/rounds/r1-dim6/proposals/r1-dim6.pa01a1ba70.md）
# 批准即冻结。**本脚本的哈希已记入 loop-state.current.assertion_hashes**，
# 改动即冻结漂移（state.py check-freeze → exit 4）。
#
# 三条断言（任一超阈 = 非零）：
#   A1 重放隔离：会话 `Environment: cwd=` 必须等于臂 worktree（失配数必须为 0）
#   A2 trace 级 release 非空（按发布线筛选可用；空值数必须为 0）
#   A3 结局单值：禁止 `agent.turn.phase=completed` 与 `agent.turn.outcome=error` 并存
#
# 用法：bash scripts/audit/assertions/dim6.sh --arm-dir <replay/arm> --expected-cwd <wt> \
#         [--metrics <metrics.json>]
# 退出码：0 = 全绿｜1 = 有断言失败｜2 = 证据缺失（UNKNOWN ≠ PASS）｜4 = 冻结漂移
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/../lib.sh"

ARM_DIR=""; EXPECTED_CWD=""; METRICS=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arm-dir) ARM_DIR="${2:?}"; shift 2 ;;
    --expected-cwd) EXPECTED_CWD="${2:?}"; shift 2 ;;
    --metrics) METRICS="${2:?}"; shift 2 ;;
    # 统一调用约定（r2-dim1 attempt 2 实测缺陷 F8）：`ratchet.sh` 与 `verdict.sh` 对
    # **所有**断言脚本统一透传 `--arm-tag`，而本脚本原先不认该参数 ⇒ `未知参数` exit=2
    # ⇒ ratchet 把 dim6 判成 MISSING（证据缺失）⇒ 任何后续维度的 verdict 恒为
    # UNKNOWN exit=2，永远走不到 PASS。dim6 的三条断言（隔离 / release / 结局）
    # 按 metrics 全量判定、不做逐臂过滤，故该参数在此语义上无需求：接受并忽略，
    # 只为对齐契约。所有 `assertions/dim*.sh` 都必须接受该参数（selftest 有机器校验）。
    --arm-tag) shift 2 ;;
    -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done
[ -n "$ARM_DIR" ] || { fail_msg "--arm-dir 必填"; exit 2; }
[ -d "$ARM_DIR" ] || { fail_msg "臂目录不存在：$ARM_DIR"; exit 2; }
[ -n "$EXPECTED_CWD" ] || { fail_msg "--expected-cwd 必填（臂 worktree 路径）"; exit 2; }

FAILED=0
MISSING=0

# ── A1 重放隔离 ────────────────────────────────────────────────────────────────
say "── A1 replay isolation (env cwd == arm worktree)"
a1_out="$(mktemp)"
python3 "$HERE/../analyze_replay_isolation.py" "$ARM_DIR" "$EXPECTED_CWD" > "$a1_out" 2>/dev/null
a1_rc=$?
if [ "$a1_rc" -eq 0 ]; then
  say "   PASS $(python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print(f\"sessions={d['session_count']} mismatch=0\")" "$a1_out")"
elif [ "$a1_rc" -eq 2 ]; then
  # 取证脚本自身判定「无输入」→ 证据缺失，绝不折算成 pass 或断言失败。
  say "   MISSING 无重放证据（$ARM_DIR 下无 *.jsonl）——UNKNOWN ≠ PASS"
  MISSING=$((MISSING + 1))
else
  python3 - "$a1_out" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    print("   FAIL 取证输出不可解析")
    raise SystemExit(0)
print(f"   FAIL sessions={d.get('session_count')} mismatch={d.get('isolation_mismatch_count')} expected={d.get('expected_cwd')}")
for s in d.get("sessions") or []:
    if not s.get("isolated"):
        print(f"     - {s.get('tag')}: env_cwd={s.get('env_cwd')}")
PY
  FAILED=$((FAILED + 1))
fi
rm -f "$a1_out"

# ── A2 / A3 依赖 Langfuse 采集的 metrics.json ──────────────────────────────────
if [ -z "$METRICS" ] || [ ! -f "$METRICS" ]; then
  say "── A2/A3 skipped: metrics.json 未提供（UNKNOWN ≠ PASS，计为证据缺失）"
  MISSING=$((MISSING + 1))
else
  say "── A2 trace-level release 必须非空（按发布线筛选可用）"
  if python3 - "$METRICS" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))
rows = data.get("sessions") or []
if not rows:
    print("   FAIL 空 metrics（无会话证据）")
    raise SystemExit(1)
missing = [row["tag"] for row in rows if not row.get("trace_release")]
print(f"   sampled={len(rows)} release_null={len(missing)}")
for tag in missing:
    print(f"     - {tag}: trace_release=null")
raise SystemExit(1 if missing else 0)
PY
  then
    say "   PASS"
  else
    FAILED=$((FAILED + 1))
  fi

  say "── A3 turn 结局单值：禁止 phase=completed 与 outcome=error 并存"
  if python3 - "$METRICS" <<'PY'
import json
import sys

rows = json.load(open(sys.argv[1], encoding="utf-8")).get("sessions") or []
bad = []
for row in rows:
    attrs = row.get("root_attributes") or {}
    if attrs.get("agent.turn.phase") == "completed" and attrs.get("agent.turn.outcome") == "error":
        bad.append((row["tag"], attrs.get("agent.turn.tool_failures"), attrs.get("agent.turn.degraded")))
print(f"   sampled={len(rows)} conflicting={len(bad)}")
for tag, failures, degraded in bad:
    print(f"     - {tag}: tool_failures={failures} degraded={degraded}")
raise SystemExit(1 if bad else 0)
PY
  then
    say "   PASS"
  else
    FAILED=$((FAILED + 1))
  fi
fi

say ""
say "verdict: failed=${FAILED} missing_evidence=${MISSING}"
if [ "$FAILED" -gt 0 ]; then
  exit 1
fi
if [ "$MISSING" -gt 0 ]; then
  exit 2
fi
exit 0
