#!/usr/bin/env bash
# scripts/audit/freeze-check.sh —— PA-169 C8：冻结检查（corpus/断言哈希漂移 + 写域越界）
#
# 契约：ADR 0101 §防作弊与安全——阈值、corpus_hash、断言哈希在批准时冻结；
# 提案写域之外的 harness diff 一律判红。
#
# 两类判据：
#   1. 冻结哈希：委托 `scripts/audit/state.py check-freeze`（corpus 聚合 + 断言脚本）；
#   2. 写域：`git diff --name-only <baseline> <candidate>` 的每个路径必须落在
#      loop-state.current.write_scope 前缀内，或属于审计自身产物白名单。
#
# 用法：bash scripts/audit/freeze-check.sh [--baseline <sha>] [--candidate <sha>]
#         [--state <loop-state.json>] [--round r1-dim6]
#
# 退出码：0 = 未漂移且无越界｜1 = 写域越界｜3 = 参数/环境不可用（缺提交）
#         4 = 冻结哈希漂移
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

BASELINE=""; CANDIDATE=""; ROUND="${ROUND:-r1-dim6}"; PATHS_FILE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --baseline) BASELINE="${2:?}"; shift 2 ;;
    --candidate) CANDIDATE="${2:?}"; shift 2 ;;
    --paths-file) PATHS_FILE="${2:?}"; shift 2 ;;
    --state) STATE_FILE="${2:?}"; shift 2 ;;
    --round) ROUND="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done

# round 快照：loop-state.current 只描述**当前**轮；旧轮收口后要复跑裁决，
# 必须读 `rounds/<round>/round-state.json`（收口时归档），否则会把新轮的
# 字段当旧轮用（r1-dim6 复跑实测：candidate_commit=null ⇒ scope 判 MISSING ⇒ 假 UNKNOWN）。
SNAPSHOT="$AUDIT_ROOT/rounds/$ROUND/round-state.json"
if [ -f "$SNAPSHOT" ]; then
  STATE_FILE="$SNAPSHOT"
  say "── state: 使用 round 快照 $SNAPSHOT"
fi

need_cmd python3 || exit 3
need_cmd git || exit 3
[ -f "$STATE_FILE" ] || { fail_msg "状态文件不存在：$STATE_FILE"; exit 3; }

# ── 1. 冻结哈希 ────────────────────────────────────────────────────────────────
say "── freeze: corpus/assertion hashes"
python3 "$HERE/state.py" --state "$STATE_FILE" --audit-root "$AUDIT_ROOT" --round "$ROUND" check-freeze
freeze_rc=$?
case "$freeze_rc" in
  0) say "   PASS freeze" ;;
  4) say "   FAIL 冻结漂移（阈值/corpus 被事后改动 → 须新提案重审）" ;;
  *) say "   FAIL freeze-check 自身失败 exit=${freeze_rc}" ;;
esac

# ── 2. 写域越界 ────────────────────────────────────────────────────────────────
# 两个基线**语义不同，不能混用**：
#   - `baseline_commit`：重放臂的固定提交（candidate 臂就是从它派生的），用于「前后对照」；
#   - `impl_base_commit`：本轮实施开始时的 HEAD，用于「写域 diff」。
# 若拿 baseline_commit 当写域基线，主线上介于两者之间的**无关提交**（别人的 ADR/模块拆分）
# 会被算进本轮 diff，把写域检查变成假红（r1-dim6 实测：38 个文件里 34 个与本轮无关）。
if [ -z "$BASELINE" ]; then
  BASELINE="$(python3 "$HERE/state.py" --state "$STATE_FILE" get --path current.impl_base_commit 2>/dev/null || true)"
fi
if [ -z "$BASELINE" ] || [ "$BASELINE" = "None" ]; then
  say "── scope: 缺 impl_base_commit → 回退 baseline_commit（可能含无关提交，结论偏严）"
  BASELINE="$(python3 "$HERE/state.py" --state "$STATE_FILE" get --path current.baseline_commit 2>/dev/null || true)"
fi
if [ -z "$CANDIDATE" ]; then
  CANDIDATE="$(python3 "$HERE/state.py" --state "$STATE_FILE" get --path current.candidate_commit 2>/dev/null || true)"
fi

scope_rc=0
if [ -n "$PATHS_FILE" ]; then
  say "── scope: 外部路径清单 $PATHS_FILE 必须落在提案写域内"
  python3 "$HERE/scope_check.py" --state "$STATE_FILE" --baseline "${BASELINE:-none}" \
    --paths-file "$PATHS_FILE"
  scope_rc=$?
  case "$scope_rc" in
    0) say "   PASS scope" ;;
    1) say "   FAIL 写域越界（提案写域外的 harness 改动）" ;;
    2) say "   MISSING scope 检查不可执行" ;;
  esac
elif [ -z "$BASELINE" ] || [ "$BASELINE" = "None" ]; then
  say "── scope: 缺 baseline_commit → 跳过（UNKNOWN ≠ PASS 由 verdict 统一裁决）"
  scope_rc=2
elif [ -z "$CANDIDATE" ] || [ "$CANDIDATE" = "None" ]; then
  say "── scope: 缺 candidate_commit（implement 尚未落提交）→ 跳过"
  scope_rc=2
else
  say "── scope: git diff $BASELINE..$CANDIDATE 必须落在提案写域内"
  python3 "$HERE/scope_check.py" --state "$STATE_FILE" --baseline "$BASELINE" --candidate "$CANDIDATE"
  scope_rc=$?
  case "$scope_rc" in
    0) say "   PASS scope" ;;
    1) say "   FAIL 写域越界（提案写域外的 harness 改动）" ;;
    2) say "   MISSING scope 检查不可执行" ;;
  esac
fi

say ""
say "freeze-check: freeze=${freeze_rc} scope=${scope_rc}"
if [ "$freeze_rc" -eq 4 ]; then
  exit 4
fi
if [ "$scope_rc" -eq 1 ]; then
  exit 1
fi
if [ "$freeze_rc" -ne 0 ] || [ "$scope_rc" -eq 2 ]; then
  exit 2
fi
exit 0
