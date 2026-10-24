#!/usr/bin/env python3
"""Wave43 Rust 空桩纯度机械校验（等价于 dev-stub-lint.py 的 Rust 侧实现）。

dev-stub-lint.py 只解析 .py/.ts/.js，对 .rs 物理不可用（实测：目录内无 py/ts/js
时输出 WARN 并 exit 0）。本脚本补齐 Rust 侧断言：

1. 每个 `fn` 体内有效语句行 <= 3；
2. 每行有效语句必须是 `unimplemented!("...")` 或 `Err("NotImplemented".into())`
   （或 `unimplemented!()`）；
3. 函数体内不得出现 `let ` / `return ` / `if ` / `match ` / `for ` / `while ` 等实现痕迹；
4. 每个 `pub` 项（fn/struct/enum/const）必须紧邻 `///` 文档注释（契约说明）。
退出码：0 通过，1 失败。
"""
import re
import sys
from pathlib import Path

MAX_BODY_LINES = 3
ALLOWED = (
    re.compile(r'^unimplemented!\(\s*"[^"]*"\s*\);$'),
    re.compile(r'^unimplemented!\(\);$'),
    re.compile(r'^Err\("NotImplemented"\.into\(\)\);$'),
)
FORBIDDEN = re.compile(
    r'\b(let |return |if |match |for |while |loop |unsafe |await |\.await)\b|\?;|=&|=\s*\w'
)
DECL = re.compile(
    r'^\s*pub(\s*\([^)]*\))?\s+(fn|struct|enum|const|static|trait|type|mod)\b'
)


def check_file(path: Path) -> list[str]:
    errors: list[str] = []
    lines = path.read_text(encoding="utf-8").splitlines()
    i = 0
    in_fn = False
    depth = 0
    body: list[str] = []
    fn_lineno = 0
    fn_name = ""
    while i < len(lines):
        raw = lines[i]
        stripped = raw.strip()

        # 4) pub 项必须有紧邻文档注释
        if DECL.match(raw):
            prev = lines[i - 1].strip() if i > 0 else ""
            if not (prev.startswith("///") or prev.startswith("//!") or prev.startswith("#[")):
                errors.append(f"{path}:{i+1}: pub 项缺少紧邻 doc 注释: {stripped}")

        if not in_fn:
            if re.search(r'\bfn\s+\w+', stripped) and "{" in stripped:
                in_fn = True
                fn_lineno = i + 1
                fn_name = re.search(r'fn\s+(\w+)', stripped).group(1)
                depth = stripped.count("{") - stripped.count("}")
                body = []
                if depth == 0:  # 单行签名体，如 `fn f() {}`
                    inner = stripped[stripped.index("{") + 1: stripped.rindex("}")].strip()
                    if inner:
                        body.append(inner)
                    in_fn = False
                    errors.extend(_judge(path, fn_lineno, fn_name, body))
            i += 1
            continue

        opened = stripped.count("{")
        closed = stripped.count("}")
        depth += opened - closed
        if depth == 0:
            content = stripped[: stripped.rindex("}")] if "}" in stripped else stripped
            if content.strip():
                body.append(content.strip())
            in_fn = False
            errors.extend(_judge(path, fn_lineno, fn_name, body))
        else:
            if stripped:
                body.append(stripped)
        i += 1

    if in_fn:
        errors.append(f"{path}: 函数 '{fn_name}' 括号未闭合（解析失败）")
    return errors


def _judge(path: Path, lineno: int, name: str, body: list[str]) -> list[str]:
    errors: list[str] = []
    effective = [
        line for line in body
        if line and not line.startswith("//") and line not in ("{", "}")
    ]
    if not effective:
        errors.append(f"{path}:{lineno}: 函数 '{name}' 体为空，必须 unimplemented!")
        return errors
    if len(effective) > MAX_BODY_LINES:
        errors.append(f"{path}:{lineno}: 函数 '{name}' 有效语句 {len(effective)} 行 > {MAX_BODY_LINES}")
    for line in effective:
        flat = " ".join(line.split())
        if not any(p.match(flat) for p in ALLOWED):
            errors.append(f"{path}:{lineno}: 函数 '{name}' 含非空桩语句: `{flat}`")
        if FORBIDDEN.search(flat):
            errors.append(f"{path}:{lineno}: 函数 '{name}' 含实现痕迹: `{flat}`")
    return errors


def main() -> int:
    targets = [Path(p) for p in sys.argv[1:]]
    files: list[Path] = []
    for t in targets:
        if t.is_file():
            files.append(t)
        elif t.is_dir():
            files.extend(sorted(t.rglob("*.rs")))
        else:
            print(f"Warning: 路径不存在: {t}", file=sys.stderr)
    if not files:
        print("[FAIL] 未找到 .rs 空桩文件", file=sys.stderr)
        return 1
    errors: list[str] = []
    for f in files:
        errors.extend(check_file(f))
    if errors:
        print(f"[FAIL] wave43-rust-stub-purity 发现 {len(errors)} 处违规:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    print(f"[PASS] wave43-rust-stub-purity 通过（{len(files)} 个 .rs 文件）")
    for f in files:
        print(f"  checked: {f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
