#!/usr/bin/env bash
# build-accel-check.sh —— 构建加速设施体检（只读，不改盘、不装包）。
#
# 为什么需要它：「编译慢」的多数可消除成本来自**设施缺失**（没有编译缓存、链接器还是 GNU ld）
# 与**产出错配**（增量目录/陈旧产物堆积）。这些东西在报错里看不出来，只能主动体检。
# 判定：本脚本只给事实与建议，永远 exit 0；需要门禁时用 `--require <项>[,<项>]`（缺失即 exit 1）。
# 项名：sccache / mold / cross-mingw
# 用法：
#   bash scripts/build-accel-check.sh
#   bash scripts/build-accel-check.sh --json
#   bash scripts/build-accel-check.sh --require sccache
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JSON=0
REQUIRE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --json) JSON=1; shift ;;
    --require) REQUIRE="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "[build-accel] 未知参数：$1" >&2; exit 2 ;;
  esac
done

# 工具解析：PATH → 用户级 bin 目录。预编译免 sudo 安装通常落在 ~/.local/bin，
# 很多会话的 PATH 不含它——只查 PATH 会把「装了」误报成 MISSING（门禁假红）。
resolve_tool() {
  local name="$1" p
  if p="$(command -v "$name" 2>/dev/null)"; then
    printf '%s\n' "$p"
    return 0
  fi
  for p in "${HOME:-/nonexistent}/.local/bin/$name" "/usr/local/bin/$name"; do
    if [ -x "$p" ]; then
      printf '%s\n' "$p"
      return 0
    fi
  done
  return 1
}

CORES="$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || echo 1)"
LOAD1="$(awk '{print $1}' /proc/loadavg 2>/dev/null || echo unknown)"
BUDGET="$REPO_ROOT/.dev-team/build-budget.json"
JOBS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8"))["max_jobs_per_lane"])' "$BUDGET" 2>/dev/null || echo unknown)"
FACTOR="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8"))["load_ceiling_factor"])' "$BUDGET" 2>/dev/null || echo unknown)"
CONF_JOBS="$(awk -F'=' '/^jobs[[:space:]]*=/ {gsub(/[[:space:]]/,"",$2); print $2}' "$REPO_ROOT/.cargo/config.toml" 2>/dev/null)"
SCCACHE="$(resolve_tool sccache || echo MISSING)"
MOLD="$(resolve_tool mold || echo MISSING)"
LLD="$(resolve_tool ld.lld || resolve_tool lld || echo MISSING)"
CLANG="$(resolve_tool clang || echo MISSING)"
MINGW="$(resolve_tool x86_64-w64-mingw32-gcc || echo MISSING)"
SYSROOT="$(rustc --print sysroot 2>/dev/null || echo "")"
TARGETS=""
if [ -n "$SYSROOT" ] && [ -d "$SYSROOT/lib/rustlib" ]; then
  TARGETS="$(ls "$SYSROOT/lib/rustlib" 2>/dev/null | grep -vE '^(src|etc|bin|lib|share|manifest-|components|cache|rust\.txt)' | tr '\n' ' ' | sed 's/ *$//')"
fi
INCREMENTAL_KB=0
# 口径必须与 clean-targets.sh --incremental 一致：debug **与** release 两侧的 incremental
# 都算中间态浪费；只数 debug 会低估体积，进而让下面的清理提示不触发（审核 P1）。
for d in "$REPO_ROOT"/target "$REPO_ROOT"/target-*; do
  for profile in debug release; do
    [ -d "$d/$profile/incremental" ] && INCREMENTAL_KB=$((INCREMENTAL_KB + $(du -sk "$d/$profile/incremental" 2>/dev/null | awk '{print $1}')))
  done
done

missing=""
[ "$SCCACHE" = "MISSING" ] && missing="$missing sccache"
[ "$MOLD" = "MISSING" ] && missing="$missing mold"

if [ "$JSON" -eq 1 ]; then
  printf '{"cores":%s,"load1":"%s","jobs":%s,"cargo_config_jobs":%s,"load_ceiling_factor":%s,"sccache":"%s","mold":"%s","ld_lld":"%s","clang":"%s","mingw":"%s","installed_targets":"%s","incremental_kb":%s,"missing":"%s"}\n' \
    "$CORES" "$LOAD1" "${JOBS:-0}" "${CONF_JOBS:-0}" "${FACTOR:-0}" "$SCCACHE" "$MOLD" "$LLD" "$CLANG" "$MINGW" "$TARGETS" "$INCREMENTAL_KB" "$(printf '%s' "$missing" | sed 's/^ *//')"
else
  echo "[build-accel] 仓库：$REPO_ROOT"
  printf '  %-26s %s\n' "CPU 核数 / load1" "$CORES / $LOAD1"
  printf '  %-26s %s\n' "lane 并发 / 配置兜底" "$JOBS / ${CONF_JOBS:-未设置}"
  printf '  %-26s %s\n' "load 熔断上限" "$(python3 -c "print(round($CORES * ${FACTOR:-1.5}, 2))" 2>/dev/null || echo unknown)（= 核数 × $FACTOR）"
  printf '  %-26s %s\n' "sccache（跨 worktree 复用）" "$SCCACHE"
  printf '  %-26s %s\n' "mold / ld.lld / clang" "$MOLD / $LLD / $CLANG"
  printf '  %-26s %s\n' "mingw（windows-gnu 交叉）" "$MINGW"
  printf '  %-26s %s\n' "已装编译目标" "${TARGETS:-未知（rustc 不可用）}"
  printf '  %-26s %s\n' "增量目录占用合计" "$(numfmt --to=iec --suffix=B $((INCREMENTAL_KB * 1024)) 2>/dev/null || echo "${INCREMENTAL_KB}K")"
  if [ "$SCCACHE" != "MISSING" ]; then
    SCCACHE_CACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache}"
    cache_kb=0
    [ -d "$SCCACHE_CACHE_DIR" ] && cache_kb="$(du -sk "$SCCACHE_CACHE_DIR" 2>/dev/null | awk '{print $1}')"
    cache_kb="${cache_kb:-0}"
    printf '  %-26s %s\n' "sccache 缓存占用" "$(numfmt --to=iec --suffix=B $((cache_kb * 1024)) 2>/dev/null || echo "${cache_kb}K")（目录 $SCCACHE_CACHE_DIR）"
    echo "[build-accel] 注意：sccache 与增量编译**硬互斥**（CARGO_INCREMENTAL 存在时 sccache 直接 exit 101）。"
    echo "              lane 默认 auto：只有冷编/全树（显式 CARGO_INCREMENTAL=0 或 --release）才注入 wrapper，"
    echo "              日常增量构建不注入。想用缓存换增量：PONY_BUILD_SCCACHE=1；关闭：=0。"
    echo "              另：RUSTC_WRAPPER 参与 cargo 指纹，首次开启会让既有产物一次性全量重编，靠缓存回收成本。"
  fi
  if [ -n "$missing" ]; then
    echo "[build-accel] 可装但未装（按需，非门禁）：$missing"
    echo "  sccache（免 sudo，推荐预编译包）："
    echo "    curl -sL https://github.com/mozilla/sccache/releases/download/v0.19.0/sccache-v0.19.0-x86_64-unknown-linux-musl.tar.gz | tar xz \\"
    echo "      && install -m755 sccache-v0.19.0-x86_64-unknown-linux-musl/sccache ~/.local/bin/sccache"
    echo "    （lane 会自动在 PATH 与 ~/.local/bin 中解析，无需改配置）"
    echo "  mold   ：apt-get install mold，再以 PONY_BUILD_FAST_LINK=1 启用（默认关，避免 RUSTFLAGS 变动导致全量重编）"
  else
    echo "[build-accel] 加速设施齐备"
  fi
  if [ "$INCREMENTAL_KB" -gt $((10 * 1024 * 1024)) ]; then
    echo "[build-accel] 提示：增量目录已超 10G，可用 bash scripts/clean-targets.sh --incremental 只删它（依赖产物保留，不动热缓存）"
  fi
fi

if [ -n "$REQUIRE" ]; then
  rc=0
  IFS=',' read -r -a want <<< "$REQUIRE"
  for item in "${want[@]}"; do
    case "${item// /}" in
      sccache) [ "$SCCACHE" != "MISSING" ] || { echo "[build-accel] FAIL：缺 sccache" >&2; rc=1; } ;;
      mold) [ "$MOLD" != "MISSING" ] || { echo "[build-accel] FAIL：缺 mold" >&2; rc=1; } ;;
      cross-mingw) [ "$MINGW" != "MISSING" ] || { echo "[build-accel] FAIL：缺 mingw-w64（windows-gnu 交叉不可用）" >&2; rc=1; } ;;
      "") ;;
      *) echo "[build-accel] FAIL：未知要求项 $item（可用：sccache,mold,cross-mingw）" >&2; rc=2 ;;
    esac
  done
  exit "$rc"
fi
exit 0
