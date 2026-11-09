#!/usr/bin/env bash
# P0 人工等价 replay（node: manual）
#
# 归属：ADR 0101 r1-dim6 collect 阶段的人工等价命令；PA-169 落地后由
# `scripts/audit/replay.sh` 取代本文件（届时删除，不双轨）。
#
# 用法：bash replay-p0.sh <arm-label> <scratch-worktree-path> [ponycli-bin]
# 纪律：
#   - 只在一次性 scratch worktree（固定提交）内跑，主仓不受影响；
#   - corpus 任务只读，复制进 worktree 供其读取（corpus 在主仓尚未被跟踪）；
#   - 凭据只经 env（PONYLLM_API_KEY）传递，绝不进 argv；
#   - **必须**用 `-w <worktree>` 显式绑定工作区：仅靠 `cd` 时 CLI 会沿用
#     cli-state 里旧会话的 workspace（r1-dim6 首轮实测跑进 /home/dm/pony_agent，
#     corpus 与源码全部不可见，9 个会话证据作废）。
set -uo pipefail

ARM="${1:?arm label required}"
WT="${2:?scratch worktree path required}"
BIN="${3:-/home/dm/pony-agent/target/debug/ponycli}"

REPO="/home/dm/pony-agent"
OUT="${PONY_AUDIT_OUT:-$REPO/docs/audit/rounds/r1-dim6/replay/$ARM}"
GW="${PONY_AUDIT_GATEWAY:-https://tokens.ponyjob.top/v1}"
MODEL="${PONY_AUDIT_MODEL:-deepseek/deepseek-v4.1-flash}"
SESSIONS="${PONY_AUDIT_SESSIONS:-3}"
# 臂归因：P0 复用既有 resource 属性 deployment.environment（不造私有 schema）；
# audit.round/dimension/arm 专用键属 P1 接线项（ADR 0101 §审计臂归因）。
export PONY_OTEL_ENVIRONMENT="${PONY_OTEL_ENVIRONMENT:-audit-${ARM}}"
export PONYLLM_API_KEY="$(python3 -c "import json,os;print(json.load(open(os.path.expanduser('~/.config/pony-agent/secrets.json')))['secrets']['provider/ponyllm/api-key'])")"

mkdir -p "$OUT"

rm -rf "$WT/docs/audit/corpus"
mkdir -p "$WT/docs/audit"
cp -r "$REPO/docs/audit/corpus" "$WT/docs/audit/corpus"

fail=0
for s in $(seq 1 "$SESSIONS"); do
  for t in t01-list-summarize t02-read-explain t03-search-locate; do
    prompt="$(awk 'BEGIN{p=0} /^# Prompt/{p=1;next} p' "$REPO/docs/audit/corpus/tasks/$t.md")"
    tag="s${s}-${t}"
    echo "=== $tag start $(date +%H:%M:%S) ==="
    ( cd "$WT" && timeout 220 "$BIN" -p "$prompt" -w "$WT" -g "$GW" -m "$MODEL" \
        --output-format jsonl --result-json "$OUT/$tag.result.json" \
        --permission-mode read-only --timeout-secs 200 \
        > "$OUT/$tag.jsonl" 2> "$OUT/$tag.stderr" )
    rc=$?
    echo "exit=$rc tag=$tag"
    [ "$rc" -eq 0 ] || fail=1
  done
done
echo "=== replay done $(date +%H:%M:%S) fail=$fail ==="
exit "$fail"
