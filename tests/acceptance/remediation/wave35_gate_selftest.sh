#!/usr/bin/env bash
# wave35_gate_selftest.sh — 对 wave35_gate.sh 的红队自证:证明门禁不是橡皮图章。
#
# 在 f70b1f3 的临时 detached worktree 里注入"改断言口径/绕过"作弊变体,
# 断言 wave35_gate.sh --static-only 必须逐条报出对应违规类别;缺任何一类 → 非零退出。
# 只写 /tmp 与 git worktree 元数据,绝不触碰被测仓库工作区。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT" || exit 3
GATE="tests/acceptance/remediation/wave35_gate.sh"
BASE_REV="${1:-f70b1f3}"
TARGET="crates/pony-agent-core/src/agent/document_conversion.rs"
PASS=0; FAIL=0
WT_A="$(mktemp -d /tmp/w35-selfA-XXXXXX)"; WT_B="$(mktemp -d /tmp/w35-selfB-XXXXXX)"
cleanup() {
  for w in "$WT_A" "$WT_B"; do git worktree remove --force "$w" >/dev/null 2>&1 || true; rm -rf "$w"; done
}
trap cleanup EXIT

newtree() { # $1 = 路径
  git worktree add --detach "$1" "$BASE_REV" >/dev/null 2>&1 || { echo "FATAL: worktree 创建失败 $1"; exit 3; }
  mkdir -p "$1/tests/acceptance/remediation"
  cp "$GATE" "$1/tests/acceptance/remediation/wave35_gate.sh"
}

run_gate() { ( cd "$1" && bash tests/acceptance/remediation/wave35_gate.sh --static-only ) 2>&1; }
expect() { # $1=输出文件 $2=期望违规类别
  if grep -q "VIOLATION \[$2\]" "$1"; then echo "  OK   捕获 $2"; PASS=$((PASS+1))
  else echo "  HOLE 未捕获 $2"; FAIL=$((FAIL+1)); fi
}

# ───────────────────────────── 场景 A:12 类作弊同时注入 ─────────────────────────────
newtree "$WT_A"
python3 - "$WT_A/$TARGET" <<'PY'
import re, sys
p = sys.argv[1]; src = open(p, encoding="utf-8").read()

def body_range(text, name):
    m = re.search(r"[ \t]*#\[test\]\n[ \t]*fn " + name + r"\([^\n]*\{", text)
    assert m, "anchor missing: " + name
    i = text.index("{", m.start()); depth = 0; j = i
    while j < len(text):
        if text[j] == "{": depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0: break
        j += 1
    return m.start(), i, j

# (1) assert_eq! → assert!(a == b)  用户点名的"改断言口径"绕过
old = 'assert_eq!(result.format.as_deref(), Some("csv"));'
assert old in src, "anchor1"
src = src.replace(old, 'assert!(result.format.as_deref() == Some("csv"));', 1)
# (2) 测试区内新增 let _ = 吞异常
marker = 'assert!(result.format.as_deref() == Some("csv"));'
src = src.replace(marker, marker + "\n        let _ = 0;", 1)

# (3) 抽空既有长测试体:保留断言,删掉其余代码行 → GUTTED_TEST
s0, i0, j0 = body_range(src, "truncates_output_with_evidence")
body = src[i0+1:j0]
kept = "\n".join(l for l in body.split("\n") if "assert" in l)
src = src[:i0+1] + "\n" + kept + "\n    " + src[j0:]

# (4) 整个删除一个既有测试函数 → TEST_FN_REMOVED / TEST_NAME_LOST
s0, i0, j0 = body_range(src, "handler_parses_arguments_and_returns_structured_json")
src = src[:s0] + src[j0+1:]

# (5) 追加作弊模块
src += '''
#[cfg(test)]
mod wave35_selftest_cheats {
    #[test]
    fn selftest_zero_assert() { let _x = 2 + 2; }

    #[test]
    #[ignore]
    fn selftest_ignored() { assert!(true); }

    #[test]
    fn selftest_tautology() { let v = vec![1u8]; assert!(v.len() == 1 || true); }

    fn check_ok(_v: u8) {}

    #[test]
    fn selftest_unwrap_or_true() { let r: Result<u8, ()> = Err(()); assert!(r.is_err()); assert!(r.map_err(|_| 1).unwrap_or(true)); }
}

#[cfg(not(test))]
mod wave35_selftest_hidden {}
'''
open(p, "w", encoding="utf-8").write(src)
print("SELFTEST_MUTATIONS_OK")
PY
[ $? -eq 0 ] || { echo "FATAL: 场景 A 注入失败"; exit 3; }
OUT_A="$(mktemp)"; run_gate "$WT_A" > "$OUT_A"; RC_A=$?
echo "场景 A: gate exit=$RC_A(期望 1);违规类别:"
grep -oE '^VIOLATION \[[A-Z_]+\]' "$OUT_A" | sort | uniq -c | sed 's/^/    /'
for c in NEW_IGNORE NEW_CFG_NOT_TEST VACUOUS_ASSERT TAUTOLOGY_ASSERT UNWRAP_OR_TRUE \
         ZERO_ASSERT_TEST EMPTY_GUARD_HELPER LET_UNDERSCORE_GROWTH ASSERT_CENSUS_DROP \
         TEST_FN_REMOVED GUTTED_TEST TEST_NAME_LOST; do expect "$OUT_A" "$c"; done
[ "$RC_A" -ne 0 ] || { echo "  HOLE 注入作弊后 gate 仍返回 0(橡皮图章)"; FAIL=$((FAIL+1)); }

# ─────────────────── 场景 B:仅加 #[cfg(windows)] 而无等价新增测试 ───────────────────
newtree "$WT_B"
python3 - "$WT_B/$TARGET" <<'PY'
import sys
p = sys.argv[1]; src = open(p, encoding="utf-8").read()
old = "    #[test]\n    fn csv_converts_to_markdown_with_detected_format()"
assert old in src, "anchorB"
src = src.replace(old, "    #[cfg(windows)]\n" + old, 1)
open(p, "w", encoding="utf-8").write(src)
print("SELFTEST_MUTATION_B_OK")
PY
[ $? -eq 0 ] || { echo "FATAL: 场景 B 注入失败"; exit 3; }
OUT_B="$(mktemp)"; run_gate "$WT_B" > "$OUT_B"; RC_B=$?
echo "场景 B: gate exit=$RC_B(期望 1)"
grep -E '^VIOLATION|^WARNING' "$OUT_B" | sed 's/^/    /'
expect "$OUT_B" "CFG_WINDOWS_UNBALANCED"
[ "$RC_B" -ne 0 ] || { echo "  HOLE 平台门禁失衡未判 FAIL"; FAIL=$((FAIL+1)); }

rm -f "$OUT_A" "$OUT_B"

# ────────── 场景 C:用 #[allow(unused_imports)] 掩盖 unused-import(Lead 裁决 c 不接受) ──────────
WT_C="$(mktemp -d /tmp/w35-selfC-XXXXXX)"
trap 'for w in "$WT_A" "$WT_B" "$WT_C"; do git worktree remove --force "$w" >/dev/null 2>&1 || true; rm -rf "$w"; done' EXIT
newtree "$WT_C"
python3 - "$WT_C/crates/pony-agent-core/tests/tool_surface_wave20_contract.rs" <<'PY'
import sys
p = sys.argv[1]; src = open(p, encoding="utf-8").read()
old = "use pony_agent_core::agent::tools::{\n    builtin_turn_tool_contract_views, canonical_tool_name, ToolRegistrySnapshot,\n};"
assert old in src, "anchorC"
src = src.replace(old, "#[allow(unused_imports)]\n" + old, 1)
open(p, "w", encoding="utf-8").write(src)
print("SELFTEST_MUTATION_C_OK")
PY
[ $? -eq 0 ] || { echo "FATAL: 场景 C 注入失败"; exit 3; }
OUT_C="$(mktemp)"; run_gate "$WT_C" > "$OUT_C"; RC_C=$?
echo "场景 C: gate exit=$RC_C(期望 1)"
grep -E '^VIOLATION' "$OUT_C" | sort -u | sed 's/^/    /'
expect "$OUT_C" "TOOL_SURFACE_ALLOW_ATTR"
[ "$RC_C" -ne 0 ] || { echo "  HOLE #[allow] 消警未被判 FAIL"; FAIL=$((FAIL+1)); }
rm -f "$OUT_C"

expect_absent() { # $1=输出文件 $2=不应出现的违规类别
  if grep -q "VIOLATION \[$2\]" "$1"; then echo "  HOLE 误报 $2(合法改动被判违规)"; FAIL=$((FAIL+1))
  else echo "  OK   未误报 $2"; PASS=$((PASS+1)); fi
}

# ───────── 场景 D:编译兼容衍生豁免(只删导入=PASS;非导入改动=FAIL) ─────────
WT_D="$(mktemp -d /tmp/w35-selfD-XXXXXX)"
trap 'for w in "$WT_A" "$WT_B" "$WT_C" "$WT_D"; do git worktree remove --force "$w" >/dev/null 2>&1 || true; rm -rf "$w"; done' EXIT
newtree "$WT_D"
LSP="$WT_D/crates/pony-agent-core/tests/wave31_lsp_client_test.rs"
python3 - "$LSP" <<'PY'
import sys
p = sys.argv[1]; src = open(p, encoding="utf-8").read()
a = "use pony_agent_core::agent::lsp::{\n    LspClient, LspPosition, LspRange, LspServerSpec,\n};"
b = "use pony_agent_core::agent::lsp::{\n    LspClient, LspPosition, LspServerSpec,\n};"
assert a in src and "use serde_json::json;\n" in src, "anchorD"
src = src.replace(a, b, 1).replace("use serde_json::json;\n", "", 1)
open(p, "w", encoding="utf-8").write(src)
print("SELFTEST_MUTATION_D1_OK(仅删 2 个未使用导入)")
PY
[ $? -eq 0 ] || { echo "FATAL: 场景 D1 注入失败"; exit 3; }
OUT_D1="$(mktemp)"; run_gate "$WT_D" > "$OUT_D1"; RC_D1=$?
echo "场景 D1(合法:仅删导入): gate exit=$RC_D1(期望 0)"
expect_absent "$OUT_D1" "DERIVED_NON_IMPORT_CHANGE"
expect_absent "$OUT_D1" "DERIVED_CENSUS_CHANGED"
[ "$RC_D1" -eq 0 ] || { echo "  HOLE 合法删导入被判 FAIL(假阳性)"; FAIL=$((FAIL+1)); }

python3 - "$LSP" <<'PY'
import sys, re
p = sys.argv[1]; src = open(p, encoding="utf-8").read()
m = re.search(r"assert!\(", src)
assert m, "anchorD2"
# 非导入的隐蔽改动:在既有断言上叠加恒真操作数
src = src[:m.end()] + "true || " + src[m.end():]
open(p, "w", encoding="utf-8").write(src)
print("SELFTEST_MUTATION_D2_OK(叠加非导入改动)")
PY
[ $? -eq 0 ] || { echo "FATAL: 场景 D2 注入失败"; exit 3; }
OUT_D2="$(mktemp)"; run_gate "$WT_D" > "$OUT_D2"; RC_D2=$?
echo "场景 D2(非法:非导入改动): gate exit=$RC_D2(期望 1)"
expect "$OUT_D2" "DERIVED_NON_IMPORT_CHANGE"
[ "$RC_D2" -ne 0 ] || { echo "  HOLE 非导入改动未被判 FAIL"; FAIL=$((FAIL+1)); }
rm -f "$OUT_D1" "$OUT_D2"

echo "-------------------------------------------------------------"
if [ "$FAIL" -gt 0 ]; then echo "SELFTEST: FAIL — 门禁存在 $FAIL 类盲区(见 HOLE)"; exit 1; fi
echo "SELFTEST: PASS — $PASS/$PASS 项全通过(A/B/C 注入全部捕获 + D1 无假阳性 + D2 非导入改动被捕获)"
exit 0