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

# 假产物：热缓存 target/ + 两个槽位 + 前端 dist。
mkdir -p "$sandbox/target/release" "$sandbox/target-test/deps" "$sandbox/target-check/deps" "$sandbox/dist"
dd if=/dev/zero of="$sandbox/target/release/pony" bs=1024 count=64 status=none
dd if=/dev/zero of="$sandbox/target-test/deps/blob" bs=1024 count=32 status=none
dd if=/dev/zero of="$sandbox/target-check/deps/blob" bs=1024 count=16 status=none
dd if=/dev/zero of="$sandbox/dist/bundle.js" bs=1024 count=8 status=none

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
