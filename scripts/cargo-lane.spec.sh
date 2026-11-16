#!/usr/bin/env bash
# cargo-lane.spec.sh —— `scripts/cargo-lane.sh` 的负样本/正样本自检。
# 期望：全部断言通过 → exit 0；任一失败 → 非零。不实际构建（只用 --dry-run）。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LANE="$REPO_ROOT/scripts/cargo-lane.sh"
FAIL=0

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

# P1：lane 默认预算 = 4 job + 全局锁 + 低 nice + 透传 cargo 参数。
out=$(bash "$LANE" --dry-run test -p pony-agent-core --lib 2>&1)
assert_contains "P1 默认并发上限" "$out" "CARGO_BUILD_JOBS=4"
assert_contains "P1 全局构建锁" "$out" "flock /tmp/pony-agent-build.lock"
assert_contains "P1 lane nice 降权" "$out" "nice -n 5"
assert_contains "P1 透传 cargo 子命令" "$out" "cargo test -p pony-agent-core --lib"

# P2：--heavy 走重活 nice 档，且仍是同一把锁。
out=$(bash "$LANE" --heavy --dry-run build --release --bin pony 2>&1)
assert_contains "P2 重活 nice 档" "$out" "nice -n 10"
assert_contains "P2 重活仍加锁" "$out" "flock /tmp/pony-agent-build.lock"
assert_contains "P2 重活透传" "$out" "cargo build --release --bin pony"

# P3：环境覆盖生效（排障通道），且不破坏锁语义。
out=$(PONY_BUILD_JOBS=2 bash "$LANE" --dry-run test 2>&1)
assert_contains "P3 环境覆盖并发" "$out" "CARGO_BUILD_JOBS=2"
assert_not_contains "P3 覆盖后不残留默认值" "$out" "CARGO_BUILD_JOBS=4"

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

if [ "$FAIL" -ne 0 ]; then
  echo "cargo-lane.spec: FAILED"
  exit 1
fi
echo "cargo-lane.spec: PASS"
