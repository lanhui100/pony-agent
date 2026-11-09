#!/usr/bin/env bash
# r1-dim6 verify 阶段重放（P0 人工等价，node: manual）。
#
# 与 `replay-p0.sh` 的差异（为什么另建而不是改它）：
#   `replay-p0.sh` 的 sha256 已在 loop-state 的 `assertion_hashes` 里**冻结**
#   （防作弊：断言工具不得在 round 内漂移），故本轮 verify 另建本脚本，
#   不改冻结件。本脚本额外满足两条纪律：
#     1. 不删除任何目录（corpus 用幂等覆盖复制）；
#     2. 臂产物目录按 `<arm>` 版本化，回执只指向不可变路径（COLLISION 复盘项 2）。
#
# 用法：bash replay-arm.sh <arm-label> <scratch-worktree> <ponycli-bin> [sessions]
# 环境：
#   PONY_AUDIT_GATEWAY / PONY_AUDIT_MODEL / PONY_AUDIT_SESSIONS 可选覆盖；
#   PONYLLM_API_KEY 必须已在 env（凭据只走 env，绝不进 argv）。
set -uo pipefail

ARM="${1:?arm label required}"
WT="${2:?scratch worktree path required}"
BIN="${3:?ponycli binary required}"
SESSIONS="${4:-${PONY_AUDIT_SESSIONS:-3}}"

REPO="/home/dm/pony-agent"
OUT="${PONY_AUDIT_OUT:-$REPO/docs/audit/rounds/r1-dim6/replay/$ARM}"
GW="${PONY_AUDIT_GATEWAY:-https://tokens.ponyjob.top/v1}"
MODEL="${PONY_AUDIT_MODEL:-deepseek/deepseek-v4.1-flash}"

if [ -z "${PONYLLM_API_KEY:-}" ]; then
  echo "PONYLLM_API_KEY 未设置：拒绝在缺凭据下重放（UNKNOWN ≠ PASS）" >&2
  exit 3
fi

export PONY_OTEL_ENVIRONMENT="${PONY_OTEL_ENVIRONMENT:-audit-${ARM}}"

mkdir -p "$OUT" "$WT/docs/audit"
# 幂等覆盖复制 corpus（不删除目标目录）。
cp -r "$REPO/docs/audit/corpus/." "$WT/docs/audit/corpus/"

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

# 派生 sessions.tsv（collect 节点输入口径）：只从 result.json 读真值，不猜。
: > "$OUT/sessions.tsv"
for f in "$OUT"/*.result.json; do
  [ -e "$f" ] || continue
  tag="$(basename "$f" .result.json)"
  sid="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get('session_id') or '')" "$f")"
  [ -n "$sid" ] && printf '%s\t%s\n' "$tag" "$sid" >> "$OUT/sessions.tsv"
done

echo "=== replay done $(date +%H:%M:%S) fail=$fail sessions=$SESSIONS ==="
exit "$fail"
