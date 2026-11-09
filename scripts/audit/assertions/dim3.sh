#!/usr/bin/env bash
# scripts/audit/assertions/dim3.sh —— 维度 3（失败语义）阈值断言
#
# 阈值口径：r3-dim3 提案（docs/audit/rounds/r3-dim3/proposals/r3-dim3.md）批准即冻结。
# 本脚本哈希记入 loop-state.current.assertion_hashes；改动即冻结漂移（exit 4）。
#
# 四条断言（任一超阈 = 非零）：
#   A1 降级保真（provider 回退）：事件流出现 `provider_followup_failed` 的会话，
#      根 span 必须 `agent.turn.outcome=degraded` 且带 `agent.turn.fallback_reason`，
#      封套必须带 `degraded_reason`（同源原文）—— 违规会话数 = 0。
#      —— 现状缺陷：provider 回退后三轮同报「纯成功」（phase=completed / exit=0 / turn:completed）。
#   A2 超时终态可判定：`result.status=timeout` 的会话必须三侧齐全 ——
#      ① 事件流有终止标记（`headless:timeout`）或 runtime 终态事件；
#      ② 封套 `failure_reason=timeout`；
#      ③ trace 侧存在终态 observation（根 span phase/outcome，或 `headless.timeout` 宿主 span）；
#      且**不得**整条 trace 缺失 —— 违规会话数 = 0。
#   A3 三侧结局无矛盾：同一会话的封套状态、事件流终态、根 span 相位不得互相否认
#      （如 result=timeout 而 trace.phase=completed，或 phase=completed 与 outcome=error 并存）
#      —— 冲突会话数 = 0。
#   A4 控制组不误报：既无 provider 回退、又无工具失败的 completed 会话**不得**被判降级
#      （防「全标 degraded」式作弊）—— 误报会话数 = 0。
#
# 证据口径：只认 --metrics（collect_dim3.py 产出的三臂超集）；--arm-dir 为统一调用约定必填，
# 本脚本不读臂内文件（node/host 事实已由采集器归一进 metrics）。--arm-tag 接受并忽略：
# 维度 3 判定的是「同一会话三侧自洽」，跨臂全量判定，不做逐臂过滤。
#
# 用法：bash scripts/audit/assertions/dim3.sh --arm-dir <dir> --expected-cwd <wt> \
#         --metrics <metrics.dim3.json> [--arm-tag <tag>]
# 退出码：0 = 全绿｜1 = 有断言失败｜2 = 证据缺失（UNKNOWN ≠ PASS）｜3 = 环境不可用
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/../lib.sh"

ARM_DIR=""; EXPECTED_CWD=""; METRICS=""; ARM_TAG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arm-dir) ARM_DIR="${2:?}"; shift 2 ;;
    --expected-cwd) EXPECTED_CWD="${2:?}"; shift 2 ;;
    --metrics) METRICS="${2:?}"; shift 2 ;;
    --arm-tag) ARM_TAG="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,30p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done
[ -n "$ARM_DIR" ] || { fail_msg "--arm-dir 必填（统一调用约定，本脚本不读臂内文件）"; exit 2; }
[ -n "$METRICS" ] || { fail_msg "--metrics 必填（collect_dim3.py 产出）"; exit 2; }
[ -f "$METRICS" ] || { fail_msg "metrics 不存在：$METRICS"; exit 2; }
: "${ARM_TAG:=}"   # 显式接受并忽略（维度 3 跨臂全量判定）

say "── A1 provider 回退降级保真（trace.outcome=degraded + fallback_reason + 封套 degraded_reason）"
say "── A2 超时终态三侧可判定（事件标记 / 封套 failure_reason / trace 终态 observation）"
say "── A3 三侧结局无矛盾"
say "── A4 控制组不误报降级"

OUT_JSON="$(mktemp)"
trap 'rm -f "$OUT_JSON"' EXIT

python3 - "$METRICS" "$OUT_JSON" <<'PY'
import json
import sys

metrics_path, out_path = sys.argv[1], sys.argv[2]
with open(metrics_path, encoding="utf-8") as handle:
    data = json.load(handle)

rows = data.get("sessions") or []
viol = {"A1": [], "A2": [], "A3": [], "A4": []}
evidence_missing = []


def attr(row, key):
    return (row.get("root_attributes") or {}).get(key)


for row in rows:
    tag = row.get("tag") or "<untagged>"
    status = row.get("result_status")
    phase = attr(row, "agent.turn.phase")
    outcome = attr(row, "agent.turn.outcome")
    fallback_attr = attr(row, "agent.turn.fallback_reason")
    terminal_events = row.get("terminal_events") or []
    host_terminal = [
        span for span in (row.get("host_timeout_spans") or [])
        if span.get("phase") or span.get("outcome")
    ]

    # 采集侧证据缺失（封套/事件流本身没落盘）：这是 harness 取证问题，不是产品判定。
    if not row.get("result_present") or not row.get("events_present"):
        evidence_missing.append(tag)

    # ── A1 provider 回退必须留下降级语义 ──────────────────────────────────────
    if row.get("provider_fallback"):
        bad = []
        if outcome != "degraded":
            bad.append(f"trace.outcome={outcome!r}（期望 degraded）")
        if not fallback_attr:
            bad.append("trace.agent.turn.fallback_reason 缺失")
        degraded_reason = row.get("result_degraded_reason") or ""
        if not degraded_reason.startswith("provider_followup_failed"):
            bad.append(f"result.degraded_reason={degraded_reason[:40]!r}（期望 provider_followup_failed 前缀）")
        if bad:
            viol["A1"].append({"tag": tag, "detail": bad})

    # ── A2 超时必须三侧可判定 ────────────────────────────────────────────────
    if status == "timeout":
        bad = []
        if not (row.get("timeout_markers") or 0) and not terminal_events:
            bad.append("events: 既无 headless:timeout 标记也无 runtime 终态")
        if (row.get("result_failure_reason") or "") != "timeout":
            bad.append(f"result.failure_reason={row.get('result_failure_reason')!r}（期望 'timeout'）")
        if row.get("missing"):
            bad.append("trace: 整条缺失（无任何 observation）")
        elif not phase and not outcome and not host_terminal:
            bad.append("trace: 无根终态且无 headless.timeout observation")
        if bad:
            viol["A2"].append({"tag": tag, "detail": bad})

    # ── A3 三侧结局不得互相否认 ──────────────────────────────────────────────
    conflict = []
    if status in ("failed", "timeout", "cancelled", "suspended"):
        if phase == "completed":
            conflict.append(f"trace.phase=completed 与 result.status={status} 矛盾")
        if "turn:completed" in terminal_events:
            conflict.append(f"events 终态=completed 与 result.status={status} 矛盾")
    if phase == "completed" and outcome == "error":
        conflict.append("phase=completed 与 outcome=error 并存")
    if phase == "failed" and status == "completed":
        conflict.append("trace.phase=failed 与 result.status=completed 矛盾")
    if conflict:
        viol["A3"].append({"tag": tag, "detail": conflict})

    # ── A4 控制组：无回退、无工具失败的 completed 不得被判降级 ─────────────────
    clean_control = (
        status == "completed"
        and not row.get("provider_fallback")
        and not (row.get("tool_error_count") or 0)
        and not (row.get("result_tool_error_count") or 0)
    )
    if clean_control:
        bad = []
        if outcome in ("degraded", "error"):
            bad.append(f"trace.outcome={outcome!r}（控制组不得判降级）")
        if row.get("result_degraded_reason"):
            bad.append("result.degraded_reason 非空（控制组不得判降级）")
        if row.get("result_failure_reason"):
            bad.append(f"result.failure_reason={row.get('result_failure_reason')!r}（控制组不得有失败原因）")
        if bad:
            viol["A4"].append({"tag": tag, "detail": bad})

by_arm = {}
for row in rows:
    arm = row.get("arm") or "<unknown>"
    by_arm.setdefault(arm, 0)
    by_arm[arm] += 1

result = {
    "schema": 1,
    "dimension": 3,
    "sessions": len(rows),
    "by_arm": by_arm,
    "violations": {key: len(value) for key, value in viol.items()},
    "details": {key: value[:20] for key, value in viol.items() if value},
    "evidence_missing": evidence_missing,
}
with open(out_path, "w", encoding="utf-8") as handle:
    json.dump(result, handle, ensure_ascii=False, indent=1)
    handle.write("\n")
PY
py_rc=$?
[ "$py_rc" -eq 0 ] || { fail_msg "metrics 解析失败（exit=${py_rc}）"; exit 2; }

python3 - "$OUT_JSON" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    result = json.load(handle)

print(f"   sessions={result['sessions']} by_arm={result['by_arm']}")
for key in ("A1", "A2", "A3", "A4"):
    count = result["violations"][key]
    state = "PASS" if count == 0 else "FAIL"
    print(f"   {key} {state} violations={count}")
    for item in result["details"].get(key, []):
        for detail in item["detail"]:
            print(f"      - {item['tag']}: {detail}")
if result["evidence_missing"]:
    print(f"   MISSING 封套/事件流缺失：{result['evidence_missing']}（UNKNOWN ≠ PASS）")
PY

FAILED=0
MISSING=0
python3 - "$OUT_JSON" <<'PY' || FAILED=1
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    result = json.load(handle)
sys.exit(1 if any(result["violations"].values()) else 0)
PY
python3 - "$OUT_JSON" <<'PY' || MISSING=1
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    result = json.load(handle)
sys.exit(1 if result["evidence_missing"] else 0)
PY

say ""
if [ "$FAILED" -ne 0 ]; then
  say "dim3: FAIL（断言超阈）"
  exit 1
fi
if [ "$MISSING" -ne 0 ]; then
  say "dim3: UNKNOWN（取证缺失，UNKNOWN ≠ PASS）"
  exit 2
fi
say "dim3: PASS（A1/A2/A3/A4 全绿）"
exit 0
