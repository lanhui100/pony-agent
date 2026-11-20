#!/usr/bin/env bash
# artifacts-status.sh —— 构建产物体积与磁盘水位门禁（只读，不删任何东西）。
#
# 用途：把「产物涨到磁盘告警才发现」变成机器可判的门。
#   水位判定：仓库所在挂载点 `df -P` 的 Use% > ARTIFACTS_DISK_MAX_PCT（默认 70）⇒ exit 1。
#   预算判定（--budget）：单目录 KiB 超过 `.dev-team/artifacts-budget.json` 的 thresholds_kb，
#     或全部槽位 incremental 合计超过 incremental_total_kb ⇒ exit 1（只报不删）。
# 用法：
#   bash scripts/artifacts-status.sh            # 人读输出 + 水位门禁
#   bash scripts/artifacts-status.sh --json     # 机器可读（CI/仪表）
#   bash scripts/artifacts-status.sh --budget   # 额外做单目录预算判定
# 退出码：0 正常 / 1 超阈值或超预算 / 2 用法错误 / 3 预算文件缺失或非法（--budget 时 fail-closed）
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAX_PCT="${ARTIFACTS_DISK_MAX_PCT:-70}"
BUDGET_FILE="${ARTIFACTS_BUDGET_FILE:-$REPO_ROOT/.dev-team/artifacts-budget.json}"
JSON=0
CHECK_BUDGET=0
for arg in "$@"; do
  case "$arg" in
    --json) JSON=1 ;;
    --budget) CHECK_BUDGET=1 ;;
    -h|--help) sed -n '2,15p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "[artifacts-status] 未知参数：$arg" >&2; exit 2 ;;
  esac
done

# 已知产物目录（缺失记 0，不报错）：Rust 多槽位 + 前端构建产物。
# 不含 node_modules：它是前端**依赖**（npm install 的输入），既不是本仓构建产物，
# clean-targets.sh 也不负责清它；计入「产物合计」只会让数字虚高（审核 P2）。
DIRS=(target target-check target-test target-perf target-test-exact-a target-test-exact-b target-test-exact-c dist coverage)
total_kb=0
rows=()
for rel in "${DIRS[@]}"; do
  if [ -d "$REPO_ROOT/$rel" ]; then
    kb="$(du -sk "$REPO_ROOT/$rel" 2>/dev/null | awk '{print $1}')"
    kb="${kb:-0}"
    total_kb=$((total_kb + kb))
    rows+=("$rel $kb")
  fi
done

usage_line="$(df -P "$REPO_ROOT" 2>/dev/null | awk 'NR==2 {gsub("%","",$5); print $5" "$2" "$3}')"
used_pct="$(printf '%s' "$usage_line" | awk '{print $1}')"
size_kb="$(printf '%s' "$usage_line" | awk '{print $2}')"
used_kb="$(printf '%s' "$usage_line" | awk '{print $3}')"
used_pct="${used_pct:-0}"

# ── 预算判定 ────────────────────────────────────────────────────────────
violations=()
incremental_kb=0
incremental_limit=0
if [ "$CHECK_BUDGET" -eq 1 ]; then
  if [ ! -f "$BUDGET_FILE" ]; then
    echo "[artifacts-status] fail-closed：缺预算文件 $BUDGET_FILE（--budget 需显式预算）" >&2
    exit 3
  fi
  budget_rows="$(python3 - "$BUDGET_FILE" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1], encoding="utf-8"))
for name, limit in sorted(cfg.get("thresholds_kb", {}).items()):
    print(f"{name} {limit}")
print(f"__incremental_total__ {cfg.get('incremental_total_kb', 0)}")
PY
)" || { echo "[artifacts-status] fail-closed：预算文件解析失败（$BUDGET_FILE）" >&2; exit 3; }

  while read -r name limit; do
    [ -z "${name:-}" ] && continue
    if [ "$name" = "__incremental_total__" ]; then
      incremental_limit="$limit"
      continue
    fi
    actual_kb=0
    for row in "${rows[@]}"; do
      [ "${row% *}" = "$name" ] && actual_kb="${row##* }"
    done
    if [ "$actual_kb" -gt "$limit" ]; then
      violations+=("$name 实测 $(numfmt --to=iec --suffix=B $((actual_kb * 1024)) 2>/dev/null || echo "${actual_kb}K") > 预算 $(numfmt --to=iec --suffix=B $((limit * 1024)) 2>/dev/null || echo "${limit}K")")
    fi
  done <<< "$budget_rows"

  if [ "$incremental_limit" -gt 0 ]; then
    # 与 clean-targets.sh 的 `--incremental` 档口径一致：debug **和** release 的 incremental
    # 都算中间态。只统计 debug 会让 release 侧膨胀漏报（门禁假绿，审核 P1）。
    while IFS= read -r slot; do
      for profile in debug release; do
        rel="$slot/$profile/incremental"
        [ -d "$REPO_ROOT/$rel" ] || continue
        kb="$(du -sk "$REPO_ROOT/$rel" 2>/dev/null | awk '{print $1}')"
        incremental_kb=$((incremental_kb + ${kb:-0}))
      done
    done < <(cd "$REPO_ROOT" && find . -maxdepth 1 -type d -name 'target*' 2>/dev/null | sed 's|^\./||')
    if [ "$incremental_kb" -gt "$incremental_limit" ]; then
      violations+=("incremental 合计 $(numfmt --to=iec --suffix=B $((incremental_kb * 1024)) 2>/dev/null || echo "${incremental_kb}K") > 预算 $(numfmt --to=iec --suffix=B $((incremental_limit * 1024)) 2>/dev/null || echo "${incremental_limit}K")")
    fi
  fi
fi

if [ "$JSON" -eq 1 ]; then
  dirs_json="{}"
  if [ "${#rows[@]}" -gt 0 ]; then
    dirs_json="{$(printf '%s\n' "${rows[@]}" | awk '{printf "%s\"%s\":%s", (NR>1?",":""), $1, $2}')}"
  fi
  violations_json="[$(printf '%s\n' "${violations[@]}" | awk 'NF {gsub(/"/,"\\\""); printf "%s\"%s\"", (NR>1?",":""), $0}')]"
  printf '{"artifacts_kb":%s,"disk_used_pct":%s,"disk_size_kb":%s,"disk_used_kb":%s,"threshold_pct":%s,"incremental_kb":%s,"violations":%s,"dirs":%s}\n' \
    "$total_kb" "$used_pct" "${size_kb:-0}" "${used_kb:-0}" "$MAX_PCT" "$incremental_kb" "$violations_json" "$dirs_json"
else
  echo "[artifacts-status] 仓库：$REPO_ROOT"
  for row in "${rows[@]}"; do
    rel="${row% *}"; kb="${row##* }"
    printf '  %-28s %8s\n' "$rel" "$(numfmt --to=iec --suffix=B $((kb * 1024)) 2>/dev/null || echo "${kb}K")"
  done
  printf '  %-28s %8s\n' "产物合计" "$(numfmt --to=iec --suffix=B $((total_kb * 1024)) 2>/dev/null || echo "${total_kb}K")"
  printf '  %-28s %8s%%（阈值 %s%%）\n' "磁盘占用" "$used_pct" "$MAX_PCT"
  if [ "$CHECK_BUDGET" -eq 1 ]; then
    printf '  %-28s %8s\n' "incremental 合计" "$(numfmt --to=iec --suffix=B $((incremental_kb * 1024)) 2>/dev/null || echo "${incremental_kb}K")"
  fi
fi

rc=0
if [ "$used_pct" -gt "$MAX_PCT" ]; then
  echo "[artifacts-status] FAIL：磁盘占用 ${used_pct}% > 阈值 ${MAX_PCT}% —— 先跑 bash scripts/clean-targets.sh --dry-run 评估，再按 AGENTS.md 深度档清理" >&2
  rc=1
fi
if [ "${#violations[@]}" -gt 0 ]; then
  echo "[artifacts-status] FAIL：产物超预算 ${#violations[@]} 项" >&2
  for v in "${violations[@]}"; do
    echo "  - $v" >&2
  done
  if [ "$incremental_kb" -gt 0 ]; then
    echo "  处置：bash scripts/clean-targets.sh --incremental（只清增量中间态，保留依赖产物）" >&2
  else
    echo "  处置：已无增量中间态可清，超预算来自陈旧依赖产物；降到健康值只能 bash scripts/clean-targets.sh --deep（冷编 20min+，需计划窗口）" >&2
  fi
  rc=1
fi
exit "$rc"
