#!/usr/bin/env python3
"""C5 锁纪律静态校验器（graph_runs 非重入锁作用域内禁止二次加锁调用）。

契约 C5（冻结于 .dev-team/contract-matrix-deadlock.md）：
    任何 `self.graph_runs.lock()` 的作用域内，不得再调用会二次加锁的方法：
      - resolve_graph_run_for_retrieval
      - load_graph_run
      - load_execution_checkpoint
      - load_active_graph_run_for_session
    以及不得在同一作用域内再次出现 `self.graph_runs.lock()`（直接自死锁）。

判定语义（机械、无启发式）：
  1. 剥离注释与字符串字面量（按字符等长替换为空格，保证行号/索引对齐）。
  2. 对每个 `self.graph_runs.lock()` 出现位置，用花括号配对求其“最内层封闭块”作为作用域。
  3. 在该作用域文本内搜索 `self.<forbidden>(` 调用；命中即 ERROR。
  4. 作用域内再出现 `self.graph_runs.lock()` 亦为 ERROR（同一作用域内二次加锁）。

退出码：
  0 = 无阻塞级违规（GREEN）
  1 = 存在阻塞级违规（RED）
  2 = 用法/文件错误

分级（最小闭包原则，见 contract-matrix-deadlock.md C5）：
  ERROR(阻塞) = 死锁路径文件（history_commands.rs）内的违规 → 本次修复必须清零
  P1(登记)    = 其它文件内的同类违规 → 登记为遗留项，不阻塞本次放行
  --strict    = 把 P1 也升级为阻塞（用于后续同类清理门禁）
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

LOCK_PATTERN = "self.graph_runs.lock()"
FORBIDDEN = (
    "resolve_graph_run_for_retrieval",
    "load_graph_run",
    "load_execution_checkpoint",
    "load_active_graph_run_for_session",
)
DEFAULT_ROOT = "crates/pony-agent-core/src/agent/control_plane"
# 本次「进入会话页卡死」死锁路径（load_session_runtime_view → resolve_graph_run_submission_plan）
BLOCKING_FILES = ("history_commands.rs",)


def strip_noncode(src: str) -> str:
    """把注释/字符串字面量替换为等长空格，保留全部偏移量与换行。"""
    out = list(src)
    i = 0
    n = len(src)

    def blank(start: int, end: int) -> None:
        for k in range(start, min(end, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        # 行注释
        if c == "/" and nxt == "/":
            j = src.find("\n", i)
            j = n if j == -1 else j
            blank(i, j)
            i = j
            continue
        # 块注释（Rust 支持嵌套）
        if c == "/" and nxt == "*":
            depth = 0
            j = i
            while j < n:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                    if depth == 0:
                        break
                else:
                    j += 1
            blank(i, j)
            i = j
            continue
        # 原始字符串 r"..." / r#"..."# / br#"..."#
        m = re.match(r'(?:b)?r(#*)"', src[i:])
        if m:
            hashes = m.group(1)
            close = '"' + hashes
            j = src.find(close, i + len(m.group(0)))
            j = n if j == -1 else j + len(close)
            blank(i, j)
            i = j
            continue
        # 普通字符串 / 字节串
        if c == '"' or (c == "b" and nxt == '"'):
            j = i + (2 if c == "b" else 1)
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            blank(i, j)
            i = j
            continue
        # 字符字面量 / 生命周期：仅剥离明确的 'x' 形式，避免误伤生命周期
        if c == "'":
            m = re.match(r"'(?:\\.|[^\\'])'", src[i:])
            if m:
                blank(i, i + len(m.group(0)))
                i += len(m.group(0))
                continue
        i += 1
    return "".join(out)


def depth_map(code: str) -> list[int]:
    """depth[i] = 处理字符 i 之前的块深度。"""
    depth = [0] * (len(code) + 1)
    d = 0
    for i, ch in enumerate(code):
        depth[i] = d
        if ch == "{":
            d += 1
        elif ch == "}":
            d = max(0, d - 1)
    depth[len(code)] = d
    return depth


def enclosing_block(code: str, depth: list[int], idx: int) -> tuple[int, int] | None:
    """返回包含 idx 的最内层花括号块 (open_idx, close_idx)。"""
    target = depth[idx] - 1
    if target < 0:
        return None
    open_idx = None
    for j in range(idx - 1, -1, -1):
        if code[j] == "{" and depth[j] == target:
            open_idx = j
            break
    if open_idx is None:
        return None
    d = 0
    for j in range(open_idx, len(code)):
        if code[j] == "{":
            d += 1
        elif code[j] == "}":
            d -= 1
            if d == 0:
                return open_idx, j
    return open_idx, len(code) - 1


def line_of(text: str, idx: int) -> int:
    return text.count("\n", 0, idx) + 1


def scan_file(path: Path) -> list[dict]:
    raw = path.read_text(encoding="utf-8")
    code = strip_noncode(raw)
    depth = depth_map(code)
    findings: list[dict] = []

    sites: list[dict] = []
    for m in re.finditer(re.escape(LOCK_PATTERN), code):
        block = enclosing_block(code, depth, m.start())
        if block is None:
            findings.append(
                {
                    "line": line_of(raw, m.start()),
                    "kind": "UNSCOPED_LOCK",
                    "detail": f"{LOCK_PATTERN} 未找到封闭块，无法判定作用域",
                }
            )
            continue
        open_idx, close_idx = block
        sites.append({"start": m.start(), "open": open_idx, "close": close_idx})

    # 同一封闭块内出现 ≥2 次 self.graph_runs.lock() → 第 2 次起为同作用域二次加锁
    by_block: dict[int, list[dict]] = {}
    for site in sites:
        by_block.setdefault(site["open"], []).append(site)

    for open_idx, group in by_block.items():
        scope = code[open_idx + 1 : group[0]["close"]]
        for name in FORBIDDEN:
            for call in re.finditer(r"self\s*\.\s*" + re.escape(name) + r"\s*\(", scope):
                abs_idx = open_idx + 1 + call.start()
                findings.append(
                    {
                        "line": line_of(raw, abs_idx),
                        "kind": "REENTRANT_CALL",
                        "detail": (
                            f"锁作用域（自 L{line_of(raw, open_idx)} 起）内调用 self.{name}()，"
                            "该方法会再次 self.graph_runs.lock() → 同线程自死锁"
                        ),
                    }
                )
        for extra in sorted(group, key=lambda x: x["start"])[1:]:
            findings.append(
                {
                    "line": line_of(raw, extra["start"]),
                    "kind": "NESTED_LOCK",
                    "detail": (
                        f"同一锁作用域（自 L{line_of(raw, open_idx)} 起）内二次出现 "
                        f"{LOCK_PATTERN} → 直接自死锁"
                    ),
                }
            )
    return findings


def main() -> int:
    ap = argparse.ArgumentParser(description="C5 graph_runs 锁纪律静态校验")
    ap.add_argument("--root", default=DEFAULT_ROOT, help="扫描根目录")
    ap.add_argument("--quiet-ok", action="store_true", help="无违规时不打印逐文件行")
    ap.add_argument(
        "--strict",
        action="store_true",
        help="把 P1（非死锁路径文件）也升级为阻塞级",
    )
    args = ap.parse_args()

    root = Path(args.root)
    if not root.is_dir():
        print(f"[check-lock-discipline] ERROR: 扫描根目录不存在: {root}", file=sys.stderr)
        return 2

    files = sorted(root.rglob("*.rs"))
    if not files:
        print(f"[check-lock-discipline] ERROR: {root} 下未找到 .rs 文件", file=sys.stderr)
        return 2

    blocking = 0
    registered = 0
    lock_sites = 0
    for f in files:
        raw = f.read_text(encoding="utf-8")
        lock_sites += raw.count(LOCK_PATTERN)
        findings = scan_file(f)
        is_blocking_file = f.name in BLOCKING_FILES
        file_blocking = [
            x for x in findings if is_blocking_file or args.strict
        ]
        file_registered = [x for x in findings if not (is_blocking_file or args.strict)]
        if findings:
            if file_blocking:
                blocking += len(file_blocking)
                print(f"FAIL(ERROR) {f}")
                for item in sorted(file_blocking, key=lambda x: x["line"]):
                    print(f"  - L{item['line']} [{item['kind']}] {item['detail']}")
            if file_registered:
                registered += len(file_registered)
                print(f"WARN(P1)    {f}  # 同类遗留项，不阻塞本次放行")
                for item in sorted(file_registered, key=lambda x: x["line"]):
                    print(f"  - L{item['line']} [{item['kind']}] {item['detail']}")
        elif not args.quiet_ok:
            print(f"ok          {f}")

    print(
        f"[check-lock-discipline] scanned_files={len(files)} "
        f"lock_sites={lock_sites} blocking_violations={blocking} "
        f"registered_p1={registered}"
    )
    if blocking:
        print("[check-lock-discipline] RESULT: RED (C5 blocking violation present)")
        return 1
    if registered:
        print("[check-lock-discipline] RESULT: GREEN (C5 blocking satisfied; P1 items registered)")
        return 0
    print("[check-lock-discipline] RESULT: GREEN (C5 satisfied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
