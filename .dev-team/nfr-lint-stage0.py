#!/usr/bin/env python3
"""ADR 0099 阶段 0 写模型 NFR 基准校验器（严格 schema + 未知字段拒绝 + 硬约束）。

用法：
    python3 .dev-team/nfr-lint-stage0.py
    python3 .dev-team/nfr-lint-stage0.py --instance .dev-team/nfr-baseline-stage0-write-model.json

校验内容：
  1. 实例必须通过 .dev-team/nfr-baseline-stage0-write-model.schema.json（Draft-07）；
  2. 未知字段拒绝（additionalProperties=false，逐层递归显式实现，含嵌套对象与数组元素）；
  3. 硬约束（不依赖 schema，防 schema 被弱化后门禁变绿）：
     - external_call_timeout_ms <= 3000 且 > 0；
     - retry.max_attempts <= 3、backoff_cap_ms <= 200、backoff_sequence_ms 非降且不越上限；
     - retry.retry_on 必须恰为 DatabaseBusy + DatabaseLocked（禁止 contains("busy") 启发式）；
     - locking：1 <= lock_acquisition_timeout_ms < deadlock_watchdog_timeout_ms <= 5000；
     - single_instance_file_lock.required == true 且 session_cas.required == true；
     - logging.format == "json" 且 logging.trace_id == true 且 trace_id_field 非空；
     - session_cas.on_mismatch 必须引用 CommitOutcome::StaleVersion（零写入语义）；
     - resource_release 非空。
退出码：0 = 全部通过；1 = 校验失败；2 = 用法/依赖/文件错误。
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    from jsonschema import Draft7Validator
except ImportError:  # pragma: no cover
    print("[stage0-nfr-lint] ERROR: 需要 python3-jsonschema", file=sys.stderr)
    raise SystemExit(2)

BASE = Path(__file__).resolve().parent
DEFAULT_SCHEMA = BASE / "nfr-baseline-stage0-write-model.schema.json"
DEFAULT_INSTANCE = BASE / "nfr-baseline-stage0-write-model.json"

TIMEOUT_MAX_MS = 3000
WATCHDOG_MAX_MS = 5000
RETRY_MAX_ATTEMPTS = 3
BACKOFF_CAP_MAX_MS = 200


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


def check_hard_constraints(doc: dict) -> list[str]:
    errors: list[str] = []

    timeout = doc.get("external_call_timeout_ms")
    if not isinstance(timeout, int) or timeout <= 0 or timeout > TIMEOUT_MAX_MS:
        errors.append(
            f"external_call_timeout_ms 必须是 1..{TIMEOUT_MAX_MS} 的整数，实际 {timeout!r}"
        )

    retry = doc.get("retry")
    if not isinstance(retry, dict):
        errors.append("retry 必须是对象")
    else:
        attempts = retry.get("max_attempts")
        if not isinstance(attempts, int) or not 1 <= attempts <= RETRY_MAX_ATTEMPTS:
            errors.append(
                f"retry.max_attempts 必须是 1..{RETRY_MAX_ATTEMPTS} 的整数，实际 {attempts!r}"
            )
        cap = retry.get("backoff_cap_ms")
        if not isinstance(cap, int) or not 1 <= cap <= BACKOFF_CAP_MAX_MS:
            errors.append(
                f"retry.backoff_cap_ms 必须是 1..{BACKOFF_CAP_MAX_MS} 的整数，实际 {cap!r}"
            )
        seq = retry.get("backoff_sequence_ms")
        if not isinstance(seq, list) or len(seq) != 3 or not all(isinstance(v, int) for v in seq):
            errors.append(f"retry.backoff_sequence_ms 必须是 3 个整数，实际 {seq!r}")
        else:
            if any(later < earlier for earlier, later in zip(seq, seq[1:])):
                errors.append(f"retry.backoff_sequence_ms 必须非降（指数退避），实际 {seq!r}")
            if isinstance(cap, int) and any(v > cap for v in seq):
                errors.append(f"retry.backoff_sequence_ms 不得超过 backoff_cap_ms={cap}")
        on = retry.get("retry_on")
        if not isinstance(on, list) or sorted(on) != ["DatabaseBusy", "DatabaseLocked"]:
            errors.append(f"retry.retry_on 必须恰为 DatabaseBusy+DatabaseLocked，实际 {on!r}")

    lock = doc.get("concurrency_lock")
    if not isinstance(lock, dict):
        errors.append("concurrency_lock 必须是对象")
    else:
        fil = lock.get("single_instance_file_lock")
        if not isinstance(fil, dict) or fil.get("required") is not True:
            errors.append("concurrency_lock.single_instance_file_lock.required 必须为 true")
        cas = lock.get("session_cas")
        if not isinstance(cas, dict) or cas.get("required") is not True:
            errors.append("concurrency_lock.session_cas.required 必须为 true")
        elif "CommitOutcome::StaleVersion" not in str(cas.get("on_mismatch", "")):
            errors.append(
                "concurrency_lock.session_cas.on_mismatch 必须声明 CommitOutcome::StaleVersion（零写入）"
            )
        if lock.get("lock_scope_bounded") is not True:
            errors.append("concurrency_lock.lock_scope_bounded 必须为 true")
        if lock.get("reentrancy_forbidden") is not True:
            errors.append("concurrency_lock.reentrancy_forbidden 必须为 true")
        acquire = lock.get("lock_acquisition_timeout_ms")
        watchdog = lock.get("deadlock_watchdog_timeout_ms")
        if not isinstance(acquire, int) or acquire < 1:
            errors.append(f"lock_acquisition_timeout_ms 必须 >= 1，实际 {acquire!r}")
        if not isinstance(watchdog, int) or watchdog > WATCHDOG_MAX_MS:
            errors.append(
                f"deadlock_watchdog_timeout_ms 必须 <= {WATCHDOG_MAX_MS}，实际 {watchdog!r}"
            )
        if isinstance(acquire, int) and isinstance(watchdog, int) and acquire >= watchdog:
            errors.append(
                "lock_acquisition_timeout_ms 必须严格小于 deadlock_watchdog_timeout_ms，"
                f"实际 {acquire} >= {watchdog}"
            )

    logging_cfg = doc.get("logging")
    if not isinstance(logging_cfg, dict):
        errors.append("logging 必须是对象")
    else:
        if logging_cfg.get("format") != "json":
            errors.append(f"logging.format 必须为 'json'，实际 {logging_cfg.get('format')!r}")
        if logging_cfg.get("trace_id") is not True:
            errors.append("logging.trace_id 必须为 true")
        if not str(logging_cfg.get("trace_id_field", "")).strip():
            errors.append("logging.trace_id_field 必须非空")

    release = doc.get("resource_release")
    if not isinstance(release, list) or not release:
        errors.append("resource_release 必须是非空数组")

    return errors


def lint(schema_path: Path, instance_path: Path) -> int:
    for path in (schema_path, instance_path):
        if not path.exists():
            print(f"[stage0-nfr-lint] ERROR: 文件不存在: {path}", file=sys.stderr)
            return 2

    try:
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        doc = json.loads(instance_path.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"[stage0-nfr-lint] ERROR: JSON 解析失败: {exc}", file=sys.stderr)
        return 2

    errors: list[str] = []
    if not isinstance(doc, dict):
        print("[stage0-nfr-lint] ERROR: 根对象必须是 JSON object", file=sys.stderr)
        return 1

    validator = Draft7Validator(schema)
    for err in sorted(validator.iter_errors(doc), key=lambda e: list(e.absolute_path)):
        location = "$" + "".join(f".{p}" for p in err.absolute_path)
        errors.append(f"{location}: schema 违规: {err.message}")

    errors.extend(f"未知字段（schema 未声明）: {key}" for key in unknown_keys(doc, schema))
    errors.extend(check_hard_constraints(doc))

    if errors:
        print(f"[FAIL] stage0-write-model NFR 基准校验失败（{instance_path}）：", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print(f"[PASS] stage0-write-model NFR 基准校验通过: {instance_path}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="ADR 0099 阶段 0 NFR 基准严格校验")
    parser.add_argument("--schema", default=str(DEFAULT_SCHEMA), help="JSON Schema 路径")
    parser.add_argument("--instance", default=str(DEFAULT_INSTANCE), help="NFR 基准 JSON 路径")
    args = parser.parse_args()
    return lint(Path(args.schema), Path(args.instance))


if __name__ == "__main__":
    sys.exit(main())
