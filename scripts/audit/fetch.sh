#!/usr/bin/env bash
# scripts/audit/fetch.sh —— PA-169 C2：按审计标签拉 Langfuse 指标 → metrics.json
#
# 契约：ADR 0101 §确定性节点 fetch（缺凭据/不可达 = 3 UNKNOWN，绝不 PASS）。
# 通道纪律继承 scripts/observability/check-langfuse.sh：探针只读、凭据只走 stdin 管道，
# 绝不进 kubectl/wget argv，不落盘不回显。
#
# 用法：bash scripts/audit/fetch.sh --arm-dir <replay/arm> [--out metrics.json]
#         [--round r1-dim6] [--allow-partial]
#   --arm-dir 需含 sessions.tsv（`<tag>\t<session_id>`，由 replay.sh 生成）
#
# 退出码：0 = 全部会话取证成功｜1 = 有会话取证失败（fetch_error/missing）
#         2 = 参数错误｜3 = 环境不可用（缺凭据/探针不可达/缺 sessions.tsv）
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$HERE/lib.sh"

ARM_DIR=""; OUT=""; ROUND="${ROUND:-r1-dim6}"; RAW=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arm-dir) ARM_DIR="${2:?}"; shift 2 ;;
    --out) OUT="${2:?}"; shift 2 ;;
    --round) ROUND="${2:?}"; shift 2 ;;
    --raw) RAW="${2:?}"; shift 2 ;;
    -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) fail_msg "未知参数: $1"; exit 2 ;;
  esac
done
# --raw 仅供本脚本内部（python 侧）复用同一凭据通道：打印响应体，退出码 0/3。
if [ -z "$RAW" ]; then
  [ -n "$ARM_DIR" ] || { fail_msg "--arm-dir 必填"; exit 2; }
  [ -f "$ARM_DIR/sessions.tsv" ] || { fail_msg "缺 $ARM_DIR/sessions.tsv（先跑 replay.sh）"; exit 3; }
  OUT="${OUT:-$ROUNDS_DIR/$ROUND/metrics.json}"
fi

need_cmd python3 || exit 3
SKILL_LIB="${OBS_SKILL_DIR:-$HOME/.agents/skills/observability-onboarding}/scripts/lib.sh"
[ -f "$SKILL_LIB" ] || { fail_msg "缺 observability skill lib：$SKILL_LIB"; exit 3; }
# shellcheck source=/dev/null
. "$SKILL_LIB"

LF_PUBLIC="${LANGFUSE_PUBLIC_KEY:-}"
LF_SECRET="${LANGFUSE_SECRET_KEY:-}"
if [ -z "$LF_PUBLIC" ] || [ -z "$LF_SECRET" ]; then
  if command -v kubectl >/dev/null 2>&1; then
    LF_PUBLIC="$(kubectl -n "${OBS_NS:-observability}" get secret observability-secrets \
      -o jsonpath='{.data.LANGFUSE_INIT_PROJECT_PUBLIC_KEY}' 2>/dev/null | base64 -d)"
    LF_SECRET="$(kubectl -n "${OBS_NS:-observability}" get secret observability-secrets \
      -o jsonpath='{.data.LANGFUSE_INIT_PROJECT_SECRET_KEY}' 2>/dev/null | base64 -d)"
  fi
fi
if [ -z "$LF_PUBLIC" ] || [ -z "$LF_SECRET" ]; then
  fail_msg "无 Langfuse 凭据 → UNKNOWN（退出码 3，绝不判 PASS）"
  exit 3
fi

lf_get() {
  local req_path="$1"
  printf '%s:%s' "$LF_PUBLIC" "$LF_SECRET" | in_probe_stdin "
CREDS=\$(cat)
AUTH=\$(printf '%s' \"\$CREDS\" | base64 | tr -d '\n')
wget -qO- -T 15 --header=\"Authorization: Basic \$AUTH\" 'http://${LANGFUSE_SVC}${req_path}'
" 2>/dev/null
}

# 凭据只经 env 下传给子进程（--raw 复用同一通道），不进 argv、不落盘。
export LANGFUSE_PUBLIC_KEY="$LF_PUBLIC"
export LANGFUSE_SECRET_KEY="$LF_SECRET"

if [ -n "$RAW" ]; then
  body="$(lf_get "$RAW")"
  if [ -z "$body" ]; then
    fail_msg "raw 请求无响应：$RAW"
    exit 3
  fi
  printf '%s' "$body"
  exit 0
fi

# 连通性前置：不可达属环境不可用（3），不得退化成「空数据 = 全缺失」。
probe="$(lf_get '/api/public/traces?limit=1')"
if ! printf '%s' "$probe" | grep -q '"data"'; then
  fail_msg "Langfuse 不可达或无响应 → UNKNOWN（退出码 3）"
  exit 3
fi

mkdir -p "$(dirname "$OUT")"
python3 - "$ARM_DIR/sessions.tsv" "$OUT" "$ROUND" "$REPO_ROOT/scripts/audit/fetch.sh" <<'PY'
import json
import os
import subprocess
import sys

sessions_path, out_path, round_id, fetch_script = sys.argv[1:5]


def fetch(path):
    """复用本脚本的 --raw：以子进程调用自身，避免凭据进 argv。"""
    result = subprocess.run(
        ["bash", fetch_script, "--raw", path],
        capture_output=True,
        text=True,
        env=os.environ,
    )
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return None


rows = []
for line in open(sessions_path, encoding="utf-8"):
    line = line.rstrip("\n")
    if not line.strip():
        continue
    tag, sid = line.split("\t")
    trace_resp = fetch(f"/api/public/traces?limit=1&sessionId={sid}")
    if not trace_resp or not trace_resp.get("data"):
        rows.append({"tag": tag, "session_id": sid, "missing": True})
        continue
    tr = trace_resp["data"][0]
    tid = tr["id"]
    ra = (tr.get("metadata") or {}).get("resourceAttributes") or {}
    obs = fetch(f"/api/public/observations?traceId={tid}&limit=100")
    if obs is None or "data" not in obs:
        rows.append({"tag": tag, "session_id": sid, "trace_id": tid,
                     "fetch_error": "observations-unavailable", "missing": True})
        continue
    observations = obs.get("data") or []
    root_attrs = {}
    for observation in observations:
        if observation.get("name") == "agent.turn":
            root_attrs = (observation.get("metadata") or {}).get("attributes") or {}
    tool_errors = []
    for observation in observations:
        if observation.get("type") == "TOOL":
            err = ((observation.get("metadata") or {}).get("attributes") or {}).get("agent.tool.error")
            if err:
                tool_errors.append(str(err)[:160])
    rows.append({
        "tag": tag,
        "session_id": sid,
        "trace_id": tid,
        "trace_release": tr.get("release"),
        "trace_version": tr.get("version"),
        "trace_total_cost": tr.get("totalCost"),
        "service_name": ra.get("service.name"),
        "client_service_name": ra.get("client.service.name"),
        "deployment_environment": ra.get("deployment.environment"),
        "observation_count": len(observations),
        "root_attributes": root_attrs,
        "tool_error_count": len(tool_errors),
        "tool_errors": tool_errors,
        "missing": False,
    })

with open(out_path, "w", encoding="utf-8") as handle:
    json.dump({"schema": 1, "round": round_id, "dimension": 6,
               "collected_from": "langfuse", "sessions": rows},
              handle, ensure_ascii=False, indent=1)
    handle.write("\n")

bad = [row["tag"] for row in rows if row.get("missing")]
print(f"collected {len(rows)} sessions -> {out_path} (failed={len(bad)})")
sys.exit(1 if bad else 0)
PY
rc=$?
say "fetch exit=$rc out=$OUT"
exit "$rc"
