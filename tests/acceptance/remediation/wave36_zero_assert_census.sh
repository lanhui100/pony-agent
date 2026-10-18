#!/usr/bin/env bash
# wave36_zero_assert_census.sh — PA-142 机械判据「测试函数体内零断言却报 ok」
#
# 背景(wave-36/PA-142):`path_permission.rs` 的 `win_prefix_normalization_strips_verbatim`
# 曾把两条断言包在 `if cfg!(windows) { … }` 里且**无 else 分支**。`cfg!` 是编译期常量,
# Linux 上函数体只剩 `.expect(…)`(无断言),rust 测试框架仍以 `test … ok` 上报 —— 等价于
# `#[ignore]` 但伪装成通过。本脚本把该形状做成**机械判据**,使其不可回退。
#
# 判据(对每个 `#[test]` / `#[tokio::test]` 函数):
#   V1 ZERO_ASSERTION_TEST        —— 函数体内断言原语计数为 0(纯执行不校验 = 假测试)。
#   V2 CFG_ONLY_ASSERTIONS        —— 所有断言都被 `if cfg!(…) { … }` 块包裹,且该块**无 else**
#                                    ⇒ 至少在某一平台上是「零断言报 ok」。
#   断言原语:assert!, assert_eq!, assert_ne!, assert_matches!, panic!, expect_err, unwrap_err。
#   `if cfg!(…) { return; }` 之类的平台早退(后续仍有断言)合法,不计 V2。
#
# 用法:
#   wave36_zero_assert_census.sh                 # 扫描 crates/pony-agent-core/src
#   wave36_zero_assert_census.sh FILE [FILE…]    # 只扫描给定文件(用于基线红相/局部核验)
#   wave36_zero_assert_census.sh --self-test     # 判据自测(含正反例)
#
# 退出码:0 = 无违规;1 = 有违规;3 = 用法/环境错误。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT" || { echo "FATAL: 无法进入仓库根 $REPO_ROOT"; exit 3; }
SRC_DEFAULT="crates/pony-agent-core/src"

python3 - "$@" <<'PY'
import os
import re
import subprocess
import sys
import tempfile

ASSERT_RE = re.compile(
    r"(?<![A-Za-z0-9_])("
    r"assert!|assert_eq!|assert_ne!|assert_matches!|panic!|expect_err|unwrap_err"
    r")"
)
# 断言委托:测试体调用以 assert_/check_/verify_/expect_ 命名的辅助函数(仓库既有风格,
# 如 `assert_denied(..)`、`assert_batch_child_rejected(..)`),其内部以 panic 断言。
DELEGATED_ASSERT_RE = re.compile(r"\b(?:assert|check|verify|expect)_[A-Za-z0-9_]*\s*\(")
# 真实失败原语(失败即 panic):`.expect(..)` / `.unwrap()`。它们不是断言,但能证明测试
# **不是**「纯运行不报错即放行」;只靠它们而无断言的用例记 WARNING(WEAK),不判违规。
FAILURE_CALL_RE = re.compile(r"\.(?:expect|unwrap)\s*\(")
TEST_ATTR_RE = re.compile(r"^#\[(?:tokio::)?test(?:\(.*\))?\]\s*$")
FN_RE = re.compile(r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)")
CFG_IF_RE = re.compile(r"\bif\s+cfg!\s*\(")

# 逐处判定后的显式豁免(精确到 文件::测试函数),每条必须带理由。
ALLOWLIST = {
    # 子进程测试辅助体:由同模块的父用例 spawn 后驱动,断言在父用例侧;
    # 该函数以 #[test] 标记仅为可被 `--exact` 单独拉起(PONY_AGENT_TEST_HELPER 门控)。
    "crates/pony-agent-core/src/agent/process.rs::helper_process":
        "subprocess test helper driven by parent case; assertions live in the spawning test",
}


def strip_noncode(line):
    """Remove // comments and simple string/char literal contents (byte-wise)."""
    out = []
    i = 0
    n = len(line)
    while i < n:
        c = line[i]
        if c == '/' and i + 1 < n and line[i + 1] == '/':
            break
        if c == '"':
            # consume string (with escapes); raw strings are rare in test code and
            # brace counting tolerates their contents.
            i += 1
            while i < n:
                if line[i] == '\\':
                    i += 2
                    continue
                if line[i] == '"':
                    i += 1
                    break
                i += 1
            out.append('""')
            continue
        if c == "'":
            # char literal: 'x' or '\n' (don't eat lifetimes like 'a)
            if i + 2 < n and line[i + 1] == '\\':
                j = i + 2
                while j < n and line[j] != "'":
                    j += 1
                if j < n:
                    out.append("' '")
                    i = j + 1
                    continue
            elif i + 2 < n and line[i + 2] == "'":
                out.append("' '")
                i += 3
                continue
        out.append(c)
        i += 1
    return ''.join(out)


def scan_file(path):
    """Return list of violations: (code, line_no, test_name, detail)."""
    try:
        raw = open(path, encoding="utf-8", errors="replace").read().split("\n")
    except OSError as exc:
        print(f"FATAL: 无法读取 {path}: {exc}", file=sys.stderr)
        sys.exit(3)

    code_lines = [strip_noncode(l) for l in raw]
    violations = []
    i = 0
    total = len(code_lines)
    while i < total:
        if not TEST_ATTR_RE.match(code_lines[i].strip()):
            i += 1
            continue
        # find the test fn signature (skip other attributes / blank lines)
        j = i + 1
        fn_name = None
        while j < total:
            t = code_lines[j].strip()
            if not t or t.startswith("#["):
                j += 1
                continue
            m = FN_RE.match(t)
            if m:
                fn_name = m.group(1)
                break
            j += 1
        if fn_name is None:
            i += 1
            continue

        # line numbers are 1-based in reports
        def_line = j + 1
        depth = 0
        seen_open = False
        total_asserts = 0
        cfg_only_asserts = 0
        failure_calls = 0
        cfg_block_no_else = 0
        stack = []  # each item: "cfg" | "normal"; "cfg" also carries no_else flag via list
        pending_cfg_if = False
        # tokens seen since last '{' / ';' on the current logical line, for `if cfg!(` detection
        k = j
        finished = False
        while k < total:
            line = code_lines[k]
            # macro-level assert counting (per line, whole body incl. cfg-guarded)
            found = ASSERT_RE.findall(line)
            delegated = DELEGATED_ASSERT_RE.findall(line)
            failure_calls += len(FAILURE_CALL_RE.findall(line))
            found_count = len(found) + len(delegated)
            if found_count:
                total_asserts += found_count
                if "cfg" in stack:
                    cfg_only_asserts += found_count
            # token scan for braces / else / cfg-if
            m_cfg = CFG_IF_RE.search(line)
            if m_cfg:
                pending_cfg_if = True
            p = 0
            while p < len(line):
                c = line[p]
                if c == '{':
                    kind = "cfg" if pending_cfg_if else "normal"
                    stack.append(kind)
                    pending_cfg_if = False
                    depth += 1
                    seen_open = True
                elif c == '}':
                    if stack:
                        kind = stack.pop()
                    else:
                        kind = "normal"
                    depth -= 1
                    if kind == "cfg" and depth >= 0:
                        # look ahead for `else` (possibly on this or next lines)
                        tail = line[p + 1:].strip()
                        if not tail.startswith("else"):
                            # peek following non-blank lines up to the next token
                            q = k + 1
                            while q < total:
                                nt = code_lines[q].strip()
                                if nt:
                                    if not nt.startswith("else"):
                                        cfg_block_no_else += 1
                                    break
                                q += 1
                            else:
                                cfg_block_no_else += 1
                    if seen_open and depth <= 0:
                        finished = True
                        break
                p += 1
            if finished:
                break
            k += 1

        allow_key = f"{path}::{fn_name}"
        if total_asserts == 0 and allow_key not in ALLOWLIST:
            if failure_calls == 0:
                violations.append((
                    "ZERO_ASSERTION_TEST",
                    def_line,
                    fn_name,
                    f"{path} 测试函数体无任何断言/断言委托/失败原语(纯执行 = 假测试)",
                ))
            else:
                print(
                    f"WARNING [WEAK_ASSERTION_TEST] {path} 仅以 expect/unwrap 作为失败原语、"
                    f"无显式断言(line {def_line}, fn {fn_name})"
                )
        elif cfg_only_asserts == total_asserts and cfg_block_no_else > 0:
            violations.append((
                "CFG_ONLY_ASSERTIONS",
                def_line,
                fn_name,
                f"{path} 全部 {total_asserts} 条断言被 if cfg!(…) 块包裹且无 else "
                f"⇒ 至少一个平台零断言报 ok",
            ))
        i = max(k, i + 1)
    return violations


def walk_sources(root):
    files = []
    for dirpath, _dirnames, filenames in os.walk(root):
        for name in sorted(filenames):
            if name.endswith(".rs"):
                files.append(os.path.join(dirpath, name))
    return sorted(files)


SELFTEST_FIXTURES = {
    "bad_zero.rs": """
#[cfg(test)]
mod tests {
    #[test]
    fn zero_assertion() {
        let value = 1 + 1;
        let _ = value;
    }
}
""",
    "bad_cfg_only.rs": """
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cfg_only_without_else() {
        let got = 2;
        if cfg!(windows) {
            assert_eq!(got, 2);
            assert!(got > 0);
        }
    }
}
""",
    "bad_zero_delegated_fake.rs": """
#[cfg(test)]
mod tests {
    #[test]
    fn delegated_but_real_assertions() {
        check_rejects("429");
        assert_denied("endpoint=127.0.0.1:42953/v1");
    }
}
""",
    "good_cfg_with_else.rs": """
#[cfg(test)]
mod tests {
    #[test]
    fn cfg_with_else() {
        let got = 2;
        if cfg!(windows) {
            assert_eq!(got, 2);
        } else {
            assert_eq!(got, 2);
        }
    }

    #[test]
    fn cfg_early_return_then_assert() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(2 + 2, 4);
    }
}
""",
}


def self_test():
    expected = {
        "bad_zero.rs": ["ZERO_ASSERTION_TEST"],
        "bad_cfg_only.rs": ["CFG_ONLY_ASSERTIONS"],
        "bad_zero_delegated_fake.rs": [],
        "good_cfg_with_else.rs": [],
    }
    failures = []
    with tempfile.TemporaryDirectory(prefix="wave36-census-selftest-") as tmp:
        for name, body in SELFTEST_FIXTURES.items():
            path = os.path.join(tmp, name)
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(body.lstrip("\n"))
            got = sorted(code for code, _ln, _fn, _d in scan_file(path))
            want = sorted(expected[name])
            if got != want:
                failures.append(f"{name}: want {want}, got {got}")
            else:
                print(f"SELFTEST  {name}: {got or 'no violation'} (expected)")
    if failures:
        for line in failures:
            print(f"SELFTEST_FAIL {line}", file=sys.stderr)
        print("SELFTEST RESULT: FAIL")
        return 1
    print("SELFTEST RESULT: PASS (V1/V2 正反例均按预期判定)")
    return 0


def main(argv):
    if argv and argv[0] == "--self-test":
        return self_test()
    if argv and argv[0] in ("-h", "--help"):
        print(__doc__ if "__doc__" in globals() else "see script header")
        return 0
    if any(arg.startswith("-") for arg in argv):
        print(f"FATAL: 未知参数 {argv}", file=sys.stderr)
        return 3

    files = list(argv) if argv else walk_sources("crates/pony-agent-core/src")
    violations = []
    for path in files:
        if not os.path.isfile(path):
            print(f"FATAL: 文件不存在 {path}", file=sys.stderr)
            return 3
        violations.extend(scan_file(path))

    for code, line_no, fn_name, detail in violations:
        print(f"VIOLATION [{code}] {detail} (line {line_no}, fn {fn_name})")
    if violations:
        print(f"RESULT: FAIL — {len(violations)} violation(s) / scanned {len(files)} file(s)")
        return 1
    print(f"RESULT: PASS — 0 violation / scanned {len(files)} file(s)")
    return 0


sys.exit(main(sys.argv[1:]))
PY
