#!/usr/bin/env bash
# scripts/audit/build-install.sh —— PA-169 C5：构建 + 本机安装替换
#
# 契约：ADR 0101 §确定性节点 build-install（npm run verify + cargo build + 本机安装替换，
# 非零 = 红）。本机 = dev 服务器（项目常载规则：dev/devserver 均指本机）。
#
# 平台差异（诚实标注，不静默降级）：
#   - Windows：`npm run verify` 全量（含 scripts/invoke-rust-target.ps1 的 cargo 检查）。
#   - 本机（Linux）：仓库脚本链依赖 pwsh（invoke-rust-target.ps1），本机无 pwsh ⇒
#     `npm run verify` 必然在 cargo:check:shared 处失败。此时按**显式等价序列**执行
#     各步（version:check / test:unit / build / contract:check / size:check / ext:check /
#     cargo check），并在回执里记录 `verify_mode=linux-fallback` 与逐步骤退出码。
#     等价序列任一步非零 ⇒ 整体红；不因「换了个跑法」判 pass。
#
# 用法：bash scripts/audit/build-install.sh [--dry-run] [--skip-verify] [--skip-install]
#          [--manifest crates/pony-agent-core/Cargo.toml] [--bin pony --bin ponycli]
#
# 退出码：0 = 构建与安装完成｜1 = 某步骤失败｜2 = 参数错误｜3 = 环境不可用（缺 npm/cargo）
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

DRY_RUN=0; SKIP_VERIFY=0; SKIP_INSTALL=0
MANIFEST="crates/pony-agent-core/Cargo.toml"
BINS=(pony ponycli)
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --skip-verify) SKIP_VERIFY=1; shift ;;
    --skip-install) SKIP_INSTALL=1; shift ;;
    --manifest) MANIFEST="${2:?}"; shift 2 ;;
    --bin) BINS+=("${2:?}"); shift 2 ;;
    -h|--help) sed -n '2,24p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done

need_cmd npm || exit 3
need_cmd cargo || exit 3

VERIFY_MODE="npm-verify"
if ! command -v pwsh >/dev/null 2>&1 && ! command -v powershell >/dev/null 2>&1; then
  VERIFY_MODE="linux-fallback"
fi

LADDER=()
if [ "$VERIFY_MODE" = "npm-verify" ]; then
  LADDER=("npm run verify")
else
  LADDER=(
    "npm run version:check"
    "npm run test:unit"
    "npm run build"
    "npm run contract:check"
    "npm run size:check"
    "npm run ext:check"
    "cargo check --manifest-path $MANIFEST"
  )
fi

say "verify_mode=$VERIFY_MODE manifest=$MANIFEST bins=${BINS[*]}"

if [ "$DRY_RUN" -eq 1 ]; then
  say "── plan (dry-run, 不执行)"
  if [ "$SKIP_VERIFY" -eq 0 ]; then
    i=0
    for step in "${LADDER[@]}"; do
      i=$((i + 1))
      say "   1.${i}) $step"
    done
  else
    say "   1) verify SKIPPED"
  fi
  say "   2) cargo build --release --manifest-path $MANIFEST $(printf -- '--bin %s ' "${BINS[@]}")"
  if [ "$SKIP_INSTALL" -eq 0 ]; then
    say "   3) install → ~/.local/bin/ (atomic cp + chmod +x)"
  else
    say "   3) install SKIPPED"
  fi
  exit 0
fi

failed=0
if [ "$SKIP_VERIFY" -eq 0 ]; then
  say "── step 1: verify (${VERIFY_MODE})"
  for step in "${LADDER[@]}"; do
    say "   \$ $step"
    ( cd "$REPO_ROOT" && eval "$step" )
    rc=$?
    say "   exit=$rc"
    [ "$rc" -eq 0 ] || { failed=1; fail_msg "verify 步骤失败：$step（exit=$rc）"; break; }
  done
else
  say "── step 1: verify SKIPPED (--skip-verify)"
fi

if [ "$failed" -eq 0 ]; then
  say "── step 2: cargo build --release"
  ( cd "$REPO_ROOT" && cargo build --release --manifest-path "$MANIFEST" \
      $(printf -- '--bin %s ' "${BINS[@]}") )
  rc=$?
  say "   exit=$rc"
  [ "$rc" -eq 0 ] || { failed=1; fail_msg "cargo build 失败（exit=$rc）"; }
fi

if [ "$failed" -eq 0 ] && [ "$SKIP_INSTALL" -eq 0 ]; then
  say "── step 3: install → ~/.local/bin"
  mkdir -p "$HOME/.local/bin"
  for bin in "${BINS[@]}"; do
    src="$REPO_ROOT/target/release/$bin"
    if [ ! -x "$src" ]; then
      fail_msg "构建产物缺失：$src"
      failed=1
      continue
    fi
    cp "$src" "$HOME/.local/bin/$bin.tmp.$$" && chmod +x "$HOME/.local/bin/$bin.tmp.$$" \
      && mv -f "$HOME/.local/bin/$bin.tmp.$$" "$HOME/.local/bin/$bin"
    rc=$?
    say "   install $bin exit=$rc"
    [ "$rc" -eq 0 ] || { failed=1; fail_msg "安装失败：$bin"; }
  done
fi

say ""
say "build-install: failed=$failed verify_mode=$VERIFY_MODE"
exit "$failed"
