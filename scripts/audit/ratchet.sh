#!/usr/bin/env bash
# scripts/audit/ratchet.sh —— PA-169 C4：已通过维度断言全集（单调增长，ADR 0090 纪律）
#
# 契约：已 pass 的维度其断言永久留在棘轮集；后续轮次任一红 = 阻断。
# 棘轮集真值源：`docs/audit/ratchet.json`（只增不减；条目只能由 verdict.sh pass 后写入）。
#
# 用法：
#   bash scripts/audit/ratchet.sh [--ratchet <ratchet.json>] [--arm-dir <dir>]
#        [--expected-cwd <wt>] [--metrics <metrics.json>] [--only dim6]
#
# 退出码：0 = 棘轮全集通过｜1 = 有维度回归（红）｜2 = 证据缺失（缺 arm/metrics）
#         3 = 棘轮文件损坏或引用的断言脚本不存在
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

RATCHET_FILE="${AUDIT_RATCHET_FILE:-$AUDIT_ROOT/ratchet.json}"
ARM_DIR=""; EXPECTED_CWD=""; METRICS=""; ONLY=""; ARM_TAG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ratchet) RATCHET_FILE="${2:?}"; shift 2 ;;
    --arm-dir) ARM_DIR="${2:?}"; shift 2 ;;
    --expected-cwd) EXPECTED_CWD="${2:?}"; shift 2 ;;
    --metrics) METRICS="${2:?}"; shift 2 ;;
    --arm-tag) ARM_TAG="${2:?}"; shift 2 ;;
    --only) ONLY="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done

need_cmd python3 || exit 3
# 首轮尚无维度通过时棘轮文件可以不存在——那等价于空集，不是证据缺失。
if [ ! -f "$RATCHET_FILE" ]; then
  say "棘轮文件不存在（$RATCHET_FILE）→ 视为空集"
  say "ratchet: regression=0 missing=0"
  exit 0
fi
python3 -c "import json,sys;json.load(open(sys.argv[1]))" "$RATCHET_FILE" 2>/dev/null \
  || { fail_msg "棘轮文件不可解析：$RATCHET_FILE"; exit 3; }

entries="$(python3 - "$RATCHET_FILE" "$ONLY" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
only = sys.argv[2]
for entry in data.get("entries") or []:
    if only and entry.get("dimension") != only:
        continue
    print(f"{entry['dimension']}\t{entry['script']}")
PY
)"

if [ -z "$entries" ]; then
  say "棘轮集为空（尚无维度通过）→ 无事可做"
  exit 0
fi

failed=0
missing=0
while IFS=$'\t' read -r dim script; do
  [ -n "$dim" ] || continue
  target="$REPO_ROOT/$script"
  if [ ! -f "$target" ]; then
    fail_msg "棘轮条目引用的断言脚本不存在：$script（dim=$dim）"
    exit 3
  fi
  say "── ratchet dim${dim} → $script"
  args=(--arm-dir "$ARM_DIR" --expected-cwd "$EXPECTED_CWD")
  [ -n "$METRICS" ] && args+=(--metrics "$METRICS")
  # 臂标签必须传下去：断言脚本按 arm 逐臂过滤，缺标签会静默命中 0 行（r2-dim1 attempt 2）。
  [ -n "$ARM_TAG" ] && args+=(--arm-tag "$ARM_TAG")
  bash "$target" "${args[@]}"
  rc=$?
  case "$rc" in
    0) say "   PASS dim${dim}" ;;
    2) say "   MISSING dim${dim}（证据缺失 ≠ PASS）"; missing=$((missing + 1)) ;;
    *) say "   REGRESSION dim${dim} exit=${rc}"; failed=$((failed + 1)) ;;
  esac
done <<< "$entries"

say ""
say "ratchet: regression=${failed} missing=${missing}"
if [ "$failed" -gt 0 ]; then
  exit 1
fi
if [ "$missing" -gt 0 ]; then
  exit 2
fi
exit 0
