#!/usr/bin/env bash
# check-ps1-portability.sh —— `scripts/*.ps1` 的「通用适配」门禁（只读，不执行 PowerShell）。
#
# 判据（每个 .ps1 必须恰好落在下面某一类，否则红）：
#   1. 已登记 POSIX 落点   ：dispatcher 的 `$PonyPosixTwins` 里有该基名，且 ps1 里真的调用了转派
#                            （`. "$PSScriptRoot/lib/posix-dispatch.ps1"` + `Invoke-PonyPosixTwin -Name '<base>'`），
#                            且映射的落点文件真实存在、扩展名与 runner 口径一致（.sh → bash / .mjs → node）。
#   2. Windows-only 白名单 ：ps1 里显式写 `-WindowsOnly`（非 Windows 主机 fail-closed exit 4，不静默）。
#   3. 自身已适配          ：ps1 内部已按 node/bash 真源判定（不依赖 Windows API），需命中登记的真源文件名。
#
# 为什么需要它：.ps1 是历史 Windows 入口，本仓在 Linux 上开发。没有这条门，「新增一个 ps1」
# 或「改了 dispatcher 映射表但漏改调用点」都会静默漂移，只有在别人的平台上才炸。
#
# 用法：
#   bash scripts/check-ps1-portability.sh          # 人读
#   bash scripts/check-ps1-portability.sh --list   # 只打印分类结果
# 退出码：0 全部登记且一致 / 1 有违规 / 2 用法错误
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPTS_DIR="$REPO_ROOT/scripts"
DISPATCHER="$SCRIPTS_DIR/lib/posix-dispatch.ps1"

LIST_ONLY=0
case "${1:-}" in
  --list) LIST_ONLY=1 ;;
  -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}"; exit 0 ;;
  "") ;;
  *) echo "[ps1-portability] 未知参数：$1" >&2; exit 2 ;;
esac

# 白名单：改这里必须同时在 ADR 里说明理由（新增 Windows-only 脚本属技术债，不是默认选项）。
WINDOWS_ONLY="bump-version"
SELF_ADAPTING="verify-decisions"

FAIL=0
fail() { echo "[ps1-portability] FAIL $*" >&2; FAIL=1; }
ok() { [ "$LIST_ONLY" -eq 1 ] && echo "[ps1-portability] OK   $*" >/dev/null; }

if [ ! -f "$DISPATCHER" ]; then
  echo "[ps1-portability] fail-closed：缺少 $DISPATCHER（转派真源不存在，无法判定）" >&2
  exit 1
fi

# 从 dispatcher 解析映射表：'<base>' = '<twin>'
map_pairs="$(grep -E "^[[:space:]]*'[^']+'[[:space:]]*=[[:space:]]*'[^']+'" "$DISPATCHER" | sed -E "s/^[[:space:]]*'([^']+)'[[:space:]]*=[[:space:]]*'([^']+)'.*/\1 \2/")"

in_list() { case " $1 " in *" $2 "*) return 0 ;; *) return 1 ;; esac; }

# 契约匹配只认「非注释行」：本仓注释惯于引用真实语句片段，若不过滤，把转派语句
# 注释掉也会静默变绿（2026-11-20 评审指出）。
strip_comments() { grep -v -E '^[[:space:]]*#' "$1"; }

if [ -z "${map_pairs//[[:space:]]/}" ]; then
  fail "无法从 $DISPATCHER 解析映射表（格式漂移？反向检查与落点校验已失去意义）"
fi

# 反向检查：映射表里的落点文件必须存在，否则「登记了但落空」
while read -r base twin; do
  [ -n "${base:-}" ] || continue
  if [ ! -f "$SCRIPTS_DIR/$twin" ]; then
    fail "映射表登记 $base → $twin，但 $SCRIPTS_DIR/$twin 不存在"
  fi
  case "$twin" in
    *.sh|*.mjs) ;;
    *) fail "映射表 $base → $twin：落点只接受 .sh / .mjs（runner 才能确定）" ;;
  esac
done <<< "$map_pairs"

# 正向检查：每个 ps1 必须落进三类之一
count=0
for ps1 in "$SCRIPTS_DIR"/*.ps1; do
  [ -e "$ps1" ] || continue
  base="$(basename "$ps1" .ps1)"
  count=$((count + 1))
  twin="$(printf '%s\n' "$map_pairs" | awk -v b="$base" '$1 == b {print $2; exit}')"

  if [ -n "$twin" ]; then
    if ! strip_comments "$ps1" | grep -qF '. "$PSScriptRoot/lib/posix-dispatch.ps1"'; then
      fail "$base.ps1 已登记 POSIX 落点（$twin），但缺少 dispatcher dot-source"
    fi
    if ! strip_comments "$ps1" | grep -qF "Invoke-PonyPosixTwin -Name '$base'"; then
      fail "$base.ps1 已登记 POSIX 落点（$twin），但未调用 Invoke-PonyPosixTwin -Name '$base'"
    fi
    [ "$LIST_ONLY" -eq 1 ] && echo "[ps1-portability] OK   $base.ps1 → $twin"
  elif in_list "$WINDOWS_ONLY" "$base"; then
    if ! strip_comments "$ps1" | grep -qF 'Invoke-PonyPosixTwin -Name '"'$base'" || ! strip_comments "$ps1" | grep -qF -- '-WindowsOnly'; then
      fail "$base.ps1 属 Windows-only 白名单，但未显式调用 -WindowsOnly 转派（非 Windows 主机必须 fail-closed）"
    fi
    [ "$LIST_ONLY" -eq 1 ] && echo "[ps1-portability] OK   $base.ps1 → Windows-only（已登记缺口）"
  elif in_list "$SELF_ADAPTING" "$base"; then
    if ! strip_comments "$ps1" | grep -qE 'check-decisions\.mjs|verify-note\.sh'; then
      fail "$base.ps1 属「自身已适配」，但未命中登记的真源（check-decisions.mjs / verify-note.sh）"
    fi
    [ "$LIST_ONLY" -eq 1 ] && echo "[ps1-portability] OK   $base.ps1 → 自身已适配（node/bash 真源）"
  else
    fail "$base.ps1 未登记：请在 scripts/lib/posix-dispatch.ps1 加 POSIX 落点映射，或显式走 -WindowsOnly 并登记白名单"
  fi
done

if [ "$count" -eq 0 ]; then
  echo "[ps1-portability] 未发现任何 scripts/*.ps1（目录不对？）" >&2
  exit 1
fi

if [ "$FAIL" -ne 0 ]; then
  echo "[ps1-portability] FAILED（ps1 与 POSIX 落点契约不一致）" >&2
  exit 1
fi
echo "[ps1-portability] PASS（$count 个 .ps1 全部登记：$(printf '%s\n' "$map_pairs" | wc -l | tr -d ' ') 个有 POSIX 落点）"
exit 0
