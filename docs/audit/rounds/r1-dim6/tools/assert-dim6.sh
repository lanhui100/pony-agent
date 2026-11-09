#!/usr/bin/env bash
# r1-dim6 断言集（维度 6：上报链路自检）——PA-169 C3 的 P0 人工等价节点。
#
# 用法：bash assert-dim6.sh <arm-dir> <expected-cwd> [metrics.json]
#   例：bash assert-dim6.sh docs/audit/rounds/r1-dim6/replay/base /tmp/audit-r1-baseline \
#         docs/audit/rounds/r1-dim6/metrics.json
#
# 退出码：0 = 全部断言通过；1 = 存在断言失败；2 = 证据缺失（UNKNOWN ≠ PASS）。
# 纪律：本脚本是 verdict 的唯一裁决者，禁止口头判 pass；阈值冻结后放宽须新提案。
set -uo pipefail

ARM_DIR="${1:?arm dir required}"
EXPECTED_CWD="${2:?expected worktree cwd required}"
METRICS="${3:-}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FAILED=0
MISSING=0

say() { printf '%s\n' "$*"; }

# ── A1 重放隔离：模型看到的 cwd 必须等于臂 worktree ──────────────────────────────
say "── A1 replay isolation (env cwd == arm worktree)"
if ! python3 "$HERE/analyze-replay-isolation.py" "$ARM_DIR" "$EXPECTED_CWD" > /tmp/a1.json 2>/dev/null; then
  python3 - <<'PY'
import json
d = json.load(open('/tmp/a1.json'))
print(f"   FAIL sessions={d['session_count']} mismatch={d['isolation_mismatch_count']} expected={d['expected_cwd']}")
for s in d['sessions']:
    if not s['isolated']:
        print(f"     - {s['tag']}: env_cwd={s['env_cwd']}")
PY
  FAILED=$((FAILED + 1))
else
  say "   PASS"
fi

# ── A2/A3 依赖 Langfuse 采集的 metrics.json ────────────────────────────────────
if [ -z "$METRICS" ] || [ ! -f "$METRICS" ]; then
  say "── A2/A3 skipped: metrics.json 未提供（UNKNOWN ≠ PASS，计为证据缺失）"
  MISSING=$((MISSING + 1))
else
  say "── A2 trace-level release 必须非空（按发布线筛选可用）"
  if python3 - "$METRICS" <<'PY'
import json
import sys

rows = json.load(open(sys.argv[1])).get('sessions') or []
if not rows:
    raise SystemExit(1)
missing = [r['tag'] for r in rows if not r.get('trace_release')]
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

rows = json.load(open(sys.argv[1])).get('sessions') or []
bad = []
for row in rows:
    attrs = row.get('root_attributes') or {}
    phase = attrs.get('agent.turn.phase')
    outcome = attrs.get('agent.turn.outcome')
    if phase == 'completed' and outcome == 'error':
        bad.append((row['tag'], attrs.get('agent.turn.tool_failures'), attrs.get('agent.turn.degraded')))
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
