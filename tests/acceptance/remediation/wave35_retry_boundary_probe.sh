#!/usr/bin/env bash
# wave35_retry_boundary_probe.sh — retry.rs `strip_endpoint_fragments` char-boundary 变异敏感性实验
#
# 目的(Lead 指派):证明"多字节字符紧贴 URL ⇒ panic"这类 P0 的回归测试**真的敏感**,
# 而不是只在修好的代码上恒绿。做法:
#   1. 从当前 retry.rs **逐字提取**生产函数,并从其测试模块**逐字提取**含 `://` 的字符串字面量,
#      自动生成探针(输入 = 生产测试真实使用的输入);
#   2. AS-IS:断言全部输入不 panic;
#   3. MUTATED:把修法**机械退回**为 `rfind(...) + (index + 1)` 字节切片,断言出现 panic ⇒ 敏感;
#   4. 逐输入给出 SENSITIVE / INSENSITIVE 明细,用于判断回归测试覆盖面。
#
# 用法: bash tests/acceptance/remediation/wave35_retry_boundary_probe.sh [retry.rs 路径]
set -uo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RETRY="${1:-$REPO_ROOT/crates/pony-agent-core/src/agent/retry.rs}"
OUT_DIR="$REPO_ROOT/tests/acceptance/receipts/wave35"
STAMP="$(date +%Y%m%dT%H%M%S)"
CRATE="$(mktemp -d /tmp/w35retryprobe-XXXXXX)"
LOG="$OUT_DIR/retry_boundary_probe_${STAMP}.log"
cleanup() { rm -rf "$CRATE"; rm -f /tmp/.probe_sens; }
trap cleanup EXIT

{
  echo "# retry boundary mutation probe @ $(date -Is)"
  echo "# retry.rs = $RETRY"
  echo "# retry.rs sha256 = $(sha256sum "$RETRY" | cut -d' ' -f1)"
  echo "# HEAD = $(cd "$REPO_ROOT" && git rev-parse HEAD)"
  echo "# retry.rs dirty = [$(cd "$REPO_ROOT" && git status --porcelain -- crates/pony-agent-core/src/agent/retry.rs | tr -d '\n')]"
} > "$LOG"

mkdir -p "$CRATE/src"
cat > "$CRATE/Cargo.toml" <<'TOML'
[package]
name = "w35retryprobe"
version = "0.0.0"
edition = "2021"
[dependencies]
TOML

sed -n '/^fn strip_endpoint_fragments/,/^}/p' "$RETRY" > "$CRATE/fn.rs"
grep -q 'strip_endpoint_fragments' "$CRATE/fn.rs" || { echo "FATAL: 未能提取生产函数" | tee -a "$LOG"; exit 3; }

python3 - "$RETRY" "$CRATE/fn.rs" "$CRATE/src/lib.rs" <<'PY' >> "$LOG" 2>&1
import re, sys
retry, fnf, outf = sys.argv[1], sys.argv[2], sys.argv[3]
src = open(retry, encoding="utf-8").read()
li = src.find("#[cfg(test)]")
region = src[li:] if li > 0 else src
# 逐字取自生产测试的字面量(不再二次转义:Rust 源里的转义序列原样嵌入即为同值)
lits = re.findall(r'"((?:[^"\\]|\\.)*://(?:[^"\\]|\\.)*)"', region)
extra = re.findall(r'format!\(\s*\n?\s*"((?:[^"\\]|\\.)*)"', region)
cands, seen = [], set()
for s in lits + extra:
    if s not in seen:
        seen.add(s); cands.append(s)
out = [open(fnf, encoding="utf-8").read(), "\n#[cfg(test)]\nmod probe {\n    use super::strip_endpoint_fragments;\n"]
for i, s in enumerate(cands, 1):
    out.append('\n    #[test]\n    fn auto_%d() {\n        let input = "%s".replace("{port}", "42953").to_ascii_lowercase();\n        let _ = strip_endpoint_fragments(&input);\n    }\n' % (i, s))
out.append('''
    #[test]
    fn unicode_cjk_adjacent() {
        let _ = strip_endpoint_fragments(&"上游校验失败:错误https://127.0.0.1:42953/v1".to_ascii_lowercase());
    }

    #[test]
    fn unicode_fullwidth_colon_adjacent() {
        let _ = strip_endpoint_fragments(&"上游校验失败:错误：https://127.0.0.1:42953/v1".to_ascii_lowercase());
    }

    #[test]
    fn unicode_astral_adjacent() {
        let _ = strip_endpoint_fragments(&"上游校验失败:🔒https://127.0.0.1:42953/v1".to_ascii_lowercase());
    }
''')
out.append("}\n")
open(outf, "w", encoding="utf-8").write("".join(out))
print("extracted_test_literals=%d (auto_1..auto_%d) + 3 unicode-adjacent probes" % (len(cands), len(cands)))
PY
[ $? -eq 0 ] || { echo "FATAL: 探针生成失败" | tee -a "$LOG"; exit 3; }

run_probe() { # $1=标签  $2=结果落盘
  echo "" >> "$LOG"; echo "================ $1 ================" >> "$LOG"
  ( cd "$CRATE" && cargo test -- --test-threads=1 2>&1 ) >> "$LOG"
  local rc=$?
  echo "EXIT=$rc" >> "$LOG"
  grep -E '^test result:' "$LOG" | tail -1 > "$CRATE/summary.txt"
  echo "[$1] exit=$rc ; $(awk '/^test result:/{r=$0} END{print r}' "$CRATE/summary.txt" 2>/dev/null)"
  grep -E '^test probe::[a-z0-9_]+ \.\.\. (ok|FAILED)$' "$LOG" | tail -60 > "$2"
  return $rc
}

run_probe "AS-IS(当前 retry.rs 修法)" "$CRATE/asis.txt"; RC_ASIS=$?

SHA_BEFORE=$(sha256sum "$CRATE/src/lib.rs" | cut -d' ' -f1)
python3 - "$CRATE/src/lib.rs" "$SHA_BEFORE" <<'PY' >> "$LOG" 2>&1
import hashlib, re, sys
p, before = sys.argv[1], sys.argv[2]
s = open(p, encoding="utf-8").read()
buggy = ("let head = &rest[..scheme_sep];\n"
         "        let scheme_start = head\n"
         "            .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))\n"
         "            .map(|index| index + 1)\n"
         "            .unwrap_or(0);\n"
         "        out.push_str(&rest[..scheme_start]);")
new, n = re.subn(r'out\.push_str\(&rest\[\.\.scheme_sep\]\);', buggy, s, count=1)
if n == 0:
    new, n = re.subn(r'let scheme_start = head.*?\.unwrap_or\(0\);',
                     buggy.replace("let head = &rest[..scheme_sep];\n        ", ""), s, count=1, flags=re.S)
if n:
    with open(p, "w", encoding="utf-8") as fh:   # 关键:必须真正落盘
        fh.write(new)
        fh.flush()
after = hashlib.sha256(open(p, "rb").read()).hexdigest()
applied = (n == 1) and (after != before) and ("index + 1" in open(p, encoding="utf-8").read())
print("MUTATION_SUBSTITUTIONS=%d" % n)
print("SHA before=%s after=%s changed=%s" % (before[:16], after[:16], after != before))
print("MUTATION_APPLIED" if applied else "MUTATION_NOT_APPLICABLE(落盘后校验失败)")
sys.exit(0 if applied else 3)
PY
MUT_RC=$?
[ "$MUT_RC" -eq 0 ] || { echo "PROBE: MUTATION_NOT_APPLICABLE(变异未落盘或未通过落盘后校验,见日志)" | tee -a "$LOG"; echo "raw: $LOG"; exit 3; }
run_probe "MUTATED(退回 index+1 字节切片)" "$CRATE/mut.txt"; RC_MUT=$?

python3 - "$CRATE/asis.txt" "$CRATE/mut.txt" "$LOG" <<'PY'
import re, sys
def load(p):
    d = {}
    for line in open(p, encoding="utf-8"):
        m = re.match(r'^test probe::([a-z0-9_]+) \.\.\. (ok|FAILED)$', line.strip())
        if m: d[m.group(1)] = m.group(2)
    return d
a, b = load(sys.argv[1]), load(sys.argv[2])
sens = [k for k in sorted(set(a) | set(b)) if a.get(k) != b.get(k)]
insens = [k for k in sorted(set(a) | set(b)) if a.get(k) == b.get(k)]
with open(sys.argv[3], "a", encoding="utf-8") as f:
    f.write("\n---- 敏感性对照(AS-IS vs MUTATED) ----\n")
    f.write("SENSITIVE(变异后 ok→FAILED): %s\n" % (", ".join(sens) or "(无)"))
    f.write("INSENSITIVE(两态相同): %s\n" % (", ".join(insens) or "(无)"))
open("/tmp/.probe_sens", "w").write(",".join(sens))
print("SENSITIVE=%s" % (",".join(sens) or "NONE"))
print("INSENSITIVE=%s" % (",".join(insens) or "NONE"))
PY

SENS=$(cat /tmp/.probe_sens 2>/dev/null)
echo "" >> "$LOG"; echo "-------------------------------------------------------------" >> "$LOG"
echo "AS-IS exit=$RC_ASIS / MUTATED exit=$RC_MUT / SENSITIVE=$SENS" >> "$LOG"
if [ "$RC_ASIS" -ne 0 ]; then
  echo "PROBE: FAIL —— 当前 retry.rs 探针未过(仍存在 panic 类缺陷)" | tee -a "$LOG"; echo "raw: $LOG"; exit 1
fi
if printf '%s' "$SENS" | grep -q 'unicode_'; then
  echo "PROBE: PASS —— 修法下全绿;退回字节切片后 unicode 探针 panic ⇒ 该类回归可被捕获" | tee -a "$LOG"
  echo "raw: $LOG"; exit 0
fi
if [ "$RC_MUT" -ne 0 ]; then
  echo "PROBE: PARTIAL —— 变异后失败但失败者非 unicode 探针(多字节紧贴 URL 场景缺覆盖)" | tee -a "$LOG"
  echo "raw: $LOG"; exit 4
fi
echo "PROBE: INSENSITIVE —— 退回字节切片后探针仍全绿:现有测试无法捕获该 P0" | tee -a "$LOG"
echo "raw: $LOG"; exit 2