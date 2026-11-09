#!/usr/bin/env python3
"""r2-dim1 collect：维度 1（时延结构）证据采集器。

口径（真值只来自 Langfuse observation，不靠叙述）：
  - turn 墙钟：`agent.turn` span 的 `latency`（秒）与属性 `agent.turn.duration_ms`
  - 逐次调用：`llm.chat` 的 `llm.call.duration_ms` / `llm.call.ttft_ms` / `llm.call.generation_ms`
  - 工具耗时：`tool.call` 的 `agent.tool.duration_ms`
  - 归属分解：Σllm + Σtool vs turn 墙钟 → 未归属时延（调度/治理/宿主开销）

用法：
  python3 collect-dim1.py --arm <tag>=<arm-dir> [--arm ...] --out <metrics.dim1.json>

凭据复用 scripts/audit/fetch.sh --raw（只走 stdin 管道，不进 argv）。
退出码：0 = 全部会话采集成功｜1 = 有会话失败｜2 = 参数错误｜3 = 环境不可用
"""
import argparse
import json
import os
import subprocess
import sys

def _find_repo_root(start):
    """向上找到含 scripts/audit/fetch.sh 的目录；找不到则按固定深度兜底。"""
    node = os.path.abspath(start)
    while node != os.path.dirname(node):
        if os.path.isfile(os.path.join(node, "scripts/audit/fetch.sh")):
            return node
        node = os.path.dirname(node)
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))))


REPO_ROOT = _find_repo_root(os.path.dirname(os.path.abspath(__file__)))


def fetch_raw(path):
    result = subprocess.run(
        ["bash", os.path.join(REPO_ROOT, "scripts/audit/fetch.sh"), "--raw", path],
        capture_output=True, text=True, env=os.environ,
    )
    if result.returncode != 0:
        return None
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError:
        return None


def num(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def summarize_session(tag, session_id, arm):
    traces = fetch_raw(f"/api/public/traces?limit=1&sessionId={session_id}")
    if not traces or not traces.get("data"):
        return {"tag": tag, "session_id": session_id, "arm": arm, "missing": True}
    trace = traces["data"][0]
    # Langfuse 对 limit 上限为 100：传 200 返回空响应体（实测会被误判成 observations-unavailable）。
    # 固定 100，并在观测数达到上限时标 evidence_truncated，不静默截断成 pass。
    observations = fetch_raw(f"/api/public/observations?traceId={trace['id']}&limit=100")
    if not observations or "data" not in observations:
        return {"tag": tag, "session_id": session_id, "arm": arm,
                "trace_id": trace["id"], "missing": True,
                "fetch_error": "observations-unavailable"}

    turn = None
    calls = []
    tools = []
    for obs in observations["data"]:
        attrs = (obs.get("metadata") or {}).get("attributes") or {}
        if obs.get("name") == "agent.turn" and obs.get("type") == "SPAN":
            turn = obs
        elif obs.get("type") == "GENERATION":
            calls.append({
                "duration_ms": num(attrs.get("llm.call.duration_ms")),
                "ttft_ms": num(attrs.get("llm.call.ttft_ms")),
                "generation_ms": num(attrs.get("llm.call.generation_ms")),
                "input_tokens": num(attrs.get("gen_ai.usage.input_tokens")),
                "output_tokens": num(attrs.get("gen_ai.usage.output_tokens")),
                "latency_s": num(obs.get("latency")),
                "time_to_first_token_s": num(obs.get("timeToFirstToken")),
                # 流式/批量口径：L3 用它把「流式却无首包」与「批量本就无首包」区分开，
                # 否则批量路径会被误判成测量缺陷（r2-dim1 审计已实测该混淆）。
                "mode": attrs.get("llm.request.mode"),
                # 传输口径（r2-dim1 提案新增）：`llm.request.mode` 对 stream 与 buffered
                # 都写 tool_followup，缺该键时「无首包」不可归因。
                "transport": attrs.get("llm.request.transport"),
            })
        elif obs.get("type") == "TOOL":
            tools.append({
                "name": obs.get("name"),
                "duration_ms": num(attrs.get("agent.tool.duration_ms")),
                "latency_s": num(obs.get("latency")),
                "status": attrs.get("agent.tool.status"),
                "error": attrs.get("agent.tool.error"),
            })

    turn_attrs = ((turn or {}).get("metadata") or {}).get("attributes") or {}
    turn_duration_ms = num(turn_attrs.get("agent.turn.duration_ms"))
    turn_latency_ms = (num((turn or {}).get("latency")) or 0.0) * 1000.0

    llm_total = sum(c["duration_ms"] for c in calls if c["duration_ms"] is not None)
    ttft_vals = [c["ttft_ms"] for c in calls if c["ttft_ms"] is not None]
    gen_total = sum(c["generation_ms"] for c in calls if c["generation_ms"] is not None)
    tool_total = sum(t["duration_ms"] for t in tools if t["duration_ms"] is not None)

    wall = turn_duration_ms if turn_duration_ms is not None else turn_latency_ms
    attributed = llm_total + tool_total
    return {
        "tag": tag,
        "arm": arm,
        "session_id": session_id,
        "trace_id": trace["id"],
        "environment": trace.get("environment"),
        "turn_duration_ms": turn_duration_ms,
        "turn_latency_ms": turn_latency_ms,
        "turn_calls_attr": num(turn_attrs.get("agent.turn.calls")),
        "llm_call_count": len(calls),
        "tool_call_count": len(tools),
        "llm_total_ms": llm_total,
        "llm_ttft_total_ms": sum(ttft_vals),
        "llm_generation_total_ms": gen_total,
        "tool_total_ms": tool_total,
        "unattributed_ms": (wall - attributed) if wall is not None else None,
        "unattributed_ratio": (round((wall - attributed) / wall, 4) if wall else None),
        "llm_share": (round(llm_total / wall, 4) if wall else None),
        "ttft_share_of_llm": (round(sum(ttft_vals) / llm_total, 4) if llm_total else None),
        "ttft_missing_calls": sum(1 for c in calls if c["ttft_ms"] is None),
        "tool_zero_duration": sum(1 for t in tools if not t["duration_ms"]),
        "tool_error_count": sum(1 for t in tools if t.get("error")),
        "calls": calls,
        "tools": tools,
        "missing": False,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--arm", action="append", required=True,
                        help="<arm-tag>=<replay arm dir>，可重复")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    rows = []
    for spec in args.arm:
        if "=" not in spec:
            print(f"--arm 需 <tag>=<dir> 形式：{spec}", file=sys.stderr)
            return 2
        tag_prefix, arm_dir = spec.split("=", 1)
        sessions_tsv = os.path.join(arm_dir, "sessions.tsv")
        if not os.path.isfile(sessions_tsv):
            print(f"缺 sessions.tsv：{sessions_tsv}", file=sys.stderr)
            return 3
        with open(sessions_tsv, encoding="utf-8") as handle:
            for line in handle:
                line = line.rstrip("\n")
                if not line.strip():
                    continue
                tag, session_id = line.split("\t")
                row = summarize_session(f"{tag_prefix}/{tag}", session_id, tag_prefix)
                rows.append(row)
                state = "MISSING" if row.get("missing") else "ok"
                print(f"{state} {row['tag']} wall={row.get('turn_duration_ms')} "
                      f"llm={row.get('llm_total_ms')} tool={row.get('tool_total_ms')} "
                      f"unattr_ratio={row.get('unattributed_ratio')}")

    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump({"schema": 1, "dimension": 1, "collected_from": "langfuse",
                   "sessions": rows}, handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    bad = [row["tag"] for row in rows if row.get("missing")]
    print(f"collected={len(rows)} failed={len(bad)} out={args.out}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
