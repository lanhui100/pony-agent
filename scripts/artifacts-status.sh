#!/usr/bin/env bash
# artifacts-status.sh —— 构建产物体积与磁盘水位门禁（只读，不删任何东西）。
#
# 用途：把「产物涨到磁盘告警才发现」变成机器可判的门：磁盘占用超过阈值即非零退出。
# 判定：对仓库所在挂载点取 `df -P` 的 Use%，> ARTIFACTS_DISK_MAX_PCT（默认 70）⇒ exit 1。
# 用法：
#   bash scripts/artifacts-status.sh            # 人读输出 + 水位门禁
#   bash scripts/artifacts-status.sh --json     # 机器可读（CI/仪表）
# 退出码：0 水位正常 / 1 超阈值 / 2 用法错误
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAX_PCT="${ARTIFACTS_DISK_MAX_PCT:-70}"
JSON=0
case "${1:-}" in
  --json) JSON=1 ;;
  -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}"; exit 0 ;;
  "") ;;
  *) echo "[artifacts-status] 未知参数：$1" >&2; exit 2 ;;
esac

# 已知产物目录（缺失记 0，不报错）：Rust 多槽位 + 前端 + 依赖。
DIRS=(target target-check target-test target-perf target-test-exact-a target-test-exact-b target-test-exact-c dist coverage node_modules)
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

if [ "$JSON" -eq 1 ]; then
  dirs_json="{}"
  if [ "${#rows[@]}" -gt 0 ]; then
    dirs_json="{$(printf '%s\n' "${rows[@]}" | awk '{printf "%s\"%s\":%s", (NR>1?",":""), $1, $2}')}"
  fi
  printf '{"artifacts_kb":%s,"disk_used_pct":%s,"disk_size_kb":%s,"disk_used_kb":%s,"threshold_pct":%s,"dirs":%s}\n' \
    "$total_kb" "$used_pct" "${size_kb:-0}" "${used_kb:-0}" "$MAX_PCT" "$dirs_json"
else
  echo "[artifacts-status] 仓库：$REPO_ROOT"
  for row in "${rows[@]}"; do
    rel="${row% *}"; kb="${row##* }"
    printf '  %-28s %8s\n' "$rel" "$(numfmt --to=iec --suffix=B $((kb * 1024)) 2>/dev/null || echo "${kb}K")"
  done
  printf '  %-28s %8s\n' "产物合计" "$(numfmt --to=iec --suffix=B $((total_kb * 1024)) 2>/dev/null || echo "${total_kb}K")"
  printf '  %-28s %8s%%（阈值 %s%%）\n' "磁盘占用" "$used_pct" "$MAX_PCT"
fi

if [ "$used_pct" -gt "$MAX_PCT" ]; then
  echo "[artifacts-status] FAIL：磁盘占用 ${used_pct}% > 阈值 ${MAX_PCT}% —— 先跑 bash scripts/clean-targets.sh --dry-run 评估，再按 AGENTS.md 深度档清理" >&2
  exit 1
fi
exit 0
