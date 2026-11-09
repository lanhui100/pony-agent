#!/usr/bin/env bash
# scripts/audit/selftest.sh —— PA-169 节点自测：用夹具断言每个节点的红相/绿相退出码
#
# 常载命约「凡机械可查的承诺，配一条非零退出的命令」：本脚本把「UNKNOWN ≠ PASS」
# 「棘轮只增不减」「冻结漂移必红」「臂产物不可变」这些纪律变成可复现断言。
# 全部在 /tmp 夹具上跑，不触碰 docs/audit 真值源与主仓工作树。
#
# 用法：bash scripts/audit/selftest.sh [--keep]
# 退出码：0 = 全部断言符合预期｜1 = 有断言不符合预期｜3 = 环境不可用
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

KEEP=0
while [ $# -gt 0 ]; do
  case "$1" in
    --keep) KEEP=1; shift ;;
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done

need_cmd python3 || exit 3
need_cmd git || exit 3

FX="$(mktemp -d /tmp/pony-audit-selftest.XXXXXX)"
cleanup() {
  if [ "$KEEP" -eq 0 ]; then rm -rf "$FX"; else say "夹具保留在 $FX"; fi
}
trap cleanup EXIT

PASS=0
FAIL=0

# expect <期望退出码> <描述> <命令...>
expect() {
  local want="$1" label="$2"
  shift 2
  "$@" >/dev/null 2>&1
  local got=$?
  if [ "$got" -eq "$want" ]; then
    say "  ok   ${label} → exit=${got}"
    PASS=$((PASS + 1))
  else
    say "  FAIL ${label} → exit=${got}，期望 ${want}"
    FAIL=$((FAIL + 1))
  fi
}

# ── 夹具：corpus + 臂 + metrics ────────────────────────────────────────────────
mkdir -p "$FX/corpus/tasks" "$FX/rounds/r1-dim6/tools"
cat > "$FX/corpus/tasks/t01-fixture.md" <<'MD'
---
id: t01-fixture
timeout_secs: 60
write_access: none
---
# Prompt

夹具任务，不执行。
MD
cp "$HERE/assertions/dim6.sh" "$FX/rounds/r1-dim6/tools/assert-dim6.sh"
cp "$HERE/analyze_replay_isolation.py" "$FX/rounds/r1-dim6/tools/analyze-replay-isolation.py"

make_arm() { # <dir> <cwd>
  local dir="$1" cwd="$2" i
  mkdir -p "$dir"
  : > "$dir/sessions.tsv"
  for i in 1 2 3; do
    printf '{"buildContextObservation":{"requestMessagesText":"Environment: cwd=%s, platform=linux"}}\n' "$cwd" \
      > "$dir/s${i}-t01-fixture.jsonl"
    printf '{"session_id":"fake-%s","status":"completed","exit_code":0,"usage":{"input_tokens":10}}\n' "$i" \
      > "$dir/s${i}-t01-fixture.result.json"
    # sessions.tsv 是 replay.sh 的产物口径（<tag>\t<session_id>），断言层的
    # 「metrics ↔ 臂同源」校验（lib.sh: metrics_arm_guard）依赖它。
    printf 's%s-t01-fixture\tfake-%s\n' "$i" "$i" >> "$dir/sessions.tsv"
  done
}

make_metrics() { # <path> <release-or-empty> <phase> <outcome> [session-prefix]
  python3 - "$1" "$2" "$3" "$4" "${5:-fake}" <<'PY'
import json, sys
path, release, phase, outcome, prefix = sys.argv[1:6]
rows = [{"tag": f"s{i}-t01-fixture", "session_id": f"{prefix}-{i}", "trace_release": release or None,
         "root_attributes": {"agent.turn.phase": phase, "agent.turn.outcome": outcome}}
        for i in (1, 2, 3)]
json.dump({"schema": 1, "sessions": rows}, open(path, "w"))
PY
}

# 带逐调用记录的 metrics 夹具（供 dim1 L3 的臂标签/空集分支使用）。
# <path> <arm-tag> <sessions.tsv 路径>；sessions.tsv 为空 ⇒ 生成「无调用记录」的会话行。
make_metrics_calls() { # <path> <arm> [sessions.tsv]
  python3 - "$1" "$2" "${3:-}" <<'PY'
import json, sys
path, arm, sessions_path = sys.argv[1:4]
want = []
if sessions_path:
    for line in open(sessions_path, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.strip():
            want.append(line.split("\t")[1])
rows = []
for sid in want:
    rows.append({
        "arm": arm,
        "tag": "s-t01-fixture",
        "session_id": sid,
        "trace_release": "0.1.158",
        "root_attributes": {"agent.turn.phase": "completed", "agent.turn.outcome": "success"},
        # 全部带 transport ⇒ L3 可归因，不产生 unattributable。
        "calls": [{"mode": "tool_followup", "transport": "stream", "ttft_ms": 100,
                   "duration_ms": 1000, "output_tokens": 10}],
    })
json.dump({"schema": 1, "sessions": rows}, open(path, "w"))
PY
}

make_arm "$FX/arm-green" "/tmp/audit-selftest-wt"
make_arm "$FX/arm-red-cwd" "/tmp/audit-selftest-other"
mkdir -p "$FX/arm-empty"
make_metrics "$FX/metrics-green.json" "0.1.158" "completed" "success"
make_metrics "$FX/metrics-null-release.json" "" "completed" "success"
make_metrics "$FX/metrics-conflict.json" "0.1.158" "completed" "error"
# 串臂夹具：内容全绿但 session_id 属于另一批会话（模拟「臂换名后沿用旧 metrics」）。
make_metrics "$FX/metrics-other-arm.json" "0.1.158" "completed" "success" "other"

HEAD_SHA="$(git -C "$REPO_ROOT" rev-parse HEAD)"
cat > "$FX/state.json" <<JSON
{"schema":1,"current":{"dimension":6,"round":"r1-dim6","phase":"verify","corpus_hash_frozen":false,
 "assertion_hashes":{},"write_scope":["crates/pony-agent-core/src/bin/cli.rs"],
 "baseline_commit":"$HEAD_SHA","candidate_commit":"$HEAD_SHA"},
 "lease":{"session_id":"selftest","expires_unix":9999999999}}
JSON

say "== assert dim6（三态）"
expect 0 "绿相：隔离+release+结局全绿" \
  bash "$HERE/assertions/dim6.sh" --arm-dir "$FX/arm-green" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-green.json"
expect 1 "红相：cwd 失配" \
  bash "$HERE/assertions/dim6.sh" --arm-dir "$FX/arm-red-cwd" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-green.json"
expect 1 "红相：trace release 为空" \
  bash "$HERE/assertions/dim6.sh" --arm-dir "$FX/arm-green" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-null-release.json"
expect 1 "红相：phase=completed 与 outcome=error 并存" \
  bash "$HERE/assertions/dim6.sh" --arm-dir "$FX/arm-green" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-conflict.json"
expect 2 "UNKNOWN：缺 metrics 不得判 pass" \
  bash "$HERE/assertions/dim6.sh" --arm-dir "$FX/arm-green" --expected-cwd /tmp/audit-selftest-wt
expect 2 "UNKNOWN：空臂（无 jsonl）不得判 pass" \
  bash "$HERE/assertions/dim6.sh" --arm-dir "$FX/arm-empty" --expected-cwd /tmp/audit-selftest-wt

say "== freeze / check-freeze"
# 空断言冻结集 ≠ 未漂移：那是「防篡改没装上」，必须判 UNKNOWN（exit 2）。
# 用独立夹具：主夹具此时 corpus_hash_frozen=false，走的是「批准前未冻结」正常分支。
python3 - "$FX/state.json" "$FX/state-empty-assertions.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
state["current"]["corpus_hash_frozen"] = True
state["current"]["assertion_hashes"] = {}
json.dump(state, open(sys.argv[2], "w", encoding="utf-8"), ensure_ascii=False, indent=1)
PY
expect 2 "红相：assertion_hashes 为空 → UNKNOWN（不是 pass）" \
  python3 "$HERE/state.py" --state "$FX/state-empty-assertions.json" --audit-root "$FX" \
    --round r1-dim6 check-freeze
expect 0 "冻结写入" \
  python3 "$HERE/state.py" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 \
    freeze --session selftest
expect 0 "冻结未漂移" \
  python3 "$HERE/state.py" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 check-freeze
printf '\n# tampered\n' >> "$FX/rounds/r1-dim6/tools/assert-dim6.sh"
expect 4 "红相：断言脚本被篡改 → 冻结漂移" \
  python3 "$HERE/state.py" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 check-freeze
cp "$HERE/assertions/dim6.sh" "$FX/rounds/r1-dim6/tools/assert-dim6.sh"
printf '\n# drift\n' >> "$FX/corpus/tasks/t01-fixture.md"
expect 4 "红相：corpus 漂移 → 冻结漂移" \
  python3 "$HERE/state.py" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 check-freeze
# 恢复被故意漂移的 corpus 并重新冻结：后续 verdict 断言必须从「未漂移」的干净状态起跑，
# 否则红相会被 freeze 抢答（exit 4），测不到 assert/ratchet 分支。
sed -i '$ d' "$FX/corpus/tasks/t01-fixture.md"
expect 0 "恢复后重新冻结" \
  python3 "$HERE/state.py" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 \
    freeze --session selftest
expect 0 "恢复后未漂移" \
  python3 "$HERE/state.py" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 check-freeze

say "== ratchet（只增不减）"
expect 0 "空棘轮文件视为空集" \
  bash "$HERE/ratchet.sh" --ratchet "$FX/ratchet.json" --arm-dir "$FX/arm-green" \
    --expected-cwd /tmp/audit-selftest-wt --metrics "$FX/metrics-green.json"
expect 0 "登记 dim6" \
  python3 "$HERE/ratchet_add.py" --dim 6 --round r1-dim6 \
    --script "scripts/audit/assertions/dim6.sh" --ratchet "$FX/ratchet.json"
expect 3 "拒绝替换已登记维度的脚本路径" \
  python3 "$HERE/ratchet_add.py" --dim 6 --round r1-dim6 \
    --script "scripts/audit/assertions/other.sh" --ratchet "$FX/ratchet.json"
expect 0 "棘轮绿相" \
  bash "$HERE/ratchet.sh" --ratchet "$FX/ratchet.json" --arm-dir "$FX/arm-green" \
    --expected-cwd /tmp/audit-selftest-wt --metrics "$FX/metrics-green.json"
expect 1 "红相：历史维度回归被棘轮阻断" \
  bash "$HERE/ratchet.sh" --ratchet "$FX/ratchet.json" --arm-dir "$FX/arm-red-cwd" \
    --expected-cwd /tmp/audit-selftest-wt --metrics "$FX/metrics-green.json"

say "== verdict（唯一裁决者）"
expect 0 "绿相：assert + ratchet 全绿并登记棘轮" \
  bash "$HERE/verdict.sh" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 --dim 6 \
    --arm-dir "$FX/arm-green" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-green.json"
expect 1 "红相：断言红 → verdict 不 pass" \
  bash "$HERE/verdict.sh" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 --dim 6 \
    --arm-dir "$FX/arm-red-cwd" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-green.json"
expect 2 "UNKNOWN：缺 metrics → verdict 不 pass" \
  bash "$HERE/verdict.sh" --state "$FX/state.json" --audit-root "$FX" --round r1-dim6 --dim 6 \
    --arm-dir "$FX/arm-green" --expected-cwd /tmp/audit-selftest-wt

say "== lease 接管（显式、可审计、CAS 守卫）"
# 缺陷来源（r2-dim1 attempt 2 实测）：用户裁定「另一会话已暂停」后本会话无法接管未过期
# lease —— take-lease 恒 exit=3，只能空等 ttl 到期，推进权事实上被冻结。
# 接管必须**显式**：默认行为不变（双跑冲突 exit 3），接管须同时给 --reason 与 --expect。
python3 - "$FX/state.json" "$FX/state-lease-other.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
state["lease"] = {"session_id": "other-session", "expires_unix": 9999999999}
json.dump(state, open(sys.argv[2], "w", encoding="utf-8"), ensure_ascii=False, indent=1)
PY
LEASE_HASH="$(sha256sum "$FX/state-lease-other.json" | cut -d' ' -f1)"
expect 3 "红相：他人持有且未过期，不带 --takeover → 双跑冲突" \
  python3 "$HERE/state.py" --state "$FX/state-lease-other.json" take-lease --session selftest
expect 2 "红相：--takeover 缺 --reason 被拒（接管必须可审计）" \
  python3 "$HERE/state.py" --state "$FX/state-lease-other.json" take-lease \
    --session selftest --takeover --expect "$LEASE_HASH"
expect 2 "红相：--takeover 缺 --expect 被拒（无 CAS 守卫不得覆盖）" \
  python3 "$HERE/state.py" --state "$FX/state-lease-other.json" take-lease \
    --session selftest --takeover --reason "用户裁定对方已暂停"
expect 1 "红相：--takeover 的 --expect 失配 → 拒绝（对方可能刚写入）" \
  python3 "$HERE/state.py" --state "$FX/state-lease-other.json" take-lease \
    --session selftest --takeover --reason "用户裁定对方已暂停" --expect deadbeef
expect 0 "绿相：--takeover + --reason + 正确 --expect → 接管成功" \
  python3 "$HERE/state.py" --state "$FX/state-lease-other.json" take-lease \
    --session selftest --takeover --reason "用户裁定对方已暂停" --expect "$LEASE_HASH"
if python3 - "$FX/state-lease-other.json" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
lease = state["lease"]
hist = state.get("history") or []
assert lease["session_id"] == "selftest", lease
assert lease["taken_over_from"] == "other-session", lease
assert any(h.get("transition") == "lease takeover: other-session -> selftest"
           and h.get("note") == "用户裁定对方已暂停" for h in hist), hist
PY
then
  say "  ok   接管落 history（transition + reason 可审计）"
  PASS=$((PASS + 1))
else
  say "  FAIL 接管未在 history 留下可审计记录"
  FAIL=$((FAIL + 1))
fi

say "== replay 参数与不可变性"
expect 2 "缺 --commit" bash "$HERE/replay.sh" --arm base
expect 3 "提交不存在" bash "$HERE/replay.sh" --arm base --commit deadbeef
expect 2 "会话数 <3 被拒" bash "$HERE/replay.sh" --arm base --commit HEAD --sessions 2
expect 3 "ponycli 缺失" \
  bash "$HERE/replay.sh" --arm base --commit HEAD --bin /tmp/definitely-not-ponycli

say "== fetch 的 UNKNOWN"
mkdir -p "$FX/fetch-arm"
: > "$FX/fetch-arm/sessions.tsv"
expect 3 "缺 sessions.tsv" bash "$HERE/fetch.sh" --arm-dir "$FX/no-such-arm"
expect 3 "缺凭据/探针不可达 → UNKNOWN" \
  env LANGFUSE_PUBLIC_KEY= LANGFUSE_SECRET_KEY= KUBECONFIG=/nonexistent \
  bash "$HERE/fetch.sh" --arm-dir "$FX/fetch-arm" --out "$FX/m.json"

say "== dim1 断言（时延结构）"
make_metrics_calls "$FX/metrics-no-calls.json" "green" ""
make_metrics_calls "$FX/metrics-arm-tag.json" "green" "$FX/arm-green/sessions.tsv"
# 本节点需要 Langfuse 真值，自测只覆盖**参数校验与证据缺失**两条本地分支
# （离线不联网是 selftest 的纪律：不把网络不确定性带进门禁）。
expect 2 "臂目录不存在 → exit=2" \
  bash "$HERE/assertions/dim1.sh" --arm-dir "$FX/no-such-dim1-arm" \
    --metrics "$FX/metrics-green.json" --out-dir "$FX/dim1-out-a"
expect 2 "缺 sessions.tsv → exit=2" \
  bash "$HERE/assertions/dim1.sh" --arm-dir "$FX/no-such-dim1-arm" --out-dir "$FX/dim1-out-b"
expect 2 "缺 --arm-dir → exit=2" bash "$HERE/assertions/dim1.sh"
# 串臂红相：metrics 属于别的会话集（臂重跑换名后沿用旧 metrics 的实测事故）。
# 判据只查 session_id 覆盖，不联网，故可在夹具上验证。
expect 2 "红相：metrics 与臂不同源 → exit=2（UNKNOWN ≠ PASS）" \
  bash "$HERE/assertions/dim1.sh" --arm-dir "$FX/arm-green" --arm-tag green \
    --expected-cwd /tmp/audit-selftest-wt --metrics "$FX/metrics-other-arm.json" \
    --out-dir "$FX/dim1-out-c"

# L3 臂标签传播（r2-dim1 attempt 2 实测缺陷）：verdict.sh / ratchet.sh 都不传 --arm-tag，
# 而臂目录名带 attempt 后缀（arm-green vs metrics 里的 green）⇒ 逐臂过滤命中 0 行。
# 直接断言反查函数的输出（不经过 L4 等无关分支，避免离线取证噪声掩盖本缺陷）。
TAG_RESOLVED="$(resolve_arm_tag "$FX/metrics-arm-tag.json" "$FX/arm-green" 2>/dev/null)"
if [ "$TAG_RESOLVED" = "green" ]; then
  say "  ok   臂标签从 metrics 反查（目录名 arm-green → 标签 green）"
  PASS=$((PASS + 1))
else
  say "  FAIL 臂标签反查得到 '$TAG_RESOLVED'，期望 'green'"
  FAIL=$((FAIL + 1))
fi
# 反查不出唯一解时必须拒绝（返回非零），不得静默回退成某个猜测值。
if resolve_arm_tag "$FX/metrics-other-arm.json" "$FX/arm-green" >/dev/null 2>&1; then
  say "  FAIL 会话不覆盖时仍返回了标签（应拒绝）"
  FAIL=$((FAIL + 1))
else
  say "  ok   红相：metrics 不覆盖本臂会话时反查拒绝"
  PASS=$((PASS + 1))
fi
# L3 空集必须单列 UNKNOWN：标签明确正确但调用记录为空时，不得折成断言失败（会掩盖假绿）。
expect 2 "红相：L3 逐臂过滤命中 0 行 → exit=2（UNKNOWN，不折成 FAILED）" \
  bash "$HERE/assertions/dim1.sh" --arm-dir "$FX/arm-green" --arm-tag green \
    --expected-cwd /tmp/audit-selftest-wt --metrics "$FX/metrics-no-calls.json" \
    --out-dir "$FX/dim1-out-e"

say ""
say "== dim3 断言（失败语义）"
# 夹具只覆盖**可离线判定**的分支：采集器已把三侧事实归一进 metrics，故断言逻辑可在夹具上
# 全量验证，不需要联网（selftest 纪律：不把网络不确定性带进门禁）。
python3 - "$FX" <<'PY'
import json
import os
import sys

root = sys.argv[1]


def session(tag, *, status="completed", exit_code=0, phase="completed", outcome=None,
            fallback_attr=None, fallback=False, degraded_reason=None, failure_reason=None,
            missing=False, host_terminal=False, tool_errors=0):
    row = {
        "tag": tag,
        "arm": "fixture",
        "result_present": True,
        "events_present": True,
        "missing": missing,
        "result_status": status,
        "result_exit_code": exit_code,
        "result_failure_reason": failure_reason,
        "result_degraded_reason": degraded_reason,
        "result_tool_error_count": tool_errors,
        "provider_fallback": fallback,
        "timeout_markers": 1 if status == "timeout" else 0,
        "terminal_events": ["turn:completed"] if status == "completed" else [],
        "tool_error_count": tool_errors,
        "root_attributes": {},
        "host_timeout_spans": [],
    }
    if phase:
        row["root_attributes"]["agent.turn.phase"] = phase
    if outcome:
        row["root_attributes"]["agent.turn.outcome"] = outcome
    if fallback_attr:
        row["root_attributes"]["agent.turn.fallback_reason"] = fallback_attr
    if host_terminal:
        row["host_timeout_spans"] = [{"phase": "timeout", "outcome": "timeout",
                                      "failed_reason": "timeout", "turn_id": "turn-x",
                                      "session_id": "sess-x", "level": "WARN"}]
    return row


green = [
    session("clean/t1"),
    session("fallback/t1", outcome="degraded", fallback_attr="provider_followup_failed: boom",
            fallback=True, degraded_reason="provider_followup_failed: boom"),
    session("timeout/t1", status="timeout", exit_code=7, phase=None,
            failure_reason="timeout", host_terminal=True),
]
red_a1 = [session("fallback/t1", fallback=True)]
red_a2 = [session("timeout/t1", status="timeout", exit_code=7, phase=None)]
red_a4 = [session("clean/t1", outcome="degraded", degraded_reason="provider_followup_failed: boom")]

for name, rows in (("green", green), ("red-a1", red_a1), ("red-a2", red_a2), ("red-a4", red_a4)):
    path = os.path.join(root, f"metrics-dim3-{name}.json")
    with open(path, "w", encoding="utf-8") as handle:
        json.dump({"schema": 1, "dimension": 3, "sessions": rows}, handle)
PY
mkdir -p "$FX/dim3-arm"
expect 0 "绿相：回退留降级语义 / 超时三侧可判定 / 控制组不误报" \
  bash "$HERE/assertions/dim3.sh" --arm-dir "$FX/dim3-arm" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-dim3-green.json"
expect 1 "红相：provider 回退后三侧同报纯成功 → exit=1" \
  bash "$HERE/assertions/dim3.sh" --arm-dir "$FX/dim3-arm" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-dim3-red-a1.json"
expect 1 "红相：超时无 trace 终态且封套无原因 → exit=1" \
  bash "$HERE/assertions/dim3.sh" --arm-dir "$FX/dim3-arm" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-dim3-red-a2.json"
expect 1 "红相：控制组被误标降级（全标 degraded 式作弊）→ exit=1" \
  bash "$HERE/assertions/dim3.sh" --arm-dir "$FX/dim3-arm" --expected-cwd /tmp/audit-selftest-wt \
    --metrics "$FX/metrics-dim3-red-a4.json"
expect 2 "缺 --metrics → exit=2" \
  bash "$HERE/assertions/dim3.sh" --arm-dir "$FX/dim3-arm"
expect 2 "metrics 不存在 → exit=2" \
  bash "$HERE/assertions/dim3.sh" --arm-dir "$FX/dim3-arm" --metrics "$FX/no-such-metrics.json"

say ""
say "== 断言脚本参数契约（--arm-tag 统一透传）"
# 缺陷来源（r2-dim1 attempt 2 实测 F8）：`ratchet.sh` / `verdict.sh` 对**所有**棘轮条目
# 统一透传 `--arm-tag`，而 `dim6.sh` 原先不认该参数 ⇒ `未知参数` exit=2 ⇒ ratchet 把
# dim6 判成 MISSING ⇒ 任何后续维度的 verdict 恒为 UNKNOWN exit=2，永远走不到 PASS。
# 这是「调用方与断言脚本的参数契约」不一致，不是某一轮的偶发问题，故在此钉死契约：
# 每个 assertions/dim*.sh 都必须接受 `--arm-tag <tag>` 且不因它改变既有判定。
for script in "$HERE"/assertions/dim*.sh; do
  name="$(basename "$script")"
  # 用「缺 --arm-dir」这一必然失败的路径来区分「参数被接受」与「参数未知」：
  # 接受时走到必填校验（exit 2 且提示 --arm-dir 必填），不接受时提示未知参数。
  out="$(bash "$script" --arm-tag some-tag 2>&1)"
  if printf '%s' "$out" | grep -q "未知参数"; then
    say "  FAIL ${name} 不接受 --arm-tag（ratchet/verdict 统一透传会使其恒 UNKNOWN）"
    FAIL=$((FAIL + 1))
  else
    say "  ok   ${name} 接受 --arm-tag"
    PASS=$((PASS + 1))
  fi
done

say ""
say "selftest: pass=${PASS} fail=${FAIL}"
[ "$FAIL" -eq 0 ] || exit 1
exit 0
