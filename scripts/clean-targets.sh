#!/usr/bin/env bash
# clean-targets.sh —— 本仓构建产物的分层清理（Linux / CI 通用；与 clean-targets.ps1 语义对齐）。
#
# 深度分层（见 AGENTS.md「构建产物保留深度」）：
#   light（默认）  只删**可再生产物**：target-check* / target-test* / target-perf / target-codex-*
#                  —— 根 target/ 与 target/release 保持不动（热缓存，删了要付冷编 20min+）。
#   --with-frontend 额外删 dist/ coverage/ playwright-report/ test-results/（前端可重建产物）。
#   --deep         额外删根 target/（只在工具链升级 / link 失败 / 依赖大换血 / 磁盘告警时用）。
#   --dry-run      只列大小与将删清单，不动磁盘。
#
# 安全边界：只删「解析后仍在仓库根内」的路径；仓库外/符号链接目标一律拒绝。
# 退出码：0 正常 / 1 有目录被运行中进程占用而跳过 / 2 用法错误 / 3 路径安全拒绝。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DRY_RUN=0
DEEP=0
WITH_FRONTEND=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --deep) DEEP=1; shift ;;
    --with-frontend) WITH_FRONTEND=1; shift ;;
    -h|--help) sed -n '2,16p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "[clean-targets] 未知参数：$1" >&2; exit 2 ;;
  esac
done

# 候选目录：固定槽位 + 语义后缀槽位（与 .gitignore 的 target-* 口径一致）。
CANDIDATES=()
while IFS= read -r path; do
  CANDIDATES+=("$path")
done < <(
  cd "$REPO_ROOT" || exit 2
  for name in target-check target-check-tests target-test target-perf; do
    [ -d "$name" ] && printf '%s\n' "$name"
  done
  find . -maxdepth 1 -type d \( -name 'target-check-*' -o -name 'target-test-*' -o -name 'target-test-exact*' -o -name 'target-codex-*' \) 2>/dev/null \
    | sed 's|^\./||' | sort
)
if [ "$WITH_FRONTEND" -eq 1 ]; then
  for name in dist coverage playwright-report test-results; do
    [ -d "$REPO_ROOT/$name" ] && CANDIDATES+=("$name")
  done
fi
if [ "$DEEP" -eq 1 ] && [ -d "$REPO_ROOT/target" ]; then
  CANDIDATES+=("target")
fi

if [ "${#CANDIDATES[@]}" -eq 0 ]; then
  echo "[clean-targets] 没有可清理的产物目录（light 深度保留根 target/）"
  exit 0
fi

# 安全闸：解析后必须仍在仓库根内（拒绝 ../ 逃逸与仓外符号链接目标）。
ROOT_REAL="$(realpath -e "$REPO_ROOT")"
safe_paths=()
for rel in "${CANDIDATES[@]}"; do
  abs="$REPO_ROOT/$rel"
  real="$(realpath -m "$abs")"
  case "$real" in
    "$ROOT_REAL"/*) safe_paths+=("$rel") ;;
    *) echo "[clean-targets] 拒绝：$rel 解析到仓库外（$real）" >&2; exit 3 ;;
  esac
done

echo "[clean-targets] 仓库：$REPO_ROOT"
echo "[clean-targets] 深度：$([ "$DEEP" -eq 1 ] && echo deep || echo light)$([ "$WITH_FRONTEND" -eq 1 ] && echo ' + frontend')"
total_kb=0
for rel in "${safe_paths[@]}"; do
  kb="$(du -sk "$REPO_ROOT/$rel" 2>/dev/null | awk '{print $1}')"
  kb="${kb:-0}"
  total_kb=$((total_kb + kb))
  printf '  %-28s %8s\n' "$rel" "$(numfmt --to=iec --suffix=B $((kb * 1024)) 2>/dev/null || echo "${kb}K")"
done
printf '  %-28s %8s\n' "合计" "$(numfmt --to=iec --suffix=B $((total_kb * 1024)) 2>/dev/null || echo "${total_kb}K")"

if [ "$DRY_RUN" -eq 1 ]; then
  echo "[clean-targets] --dry-run：未删除任何内容"
  exit 0
fi

if pgrep -af 'cargo (build|test|check|run|clippy)' >/dev/null 2>&1; then
  echo "[clean-targets] 检测到运行中的 cargo 进程（只读观察，不杀）：本实现仍按目录逐个尝试，被占用者跳过" >&2
fi

skipped=0
for rel in "${safe_paths[@]}"; do
  if rm -rf -- "$REPO_ROOT/$rel" 2>/dev/null && [ ! -e "$REPO_ROOT/$rel" ]; then
    echo "[clean-targets] 已删除：$rel"
  else
    echo "[clean-targets] [SKIP] 无法删除（可能被进程占用）：$rel" >&2
    skipped=$((skipped + 1))
  fi
done

if [ "$skipped" -ne 0 ]; then
  exit 1
fi
echo "[clean-targets] 完成（保留根 target/ 热缓存$([ "$DEEP" -eq 1 ] && echo '——本轮为 deep，已删）' || echo '）')"
