#!/usr/bin/env bash
# artifacts-tools.spec.sh —— `clean-targets.sh` / `artifacts-status.sh` 的正负样本自检。
# 全部在 mktemp 沙箱内进行，不碰真实产物。期望：全绿 exit 0。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FAIL=0

ok() { printf 'PASS %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; FAIL=1; }

sandbox="$(mktemp -d)"
mkdir -p "$sandbox/scripts"
cp "$REPO_ROOT/scripts/clean-targets.sh" "$REPO_ROOT/scripts/artifacts-status.sh" "$sandbox/scripts/"

# 假产物：热缓存 target/ + 两个槽位 + 前端 dist + 增量中间态。
mkdir -p "$sandbox/target/release" "$sandbox/target-test/deps" "$sandbox/target-check/deps" "$sandbox/dist"
mkdir -p "$sandbox/target/debug/incremental" "$sandbox/target-test/debug/incremental"
dd if=/dev/zero of="$sandbox/target/release/pony" bs=1024 count=64 status=none
dd if=/dev/zero of="$sandbox/target-test/deps/blob" bs=1024 count=32 status=none
dd if=/dev/zero of="$sandbox/target-check/deps/blob" bs=1024 count=16 status=none
dd if=/dev/zero of="$sandbox/dist/bundle.js" bs=1024 count=8 status=none
dd if=/dev/zero of="$sandbox/target/debug/incremental/chunk.o" bs=1024 count=128 status=none
dd if=/dev/zero of="$sandbox/target-test/debug/incremental/chunk.o" bs=1024 count=64 status=none

# P1：--dry-run 只报告、不动盘。
out="$(bash "$sandbox/scripts/clean-targets.sh" --dry-run 2>&1)"; code=$?
if [ "$code" -eq 0 ] && printf '%s' "$out" | grep -qF -- "--dry-run" \
  && [ -d "$sandbox/target-test" ] && [ -d "$sandbox/target-check" ]; then
  ok "P1 dry-run 不删目录（exit 0）"
else
  bad "P1 dry-run 行为异常（exit $code）"
fi
if printf '%s' "$out" | grep -qF -- "target-check" && printf '%s' "$out" | grep -qF -- "target-test"; then
  ok "P1b dry-run 列出待删槽位与体积"
else
  bad "P1b dry-run 未列出槽位：$out"
fi

# P1c：--incremental 只删增量中间态，保留 deps 与 release 二进制。
out="$(bash "$sandbox/scripts/clean-targets.sh" --incremental 2>&1)"; code=$?
if [ "$code" -eq 0 ] && [ ! -e "$sandbox/target/debug/incremental" ] \
  && [ ! -e "$sandbox/target-test/debug/incremental" ] \
  && [ -f "$sandbox/target/release/pony" ] && [ -f "$sandbox/target-test/deps/blob" ] \
  && [ -f "$sandbox/target-check/deps/blob" ]; then
  ok "P1c incremental 只删增量中间态，依赖产物与 release 保留"
else
  bad "P1c incremental 语义异常（exit $code，输出：$out）"
fi

# P1d：增量档空场景不误报。
out="$(bash "$sandbox/scripts/clean-targets.sh" --incremental 2>&1)"; code=$?
if [ "$code" -eq 0 ] && printf '%s' "$out" | grep -qF "没有 incremental"; then
  ok "P1d 无增量目录时 exit 0"
else
  bad "P1d 空增量场景异常（exit $code，输出：$out）"
fi

# P1e：产物预算门——超预算 exit 1、放宽 exit 0、缺预算文件 fail-closed（exit 3）。
printf '{"version":1,"thresholds_kb":{"target-test":1},"incremental_total_kb":0}\n' > "$sandbox/tight.json"
printf '{"version":1,"thresholds_kb":{"target-test":1048576},"incremental_total_kb":1048576}\n' > "$sandbox/loose.json"
ARTIFACTS_DISK_MAX_PCT=100 ARTIFACTS_BUDGET_FILE="$sandbox/tight.json" bash "$sandbox/scripts/artifacts-status.sh" --budget >/dev/null 2>&1; code=$?
[ "$code" -eq 1 ] && ok "P1e 超预算必须 exit 1" || bad "P1e 超预算未拦截（exit $code）"
ARTIFACTS_DISK_MAX_PCT=100 ARTIFACTS_BUDGET_FILE="$sandbox/loose.json" bash "$sandbox/scripts/artifacts-status.sh" --budget >/dev/null 2>&1; code=$?
[ "$code" -eq 0 ] && ok "P1e2 预算内放行（exit 0）" || bad "P1e2 预算内被误拦（exit $code）"
ARTIFACTS_DISK_MAX_PCT=100 ARTIFACTS_BUDGET_FILE="$sandbox/nope.json" bash "$sandbox/scripts/artifacts-status.sh" --budget >/dev/null 2>&1; code=$?
[ "$code" -eq 3 ] && ok "P1e3 缺预算文件 fail-closed（exit 3）" || bad "P1e3 缺预算文件未 fail-closed（exit $code）"

# P1f：增量合计必须覆盖 release 档——只统计 debug/incremental 会让 release 侧膨胀漏报（门禁假绿）。
mkdir -p "$sandbox/target-test/release/incremental"
dd if=/dev/zero of="$sandbox/target-test/release/incremental/blob" bs=1024 count=64 status=none
printf '{"version":1,"thresholds_kb":{},"incremental_total_kb":8}\n' > "$sandbox/incr-tight.json"
ARTIFACTS_DISK_MAX_PCT=100 ARTIFACTS_BUDGET_FILE="$sandbox/incr-tight.json" bash "$sandbox/scripts/artifacts-status.sh" --budget >/dev/null 2>&1; code=$?
[ "$code" -eq 1 ] && ok "P1f release/incremental 计入增量合计（exit 1）" || bad "P1f release 增量未计入（exit $code）"

# P1g：`--incremental --with-frontend` 组合必须双档同时生效。
# 曾经的缺陷：增量分支重置候选列表，把前端目录吞掉，而日志仍打印「+ frontend」（审核 P1）。
mkdir -p "$sandbox/target/debug/incremental"
dd if=/dev/zero of="$sandbox/target/debug/incremental/chunk.o" bs=1024 count=32 status=none
out="$(bash "$sandbox/scripts/clean-targets.sh" --incremental --with-frontend 2>&1)"; code=$?
if [ "$code" -eq 0 ] && [ ! -e "$sandbox/target/debug/incremental" ] && [ ! -e "$sandbox/dist" ] \
  && [ -f "$sandbox/target/release/pony" ]; then
  ok "P1g incremental+frontend 组合双档同时生效"
else
  bad "P1g 组合档语义异常（exit $code，输出：$out）"
fi
# 复原前端产物：P2 要断言「light 档保留 dist/」，不能让 P1g 删掉的目录污染下一例。
mkdir -p "$sandbox/dist"
dd if=/dev/zero of="$sandbox/dist/bundle.js" bs=1024 count=8 status=none

# P2：light 深度删槽位、保留热缓存 target/ 与前端 dist/。
out="$(bash "$sandbox/scripts/clean-targets.sh" 2>&1)"; code=$?
if [ "$code" -eq 0 ] && [ ! -e "$sandbox/target-test" ] && [ ! -e "$sandbox/target-check" ] \
  && [ -f "$sandbox/target/release/pony" ] && [ -f "$sandbox/dist/bundle.js" ]; then
  ok "P2 light 只删槽位，保留 target/ 与 dist/"
else
  bad "P2 light 深度语义异常（exit $code）"
fi

# P3：--deep --with-frontend 才动热缓存与前端产物。
bash "$sandbox/scripts/clean-targets.sh" --deep --with-frontend >/dev/null 2>&1; code=$?
if [ "$code" -eq 0 ] && [ ! -e "$sandbox/target" ] && [ ! -e "$sandbox/dist" ]; then
  ok "P3 deep+frontend 删除热缓存与前端产物"
else
  bad "P3 deep 深度语义异常（exit $code）"
fi

# P4：无可清理目录时明确退出 0（不误报）。
out="$(bash "$sandbox/scripts/clean-targets.sh" 2>&1)"; code=$?
if [ "$code" -eq 0 ] && printf '%s' "$out" | grep -qF "没有可清理"; then
  ok "P4 空产物场景 exit 0"
else
  bad "P4 空产物场景异常（exit $code，输出：$out）"
fi

# P5：磁盘水位门禁——高阈值放行、0 阈值必须非零。
ARTIFACTS_DISK_MAX_PCT=100 bash "$sandbox/scripts/artifacts-status.sh" >/dev/null 2>&1; code=$?
[ "$code" -eq 0 ] && ok "P5a 高阈值放行（exit 0）" || bad "P5a 高阈值未放行（exit $code）"
ARTIFACTS_DISK_MAX_PCT=0 bash "$sandbox/scripts/artifacts-status.sh" >/dev/null 2>&1; code=$?
[ "$code" -eq 1 ] && ok "P5b 0 阈值必须拦截（exit 1）" || bad "P5b 0 阈值未拦截（exit $code）"

# P6：--json 输出可被 JSON 解析。
json="$(ARTIFACTS_DISK_MAX_PCT=100 bash "$sandbox/scripts/artifacts-status.sh" --json 2>/dev/null)"
if printf '%s' "$json" | python3 -c 'import json,sys; json.load(sys.stdin)' >/dev/null 2>&1; then
  ok "P6 --json 可解析"
else
  bad "P6 --json 非法：$json"
fi

rm -rf "$sandbox"
if [ "$FAIL" -ne 0 ]; then
  echo "artifacts-tools.spec: FAILED"
  exit 1
fi
echo "artifacts-tools.spec: PASS"
