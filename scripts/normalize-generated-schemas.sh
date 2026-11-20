#!/usr/bin/env bash
# normalize-generated-schemas.sh —— `src-tauri/gen/schemas/*.json` 文本约定补齐（POSIX 版）。
#
# 背景（与 normalize-generated-schemas.ps1 同一决策，PA-102 §2）：`tauri dev` / `tauri build`
# 每次重新生成这些权限契约文件且**不写结尾换行**，于是工作树恒定出现 4 个空 diff 的「已修改」。
# 本脚本只补文本约定（LF + 恰好一个结尾换行），不改 JSON 语义。
#
# 用法：
#   bash scripts/normalize-generated-schemas.sh            # 规范化一次
#   bash scripts/normalize-generated-schemas.sh --watch    # 轮询监听，Ctrl+C 结束
#   bash scripts/normalize-generated-schemas.sh --quiet    # 无变更时不输出
# 退出码：0 正常 / 2 用法错误
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCHEMA_DIR="$REPO_ROOT/src-tauri/gen/schemas"

WATCH=0
QUIET=0
while [ $# -gt 0 ]; do
  case "$1" in
    --watch) WATCH=1; shift ;;
    --quiet|-q) QUIET=1; shift ;;
    -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "[schemas] 未知参数：$1" >&2; exit 2 ;;
  esac
done

normalize_once() {
  [ -d "$SCHEMA_DIR" ] || return 0
  local changed=0 file
  while IFS= read -r -d '' file; do
    local before after tmp
    before="$(sha256sum "$file" | awk '{print $1}')"
    tmp="$(mktemp)"
    # CRLF → LF，并保证结尾恰好一个换行（去掉尾部所有换行后补一个）。
    sed -e 's/\r$//' "$file" | awk 'BEGIN{RS="\0"} {gsub(/\n+$/,""); printf "%s\n", $0}' > "$tmp"
    # 内容一致就不落盘：无条件 mv 会刷新 mtime，而 --watch（1s 轮询）+ Vite/Tauri 文件监视器
    # 会把「永远在变」的文件当成变更源，形成无限重载（审核 P1）。无变更只删临时文件。
    if cmp -s "$tmp" "$file"; then
      rm -f "$tmp"
      continue
    fi
    mv "$tmp" "$file"
    after="$(sha256sum "$file" | awk '{print $1}')"
    if [ "$before" != "$after" ]; then
      changed=$((changed + 1))
      [ "$QUIET" -eq 0 ] && echo "[schemas] 规范化：${file#"$REPO_ROOT"/}"
    fi
  done < <(find "$SCHEMA_DIR" -maxdepth 1 -name '*.json' -print0 2>/dev/null)
  [ "$QUIET" -eq 0 ] && echo "[schemas] 完成（变更 $changed 个）"
  return 0
}

if [ "$WATCH" -eq 0 ]; then
  normalize_once
  exit 0
fi

echo "[schemas] 监听 $SCHEMA_DIR（1s 轮询，Ctrl+C 结束）"
trap 'echo "[schemas] 结束"; exit 0' INT TERM
while :; do
  sleep 1
  normalize_once >/dev/null 2>&1 || true
done
