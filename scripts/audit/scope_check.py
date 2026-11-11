#!/usr/bin/env python3
"""写域越界检查（PA-169 C8 的实现体）。

判据：`git diff --name-only <baseline> <candidate>` 的每个路径必须
  - 落在 loop-state.current.write_scope 的任一路径前缀内，或
  - 属于审计自身产物白名单（docs/audit/**、.agents/skills/trace-audit-loop/**、
    scripts/audit/**、管理看板三件），或
  - 命中提案里显式声明的 `write_scope_extra`。

退出码：0 = 全部在写域内｜1 = 存在越界路径｜2 = 检查不可执行（git 失败/缺字段）
"""
import argparse
import json
import subprocess
import sys

AUDIT_WHITELIST_PREFIXES = (
    "docs/audit/",
    "scripts/audit/",
    ".agents/skills/trace-audit-loop/",
    "docs/archive/task-system/",
    ".agents/notes/",
)


def git(repo, *args):
    return subprocess.run(
        ["git", "-C", repo, *args], capture_output=True, text=True, check=False
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--state", required=True)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", default="")
    parser.add_argument("--paths-file", default="",
                        help="改用外部给定的变更路径清单（每行一个），用于并行 WIP 污染工作区时")
    parser.add_argument("--repo", default=".")
    args = parser.parse_args()

    try:
        state = json.load(open(args.state, encoding="utf-8"))
    except Exception as error:
        print(f"   FAIL 状态文件不可解析：{error}", file=sys.stderr)
        return 2
    current = state.get("current") or {}
    write_scope = [item for item in (current.get("write_scope") or []) if item]
    write_scope_extra = [item for item in (current.get("write_scope_extra") or []) if item]

    if args.paths_file:
        with open(args.paths_file, encoding="utf-8") as handle:
            paths = [line.strip() for line in handle if line.strip()]
    else:
        if not args.candidate:
            print("   FAIL 需要 --candidate 或 --paths-file", file=sys.stderr)
            return 2
        diff = git(args.repo, "diff", "--name-only", f"{args.baseline}..{args.candidate}")
        if diff.returncode != 0:
            print(f"   FAIL git diff 失败：{diff.stderr.strip()}", file=sys.stderr)
            return 2
        paths = [line.strip() for line in diff.stdout.splitlines() if line.strip()]
    if not paths:
        print("   PASS 空 diff（无改动）")
        return 0

    if not write_scope:
        print("   FAIL 提案未声明 write_scope（无法判定越界；批准时须冻结写域）", file=sys.stderr)
        return 2

    violations = []
    for path in paths:
        if any(path.startswith(prefix) for prefix in AUDIT_WHITELIST_PREFIXES):
            continue
        if any(path == scope or path.startswith(scope.rstrip("/") + "/") for scope in write_scope):
            continue
        if any(path == scope or path.startswith(scope.rstrip("/") + "/") for scope in write_scope_extra):
            continue
        violations.append(path)

    print(f"   changed={len(paths)} scope_entries={len(write_scope)} violations={len(violations)}")
    for path in violations:
        print(f"     - 越界: {path}")
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
