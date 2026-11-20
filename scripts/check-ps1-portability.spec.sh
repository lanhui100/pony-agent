#!/usr/bin/env bash
# check-ps1-portability.spec.sh —— `check-ps1-portability.sh` 的正负样本自检（全部在 mktemp 沙箱内）。
# 期望：全绿 exit 0。每个负样本都要求门禁**非零退出且指名违规项**，防止「门永远绿」。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FAIL=0
ok() { printf 'PASS %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; FAIL=1; }

sandbox="$(mktemp -d)"
trap 'rm -rf "$sandbox"' EXIT

seed() {
  rm -rf "$sandbox/scripts"
  mkdir -p "$sandbox/scripts/lib"
  cp "$REPO_ROOT/scripts/check-ps1-portability.sh" "$sandbox/scripts/"
  cp "$REPO_ROOT/scripts/lib/posix-dispatch.ps1" "$sandbox/scripts/lib/"
  cp "$REPO_ROOT"/scripts/*.ps1 "$sandbox/scripts/"
  # 落点桩：只要存在即可（门只判存在与扩展名，不执行内容）
  for f in clean-targets.sh normalize-generated-schemas.sh start-tauri-dev.sh invoke-rust-target.mjs check-version-sync.mjs; do
    : > "$sandbox/scripts/$f"
  done
}

run_gate() { bash "$sandbox/scripts/check-ps1-portability.sh" 2>&1; }

# P1：真实仓库形态 → 通过
seed
out="$(run_gate)"; code=$?
if [ "$code" -eq 0 ]; then ok "P1 原样通过（exit 0）"; else bad "P1 原样未通过（exit $code）：$out"; fi

# P2：ps1 删掉转派调用 → 必须是红，且点名该脚本
seed
python3 - "$sandbox/scripts/clean-targets.ps1" <<'PY'
import sys
p = sys.argv[1]
lines = [l for l in open(p, encoding="utf-8").read().split("\n") if "Invoke-PonyPosixTwin" not in l]
open(p, "w", encoding="utf-8").write("\n".join(lines))
PY
out="$(run_gate)"; code=$?
if [ "$code" -ne 0 ] && printf '%s' "$out" | grep -q "clean-targets"; then
  ok "P2 缺转派调用被拦截（exit $code，点名 clean-targets）"
else
  bad "P2 缺转派调用未拦截（exit $code）：$out"
fi

# P3：映射表登记了落点、但落点文件不存在 → 必须红
seed
rm -f "$sandbox/scripts/invoke-rust-target.mjs"
out="$(run_gate)"; code=$?
if [ "$code" -ne 0 ] && printf '%s' "$out" | grep -q "invoke-rust-target.mjs 不存在"; then
  ok "P3 落点缺失被拦截（exit $code）"
else
  bad "P3 落点缺失未拦截（exit $code）：$out"
fi

# P4：Windows-only 白名单脚本去掉 -WindowsOnly → 必须红
seed
sed -i 's/ -WindowsOnly//' "$sandbox/scripts/bump-version.ps1"
out="$(run_gate)"; code=$?
if [ "$code" -ne 0 ] && printf '%s' "$out" | grep -q "bump-version"; then
  ok "P4 Windows-only 缺 fail-closed 标记被拦截（exit $code）"
else
  bad "P4 Windows-only 标记缺失未拦截（exit $code）：$out"
fi

# P5：新增未登记 ps1 → 必须红
seed
printf 'Write-Host "hi"\n' > "$sandbox/scripts/unregistered.ps1"
out="$(run_gate)"; code=$?
if [ "$code" -ne 0 ] && printf '%s' "$out" | grep -q "unregistered.ps1 未登记"; then
  ok "P5 未登记 ps1 被拦截（exit $code）"
else
  bad "P5 未登记 ps1 未拦截（exit $code）：$out"
fi

# P6：映射表登记了非 .sh/.mjs 落点 → 必须红（runner 口径不可判）
seed
python3 - "$sandbox/scripts/lib/posix-dispatch.ps1" <<'PY'
import sys
p = sys.argv[1]
text = open(p, encoding="utf-8").read()
text = text.replace("  'check-version-sync'          = 'check-version-sync.mjs'",
                    "  'check-version-sync'          = 'check-version-sync.ps1'")
open(p, "w", encoding="utf-8").write(text)
PY
out="$(run_gate)"; code=$?
if [ "$code" -ne 0 ] && printf '%s' "$out" | grep -q "只接受 .sh / .mjs"; then
  ok "P6 非法落点扩展名被拦截（exit $code）"
else
  bad "P6 非法落点扩展名未拦截（exit $code）：$out"
fi

# P7：把转派语句「注释掉」→ 必须红（防 grep 命中注释造成的假绿）
seed
python3 - "$sandbox/scripts/clean-targets.ps1" <<'PY'
import sys
p = sys.argv[1]
lines = open(p, encoding="utf-8").read().split("\n")
lines = [("# " + l if "Invoke-PonyPosixTwin" in l and not l.strip().startswith("#") else l) for l in lines]
open(p, "w", encoding="utf-8").write("\n".join(lines))
PY
out="$(run_gate)"; code=$?
if [ "$code" -ne 0 ] && printf '%s' "$out" | grep -q "clean-targets"; then
  ok "P7 转派语句被注释掉仍被拦截（exit $code）"
else
  bad "P7 注释掉的转派语句未被拦截（exit $code）：$out"
fi

if [ "$FAIL" -ne 0 ]; then
  echo "check-ps1-portability.spec: FAILED"
  exit 1
fi
echo "check-ps1-portability.spec: PASS"
