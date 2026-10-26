#!/usr/bin/env bash
# Langfuse 语义轨动态检查（补齐 audit-project.sh 的盲区）
#
# ## 为什么需要它
#
# `ponygo observe audit` 的 T5 只做**静态**检查（源码里有没有 gen_ai.* / langfuse.* 属性），
# 从不验证「语义轨到底有没有数据」。而本项目 AI 形态 = agent ⇒ 按契约 `references/contract.json`
# 的 langfuse_applicability，Langfuse 是**必需**维度。
# 结果就是：Tempo 有 trace、Langfuse 空，巡检依然全绿 —— 思考树 / 工具调用透视 / 会话成本
# 全丢却不报警。本脚本补上这一格。
#
# ## 判据（与 contract.json 的 fan-out 契约对齐）
#
# 带 `gen_ai.system` 或任意 `langfuse.*` 属性的 span 会被 otel-collector **副本**送进 Langfuse。
# 因此判定分两步：
#   1. Tempo 里**存在**语义轨候选 span（有 gen_ai.* / langfuse.* 的 trace）；
#   2. 同一时间窗内 Langfuse 侧有对应 trace。
# 第 2 步需要 Langfuse API 凭据；拿不到凭据时**判 UNKNOWN，绝不判 PASS**（纪律：UNKNOWN ≠ PASS）。
#
# 用法：
#   bash scripts/observability/check-langfuse.sh [--since <秒>] [--json]
# 退出码：0 = 两轨都有数据｜2 = 语义轨缺口（Tempo 有、Langfuse 无）｜3 = 证据不足（缺凭据/不可达）
set -uo pipefail

SKILL_DIR="${OBS_SKILL_DIR:-$HOME/.agents/skills/observability-onboarding}"
# shellcheck source=/dev/null
. "${SKILL_DIR}/scripts/lib.sh"

SINCE=86400
JSON=0
while [ $# -gt 0 ]; do
  case "$1" in
    --since) SINCE="${2:-86400}"; shift 2 ;;
    --json)  JSON=1; shift ;;
    *) echo "未知参数: $1" >&2; exit 3 ;;
  esac
done

# ---- 第一步：Tempo 侧语义轨候选（确定性证据） ----
TRACE_CANDIDATES=0
if command -v tcount_traceql >/dev/null 2>&1; then
  raw="$(tcount_traceql '{ resource.service.name =~ ".*" && (span.gen_ai.system != "" || span.langfuse.trace.id != "") }' "$SINCE" 2>/dev/null || true)"
  TRACE_CANDIDATES="$(printf '%s' "$raw" | python3 -c 'import sys,json;
try: print(json.load(sys.stdin).get("raw_count",0))
except Exception: print(-1)' 2>/dev/null || echo -1)"
fi

# ---- 第二步：Langfuse 侧（需要凭据；缺则 UNKNOWN） ----
LANGFUSE_STATE="unknown"
LANGFUSE_COUNT=-1
LF_PUBLIC="${LANGFUSE_PUBLIC_KEY:-}"
LF_SECRET="${LANGFUSE_SECRET_KEY:-}"
if [ -n "$LF_PUBLIC" ] && [ -n "$LF_SECRET" ]; then
  # Langfuse API: GET /api/public/traces?limit=1（Basic auth: public:secret）
  # 凭据只走 stdin 管道进入远端 shell，绝不进 kubectl/命令的 argv（skill 纪律：凭据只走管道）。
  body="$(printf '%s:%s' "$LF_PUBLIC" "$LF_SECRET" | in_probe_stdin \
    "CREDS=\$(cat); wget -qO- -T 10 \
     --header=\"Authorization: Basic \$(printf '%s' \"\$CREDS\" | base64 | tr -d '\\n')\" \
     'http://${LANGFUSE_SVC}/api/public/traces?limit=1'" 2>/dev/null || true)"
  if printf '%s' "$body" | grep -qi "401 Unauthorized"; then
    LANGFUSE_STATE="unauthorized"
  elif printf '%s' "$body" | grep -q '"data"'; then
    LANGFUSE_STATE="reachable"
    LANGFUSE_COUNT="$(printf '%s' "$body" | python3 -c 'import sys,json;
try: print(len(json.load(sys.stdin).get("data") or []))
except Exception: print(-1)' 2>/dev/null || echo -1)"
  fi
else
  LANGFUSE_STATE="no-credentials"
fi

# ---- 判定 ----
verdict="unknown"; exit_code=3; reason=""
if [ "$TRACE_CANDIDATES" -lt 0 ]; then
  reason="Tempo 查询失败，无法判定语义轨候选"
elif [ "$TRACE_CANDIDATES" -eq 0 ]; then
  # 没有语义轨候选 span ⇒ Langfuse 空是「上游没数据」，不是 fan-out 断链
  verdict="no-candidates"; exit_code=0
  reason="时间窗内没有带 gen_ai.*/langfuse.* 的 span，语义轨无候选（先修 trace 出口）"
elif [ "$LANGFUSE_STATE" = "no-credentials" ]; then
  reason="缺 LANGFUSE_PUBLIC_KEY / LANGFUSE_SECRET_KEY，无法验证语义轨（UNKNOWN ≠ PASS）"
elif [ "$LANGFUSE_STATE" = "unauthorized" ]; then
  reason="Langfuse API 返回 401：公钥/私钥无效或项目不匹配"
elif [ "$LANGFUSE_STATE" != "reachable" ]; then
  reason="Langfuse API 不可达（网络或服务异常）"
elif [ "$LANGFUSE_COUNT" -le 0 ]; then
  verdict="gap"; exit_code=2
  reason="Tempo 有 ${TRACE_CANDIDATES} 条语义轨候选，但 Langfuse 侧 0 条 ⇒ fan-out 断链或采样不一致"
else
  verdict="pass"; exit_code=0
  reason="Tempo 候选 ${TRACE_CANDIDATES} 条，Langfuse 侧有数据"
fi

if [ "$JSON" = "1" ]; then
  python3 - "$verdict" "$exit_code" "$reason" "$TRACE_CANDIDATES" "$LANGFUSE_STATE" "$LANGFUSE_COUNT" "$SINCE" <<'PY'
import json,sys
v,ec,reason,cand,lf_state,lf_count,since = sys.argv[1:8]
print(json.dumps({
  "dimension": "tracing.semantic_track",
  "verdict": v,
  "exit_code": int(ec),
  "reason": reason,
  "window_seconds": int(since),
  "tempo_candidates": int(cand),
  "langfuse_state": lf_state,
  "langfuse_traces_sampled": int(lf_count),
}, ensure_ascii=False, indent=1))
PY
else
  printf '== Langfuse 语义轨检查 ==\n'
  printf '  判定: %s (exit=%d)\n' "$verdict" "$exit_code"
  printf '  理由: %s\n' "$reason"
  printf '  时间窗: %ss\n' "$SINCE"
fi

exit "$exit_code"
