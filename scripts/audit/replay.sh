#!/usr/bin/env bash
# scripts/audit/replay.sh —— PA-169 C1：scratch worktree 双臂重放
#
# 契约：ADR 0101 §确定性节点 replay / §重放隔离（scratch worktree）。
#   - 每臂固定一个提交，在**一次性 git worktree** 内执行，主仓零污染；
#   - 每臂 ≥3 会话 × corpus 任务全集，`ponycli --output-format jsonl --result-json
#     --permission-mode read-only --timeout-secs`；
#   - 产物不可变：臂目录带 attempt/commit 后缀，重复执行会拒绝覆盖（防止
#     「同名重跑覆盖旧证据」这类假证据，r1-dim6 首轮已实测踩坑）。
#
# 用法：
#   bash scripts/audit/replay.sh --arm base --commit <sha> [--sessions 3] \
#        [--label <slug>] [--worktree <path>] [--bin <ponycli>] [--tasks t01,t02]
#
# 退出码：0 = 全部会话退出码 0 且产物齐全｜1 = 存在非零会话或产物缺失
#         2 = 参数错误｜3 = 环境不可用（缺 git/ponycli/corpus/凭据）
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

ARM=""; COMMIT=""; SESSIONS=3; LABEL=""; WORKTREE=""; BIN=""; TASKS=""
GATEWAY="${PONY_AUDIT_GATEWAY:-https://tokens.ponyjob.top/v1}"
MODEL="${PONY_AUDIT_MODEL:-deepseek/deepseek-v4.1-flash}"
TURN_TIMEOUT="${PONY_AUDIT_TIMEOUT_SECS:-200}"

while [ $# -gt 0 ]; do
  case "$1" in
    --arm) ARM="${2:?}"; shift 2 ;;
    --commit) COMMIT="${2:?}"; shift 2 ;;
    --sessions) SESSIONS="${2:?}"; shift 2 ;;
    --label) LABEL="${2:-}"; shift 2 ;;
    --worktree) WORKTREE="${2:?}"; shift 2 ;;
    --bin) BIN="${2:?}"; shift 2 ;;
    --tasks) TASKS="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done

[ -n "$ARM" ] || { fail_msg "--arm 必填（base|candidate）"; exit 2; }
[ -n "$COMMIT" ] || { fail_msg "--commit 必填（固定提交，禁止浮动的 HEAD）"; exit 2; }
[ "$SESSIONS" -ge 3 ] || { fail_msg "--sessions 必须 ≥3（ADR 0101 每臂最少会话数）"; exit 2; }

need_cmd git || exit 3
need_cmd python3 || exit 3
git -C "$REPO_ROOT" cat-file -e "${COMMIT}^{commit}" 2>/dev/null \
  || { fail_msg "提交不存在：$COMMIT"; exit 3; }

BIN="${BIN:-$REPO_ROOT/target/debug/ponycli}"
[ -x "$BIN" ] || { fail_msg "ponycli 不可执行：$BIN（先构建；缺失属环境不可用，不是 pass）"; exit 3; }

CORPUS="$AUDIT_ROOT/corpus"
[ -d "$CORPUS/tasks" ] || { fail_msg "corpus 缺失：$CORPUS/tasks"; exit 3; }

# 凭据只经 env 传递，绝不进 argv。
if [ -z "${PONYLLM_API_KEY:-}" ]; then
  SECRETS="$HOME/.config/pony-agent/secrets.json"
  if [ -f "$SECRETS" ]; then
    PONYLLM_API_KEY="$(python3 -c "
import json,sys
try:
    print(json.load(open(sys.argv[1]))['secrets']['provider/ponyllm/api-key'])
except Exception:
    pass
" "$SECRETS")"
    export PONYLLM_API_KEY
  fi
fi
[ -n "${PONYLLM_API_KEY:-}" ] || { fail_msg "缺少 PONYLLM_API_KEY（env 或 secrets.json）"; exit 3; }

SLUG="${LABEL:-$ARM}"
OUT="$ROUNDS_DIR/${ROUND:-r1-dim6}/replay/${SLUG}"
if [ -e "$OUT/sessions.tsv" ]; then
  fail_msg "臂产物已存在且不可变：$OUT（换 --label 重跑，禁止覆盖旧证据）"
  exit 2
fi
mkdir -p "$OUT"

WT="${WORKTREE:-/tmp/audit-${ROUND:-r1-dim6}-${SLUG}}"
if [ -e "$WT" ]; then
  git -C "$REPO_ROOT" worktree remove --force "$WT" >/dev/null 2>&1 || true
fi
git -C "$REPO_ROOT" worktree add --detach "$WT" "$COMMIT" >/dev/null 2>&1 \
  || { fail_msg "worktree 创建失败：$WT @ $COMMIT"; exit 3; }

# corpus 在主仓尚未被跟踪（docs/audit 为新增），复制进 worktree 供只读任务读取。
rm -rf "$WT/docs/audit/corpus"
mkdir -p "$WT/docs/audit"
cp -r "$CORPUS" "$WT/docs/audit/corpus"

TASK_IDS="$TASKS"
if [ -z "$TASK_IDS" ]; then
  TASK_IDS="$(ls "$CORPUS/tasks" | sed 's/\.md$//' | paste -sd, -)"
fi

# 臂归因：复用既有 resource 属性 deployment.environment，不造私有 schema。
export PONY_OTEL_ENVIRONMENT="audit-${SLUG}"

say "arm=$ARM slug=$SLUG commit=$COMMIT worktree=$WT sessions=$SESSIONS tasks=$TASK_IDS"
fail=0
: > "$OUT/sessions.tsv"
for s in $(seq 1 "$SESSIONS"); do
  IFS=',' read -ra IDS <<< "$TASK_IDS"
  for t in "${IDS[@]}"; do
    prompt_file="$CORPUS/tasks/$t.md"
    [ -f "$prompt_file" ] || { fail_msg "corpus 任务缺失：$prompt_file"; exit 3; }
    prompt="$(awk 'BEGIN{p=0} /^# Prompt/{p=1;next} p' "$prompt_file")"
    tag="s${s}-${t}"
    ( cd "$WT" && timeout $((TURN_TIMEOUT + 20)) "$BIN" -p "$prompt" -w "$WT" -g "$GATEWAY" -m "$MODEL" \
        --output-format jsonl --result-json "$OUT/$tag.result.json" \
        --permission-mode read-only --timeout-secs "$TURN_TIMEOUT" \
        > "$OUT/$tag.jsonl" 2> "$OUT/$tag.stderr" )
    rc=$?
    sid="$(python3 -c "
import json,sys
try:
    print(json.load(open(sys.argv[1])).get('session_id') or '')
except Exception:
    print('')
" "$OUT/$tag.result.json" 2>/dev/null)"
    [ -n "$sid" ] && printf '%s\t%s\n' "$tag" "$sid" >> "$OUT/sessions.tsv"
    say "exit=$rc tag=$tag session=${sid:-<none>}"
    [ "$rc" -eq 0 ] || fail=1
    [ -s "$OUT/$tag.jsonl" ] || { fail_msg "产物缺失：$OUT/$tag.jsonl"; fail=1; }
  done
done

python3 - "$OUT" "$ARM" "$SLUG" "$COMMIT" "$WT" "$SESSIONS" "$TASK_IDS" "$fail" <<'PY'
import json, os, sys, time
out, arm, slug, commit, wt, sessions, tasks, fail = sys.argv[1:9]
meta = {
    "schema": "pony.audit.arm.v1",
    "arm": arm,
    "slug": slug,
    "commit": commit,
    "worktree": wt,
    "sessions": int(sessions),
    "tasks": tasks.split(","),
    "created_unix": int(time.time()),
    "exit_all_zero": fail == "0",
}
with open(os.path.join(out, "arm.json"), "w", encoding="utf-8") as fh:
    json.dump(meta, fh, ensure_ascii=False, indent=1)
    fh.write("\n")
PY

say "arm=$ARM done fail=$fail out=$OUT"
exit "$fail"
