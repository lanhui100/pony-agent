#!/usr/bin/env bash
# scripts/audit/lib.sh —— trace 审核 loop 确定性节点公共库（被 source，不单独执行）
#
# 契约来源：ADR 0101 §确定性节点 / PA-169 C1–C8。本文件是「回执怎么写、状态怎么 CAS、
# 阈值哈希怎么核」的唯一实现处；各节点脚本必须复用，禁止各自发明格式。
#
# 退出码语义（全节点统一，继承 check-langfuse.sh 纪律）：
#   0 = pass ｜ 1 = fail（有断言/门禁红） ｜ 2 = 证据缺失（UNKNOWN ≠ PASS）
#   3 = 环境不可用（缺凭据/不可达/缺工具链） ｜ 4 = 冻结被破坏（哈希漂移/篡改）
set -uo pipefail

AUDIT_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$AUDIT_LIB_DIR/../.." && pwd)"
AUDIT_ROOT="${AUDIT_ROOT:-$REPO_ROOT/docs/audit}"
STATE_FILE="${AUDIT_STATE_FILE:-$AUDIT_ROOT/loop-state.json}"
ROUNDS_DIR="${AUDIT_ROUNDS_DIR:-$AUDIT_ROOT/rounds}"

say() { printf '%s\n' "$*"; }
warn() { printf 'WARN %s\n' "$*" >&2; }
fail_msg() { printf 'FAIL %s\n' "$*" >&2; }

now_unix() { date +%s; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || { fail_msg "缺少命令: $1"; return 1; }
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

# corpus 聚合哈希口径（与 docs/audit/corpus/manifest.json 的 aggregate_method 同源）：
# tasks/*.md 按文件名字典序拼接后 sha256。
corpus_aggregate_hash() {
  local corpus_dir="${1:-$AUDIT_ROOT/corpus}"
  ( cd "$corpus_dir" && cat tasks/*.md | sha256sum | awk '{print $1}' )
}

state_get() {
  python3 "$AUDIT_LIB_DIR/state.py" get --state "$STATE_FILE" --path "$1"
}

# 回执落盘：<receipts_dir>/<seq>-<phase>-<node>.json（机器证据，只认退出码）。
# 用法：write_receipt <receipts_dir> <seq> <phase> <node> <exit_code> <artifact> <summary> [command]
write_receipt() {
  local dir="$1" seq="$2" phase="$3" node="$4" exit_code="$5" artifact="$6" summary="$7" command="${8:-}"
  mkdir -p "$dir"
  python3 - "$dir" "$seq" "$phase" "$node" "$exit_code" "$artifact" "$summary" "$command" <<'PY'
import json, os, sys, time
d, seq, phase, node, exit_code, artifact, summary, command = sys.argv[1:9]
receipt = {
    "schema": "pony.audit.receipt.v1",
    "seq": seq,
    "phase": phase,
    "node": node,
    "command": command,
    "exit_code": int(exit_code),
    "ts_unix": int(time.time()),
    "artifact": artifact,
    "summary": summary,
}
path = os.path.join(d, f"{seq}-{phase}-{node}.json")
with open(path, "w", encoding="utf-8") as fh:
    json.dump(receipt, fh, ensure_ascii=False, indent=1)
    fh.write("\n")
print(path)
PY
}

# 读取冻结的断言哈希（loop-state.current.assertion_hashes），用于防篡改核对。
frozen_assertion_hash() {
  python3 - "$STATE_FILE" "$1" <<'PY'
import json, sys
state = json.load(open(sys.argv[1], encoding="utf-8"))
print(((state.get("current") or {}).get("assertion_hashes") or {}).get(sys.argv[2], ""))
PY
}

# metrics ↔ 臂同源校验（r2-dim1 实测缺陷的机械化修复）。
#
# 为什么必须有：`verdict.sh` / `ratchet.sh` 只接受**单个** `--metrics`，而臂目录带 attempt
# 后缀（base→base-a2 / candidate→candidate-a2）。臂重跑换名后若沿用旧 metrics，断言会拿
# A 臂的调用/属性去判 B 臂的 span —— 产出假红（本次实测：新臂 candidate-a2 被旧 metrics
# 判成 L3 不可归因 4 条、L4 超阈 1 条，实际新 metrics 下 L3 为 0）或假绿。
#
# 判据：metrics 的 session_id 集合必须**覆盖**臂 sessions.tsv 的全部会话。
# 退出码：0 = 同源｜2 = 证据缺失或不同源（UNKNOWN ≠ PASS，不得折算成断言失败或通过）。
metrics_arm_guard() {
  local metrics="${1:-}" arm_dir="${2:-}"
  [ -n "$metrics" ] || return 0
  [ -f "$metrics" ] || { warn "metrics 不存在：$metrics"; return 2; }
  [ -f "$arm_dir/sessions.tsv" ] || { warn "臂缺 sessions.tsv：$arm_dir"; return 2; }
  python3 - "$metrics" "$arm_dir/sessions.tsv" <<'PY'
import json
import sys

metrics_path, sessions_path = sys.argv[1:3]
rows = json.load(open(metrics_path, encoding="utf-8")).get("sessions") or []
have = {row.get("session_id") for row in rows if row.get("session_id")}
want = []
for line in open(sessions_path, encoding="utf-8"):
    line = line.rstrip("\n")
    if line.strip():
        want.append(line.split("\t")[1])
missing = [sid for sid in want if sid not in have]
if missing:
    print(f"   MISSING metrics 与臂不同源：metrics 缺 {len(missing)}/{len(want)} 个会话", file=sys.stderr)
    for sid in missing[:5]:
        print(f"     - 缺 {sid}", file=sys.stderr)
    raise SystemExit(2)
print(f"   metrics↔arm 同源：{len(want)} 会话")
PY
}

# 臂标签解析（r2-dim1 attempt 2 缺陷的机械化修复）。
#
# 为什么必须有：`dim1.sh` 的 L3 按 `row.arm == ARM_TAG` 逐臂过滤，而 `ARM_TAG` 缺省取
# 臂目录 basename（`candidate-a2`），metrics 里的 arm 字段却是 replay 的 `--arm`
# （`candidate`）⇒ 过滤静默命中 0 行。此时 L3 的 `not calls` 会把它判成「失败」，
# 看起来是红的，但只要另一处判据略宽就是**假绿**（空集恒等于「无违规」）。
# 因此标签不能靠目录名猜，必须从 metrics 反查：找出覆盖本臂全部会话的那个 arm 值。
#
# 用法：resolve_arm_tag <metrics.json> <arm-dir>
# 输出：匹配到的 arm 标签（stdout，单个）；失败不输出。
# 退出码：0 = 唯一解｜2 = 无解/多解（UNKNOWN，不得折算成通过或失败）
resolve_arm_tag() {
  local metrics="${1:-}" arm_dir="${2:-}"
  [ -n "$metrics" ] && [ -f "$metrics" ] || return 2
  [ -n "$arm_dir" ] && [ -f "$arm_dir/sessions.tsv" ] || return 2
  python3 - "$metrics" "$arm_dir/sessions.tsv" <<'PY'
import json
import sys

metrics_path, sessions_path = sys.argv[1:3]
rows = json.load(open(metrics_path, encoding="utf-8")).get("sessions") or []
want = []
for line in open(sessions_path, encoding="utf-8"):
    line = line.rstrip("\n")
    if line.strip():
        want.append(line.split("\t")[1])
want_set = set(want)
by_arm = {}
for row in rows:
    arm = row.get("arm")
    if not arm:
        continue
    by_arm.setdefault(arm, set()).add(row.get("session_id"))
covering = [arm for arm, ids in by_arm.items() if want_set <= ids]
if len(covering) == 1:
    print(covering[0])
    raise SystemExit(0)
print(f"   MISSING 无法从 metrics 唯一确定臂标签（候选={sorted(covering)}，"
      f"metrics 中 arm={sorted(by_arm)}）", file=sys.stderr)
raise SystemExit(2)
PY
}
