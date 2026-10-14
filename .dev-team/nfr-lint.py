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
  4. logging 适用范围收窄硬约束（Lead 裁定，task-5 / task-9 整改）：
       logging_scope                  == "new_structured_logging_surface_only"
       logging_legacy_eprintln_exempt == true 且必须绑定 logging_debt_ref
       logging_eprintln_exempt_scope  == "existing_style_non_json_lock_poison_recovery_diagnostics"
       logging_eprintln_exempt_covers_new_lines >= 1（显式承认豁免覆盖本波新增的 1 处）
       logging_compliance_claimed     == false（恒真防护：不主张 logging NFR 已满足）
       logging.trace_id_required      == true（**python 独占强制分支**，schema 仅 type:boolean）
       （logging{format,trace_id_required} 原字段保留，不得删除）
  5. --self-test：注入反例，证明该校验器不是恒真（零假测试）。
  6. --mutation-test：钝化 check_logging_scope 后 self-test 必须失败，
     证明 python 独占分支（trace_id_required）真的被覆盖（task-9 §2 自检盲区修复）。

退出码：0 = 全部通过；1 = 校验失败；2 = 用法/文件错误。
"""

from __future__ import annotations

import argparse
import json
import re
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
LOGGING_SCOPE = "new_structured_logging_surface_only"
LOGGING_EXEMPT_SCOPE = "existing_style_non_json_lock_poison_recovery_diagnostics"
# --mutation-test 用：把 check_logging_scope 钝化为恒返回 []，用于证明 python 独占分支真的被 self-test 覆盖
NEUTRALIZE_LOGGING_SCOPE = False


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


def check_logging_scope(doc: dict) -> list[str]:
    """logging 适用范围收窄 + 豁免范围显式化 + 恒真防护（task-5 / task-9 整改）。

    分支归属（L3-P 复审要求显式声明，不得含糊）：
      * 本函数是 **python 侧唯一独占强制分支** 的载体：
        - `logging.trace_id_required != true`（schema 仅 `type: boolean`，无 const → 只有本分支能拒）
        - `logging_eprintln_exempt_covers_new_lines < 1`（schema 有 minimum，本分支为纵深防御）
        - `logging_eprintln_exempt_scope` 取值（schema 有 enum，本分支为纵深防御）
        - `logging_compliance_claimed is not False`（schema 有 const，本分支为纵深防御）
      * `logging_debt_ref` 由 **schema `const: "PA-119"` 钉死**；下方正则为纵深防御，
        **不构成可独立触发的强制分支**，不得声称其可独立拒绝（task-9 §2 纠正）。
    """
    errors: list[str] = []
    scope = doc.get("logging_scope")
    if scope != LOGGING_SCOPE:
        errors.append(f"logging_scope 必须为 {LOGGING_SCOPE!r}，实际 {scope!r}")
    exempt = doc.get("logging_legacy_eprintln_exempt")
    if exempt is not True:
        errors.append(f"logging_legacy_eprintln_exempt 必须为 true，实际 {exempt!r}")
    debt = doc.get("logging_debt_ref")
    if not isinstance(debt, str) or not re.fullmatch(r"PA-\d+", debt):
        # 纵深防御：schema const 已钉死 PA-119，此分支不可独立触发
        errors.append(
            f"logging_debt_ref 必须是形如 PA-<数字> 的债务单号（豁免到期条款绑定），实际 {debt!r}"
        )
    exempt_scope = doc.get("logging_eprintln_exempt_scope")
    if exempt_scope != LOGGING_EXEMPT_SCOPE:
        errors.append(
            f"logging_eprintln_exempt_scope 必须为 {LOGGING_EXEMPT_SCOPE!r}，实际 {exempt_scope!r}"
        )
    covers = doc.get("logging_eprintln_exempt_covers_new_lines")
    if not isinstance(covers, int) or covers < 1:
        errors.append(
            "logging_eprintln_exempt_covers_new_lines 必须 >= 1"
            f"（禁止声明豁免却覆盖 0 处新增行），实际 {covers!r}"
        )
    claimed = doc.get("logging_compliance_claimed")
    if claimed is not False:
        errors.append(
            "logging_compliance_claimed 必须为 false"
            f"（logging 无代码级检查点，禁止主张已满足），实际 {claimed!r}"
        )
    logging_cfg = doc.get("logging")
    if not isinstance(logging_cfg, dict):
        errors.append("logging 原字段必须保留为对象（不得删除）")
    else:
        if logging_cfg.get("format") != "json":
            errors.append(f"logging.format 必须为 'json'，实际 {logging_cfg.get('format')!r}")
        if logging_cfg.get("trace_id_required") is not True:
            errors.append(
                f"logging.trace_id_required 必须为 true，实际 {logging_cfg.get('trace_id_required')!r}"
            )
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
    if not NEUTRALIZE_LOGGING_SCOPE:
        errors.extend(f"logging-scope: {msg}" for msg in check_logging_scope(doc))
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

    # --- task-5：logging 适用范围收窄 / 豁免到期条款绑定 ---
    bad_unknown_logging = json.loads(json.dumps(base))
    bad_unknown_logging["logging_sink"] = "stdout"
    cases.append(("unknown-logging-field-rejected", bad_unknown_logging, False))

    bad_missing_scope = json.loads(json.dumps(base))
    bad_missing_scope.pop("logging_scope")
    cases.append(("missing-logging-scope-rejected", bad_missing_scope, False))

    bad_scope_value = json.loads(json.dumps(base))
    bad_scope_value["logging_scope"] = "all_surfaces"
    cases.append(("wrong-logging-scope-rejected", bad_scope_value, False))

    bad_exempt_false = json.loads(json.dumps(base))
    bad_exempt_false["logging_legacy_eprintln_exempt"] = False
    cases.append(("logging-exempt-false-rejected", bad_exempt_false, False))

    bad_debt = json.loads(json.dumps(base))
    bad_debt["logging_debt_ref"] = "N/A"
    cases.append(("logging-debt-ref-invalid-rejected", bad_debt, False))

    bad_missing_debt = json.loads(json.dumps(base))
    bad_missing_debt.pop("logging_debt_ref")
    cases.append(("missing-logging-debt-ref-rejected", bad_missing_debt, False))

    bad_dropped_logging = json.loads(json.dumps(base))
    bad_dropped_logging.pop("logging")
    cases.append(("dropped-logging-object-rejected", bad_dropped_logging, False))

    bad_logging_json = json.loads(json.dumps(base))
    bad_logging_json["logging"]["format"] = "text"
    cases.append(("non-json-logging-format-rejected", bad_logging_json, False))

    # --- task-9 §2：自检盲区修复（python 独占分支覆盖）---
    bad_trace_id = json.loads(json.dumps(base))
    bad_trace_id["logging"]["trace_id_required"] = False
    cases.append(("logging-trace-id-false-rejected", bad_trace_id, False))

    bad_exempt_scope = json.loads(json.dumps(base))
    bad_exempt_scope["logging_eprintln_exempt_scope"] = "legacy_only"
    cases.append(("wrong-logging-exempt-scope-rejected", bad_exempt_scope, False))

    bad_missing_exempt_scope = json.loads(json.dumps(base))
    bad_missing_exempt_scope.pop("logging_eprintln_exempt_scope")
    cases.append(("missing-logging-exempt-scope-rejected", bad_missing_exempt_scope, False))

    bad_covers_zero = json.loads(json.dumps(base))
    bad_covers_zero["logging_eprintln_exempt_covers_new_lines"] = 0
    cases.append(("logging-exempt-covers-zero-new-lines-rejected", bad_covers_zero, False))

    bad_missing_covers = json.loads(json.dumps(base))
    bad_missing_covers.pop("logging_eprintln_exempt_covers_new_lines")
    cases.append(("missing-logging-exempt-covers-rejected", bad_missing_covers, False))

    bad_claimed = json.loads(json.dumps(base))
    bad_claimed["logging_compliance_claimed"] = True
    cases.append(("logging-compliance-claimed-true-rejected", bad_claimed, False))

    bad_missing_claimed = json.loads(json.dumps(base))
    bad_missing_claimed.pop("logging_compliance_claimed")
    cases.append(("missing-logging-compliance-claimed-rejected", bad_missing_claimed, False))

    failures = 0
    failing_names: list[str] = []
    for name, doc, expect_ok in cases:
        errors = validate(doc, schema)
        ok = not errors
        status = "PASS" if ok == expect_ok else "FAIL"
        if status == "FAIL":
            failures += 1
            failing_names.append(name)
        detail = "no error" if ok else f"{len(errors)} error(s): {errors[0]}"
        print(f"[self-test] {status} {name} expect_ok={expect_ok} -> {detail}")
    print(f"[nfr-lint] self-test cases={len(cases)} failures={failures}")
    return failures, failing_names


# python 独占强制分支：这些用例只能由 check_logging_scope 拒绝
# （schema 对 trace_id_required 仅约束 type=boolean，无 const/enum）
PYTHON_ONLY_CASES = ("logging-trace-id-false-rejected",)


def run_mutation_test(schema: dict) -> int:
    """钝化 check_logging_scope 后自检必须失败 → 证明 python 独占分支真的被覆盖。

    步骤：
      1. 正常模式跑 self-test，必须 0 failures（基线）；
      2. 置 NEUTRALIZE_LOGGING_SCOPE=True（等价于把 check_logging_scope 钝化为 lambda doc: []）再跑；
      3. 断言 PYTHON_ONLY_CASES 中的用例在钝化后**必须**由 PASS 变 FAIL。
    """
    global NEUTRALIZE_LOGGING_SCOPE
    print("[mutation-test] === 步骤1：基线 self-test（未钝化）===")
    base_failures, _ = run_self_test(schema)
    if base_failures:
        print(f"[mutation-test] FAIL 基线自检已存在 {base_failures} 个失败，无法进行钝化对照")
        return 1

    print("[mutation-test] === 步骤2：钝化 check_logging_scope（NEUTRALIZE_LOGGING_SCOPE=True）===")
    NEUTRALIZE_LOGGING_SCOPE = True
    try:
        mutated_failures, mutated_failing = run_self_test(schema)
    finally:
        NEUTRALIZE_LOGGING_SCOPE = False

    print(
        f"[mutation-test] 钝化后 failures={mutated_failures} failing_cases={mutated_failing}"
    )
    if mutated_failures == 0:
        print(
            "[mutation-test] FAIL 钝化 check_logging_scope 后仍全绿 → python 独占分支未被 self-test 覆盖（盲区仍在）"
        )
        return 1

    uncovered = [name for name in PYTHON_ONLY_CASES if name not in mutated_failing]
    if uncovered:
        print(f"[mutation-test] FAIL 以下 python 独占分支用例未被覆盖: {uncovered}")
        return 1

    print(
        "[mutation-test] PASS 钝化后 "
        f"{len(mutated_failing)} 个用例由 PASS 翻为 FAIL，其中 python 独占分支用例 "
        f"{list(PYTHON_ONLY_CASES)} 全部被覆盖"
    )
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="NFR 基准机械校验")
    ap.add_argument("--schema", default=str(DEFAULT_SCHEMA))
    ap.add_argument("--instance", default=str(DEFAULT_INSTANCE))
    ap.add_argument("--self-test", action="store_true", help="注入反例自检（证明门禁非恒真）")
    ap.add_argument(
        "--mutation-test",
        action="store_true",
        help="钝化 check_logging_scope 后自检必须失败（证明 python 独占分支被覆盖）",
    )
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
        failures, _ = run_self_test(schema)
        return 1 if failures else 0

    if args.mutation_test:
        return run_mutation_test(schema)

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
        f"lock_forbidden_calls={len(doc['lock_forbidden_calls'])} "
        f"logging_scope={doc['logging_scope']!r} "
        f"logging_legacy_eprintln_exempt={doc['logging_legacy_eprintln_exempt']} "
        f"logging_debt_ref={doc['logging_debt_ref']!r} "
        f"logging_eprintln_exempt_covers_new_lines={doc['logging_eprintln_exempt_covers_new_lines']} "
        f"logging_compliance_claimed={doc['logging_compliance_claimed']}"
    )
    print(
        "[nfr-lint] NOTE(logging): logging{format=json,trace_id_required} 当前**无代码级检查点**"
        "（全仓库无结构化日志面），属 PA-119 债务；本波**不主张该 NFR 已满足**，"
        "仅主张「已显式登记债务 + 有到期条款 + 有恒真防护（logging_compliance_claimed=false）」。"
        "禁止把该字段读作恒真放行。"
    )
    print("[nfr-lint] RESULT: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
