#!/usr/bin/env bash
# scripts/audit/assertions/dim1.sh —— 维度 1（时延结构）阈值断言
#
# 阈值口径：r2-dim1 提案（docs/audit/rounds/r2-dim1/proposals/r2-dim1.md）批准即冻结。
# 本脚本哈希记入 loop-state.current.assertion_hashes；改动即冻结漂移（exit 4）。
#
# 四条断言（任一超阈 = 非零）：
#   L1 根 span 完整性：每会话恰有 1 个 `agent.turn` 根 span（缺失数 = 0）
#      —— 根 span 缺失 ⇒ 墙钟分母不存在，时延结构不可审计。
#   L2 调用保真：`GENERATION` span 数必须等于 provider 成功调用数（丢 span = 0）
#      —— 真值取自 replay stderr 的 usage 行，与 span 独立来源，防止自证。
#   L3 首包可分解：`mode=tool_followup`（流式）调用中缺 `llm.call.ttft_ms` 的占比 ≤ 10%
#      —— 流式却无首包 ⇒ 「排队等待」与「解码生成」不可分离。
#   L4 未归属缺口：单会话未被任何子 span 覆盖的时延缺口 > 5000ms 的会话数 = 0
#      —— 缺口通常来自 provider 重试/回退，必须由子 span 归因，不得倒进「未归属」。
#
# 用法：bash scripts/audit/assertions/dim1.sh --arm-dir <replay/arm> --expected-cwd <wt> \
#         --metrics <metrics.dim1.json> [--out-dir <dir>]
# 退出码：0 = 全绿｜1 = 有断言失败｜2 = 证据缺失（UNKNOWN ≠ PASS）｜3 = 环境不可用
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/../lib.sh"

ARM_DIR=""; EXPECTED_CWD=""; METRICS=""; OUT_DIR=""; ARM_TAG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arm-dir) ARM_DIR="${2:?}"; shift 2 ;;
    --arm-tag) ARM_TAG="${2:?}"; shift 2 ;;
    --expected-cwd) EXPECTED_CWD="${2:?}"; shift 2 ;;
    --metrics) METRICS="${2:?}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,22p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done
[ -n "$ARM_DIR" ] || { fail_msg "--arm-dir 必填"; exit 2; }
[ -d "$ARM_DIR" ] || { fail_msg "臂目录不存在：$ARM_DIR"; exit 2; }
[ -f "$ARM_DIR/sessions.tsv" ] || { fail_msg "缺 $ARM_DIR/sessions.tsv"; exit 2; }

need_cmd python3 || exit 3

OUT_DIR="${OUT_DIR:-$(mktemp -d /tmp/dim1-assert.XXXXXX)}"
mkdir -p "$OUT_DIR"
# 臂标签与目录名**解耦**：metrics 里的 arm 字段是 replay 的 --arm（base|candidate），
# 而目录名带 attempt 后缀（base-a2 / candidate-clean）。混用会让逐臂过滤静默命中 0 行。
#
# r2-dim1 attempt 2：目录名兜底本身就是缺陷来源 —— `verdict.sh` / `ratchet.sh` 都不传
# `--arm-tag`，于是 L3 用 `candidate-a2` 去过滤 `arm=candidate` 的 metrics，命中 0 行。
# 现在改为：未显式给标签且有 metrics 时，从 metrics 反查覆盖本臂全部会话的 arm 值；
# 反查不出唯一解就保留目录名兜底，并由 L3 的空集判定（exit 2 / UNKNOWN）兜住。
if [ -z "$ARM_TAG" ] && [ -n "$METRICS" ] && [ -f "$METRICS" ]; then
  RESOLVED="$(resolve_arm_tag "$METRICS" "$ARM_DIR" 2>/dev/null || true)"
  [ -n "$RESOLVED" ] && ARM_TAG="$RESOLVED"
fi
ARM_TAG="${ARM_TAG:-$(basename "$ARM_DIR")}"

FAILED=0
MISSING=0

# ── 0. 证据同源：metrics 必须属于本臂（否则断言判的是别的会话） ──────────────────
if [ -n "$METRICS" ] && [ -f "$METRICS" ]; then
  if ! metrics_arm_guard "$METRICS" "$ARM_DIR"; then
    say ""
    say "verdict: failed=0 missing_evidence=1（metrics 与臂不同源 → UNKNOWN ≠ PASS）"
    exit 2
  fi
fi

# ── L1/L2：span 保真度（现场从 stderr + Langfuse 对拍，不读缓存） ────────────────
say "── L1/L2 span fidelity（provider 调用数 vs GENERATION span 数）"
FIDELITY="$OUT_DIR/fidelity.$ARM_TAG.json"
python3 "$HERE/../analyze_span_fidelity.py" --arm "$ARM_TAG=$ARM_DIR" --out "$FIDELITY" \
  > "$OUT_DIR/fidelity.log" 2>&1
fid_rc=$?
if [ "$fid_rc" -eq 3 ]; then
  say "   MISSING 取证不可执行（缺凭据/不可达）——UNKNOWN ≠ PASS"
  MISSING=$((MISSING + 1))
elif [ ! -f "$FIDELITY" ]; then
  say "   MISSING 取证产物缺失：$FIDELITY"
  MISSING=$((MISSING + 1))
else
  python3 - "$FIDELITY" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))
rows = data.get("sessions") or []
totals = data.get("totals") or {}
root_missing = totals.get("root_missing", 0)
lost = totals.get("lost", 0)
print(f"   L1 root_span_missing={root_missing}/{len(rows)}")
for row in rows:
    if row.get("missing"):
        print(f"     - {row['tag']}: 采集失败（缺 trace/observations）")
    elif row.get("turn_spans") != 1:
        print(f"     - {row['tag']}: turn_spans={row.get('turn_spans')}")
print(f"   L2 span_lost={lost}/{totals.get('provider_calls')} "
      f"spans={totals.get('generation_spans')}")
for row in rows:
    if not row.get("missing") and row.get("span_loss"):
        print(f"     - {row['tag']}: provider={row['provider_calls']} "
              f"spans={row['generation_spans']} loss={row['span_loss']}")
missing_sessions = sum(1 for row in rows if row.get("missing"))
raise SystemExit(1 if (root_missing or lost or missing_sessions) else 0)
PY
  [ $? -eq 0 ] || FAILED=$((FAILED + 1))
fi

# ── L3/L4：依赖 metrics（collect-dim1 产物） ────────────────────────────────────
if [ -z "$METRICS" ] || [ ! -f "$METRICS" ]; then
  say "── L3/L4 skipped: metrics 未提供（UNKNOWN ≠ PASS，计为证据缺失）"
  MISSING=$((MISSING + 1))
else
  say "── L3 首包缺失必须可归因：无 llm.call.ttft_ms 的调用须带 llm.request.transport（stream|buffered）"
  # 口径来源（r2-dim1 实测，18/18 会话逐一对拍）：
  #   ttft_missing_calls 恒等于该会话的 buffered HTTP 调用数（followup-sync / -sync-batch），
  #   而 `llm.request.mode` 对两者都写 tool_followup ⇒ 下游无法区分「批量本就无首包」
  #   与「流式测量丢包」。故断言改为「缺失须自带传输口径」，而不是「流式必须全部有 ttft」。
  # 逐臂过滤：metrics 文件可同时含 base/candidate 两臂，混算会让两臂得到同一数字。
  python3 - "$METRICS" "$ARM_TAG" <<'PY'
import json
import sys

rows = json.load(open(sys.argv[1], encoding="utf-8")).get("sessions") or []
arm_tag = sys.argv[2]
rows = [row for row in rows if row.get("arm") == arm_tag]
calls = [c for row in rows for c in (row.get("calls") or [])]
missing = [c for c in calls if c.get("ttft_ms") is None]
unattributable = [c for c in missing if not c.get("transport")]
print(f"   arm={arm_tag} sessions={len(rows)} calls={len(calls)} "
      f"ttft_missing={len(missing)} unattributable={len(unattributable)}")
for call in unattributable[:5]:
    print(f"     - mode={call.get('mode')} dur={call.get('duration_ms')} "
          f"out={call.get('output_tokens')}: 无首包且无传输口径")
# 空集 ≠ 通过，也 ≠ 断言失败：逐臂过滤命中 0 行说明**证据与标签不同源**（r2-dim1
# attempt 1 实测：arm 传 candidate-a2 而 metrics 写 candidate ⇒ 0 行）。旧实现把空集
# 折进 `not calls` 判 1，一旦别处判据变宽就是假绿，故单列为 exit 2（UNKNOWN）。
if not calls:
    print(f"   MISSING 臂标签 {arm_tag!r} 在 metrics 中无任何调用记录 → UNKNOWN ≠ PASS",
          file=sys.stderr)
    raise SystemExit(2)
raise SystemExit(1 if unattributable else 0)
PY
  l3_rc=$?
  case "$l3_rc" in
    0) ;;
    2) MISSING=$((MISSING + 1)) ;;
    *) FAILED=$((FAILED + 1)) ;;
  esac

  say "── L4 单会话未归属缺口 > 5000ms 的会话数 = 0"
  GAPS="$OUT_DIR/gaps.$ARM_TAG.json"
  gap_args=(python3 "$HERE/../analyze_latency_gaps.py")
  while IFS=$'\t' read -r _tag sid; do
    [ -n "$sid" ] && gap_args+=(--session "$sid")
  done < "$ARM_DIR/sessions.tsv"
  gap_args+=(--out "$GAPS")
  "${gap_args[@]}" > "$OUT_DIR/gaps.log" 2>&1
  if [ ! -f "$GAPS" ]; then
    say "   MISSING 缺口取证失败（见 $OUT_DIR/gaps.log）"
    MISSING=$((MISSING + 1))
  else
    python3 - "$GAPS" <<'PY'
import json
import sys

rows = json.load(open(sys.argv[1], encoding="utf-8")).get("sessions") or []
measured = [r for r in rows if r.get("root_span")]
unmeasurable = len(rows) - len(measured)
big = [r for r in measured if (r.get("top_gaps") or [{}])[0].get("ms", 0) > 5000]
print(f"   measured={len(measured)}/{len(rows)} unmeasurable={unmeasurable} "
      f"gap_over_5s={len(big)}")
for row in big:
    top = row["top_gaps"][0]
    print(f"     - {row['session_id'][:18]}: gap={top['ms']}ms "
          f"after={top['after']} before={top['before']}")
raise SystemExit(1 if (big or unmeasurable) else 0)
PY
    [ $? -eq 0 ] || FAILED=$((FAILED + 1))
  fi
fi

say ""
say "verdict: failed=${FAILED} missing_evidence=${MISSING} artifacts=${OUT_DIR}"
if [ "$FAILED" -gt 0 ]; then
  exit 1
fi
if [ "$MISSING" -gt 0 ]; then
  exit 2
fi
exit 0
