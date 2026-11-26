#!/usr/bin/env bash
# cargo-lane.sh —— 本仓**唯一**的并发构建入口（并发预算 + 负载熔断 + 跨输出目录全局锁 + nice 降权）。
#
# 为什么需要它：cargo 的锁按 target 目录加锁（target/debug、target-test、target-check、target/release
# 各一把），多 lane 并行时互不阻塞；每条 cargo 又默认占满整机核数，于是 N lane × 核数 个 rustc
# 同时跑，形成 2–3 倍 CPU 超订（2026-11-17 实测 load 45.86 / 16 核，release 构建 90min 未完成，
# 单跑 3m07s–5m32s）。
#
# 真源：`.dev-team/build-budget.json`（形状由 dev-team 的 dev-build-budget-lint.py 校验）。
# 用法：
#   bash scripts/cargo-lane.sh test -p pony-agent-core --lib        # lane 构建/测试
#   bash scripts/cargo-lane.sh --heavy build --release --bin pony   # 重活（仅 Lead、独跑）
#   bash scripts/cargo-lane.sh --dry-run test -p pony-agent-core    # 打印最终命令，不执行
#
# 负载熔断（`load_ceiling_factor` 的实现；此前该字段只在配置里声明、脚本并未执行）：
#   1 分钟 load > factor × 核数 时先等（默认 300s，10s 轮询）。超时后**继续执行并大声告警**——
#   共享机器上的负载常来自本仓之外的进程，硬失败只会让门禁假红。需要硬失败用 `--strict`（exit 3），
#   完全跳过判定用 `--allow-busy`，立即放弃用 `--no-wait`（exit 3）。
#
# 加速通道（缺失即跳过，不改变语义）：
#   sccache：与增量编译**硬互斥**（CARGO_INCREMENTAL 存在时 sccache 直接 exit 101）。默认 auto——
#            仅在「已给 CARGO_INCREMENTAL=0」或「--release」的冷编/全树场景注入 wrapper，并压 CARGO_INCREMENTAL=0；
#            日常增量构建不注入（装了也只用增量）。强制开关：PONY_BUILD_SCCACHE=1 开（放弃增量）/ =0 关。
#   mold：默认 auto，Linux 下探测到 mold 且支持 `mold -run` 时自动包装 cargo（零 RUSTFLAGS 变动，
#         不使既有缓存失效）；PONY_BUILD_FAST_LINK=0 可关闭，=1 强制要求。
#
# 防卡死护栏（B1795589917594 + 2026-11-26 根治加固）：
#   cgroup：Linux 下默认 auto，探测 systemd-run --user --scope（固定 --unit 便于整树回收）并设置
#           CPUQuota=600%、MemoryHigh=8G、MemoryMax=10G（若 MemAvailable < 15G 则自动按余量下调封顶；
#           若 cgroup v2 委托了 io 控制器再附带 IOWeight=50）；无 user-systemd 时安全降级，显式 PONY_BUILD_CGROUP=1 时 fail-closed。
#   ionice：默认 auto，可用时注入 ionice -c 3（空闲级磁盘 IO），PONY_BUILD_IONICE=0 可关闭。
#   test  ：test/nextest 主子命令默认注入 timeout(600s, 置于 flock 内部) + RUST_TEST_THREADS=4（PONY_TEST_THREADS=0 可关闭）
#           + 自动隔离 PONY_AGENT_DATA_DIR（防集成测试误读真实 sessions.db；PONY_TEST_ISOLATE_DATA_DIR=0 可关闭）。
#
# 环境覆盖（排障用，不改变默认预算）：
#   PONY_BUILD_JOBS / PONY_BUILD_LOCK / PONY_BUILD_NICE / PONY_BUILD_LOAD_FACTOR
#   PONY_BUILD_CGROUP / PONY_BUILD_CPU_QUOTA / PONY_BUILD_MEM_HIGH / PONY_BUILD_MEM_MAX / PONY_BUILD_IO_WEIGHT
#   PONY_BUILD_IONICE / PONY_TEST_THREADS / PONY_TEST_TIMEOUT / PONY_TEST_ULIMIT_KB / PONY_TEST_ISOLATE_DATA_DIR
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="$REPO_ROOT/.dev-team/build-budget.json"

HEAVY=0
DRY_RUN=0
ALLOW_BUSY=0
NO_WAIT=0
STRICT=0
MAX_WAIT=300
while [ $# -gt 0 ]; do
  case "$1" in
    --heavy) HEAVY=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --allow-busy) ALLOW_BUSY=1; shift ;;
    --no-wait) NO_WAIT=1; shift ;;
    --strict) STRICT=1; shift ;;
    --max-wait) MAX_WAIT="${2:-300}"; shift 2 ;;
    --) shift; break ;;
    *) break ;;
  esac
done

if [ ! -f "$CONFIG" ]; then
  echo "[cargo-lane] fail-closed：缺少 $CONFIG（并发预算未声明，拒绝无界构建）" >&2
  exit 2
fi

read -r JOBS LOCK NICE_VALUE LOAD_FACTOR < <(python3 - "$CONFIG" <<'PY'
import json, sys
config = json.load(open(sys.argv[1], encoding="utf-8"))
print(config["max_jobs_per_lane"], config["serialize_lock_path"], config["heavy_nice"], config["load_ceiling_factor"])
PY
) || { echo "[cargo-lane] fail-closed：无法解析 $CONFIG" >&2; exit 2; }

JOBS="${PONY_BUILD_JOBS:-$JOBS}"
LOCK="${PONY_BUILD_LOCK:-$LOCK}"
LOAD_FACTOR="${PONY_BUILD_LOAD_FACTOR:-$LOAD_FACTOR}"
if [ "$HEAVY" -eq 1 ]; then
  NICE_VALUE="${PONY_BUILD_NICE:-$NICE_VALUE}"
else
  NICE_VALUE="${PONY_BUILD_NICE:-5}"
fi

# ── 跨输出目录串行化：flock 优先，缺失时降级为 mkdir 原子锁 ──────────────────
# 为什么不再「缺 flock 就拒绝」：macOS 自带环境没有 flock(1)，而 CI 的
# core-platform-check(macos-latest) 与本机 macOS 开发走的是同一条 lane。
# 硬拒绝等于「本仓在 macOS 上不可构建」——2026-11-20 PR #34 实测到的正是这个后果
# （.ps1 收口后 CI 首次真正进入 lane，随即 exit 1）。mkdir 是 POSIX 原子的，
# 用它实现同一把锁；锁目录内存 pid，用于回收崩溃残留（只删自身 pid 再 rmdir，不递归删）。
LOCK_IMPL="${PONY_BUILD_LOCK_IMPL:-auto}"
case "$LOCK_IMPL" in
  auto)
    if command -v flock >/dev/null 2>&1; then LOCK_IMPL=flock; else LOCK_IMPL=mkdir; fi ;;
  flock|mkdir) ;;
  *) echo "[cargo-lane] 用法错误：PONY_BUILD_LOCK_IMPL 只接受 auto|flock|mkdir，收到 $LOCK_IMPL" >&2; exit 2 ;;
esac

if [ "$LOCK_IMPL" = "flock" ] && ! command -v flock >/dev/null 2>&1; then
  echo "[cargo-lane] fail-closed：显式要求 flock 但本机缺少 flock(1)（可用 PONY_BUILD_LOCK_IMPL=mkdir 降级）" >&2
  exit 2
fi

# ── 负载熔断 ────────────────────────────────────────────────────────────
read_load1() {
  if [ -r /proc/loadavg ]; then
    awk '{print $1}' /proc/loadavg
  elif command -v sysctl >/dev/null 2>&1 && sysctl -n vm.loadavg >/dev/null 2>&1; then
    sysctl -n vm.loadavg | awk '{print $2}'
  else
    printf '0\n'
  fi
}

CORES="$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || echo 1)"
CEILING="$(python3 -c "print(round($CORES * $LOAD_FACTOR, 2))" 2>/dev/null || echo 0)"

gate_passes() {
  awk -v l="$(read_load1)" -v c="$CEILING" 'BEGIN { exit !(l <= c) }'
}

if [ "$ALLOW_BUSY" -eq 1 ]; then
  echo "[cargo-lane] load 熔断：跳过（--allow-busy，当前 load1=$(read_load1)，上限 $CEILING）"
elif gate_passes; then
  echo "[cargo-lane] load 熔断：通过（load1=$(read_load1) ≤ 上限 $CEILING = $CORES 核 × $LOAD_FACTOR）"
elif [ "$DRY_RUN" -eq 1 ]; then
  echo "[cargo-lane] load 熔断：当前 load1=$(read_load1) > 上限 $CEILING，实际执行会先等待（dry-run 不等待）"
elif [ "$NO_WAIT" -eq 1 ]; then
  echo "[cargo-lane] load 熔断：拒绝执行（--no-wait），load1=$(read_load1) > 上限 $CEILING" >&2
  exit 3
else
  echo "[cargo-lane] load 熔断：load1=$(read_load1) > 上限 $CEILING，最多等待 ${MAX_WAIT}s（10s 轮询）"
  waited=0
  while ! gate_passes; do
    if [ "$waited" -ge "$MAX_WAIT" ]; then
      if [ "$STRICT" -eq 1 ]; then
        echo "[cargo-lane] load 熔断：等待 ${MAX_WAIT}s 后仍超上限（--strict，exit 3）" >&2
        exit 3
      fi
      echo "[cargo-lane] load 熔断：等待 ${MAX_WAIT}s 后仍超上限（load1=$(read_load1) > $CEILING），按默认策略继续——负载可能来自本仓之外，需要止损请用 --no-wait" >&2
      break
    fi
    sleep 10
    waited=$((waited + 10))
  done
  if [ "$waited" -gt 0 ] && [ "$waited" -lt "$MAX_WAIT" ]; then
    echo "[cargo-lane] load 熔断：等待 ${waited}s 后恢复（load1=$(read_load1) ≤ $CEILING）"
  fi
fi

# ── 加速通道（缺失即跳过）──────────────────────────────────────────────
# 工具解析顺序：PATH → 用户级 bin 目录。后者必须显式搜：预编译安装（免 sudo）通常落在
# ~/.local/bin，而很多会话的 PATH 并不含它——只查 PATH 会让「装了但没生效」静默发生。
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

ACCEL_ENV=()
ACCEL_NOTE=""
ACCEL_SKIP_NOTE=""
SCCACHE_BIN=""
# sccache 与增量编译**硬互斥**：CARGO_INCREMENTAL 存在时 sccache 直接拒绝编译
# （实测 2026-11-20：`sccache: incremental compilation is prohibited`，cargo exit 101）。
# 而 `.cargo/config.toml` 的 dev/test profile 默认 incremental = true —— 所以「装了 sccache 就自动注入」
# 会让**每一次日常构建当场红**。注入前提 = 主动放弃增量：显式 PONY_BUILD_SCCACHE=1（额外压 CARGO_INCREMENTAL=0），
# 或 auto 模式下的冷编/全树场景（调用方已给 CARGO_INCREMENTAL=0 或走 --release）。
WANT_SCCACHE="${PONY_BUILD_SCCACHE:-auto}"
SCCACHE_ENABLE=0
case "$WANT_SCCACHE" in
  0|off) SCCACHE_ENABLE=0 ;;
  1|on) SCCACHE_ENABLE=1 ;;
  *)
    # 精确按参数列表判定，不用子串匹配：`cargo test -- release_flag` / `-p crate--release`
    # 这类子串会误触发 sccache 注入并强行改写 CARGO_INCREMENTAL（审核 P1）。
    want_release=0
    for arg in "$@"; do
      if [ "$arg" = "--release" ]; then want_release=1; break; fi
    done
    if [ "${CARGO_INCREMENTAL:-}" = "0" ] || [ "$want_release" -eq 1 ]; then
      SCCACHE_ENABLE=1
    fi
    ;;
esac
if [ "$SCCACHE_ENABLE" -eq 1 ] && SCCACHE_BIN="$(resolve_tool sccache)"; then
  # 绝对路径注入：子进程 PATH 未必含该目录，写裸名会「找不到 wrapper」。
  ACCEL_ENV+=("RUSTC_WRAPPER=$SCCACHE_BIN" "SCCACHE_DIR=${SCCACHE_DIR:-$HOME/.cache/sccache}" "CARGO_INCREMENTAL=0")
  ACCEL_NOTE="sccache($SCCACHE_BIN, 增量关闭)"
elif [ "$SCCACHE_ENABLE" -eq 0 ] && [ "$WANT_SCCACHE" != "0" ] && [ "$WANT_SCCACHE" != "off" ] \
     && SCCACHE_BIN="$(resolve_tool sccache)"; then
  ACCEL_SKIP_NOTE="sccache 已装但本轮未启用（增量编译优先）：冷编/跨 worktree 请 PONY_BUILD_SCCACHE=1"
fi
FAST_LINK_CMD=()
WANT_FAST_LINK="${PONY_BUILD_FAST_LINK:-auto}"
case "$WANT_FAST_LINK" in
  auto|1|on|0|off) ;;
  *) echo "[cargo-lane] 用法错误：PONY_BUILD_FAST_LINK 只接受 auto|1|on|0|off，收到 $WANT_FAST_LINK" >&2; exit 2 ;;
esac

if [ "$WANT_FAST_LINK" != "0" ] && [ "$WANT_FAST_LINK" != "off" ]; then
  if [ "$(uname -s 2>/dev/null || echo unknown)" = "Linux" ] \
     && MOLD_BIN="$(resolve_tool mold)" \
     && "$MOLD_BIN" -run true >/dev/null 2>&1; then
    FAST_LINK_CMD=("$MOLD_BIN" -run)
    ACCEL_NOTE="${ACCEL_NOTE:+$ACCEL_NOTE+}mold(-run)"
  elif [ "$WANT_FAST_LINK" = "1" ] || [ "$WANT_FAST_LINK" = "on" ]; then
    echo "[cargo-lane] fail-closed：显式要求 mold（PONY_BUILD_FAST_LINK=$WANT_FAST_LINK），但当前环境未就绪" >&2
    exit 2
  fi
fi

# 保证子进程 cargo 与 rustc/clippy 统一优先使用 ~/.cargo/bin（置于 /usr/local/bin 之前，避免混用导致 E0514 且不抢测试 shim）
if [ -n "${HOME:-}" ]; then
  if [ -d "$HOME/.cargo/bin" ]; then
    case ":${PATH:-}:" in
      *":$HOME/.cargo/bin:"*) ;;
      *":/usr/local/bin:"*)
        PATH=":${PATH}:"
        PATH="${PATH/:/usr/local/bin:/:/$HOME/.cargo/bin:/usr/local/bin:}"
        PATH="${PATH#:}"
        export PATH="${PATH%:}"
        ;;
      *) export PATH="${PATH:+$PATH:}$HOME/.cargo/bin" ;;
    esac
  fi
  if [ -d "$HOME/.local/bin" ]; then
    case ":${PATH:-}:" in
      *":$HOME/.local/bin:"*) ;;
      *) export PATH="${PATH:+$PATH:}$HOME/.local/bin" ;;
    esac
  fi
fi
if [ -n "$ACCEL_NOTE" ]; then
  echo "[cargo-lane] 加速通道：$ACCEL_NOTE"
fi
if [ -n "$ACCEL_SKIP_NOTE" ]; then
  echo "[cargo-lane] 加速通道：$ACCEL_SKIP_NOTE"
fi

# ── 测试执行期硬超时 + 并发限流 + 内核级 cgroup/ionice 护栏（防拖死主机）───
# 仅在 Cargo 主子命令为 `test` 或 `nextest` 时注入：避免 `-- test_filter` 或 `build` 被误杀；
# `timeout` 置于 `flock`/mkdir 锁**内部**，防止等锁排队耗时扣减测试预算。
# 默认 600s（兼顾全量单测冷链接 + 慢回归）；可用 PONY_TEST_TIMEOUT=0 / none 关闭。
detect_cargo_subcommand() {
  local expect_value=0 arg
  for arg in "$@"; do
    if [ "$expect_value" -eq 1 ]; then
      expect_value=0
      continue
    fi
    case "$arg" in
      --) break ;;
      +*) continue ;;
      --manifest-path|--target-dir|--color|--config|-C|-Z|-p|--package)
        expect_value=1
        continue
        ;;
      -*) continue ;;
      *)
        printf '%s\n' "$arg"
        return 0
        ;;
    esac
  done
}

TEST_TIMEOUT_CMD=()
TEST_ENV=()
ULIMIT_CMD=()
CARGO_SUBCMD="$(detect_cargo_subcommand "$@")"
is_test=0
if [ "$CARGO_SUBCMD" = "test" ] || [ "$CARGO_SUBCMD" = "nextest" ]; then
  is_test=1
fi

GUARD_NOTES=()

# 1) cgroup v2 资源隔离（Linux user-systemd scope，不可用时安全降级）
WANT_CGROUP="${PONY_BUILD_CGROUP:-auto}"
case "$WANT_CGROUP" in
  auto|on|1|off|0) ;;
  *) echo "[cargo-lane] 用法错误：PONY_BUILD_CGROUP 只接受 auto|on|1|off|0，收到 $WANT_CGROUP" >&2; exit 2 ;;
esac

clamp_default_mem_limits() {
  local mem_avail_kb cap_max_gb cap_high_gb
  [ -r /proc/meminfo ] || return 0
  mem_avail_kb="$(awk '/^MemAvailable:/ {print $2; exit}' /proc/meminfo 2>/dev/null || true)"
  case "$mem_avail_kb" in
    ''|*[!0-9]*) return 0 ;;
  esac
  # 共享机器余量低于 15GiB 时按可用内存折算上限，防止单个 lane scope 拖垮全机引发 Page Thrashing
  if [ "$mem_avail_kb" -lt 15728640 ]; then
    cap_max_gb="$(awk -v kb="$mem_avail_kb" 'BEGIN { g = int((kb * 0.65) / 1048576); if (g < 4) g = 4; if (g > 10) g = 10; print g }')"
    cap_high_gb="$(awk -v kb="$mem_avail_kb" -v m="$cap_max_gb" 'BEGIN { g = int((kb * 0.50) / 1048576); if (g < 3) g = 3; if (g >= m) g = m - 1; print g }')"
    if [ -z "${PONY_BUILD_MEM_MAX:-}" ]; then
      MEM_MAX="${cap_max_gb}G"
    fi
    if [ -z "${PONY_BUILD_MEM_HIGH:-}" ]; then
      MEM_HIGH="${cap_high_gb}G"
    fi
  fi
}

CGROUP_CMD=()
SCOPE_UNIT=""
if [ "$WANT_CGROUP" != "0" ] && [ "$WANT_CGROUP" != "off" ]; then
  if [ "$(uname -s 2>/dev/null || echo unknown)" = "Linux" ] \
     && command -v systemd-run >/dev/null 2>&1 \
     && command -v systemctl >/dev/null 2>&1 \
     && systemctl --user show-environment >/dev/null 2>&1; then
    CPU_QUOTA="${PONY_BUILD_CPU_QUOTA:-600%}"
    MEM_HIGH="${PONY_BUILD_MEM_HIGH:-8G}"
    MEM_MAX="${PONY_BUILD_MEM_MAX:-10G}"
    if [ -z "${PONY_BUILD_MEM_HIGH:-}" ] || [ -z "${PONY_BUILD_MEM_MAX:-}" ]; then
      clamp_default_mem_limits
    fi
    IO_WEIGHT="${PONY_BUILD_IO_WEIGHT:-}"
    if [ -z "$IO_WEIGHT" ]; then
      CTRL_FILE="/sys/fs/cgroup/user.slice/user-$(id -u).slice/cgroup.controllers"
      if [ -r "$CTRL_FILE" ] && grep -qw "io" "$CTRL_FILE" 2>/dev/null; then
        IO_WEIGHT="50"
      fi
    fi

    CGROUP_PROPS=()
    CGROUP_DESC=()
    if [ -n "$CPU_QUOTA" ] && [ "$CPU_QUOTA" != "0" ] && [ "$CPU_QUOTA" != "none" ]; then
      CGROUP_PROPS+=(-p "CPUQuota=$CPU_QUOTA")
      CGROUP_DESC+=("CPUQuota=$CPU_QUOTA")
    fi
    if [ -n "$MEM_HIGH" ] && [ "$MEM_HIGH" != "0" ] && [ "$MEM_HIGH" != "none" ]; then
      CGROUP_PROPS+=(-p "MemoryHigh=$MEM_HIGH")
      CGROUP_DESC+=("MemoryHigh=$MEM_HIGH")
    fi
    if [ -n "$MEM_MAX" ] && [ "$MEM_MAX" != "0" ] && [ "$MEM_MAX" != "none" ]; then
      CGROUP_PROPS+=(-p "MemoryMax=$MEM_MAX")
      CGROUP_DESC+=("MemoryMax=$MEM_MAX")
    fi
    if [ -n "$IO_WEIGHT" ] && [ "$IO_WEIGHT" != "0" ] && [ "$IO_WEIGHT" != "none" ]; then
      CGROUP_PROPS+=(-p "IOWeight=$IO_WEIGHT")
      CGROUP_DESC+=("IOWeight=$IO_WEIGHT")
    fi

    SCOPE_UNIT="pony-cargo-lane-$$"
    CGROUP_PROPS+=(-p "TimeoutStopSec=5s")
    CGROUP_CMD=(systemd-run --user --scope --quiet "--unit=$SCOPE_UNIT" "${CGROUP_PROPS[@]}" --)
    GUARD_NOTES+=("cgroup($(IFS=,; echo "${CGROUP_DESC[*]:-default}"))")
  elif [ "$WANT_CGROUP" = "on" ] || [ "$WANT_CGROUP" = "1" ]; then
    echo "[cargo-lane] fail-closed：显式要求 cgroup（PONY_BUILD_CGROUP=$WANT_CGROUP），但当前环境不支持 systemd-run --user --scope" >&2
    exit 2
  else
    GUARD_NOTES+=("cgroup(降级: 无 user-systemd)")
  fi
else
  GUARD_NOTES+=("cgroup(off)")
fi

# 2) 空闲级磁盘 IO（ionice -c 3）
WANT_IONICE="${PONY_BUILD_IONICE:-auto}"
case "$WANT_IONICE" in
  auto|on|1|off|0) ;;
  *) echo "[cargo-lane] 用法错误：PONY_BUILD_IONICE 只接受 auto|on|1|off|0，收到 $WANT_IONICE" >&2; exit 2 ;;
esac

IONICE_CMD=()
if [ "$WANT_IONICE" != "0" ] && [ "$WANT_IONICE" != "off" ]; then
  if command -v ionice >/dev/null 2>&1 && ionice -c 3 true >/dev/null 2>&1; then
    IONICE_CMD=(ionice -c 3)
    GUARD_NOTES+=("ionice(idle)")
  elif [ "$WANT_IONICE" = "on" ] || [ "$WANT_IONICE" = "1" ]; then
    echo "[cargo-lane] fail-closed：显式要求 ionice（PONY_BUILD_IONICE=$WANT_IONICE），但当前环境不支持 ionice -c 3" >&2
    exit 2
  fi
fi

# 3) 测试路径：超时 + 运行期线程限流 + 无 cgroup 时的可选 ulimit 降级
if [ "$is_test" -eq 1 ]; then
  TEST_TIMEOUT="${PONY_TEST_TIMEOUT:-600s}"
  if [ "$TEST_TIMEOUT" != "0" ] && [ "$TEST_TIMEOUT" != "none" ] && command -v timeout >/dev/null 2>&1; then
    TEST_TIMEOUT_CMD=(timeout --signal=TERM --kill-after=15s "$TEST_TIMEOUT")
    GUARD_NOTES+=("timeout=$TEST_TIMEOUT")
  fi

  WANT_TEST_THREADS="${PONY_TEST_THREADS:-${RUST_TEST_THREADS:-4}}"
  if [ "$WANT_TEST_THREADS" != "0" ] && [ "$WANT_TEST_THREADS" != "none" ]; then
    case "$WANT_TEST_THREADS" in
      ''|*[!0-9]*)
        echo "[cargo-lane] 用法错误：PONY_TEST_THREADS 必须为正整数或 0/none，收到 $WANT_TEST_THREADS" >&2
        exit 2
        ;;
      0*)
        echo "[cargo-lane] 用法错误：PONY_TEST_THREADS 必须为正整数或 0/none，收到 $WANT_TEST_THREADS" >&2
        exit 2
        ;;
    esac
    TEST_ENV+=("RUST_TEST_THREADS=$WANT_TEST_THREADS")
    GUARD_NOTES+=("test-threads=$WANT_TEST_THREADS")
  fi

  if [ "${#CGROUP_CMD[@]}" -eq 0 ] && [ -n "${PONY_TEST_ULIMIT_KB:-}" ] && [ "${PONY_TEST_ULIMIT_KB}" != "0" ] && [ "${PONY_TEST_ULIMIT_KB}" != "none" ]; then
    case "$PONY_TEST_ULIMIT_KB" in
      ''|*[!0-9]*)
        echo "[cargo-lane] 用法错误：PONY_TEST_ULIMIT_KB 必须为正整数（KB），收到 $PONY_TEST_ULIMIT_KB" >&2
        exit 2
        ;;
    esac
    ULIMIT_CMD=(bash -c 'ulimit -v "$1" && shift && exec "$@"' _ "$PONY_TEST_ULIMIT_KB")
    GUARD_NOTES+=("ulimit-v=${PONY_TEST_ULIMIT_KB}KB")
  fi

  WANT_ISOLATE_DATA_DIR="${PONY_TEST_ISOLATE_DATA_DIR:-auto}"
  case "$WANT_ISOLATE_DATA_DIR" in
    auto|1|on|0|off) ;;
    *) echo "[cargo-lane] 用法错误：PONY_TEST_ISOLATE_DATA_DIR 只接受 auto|on|1|off|0，收到 $WANT_ISOLATE_DATA_DIR" >&2; exit 2 ;;
  esac
  if [ "$WANT_ISOLATE_DATA_DIR" != "0" ] && [ "$WANT_ISOLATE_DATA_DIR" != "off" ]; then
    if [ -n "${PONY_AGENT_DATA_DIR:-}" ]; then
      GUARD_NOTES+=("data-dir(external)")
    elif [ "$DRY_RUN" -eq 1 ]; then
      TEST_ENV+=("PONY_AGENT_DATA_DIR=${TMPDIR:-/tmp}/pony-test-data-dry-run")
      GUARD_NOTES+=("data-dir(isolated)")
    else
      AUTO_DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/pony-test-data.XXXXXX")"
      TEST_ENV+=("PONY_AGENT_DATA_DIR=$AUTO_DATA_DIR")
      GUARD_NOTES+=("data-dir(isolated)")
    fi
  fi
fi

if [ "${#GUARD_NOTES[@]}" -gt 0 ]; then
  echo "[cargo-lane] 护栏：$(IFS=' + '; echo "${GUARD_NOTES[*]}")"
fi

if [ "$LOCK_IMPL" = "flock" ]; then
  CMD=(flock "$LOCK" "${CGROUP_CMD[@]}" "${IONICE_CMD[@]}" "${TEST_TIMEOUT_CMD[@]}" nice -n "$NICE_VALUE" "${ULIMIT_CMD[@]}" env "${ACCEL_ENV[@]}" "${TEST_ENV[@]}" "CARGO_BUILD_JOBS=$JOBS" "${FAST_LINK_CMD[@]}" cargo "$@")
else
  CMD=("${CGROUP_CMD[@]}" "${IONICE_CMD[@]}" "${TEST_TIMEOUT_CMD[@]}" nice -n "$NICE_VALUE" "${ULIMIT_CMD[@]}" env "${ACCEL_ENV[@]}" "${TEST_ENV[@]}" "CARGO_BUILD_JOBS=$JOBS" "${FAST_LINK_CMD[@]}" cargo "$@")
fi

if [ "$DRY_RUN" -eq 1 ]; then
  # 空数组展开可能留下双空格，这里只影响 dry-run 展示
  if [ "$LOCK_IMPL" = "flock" ]; then
    printf '%s\n' "${CMD[*]}" | sed 's/  */ /g'
  else
    # 无 flock 的平台要如实展示实际用的锁，不能假装有 flock
    printf 'mkdir-lock %s.d ; %s\n' "$LOCK" "$(printf '%s' "${CMD[*]}" | sed 's/  */ /g')"
  fi
  exit 0
fi

# ── mkdir 锁路径（无 flock 平台）────────────────────────────────────────
LOCK_DIR="$LOCK.d"
acquire_mkdir_lock() {
  local waited=0 stale_pid=""
  while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    if [ -f "$LOCK_DIR/pid" ]; then
      stale_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
      if [ -n "${stale_pid:-}" ] && ! kill -0 "$stale_pid" 2>/dev/null; then
        echo "[cargo-lane] 回收陈旧锁：$LOCK_DIR（pid $stale_pid 已不存在）" >&2
        rm -f "$LOCK_DIR/pid"
        rmdir "$LOCK_DIR" 2>/dev/null || true
        continue
      fi
    fi
    sleep 0.2
    waited=$((waited + 1))
    if [ "$waited" -ge 1500 ]; then
      echo "[cargo-lane] fail-closed：等待 mkdir 锁 $LOCK_DIR 超过 300s（持有者 pid：${stale_pid:-未知}）" >&2
      exit 3
    fi
  done
  printf '%s\n' "$$" > "$LOCK_DIR/pid"
}
release_mkdir_lock() {
  rm -f "$LOCK_DIR/pid"
  rmdir "$LOCK_DIR" 2>/dev/null || true
}

CMD_PID=""
LANE_SIGNALLED=0
cleanup_lane() {
  if [ -n "${CMD_PID:-}" ] && kill -0 "$CMD_PID" 2>/dev/null; then
    pkill -TERM -P "$CMD_PID" 2>/dev/null || true
    kill -TERM "$CMD_PID" 2>/dev/null || true
  fi
  if [ -n "${SCOPE_UNIT:-}" ] && command -v systemctl >/dev/null 2>&1; then
    if [ "$LANE_SIGNALLED" -eq 1 ]; then
      systemctl --user kill --signal=TERM "$SCOPE_UNIT" >/dev/null 2>&1 || true
      sleep 0.2
      systemctl --user kill --signal=KILL "$SCOPE_UNIT" >/dev/null 2>&1 || true
    fi
    systemctl --user stop "$SCOPE_UNIT" >/dev/null 2>&1 || true
  fi
  if [ -n "${CMD_PID:-}" ] && kill -0 "$CMD_PID" 2>/dev/null; then
    pkill -KILL -P "$CMD_PID" 2>/dev/null || true
    kill -KILL "$CMD_PID" 2>/dev/null || true
  fi
  if [ -n "${AUTO_DATA_DIR:-}" ] && [ -d "$AUTO_DATA_DIR" ]; then
    rm -rf "$AUTO_DATA_DIR"
  fi
  if [ "$LOCK_IMPL" != "flock" ]; then
    release_mkdir_lock
  fi
}

if [ "$LOCK_IMPL" != "flock" ]; then
  acquire_mkdir_lock
fi

trap 'cleanup_lane' EXIT
trap 'LANE_SIGNALLED=1; exit 130' INT
trap 'LANE_SIGNALLED=1; exit 143' TERM

# 后台等待使外层收到 SIGTERM/SIGINT 时立即唤醒 trap 并整树回收 scope，避免子进程逃逸成孤儿
"${CMD[@]}" &
CMD_PID=$!
wait "$CMD_PID"
rc=$?
CMD_PID=""
exit "$rc"
