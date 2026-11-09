#!/usr/bin/env bash
# scripts/audit/verdict.sh —— PA-169 C6：唯一裁决者（assert + ratchet + 前后对照）
#
# 契约：ADR 0101 §确定性节点 verdict。**禁止口头判 pass**：本脚本的退出码就是结论。
#   pass 时把本维度断言写入棘轮集（docs/audit/ratchet.json，只增不减）；
#   fail 时不动棘轮，由 loop 决定重试（attempt<3）或 escalated。
#
# 用法：
#   bash scripts/audit/verdict.sh --round r1-dim6 --dim 6 \
#        --arm-dir <candidate replay arm> --expected-cwd <candidate worktree> \
#        [--metrics <metrics.json>] [--state <loop-state.json>] [--no-ratchet]
#
# 退出码：0 = pass（已入棘轮）｜1 = fail（断言或棘轮回归）
#         2 = 证据缺失（UNKNOWN ≠ PASS）｜4 = 冻结漂移/篡改
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

ROUND="${ROUND:-r1-dim6}"; DIM=""; ARM_DIR=""; EXPECTED_CWD=""; METRICS=""; DO_RATCHET=1
while [ $# -gt 0 ]; do
  case "$1" in
    --round) ROUND="${2:?}"; shift 2 ;;
    --dim) DIM="${2:?}"; shift 2 ;;
    --arm-dir) ARM_DIR="${2:?}"; shift 2 ;;
    --expected-cwd) EXPECTED_CWD="${2:?}"; shift 2 ;;
    --metrics) METRICS="${2:?}"; shift 2 ;;
    --state) STATE_FILE="${2:?}"; shift 2 ;;
    --audit-root) AUDIT_ROOT="${2:?}"; shift 2 ;;
    --no-ratchet) DO_RATCHET=0; shift ;;
    -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done
# 子节点（freeze-check / ratchet / assert）都以 AUDIT_ROOT 定位 corpus 与断言目录，
# 这里显式 export 保证「同一轮所有节点看同一份真值」。
export AUDIT_ROOT STATE_FILE
# lib.sh 在参数解析前就已算出 ROUNDS_DIR；--audit-root 覆盖后必须重算，
# 否则回执会落到仓库真值目录（夹具自测曾因此写入假回执）。
if [ -z "${AUDIT_ROUNDS_DIR:-}" ]; then
  ROUNDS_DIR="$AUDIT_ROOT/rounds"
fi
[ -n "$DIM" ] || { fail_msg "--dim 必填"; exit 2; }
[ -n "$ARM_DIR" ] || { fail_msg "--arm-dir 必填（candidate 臂）"; exit 2; }
[ -n "$EXPECTED_CWD" ] || { fail_msg "--expected-cwd 必填"; exit 2; }

ASSERT_SCRIPT="$HERE/assertions/dim${DIM}.sh"
[ -f "$ASSERT_SCRIPT" ] || { fail_msg "维度 ${DIM} 断言脚本缺失：$ASSERT_SCRIPT"; exit 2; }

RECEIPTS="$ROUNDS_DIR/$ROUND/receipts"
mkdir -p "$RECEIPTS"

# ── 0. 冻结检查（漂移优先于一切结论：被篡改的阈值不允许产生 pass） ────────────────
say "== step 0: freeze-check"
bash "$HERE/freeze-check.sh" --round "$ROUND" --state "$STATE_FILE"
freeze_rc=$?
if [ "$freeze_rc" -eq 4 ]; then
  say "verdict: FAIL (freeze drift) exit=4"
  write_receipt "$RECEIPTS" "verdict-dim${DIM}" verdict script 4 "$RECEIPTS" \
    "冻结漂移：阈值/corpus/断言哈希被事后改动，须新提案重审" \
    "bash scripts/audit/freeze-check.sh --round $ROUND" >/dev/null
  exit 4
fi
if [ "$freeze_rc" -eq 1 ]; then
  say "verdict: FAIL (write-scope violation) exit=1"
  write_receipt "$RECEIPTS" "verdict-dim${DIM}" verdict script 1 "$RECEIPTS" \
    "写域越界：diff 命中提案写域之外的 harness 路径" \
    "bash scripts/audit/freeze-check.sh --round $ROUND" >/dev/null
  exit 1
fi
if [ "$freeze_rc" -ne 0 ]; then
  # 冻结/写域检查自身不可执行（缺 baseline/candidate、缺字段）⇒ UNKNOWN，绝不判 pass。
  say "verdict: UNKNOWN (freeze-check unavailable exit=${freeze_rc}) exit=2"
  write_receipt "$RECEIPTS" "verdict-dim${DIM}" verdict script 2 "$RECEIPTS" \
    "冻结/写域检查不可执行（exit=${freeze_rc}）：缺 baseline/candidate 或未声明 write_scope" \
    "bash scripts/audit/freeze-check.sh --round $ROUND" >/dev/null
  exit 2
fi

# ── 1. 本维度断言 ──────────────────────────────────────────────────────────────
say "== step 1: assert dim${DIM}"
# 臂标签（r2-dim1 attempt 2）：metrics 的 arm 字段是 replay 的 --arm（base|candidate），
# 而臂目录名带 attempt 后缀（base-a2 / candidate-clean）。断言脚本按 arm 逐臂过滤，
# 标签错 = 静默命中 0 行。这里统一从 metrics 反查并**显式**传下去，不靠各脚本自己猜。
ARM_TAG=""
if [ -n "$METRICS" ] && [ -f "$METRICS" ]; then
  ARM_TAG="$(resolve_arm_tag "$METRICS" "$ARM_DIR" 2>/dev/null || true)"
fi
assert_args=(--arm-dir "$ARM_DIR" --expected-cwd "$EXPECTED_CWD")
[ -n "$METRICS" ] && assert_args+=(--metrics "$METRICS")
[ -n "$ARM_TAG" ] && assert_args+=(--arm-tag "$ARM_TAG")
bash "$ASSERT_SCRIPT" "${assert_args[@]}"
assert_rc=$?

# ── 2. 棘轮全集（不得因本维度通过而放过历史维度回归） ────────────────────────────
ratchet_rc=0
if [ "$DO_RATCHET" -eq 1 ]; then
  say "== step 2: ratchet (all passed dimensions)"
  ratchet_args=(--arm-dir "$ARM_DIR" --expected-cwd "$EXPECTED_CWD")
  [ -n "$METRICS" ] && ratchet_args+=(--metrics "$METRICS")
  [ -n "$ARM_TAG" ] && ratchet_args+=(--arm-tag "$ARM_TAG")
  bash "$HERE/ratchet.sh" "${ratchet_args[@]}"
  ratchet_rc=$?
else
  say "== step 2: ratchet skipped (--no-ratchet)"
fi

say ""
say "verdict: assert=${assert_rc} ratchet=${ratchet_rc} freeze=${freeze_rc}"

if [ "$assert_rc" -eq 0 ] && [ "$ratchet_rc" -eq 0 ]; then
  if [ "$DO_RATCHET" -eq 1 ]; then
    python3 "$HERE/ratchet_add.py" --dim "$DIM" --round "$ROUND" \
      --script "scripts/audit/assertions/dim${DIM}.sh" \
      --ratchet "${AUDIT_RATCHET_FILE:-$AUDIT_ROOT/ratchet.json}" || exit 3
  fi
  write_receipt "$RECEIPTS" "verdict-dim${DIM}" verdict script 0 "$RECEIPTS" \
    "PASS：assert 与 ratchet 全绿，维度 ${DIM} 已入棘轮" \
    "bash scripts/audit/verdict.sh --dim $DIM --arm-dir $ARM_DIR" >/dev/null
  say "verdict: PASS (dim${DIM} ratcheted)"
  exit 0
fi

if [ "$assert_rc" -eq 2 ] || [ "$ratchet_rc" -eq 2 ]; then
  write_receipt "$RECEIPTS" "verdict-dim${DIM}" verdict script 2 "$RECEIPTS" \
    "UNKNOWN：证据缺失（缺 metrics/arm），不得判 pass" \
    "bash scripts/audit/verdict.sh --dim $DIM" >/dev/null
  say "verdict: UNKNOWN (evidence missing) exit=2"
  exit 2
fi

write_receipt "$RECEIPTS" "verdict-dim${DIM}" verdict script 1 "$RECEIPTS" \
  "FAIL：assert=${assert_rc} ratchet=${ratchet_rc}" \
  "bash scripts/audit/verdict.sh --dim $DIM --arm-dir $ARM_DIR" >/dev/null
say "verdict: FAIL exit=1"
exit 1
