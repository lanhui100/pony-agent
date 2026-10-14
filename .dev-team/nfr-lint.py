#!/usr/bin/env python3
"""NFR 基准机械校验器（本次「进入会话页卡死」修复的并发锁 / 有界等待承诺门禁）。

等价替代 `dev-nfr-lint.py`（该脚本在本仓库不存在，见 .dev-team/contract-matrix-deadlock.md 物理收据）。

校验内容：
  1. .dev-team/nfr-baseline.json 必须通过 .dev-team/nfr-baseline.schema.json（Draft-07）；
  2. 未知字段拒绝（additionalProperties=false，逐层递归检查，含嵌套对象）；
  3. 有界等待硬约束：
       lock_scope_bounded            == true
       lock_reentrancy_forbidden     == true
       1 <= lock_acquisition_timeout_ms < deadlock_watchdog_timeout_ms <= 5000
       lock_forbidden_calls 必须覆盖 C5 四个禁止二次加锁方法。
  4. --self-test：注入 5 个反例，证明该校验器不是恒真（零假测试）。

退出码：0 = 全部通过；1 = 校验失败；2 = 用法/文件错误。
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    import jsonschema
    from jsonschema import Draft7Validator
except ImportError:  # pragma: no cover
    print("[nfr-lint] ERROR: 需要 python3-jsonschema (pip install jsonschema)", file=sys.stderr)
    raise SystemExit(2)

BASE = Path(__file__).resolve().parent
DEFAULT_SCHEMA = BASE / "nfr-baseline.schema.json"
DEFAULT_INSTANCE = BASE / "nfr-baseline.json"

REQUIRED_FORBIDDEN = (
    "resolve_graph_run_for_retrieval",
    "load_graph_run",
    "load_execution_checkpoint",
    "load_active_graph_run_for_session",
)
WATCHDOG_MAX_MS = 5000


def jsonschema_version() -> str:
    try:
        from importlib.metadata import version

        return version("jsonschema")
    except Exception:  # pragma: no cover
        return "unknown"


def unknown_keys(instance: object, schema: object, path: str = "$") -> list[str]:
    """递归列出 schema 未声明的字段（additionalProperties=false 语义的显式实现）。"""
    found: list[str] = []
    if not isinstance(schema, dict):
        return found
    if isinstance(instance, dict):
        props = schema.get("properties", {})
        allows_extra = schema.get("additionalProperties", True)
        for key, value in instance.items():
            if key in props:
                found.extend(unknown_keys(value, props[key], f"{path}.{key}"))
            elif allows_extra is False:
                found.append(f"{path}.{key}")
    if isinstance(instance, list):
        items = schema.get("items")
        if isinstance(items, dict):
            for idx, value in enumerate(instance):
                found.extend(unknown_keys(value, items, f"{path}[{idx}]"))
    return found


def check_bounded_wait(doc: dict) -> list[str]:
    errors: list[str] = []
    if doc.get("lock_scope_bounded") is not True:
        errors.append("lock_scope_bounded 必须为 true（锁作用域有界承诺）")
    if doc.get("lock_reentrancy_forbidden") is not True:
        errors.append("lock_reentrancy_forbidden 必须为 true（禁止同作用域二次加锁）")
    watchdog = doc.get("deadlock_watchdog_timeout_ms")
    acquire = doc.get("lock_acquisition_timeout_ms")
    if not isinstance(watchdog, int) or watchdog > WATCHDOG_MAX_MS:
        errors.append(f"deadlock_watchdog_timeout_ms 必须 <= {WATCHDOG_MAX_MS}，实际 {watchdog!r}")
    if not isinstance(acquire, int) or acquire < 1:
        errors.append(f"lock_acquisition_timeout_ms 必须 >= 1，实际 {acquire!r}")
    if isinstance(watchdog, int) and isinstance(acquire, int) and acquire >= watchdog:
        errors.append(
            "lock_acquisition_timeout_ms 必须严格小于 deadlock_watchdog_timeout_ms，"
            f"实际 {acquire} >= {watchdog}"
        )
    listed = doc.get("lock_forbidden_calls")
    if not isinstance(listed, list):
        errors.append("lock_forbidden_calls 必须是数组")
    else:
        missing = [name for name in REQUIRED_FORBIDDEN if name not in listed]
        if missing:
            errors.append(f"lock_forbidden_calls 缺少 C5 必需条目: {missing}")
    return errors


def validate(doc: dict, schema: dict) -> list[str]:
    errors: list[str] = []
    validator = Draft7Validator(schema)
    for err in sorted(validator.iter_errors(doc), key=lambda e: list(e.absolute_path)):
        loc = "$" + "".join(f".{p}" for p in err.absolute_path)
        errors.append(f"schema: {loc}: {err.message}")
    for key in unknown_keys(doc, schema):
        errors.append(f"unknown-field: {key} 未在 schema 中声明（未知字段拒绝）")
    errors.extend(f"bounded-wait: {msg}" for msg in check_bounded_wait(doc))
    return errors


def run_self_test(schema: dict) -> int:
    base = json.loads(DEFAULT_INSTANCE.read_text(encoding="utf-8"))
    cases: list[tuple[str, dict, bool]] = []

    cases.append(("valid-baseline-passes", json.loads(json.dumps(base)), True))

    bad_unknown = json.loads(json.dumps(base))
    bad_unknown["unknown_field_xyz"] = 1
    cases.append(("unknown-top-level-field-rejected", bad_unknown, False))

    bad_nested_unknown = json.loads(json.dumps(base))
    bad_nested_unknown["logging"]["extra_sink"] = "stdout"
    cases.append(("unknown-nested-field-rejected", bad_nested_unknown, False))

    bad_missing = json.loads(json.dumps(base))
    bad_missing.pop("deadlock_watchdog_timeout_ms")
    cases.append(("missing-deadlock-watchdog-rejected", bad_missing, False))

    bad_bound = json.loads(json.dumps(base))
    bad_bound["deadlock_watchdog_timeout_ms"] = 6000
    cases.append(("watchdog-over-5000-rejected", bad_bound, False))

    bad_calls = json.loads(json.dumps(base))
    bad_calls["lock_forbidden_calls"] = bad_calls["lock_forbidden_calls"][:3]
    cases.append(("incomplete-forbidden-list-rejected", bad_calls, False))

    failures = 0
    for name, doc, expect_ok in cases:
        errors = validate(doc, schema)
        ok = not errors
        status = "PASS" if ok == expect_ok else "FAIL"
        if status == "FAIL":
            failures += 1
        detail = "no error" if ok else f"{len(errors)} error(s): {errors[0]}"
        print(f"[self-test] {status} {name} expect_ok={expect_ok} -> {detail}")
    print(f"[nfr-lint] self-test cases={len(cases)} failures={failures}")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description="NFR 基准机械校验")
    ap.add_argument("--schema", default=str(DEFAULT_SCHEMA))
    ap.add_argument("--instance", default=str(DEFAULT_INSTANCE))
    ap.add_argument("--self-test", action="store_true", help="注入反例自检（证明门禁非恒真）")
    args = ap.parse_args()

    schema_path, instance_path = Path(args.schema), Path(args.instance)
    for path in (schema_path, instance_path):
        if not path.is_file():
            print(f"[nfr-lint] ERROR: 文件不存在: {path}", file=sys.stderr)
            return 2

    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    doc = json.loads(instance_path.read_text(encoding="utf-8"))
    print(f"[nfr-lint] jsonschema={jsonschema_version()}")
    print(f"[nfr-lint] schema={schema_path}")
    print(f"[nfr-lint] instance={instance_path}")

    if args.self_test:
        return run_self_test(schema)

    errors = validate(doc, schema)
    if errors:
        for err in errors:
            print(f"[nfr-lint] FAIL {err}")
        print(f"[nfr-lint] RESULT: FAIL ({len(errors)} error(s))")
        return 1

    print(
        "[nfr-lint] ok: schema 校验通过；未知字段 0 个；"
        f"lock_scope_bounded={doc['lock_scope_bounded']} "
        f"lock_reentrancy_forbidden={doc['lock_reentrancy_forbidden']} "
        f"lock_acquisition_timeout_ms={doc['lock_acquisition_timeout_ms']} "
        f"deadlock_watchdog_timeout_ms={doc['deadlock_watchdog_timeout_ms']} "
        f"lock_forbidden_calls={len(doc['lock_forbidden_calls'])}"
    )
    print("[nfr-lint] RESULT: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
