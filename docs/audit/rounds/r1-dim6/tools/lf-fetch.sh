#!/usr/bin/env bash
# Langfuse trace 拉取（P0 人工等价节点，node: manual）
#
# 归属：ADR 0101 r1-dim6 collect/verify 的 Langfuse 侧证据采集；PA-169 落地后由
# `scripts/audit/fetch.sh` 取代。
#
# 用法：bash lf-fetch.sh <path-and-query>   例："/api/public/traces?limit=3&name=x"
# 凭据：优先 env（LANGFUSE_PUBLIC_KEY / LANGFUSE_SECRET_KEY）；缺则从集群
#       observability/observability-secrets 读取（LANGFUSE_INIT_PROJECT_*）。
# 纪律：凭据只走 stdin 管道进入远端探针，绝不进 kubectl/wget argv；不落盘不回显。
set -uo pipefail

REQ_PATH="${1:?request path required}"
# shellcheck source=/dev/null
. "${OBS_SKILL_DIR:-$HOME/.agents/skills/observability-onboarding}/scripts/lib.sh"

LF_PUBLIC="${LANGFUSE_PUBLIC_KEY:-}"
LF_SECRET="${LANGFUSE_SECRET_KEY:-}"
if [ -z "$LF_PUBLIC" ] || [ -z "$LF_SECRET" ]; then
  LF_PUBLIC="$(kubectl -n observability get secret observability-secrets \
    -o jsonpath='{.data.LANGFUSE_INIT_PROJECT_PUBLIC_KEY}' 2>/dev/null | base64 -d)"
  LF_SECRET="$(kubectl -n observability get secret observability-secrets \
    -o jsonpath='{.data.LANGFUSE_INIT_PROJECT_SECRET_KEY}' 2>/dev/null | base64 -d)"
fi
if [ -z "$LF_PUBLIC" ] || [ -z "$LF_SECRET" ]; then
  echo '{"error":"no-credentials"}' >&2
  exit 3
fi

printf '%s:%s' "$LF_PUBLIC" "$LF_SECRET" | in_probe_stdin "
CREDS=\$(cat)
AUTH=\$(printf '%s' \"\$CREDS\" | base64 | tr -d '\n')
wget -qO- -T 15 --header=\"Authorization: Basic \$AUTH\" 'http://${LANGFUSE_SVC}${REQ_PATH}'
" 2>/dev/null
