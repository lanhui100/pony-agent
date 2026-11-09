#!/usr/bin/env python3
"""r1-dim6 collect 节点：按证据会话列表拉 Langfuse，产出 metrics.json。

归属：ADR 0101 collect 阶段的人工等价实现；PA-169 落地后由 `scripts/audit/fetch.sh`
取代本文件（届时删除，不双轨）。

用法：python3 collect-dim6.py <sessions.tsv> <out.json>
  sessions.tsv 每行 `<tag>\t<session_id>`，由 replay 的 result.json 派生。

纪律：凭据只经 lf-fetch.sh 的 stdin 管道；本脚本不读不写凭据。
"""
import json
import os
import subprocess
import sys

REPO = "/home/dm/pony-agent"
FETCH = f"{REPO}/docs/audit/rounds/r1-dim6/tools/lf-fetch.sh"


def fetch(path):
    r = subprocess.run(["bash", FETCH, path], capture_output=True, text=True, cwd=REPO)
    if r.returncode != 0:
        return None
    try:
        return json.loads(r.stdout)
    except Exception:
        return None


def main():
    sessions_path, out_path = sys.argv[1], sys.argv[2]
    rows = []
    for line in open(sessions_path, encoding="utf-8"):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        tag, sid = line.split("\t")
        trace_resp = fetch(f"/api/public/traces?limit=1&sessionId={sid}")
        if not trace_resp or not trace_resp.get("data"):
            rows.append({"tag": tag, "session_id": sid, "trace_id": None, "missing": True})
            continue
        tr = trace_resp["data"][0]
        tid = tr["id"]
        ra = (tr.get("metadata") or {}).get("resourceAttributes") or {}
        # Langfuse 单页上限 100（limit>100 返回 4xx）。取数失败必须显式标红，
        # 不得退化成「空列表 = 属性缺失」——那是假证据（纪律：UNKNOWN ≠ PASS）。
        obs = fetch(f"/api/public/observations?traceId={tid}&limit=100")
        if obs is None or "data" not in obs:
            rows.append({"tag": tag, "session_id": sid, "trace_id": tid,
                         "fetch_error": "observations-unavailable", "missing": True})
            continue
        observations = obs.get("data") or []
        root_attrs = {}
        for o in observations:
            if o.get("name") == "agent.turn":
                root_attrs = (o.get("metadata") or {}).get("attributes") or {}
        tool_errors = []
        for o in observations:
            if o.get("type") == "TOOL" or o.get("name") in (
                "Read", "Run", "List", "Glob", "Search", "BatchExecute", "TodoWrite"
            ):
                err = ((o.get("metadata") or {}).get("attributes") or {}).get("agent.tool.error")
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
            "observation_count": len(observations),
            "root_attributes": root_attrs,
            "tool_errors": tool_errors,
            "tool_error_count": len(tool_errors),
            "missing": False,
        })
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump({"schema": 1, "round": "r1-dim6", "dimension": 6,
                   "collected_from": "langfuse", "sessions": rows},
                  fh, ensure_ascii=False, indent=1)
    print(f"collected {len(rows)} sessions -> {out_path}")


if __name__ == "__main__":
    main()
