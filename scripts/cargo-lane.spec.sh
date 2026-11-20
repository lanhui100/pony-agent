#!/usr/bin/env bash
# cargo-lane.spec.sh —— `scripts/cargo-lane.sh` 的负样本/正样本自检。
# 期望：全部断言通过 → exit 0；任一失败 → 非零。不实际构建（只用 --dry-run）。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LANE="$REPO_ROOT/scripts/cargo-lane.sh"
FAIL=0

# 期望并发值从**唯一真源**派生，不在用例里写死数字——否则「标定后调值」要连带改一堆断言，
# 而且写死的数字本身就成了第三份真源。
BUDGET_JOBS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8"))["max_jobs_per_lane"])' \
  "$REPO_ROOT/.dev-team/build-budget.json" 2>/dev/null || echo 4)"
# 环境覆盖用例必须取一个与默认值不同的值，否则断言恒真。
if [ "$BUDGET_JOBS" = "2" ]; then ALT_JOBS=3; else ALT_JOBS=2; fi

assert_contains() {
  local label="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -qF -- "$needle"; then
    printf 'PASS %s\n' "$label"
  else
    printf 'FAIL %s（期望包含 %q，实际：%s）\n' "$label" "$needle" "$haystack"
    FAIL=1
  fi
}

assert_not_contains() {
  local label="$1" haystack="$2" needle="$3"
  if printf '%s' "$haystack" | grep -qF -- "$needle"; then
    printf 'FAIL %s（不应包含 %q，实际：%s）\n' "$label" "$needle" "$haystack"
    FAIL=1
  else
    printf 'PASS %s\n' "$label"
  fi
}

# P1：lane 默认预算 = 预算值（max_jobs_per_lane）+ 全局锁 + 低 nice + 透传 cargo 参数。
out=$(bash "$LANE" --dry-run test -p pony-agent-core --lib 2>&1)
assert_contains "P1 默认并发上限" "$out" "CARGO_BUILD_JOBS=$BUDGET_JOBS"
assert_contains "P1 全局构建锁" "$out" "flock /tmp/pony-agent-build.lock"
assert_contains "P1 lane nice 降权" "$out" "nice -n 5"
assert_contains "P1 透传 cargo 子命令" "$out" "cargo test -p pony-agent-core --lib"

# P2：--heavy 走重活 nice 档，且仍是同一把锁。
out=$(bash "$LANE" --heavy --dry-run build --release --bin pony 2>&1)
assert_contains "P2 重活 nice 档" "$out" "nice -n 10"
assert_contains "P2 重活仍加锁" "$out" "flock /tmp/pony-agent-build.lock"
assert_contains "P2 重活透传" "$out" "cargo build --release --bin pony"

# P3：环境覆盖生效（排障通道），且不破坏锁语义。
out=$(PONY_BUILD_JOBS="$ALT_JOBS" bash "$LANE" --dry-run test 2>&1)
assert_contains "P3 环境覆盖并发" "$out" "CARGO_BUILD_JOBS=$ALT_JOBS"
assert_not_contains "P3 覆盖后不残留默认值" "$out" "CARGO_BUILD_JOBS=$BUDGET_JOBS"

# P4：缺少预算配置时 fail-closed（拒绝无界构建）。
tmp_repo="$(mktemp -d)"
mkdir -p "$tmp_repo/scripts"
cp "$LANE" "$tmp_repo/scripts/cargo-lane.sh"
out=$(bash "$tmp_repo/scripts/cargo-lane.sh" --dry-run test 2>&1); code=$?
rm -rf "$tmp_repo"
if [ "$code" -eq 2 ] && printf '%s' "$out" | grep -qF "fail-closed"; then
  printf 'PASS P4 缺配置 fail-closed（exit 2）\n'
else
  printf 'FAIL P4 缺配置未 fail-closed（exit %s，输出：%s）\n' "$code" "$out"
  FAIL=1
fi

# P5：契约声明（build-budget.json）自身合法。
if python3 "$HOME/.agents/skills/dev-team/scripts/dev-build-budget-lint.py" "$REPO_ROOT/.dev-team/build-budget.json" >/dev/null 2>&1; then
  printf 'PASS P5 build-budget.json 形状合法\n'
else
  printf 'FAIL P5 build-budget.json 未通过 dev-build-budget-lint.py\n'
  FAIL=1
fi

# P6：并发值单源——.cargo/config.toml 的 jobs 必须等于预算 max_jobs_per_lane（漂移即红）。
conf_jobs="$(awk -F'=' '/^jobs[[:space:]]*=/ {gsub(/[[:space:]]/,"",$2); print $2}' "$REPO_ROOT/.cargo/config.toml")"
budget_jobs="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1],encoding="utf-8"))["max_jobs_per_lane"])' "$REPO_ROOT/.dev-team/build-budget.json" 2>/dev/null)"
if [ -n "$conf_jobs" ] && [ "$conf_jobs" = "$budget_jobs" ]; then
  printf 'PASS P6 并发值单源一致（jobs=%s）\n' "$conf_jobs"
else
  printf 'FAIL P6 并发值漂移：.cargo/config.toml jobs=%s vs build-budget max_jobs_per_lane=%s\n' "${conf_jobs:-<缺>}" "${budget_jobs:-<缺>}"
  FAIL=1
fi

# P7：负载熔断在 dry-run 下必须给出判定（不再只是配置里声明）。
out=$(PONY_BUILD_LOAD_FACTOR=0 bash "$LANE" --dry-run test 2>&1)
assert_contains "P7 负载熔断可见" "$out" "load 熔断"
assert_contains "P7 熔断上限按 核数×系数 计算" "$out" "上限 0"
assert_contains "P7 dry-run 不等待但仍透传命令" "$out" "CARGO_BUILD_JOBS=$BUDGET_JOBS"

# P8：--allow-busy 明确跳过判定（排障通道）。
out=$(PONY_BUILD_LOAD_FACTOR=0 bash "$LANE" --allow-busy --dry-run test 2>&1)
assert_contains "P8 allow-busy 跳过熔断" "$out" "跳过"

# P9：超上限 + --no-wait 必须 fail-fast（exit 3，不得启动 cargo）。
PONY_BUILD_LOAD_FACTOR=0 bash "$LANE" --no-wait test --this-should-not-run >/dev/null 2>&1; code=$?
if [ "$code" -eq 3 ]; then
  printf 'PASS P9 超上限 --no-wait fail-fast（exit 3）\n'
else
  printf 'FAIL P9 超上限 --no-wait 未拦截（exit %s）\n' "$code"
  FAIL=1
fi

# P10：加速通道缺失即跳过——不得因为没装 sccache/mold 改变命令形状。
# 本用例显式关掉 sccache（PONY_BUILD_SCCACHE=0）来**模拟**缺失态：本机装了 sccache 时
# 不加这个开关，用例会因为「注入生效」而失败——那不是回归，是环境变了。
out=$(PONY_BUILD_SCCACHE=0 PONY_BUILD_FAST_LINK=0 bash "$LANE" --dry-run test 2>&1)
assert_contains "P10 无加速工具时命令形状不变" "$out" "env CARGO_BUILD_JOBS=$BUDGET_JOBS cargo test"

# P11：sccache 与增量编译硬互斥——强制开启时必须同时压 CARGO_INCREMENTAL=0，
# 否则 rustc 侧 sccache 直接 exit 101（实测），日常构建当场红。
# 同时要求绝对路径 injection（裸名在子进程 PATH 里可能解析不到，静默退化成没缓存）。
if [ -x "${HOME}/.local/bin/sccache" ] || command -v sccache >/dev/null 2>&1; then
  out=$(PONY_BUILD_SCCACHE=1 bash "$LANE" --dry-run test 2>&1)
  assert_contains "P11 sccache 注入绝对路径 wrapper" "$out" "RUSTC_WRAPPER=/"
  assert_contains "P11 sccache 强制关增量" "$out" "CARGO_INCREMENTAL=0"
  assert_contains "P11 加速通道可见" "$out" "加速通道：sccache"

  # P12：默认 auto + 非 release + 未显式关增量 ⇒ **不得**注入（日常增量构建优先），且要给出提示。
  out=$(bash "$LANE" --dry-run test 2>&1)
  assert_not_contains "P12 auto 模式不劫持日常增量构建" "$out" "RUSTC_WRAPPER="
  assert_contains "P12 auto 模式给出启用提示" "$out" "PONY_BUILD_SCCACHE=1"

  # P13：auto 模式在 --release（增量天然关闭）时必须自动注入。
  out=$(bash "$LANE" --dry-run build --release --bin pony 2>&1)
  assert_contains "P13 release 自动启用 sccache" "$out" "RUSTC_WRAPPER=/"
else
  printf 'SKIP P11-P13 未安装 sccache\n'
fi

# P14：无 flock 平台降级为 mkdir 锁（dry-run 必须如实展示，不假装有 flock）
out=$(PONY_BUILD_LOCK_IMPL=mkdir bash "$LANE" --dry-run test -p pony-agent-core 2>&1)
assert_contains "P14 mkdir 锁可见" "$out" "mkdir-lock /tmp/pony-agent-build.lock.d"
assert_not_contains "P14 不假装 flock" "$out" "flock /tmp/pony-agent-build.lock"

# P15：显式 flock 且有 flock 时仍是原来的锁（回归保护）
out=$(PONY_BUILD_LOCK_IMPL=flock bash "$LANE" --dry-run test 2>&1)
assert_contains "P15 显式 flock 生效" "$out" "flock /tmp/pony-agent-build.lock"

# P16：非法锁实现必须 fail-closed（不静默回落到 auto）
out=$(PONY_BUILD_LOCK_IMPL=bogus bash "$LANE" --dry-run test 2>&1); code=$?
if [ "$code" -eq 2 ] && printf '%s' "$out" | grep -qF "只接受 auto|flock|mkdir"; then
  printf 'PASS P16 非法锁实现 fail-closed（exit 2）\n'
else
  printf 'FAIL P16 非法锁实现未 fail-closed（exit %s，输出：%s）\n' "$code" "$out"
  FAIL=1
fi

if [ "$FAIL" -ne 0 ]; then
  echo "cargo-lane.spec: FAILED"
  exit 1
fi
echo "cargo-lane.spec: PASS"
