#!/usr/bin/env bash
# cross-target-status.sh —— 交叉编译可行性体检（只读，不装任何东西）。
#
# 为什么需要它：本仓产物有**三种平台边界完全不同**的目标，混在一起谈「交叉编译」只会得出错结论：
#   1. 核心二进制 pony / ponycli（crates/pony-agent-core）—— 纯 Rust + 少量平台 FFI，
#      在 Linux 上装 mingw-w64 + `rustup target add x86_64-pc-windows-gnu` 即可产出 Windows 版；
#   2. Tauri 桌面端（src-tauri）—— 依赖 WebView2 / MSVC 运行库 / 窗口特效，
#      在 Linux 上**不能**直接交叉到 Windows（要 Windows SDK，通常用 cargo-xwin 或直接上 Windows 机）；
#   3. macOS —— 需要 Apple 工具链，Linux 上不可行。
# 判定：只报事实，exit 0；需要门禁时 `--require <目标三元组>[,<三元组>]`（缺失即 exit 1）。
# 用法：
#   bash scripts/cross-target-status.sh
#   bash scripts/cross-target-status.sh --json
#   bash scripts/cross-target-status.sh --require x86_64-pc-windows-gnu
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JSON=0
REQUIRE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --json) JSON=1; shift ;;
    --require) REQUIRE="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,16p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "[cross-target] 未知参数：$1" >&2; exit 2 ;;
  esac
done

have() { command -v "$1" >/dev/null 2>&1; }

HOST="$(rustc -vV 2>/dev/null | awk '/^host:/ {print $2}')"
SYSROOT="$(rustc --print sysroot 2>/dev/null || echo "")"
TARGETS=""
if [ -n "$SYSROOT" ] && [ -d "$SYSROOT/lib/rustlib" ]; then
  TARGETS="$(ls "$SYSROOT/lib/rustlib" 2>/dev/null | grep -vE '^(src|etc|bin|lib|share|manifest-|components|cache|rust\.txt)$' | tr '\n' ' ' | sed 's/ *$//')"
fi
RUSTUP="$(command -v rustup 2>/dev/null || echo MISSING)"
MINGW="$(command -v x86_64-w64-mingw32-gcc 2>/dev/null || echo MISSING)"
XWIN="$(command -v cargo-xwin 2>/dev/null || echo MISSING)"
ZIG="$(command -v zig 2>/dev/null || echo MISSING)"

has_target() { printf ' %s ' "$TARGETS" | grep -q " $1 "; }

if [ "$JSON" -eq 1 ]; then
  printf '{"host":"%s","installed_targets":"%s","rustup":"%s","mingw_gcc":"%s","cargo_xwin":"%s","zig":"%s","core_windows_gnu_ready":%s}\n' \
    "${HOST:-unknown}" "$TARGETS" "$RUSTUP" "$MINGW" "$XWIN" "$ZIG" \
    "$( { [ "$MINGW" != "MISSING" ] && has_target x86_64-pc-windows-gnu; } && echo true || echo false)"
else
  echo "[cross-target] 仓库：$REPO_ROOT"
  printf '  %-30s %s\n' "宿主三元组" "${HOST:-未知}"
  printf '  %-30s %s\n' "已装编译目标" "${TARGETS:-未知}"
  printf '  %-30s %s\n' "rustup / mingw / cargo-xwin / zig" "$RUSTUP / $MINGW / $XWIN / $ZIG"
  echo "[cross-target] 按目标分层的事实："
  if [ "$MINGW" != "MISSING" ] && has_target x86_64-pc-windows-gnu; then
    echo "  核心二进制（pony / ponycli）：✅ 可交叉到 Windows（mingw + windows-gnu 目标就绪）"
    echo "    示例：bash scripts/cargo-lane.sh build --release --target x86_64-pc-windows-gnu --bin pony --bin ponycli"
  else
    echo "  核心二进制（pony / ponycli）：❌ 当前不可交叉到 Windows"
    echo "    需要：apt-get install mingw-w64  &&  rustup target add x86_64-pc-windows-gnu"
    [ "$RUSTUP" = "MISSING" ] && echo "    注意：本机没有 rustup（rustc 来自系统包），装 target 需与发行版工具链一致的方式"
  fi
  echo "  Tauri 桌面端（src-tauri）：❌ 不能从 Linux 交叉到 Windows/macOS"
  echo "    —— 依赖 WebView2 + MSVC 运行库 + 平台窗口 API；官方路径是在 Windows 上构建，"
  echo "       或 cargo-xwin（$([ "$XWIN" = "MISSING" ] && echo '未安装' || echo '已安装')）配合 Windows SDK 头/库"
  echo "  macOS：❌ Linux 上不可行（需要 Apple 工具链与签名）"
  echo "[cross-target] 平台无关的脚本入口已就绪：npm run dev:tauri / clean:tauri:light / schemas:normalize 走 run-portable 分派"
fi

if [ -n "$REQUIRE" ]; then
  rc=0
  IFS=',' read -r -a want <<< "$REQUIRE"
  for target in "${want[@]}"; do
    target="${target// /}"
    [ -z "$target" ] && continue
    if has_target "$target"; then
      echo "[cross-target] OK：目标 $target 已安装"
    else
      echo "[cross-target] FAIL：目标 $target 未安装（已装：${TARGETS:-无}）" >&2
      rc=1
    fi
  done
  exit "$rc"
fi
exit 0
