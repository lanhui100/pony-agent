#!/usr/bin/env python3
"""r2-dim1 audit 取证：用 observation 时间轴算「真实未归属时延」。

为什么不用 `wall - Σllm - Σtool`：那个口径会把「工具耗时未测到（键=0）」「子 span 缺失」
「span 时间戳与墙钟不同源」全部倒进「未归属」，把测量缺陷伪装成调度开销。
本脚本直接取每个 observation 的 startTime/endTime 求并集，未覆盖区间按时间排序列出，
并标注前后紧邻的 observation —— 缺口是「模型之外」还是「模型之内漏记」一眼可辨。

用法：
  python3 analyze-latency-gaps.py --session <sid> [--session ...] --out <report.json>

退出码：0 = 全部分析成功｜2 = 参数错误｜3 = 环境不可用（凭据/不可达）
"""
import argparse
import json
import os
import subprocess
import sys
from datetime import datetime


def _find_repo_root(start):
    node = os.path.abspath(start)
    while node != os.path.dirname(node):
        if os.path.isfile(os.path.join(node, "scripts/audit/fetch.sh")):
            return node
        node = os.path.dirname(node)
    return os.getcwd()


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


def ts(value):
    if not value:
        return None
    return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()


def analyze(session_id):
    traces = fetch_raw(f"/api/public/traces?limit=1&sessionId={session_id}")
    if not traces or not traces.get("data"):
        return {"session_id": session_id, "missing": True}
    trace = traces["data"][0]
    payload = fetch_raw(f"/api/public/observations?traceId={trace['id']}&limit=100")
    if not payload or "data" not in payload:
        return {"session_id": session_id, "trace_id": trace["id"], "missing": True}

    spans = []
    for obs in payload["data"]:
        attrs = (obs.get("metadata") or {}).get("attributes") or {}
        start, end = ts(obs.get("startTime")), ts(obs.get("endTime"))
        if start is None or end is None:
            continue
        spans.append({
            "name": obs.get("name"),
            "type": obs.get("type"),
            "start": start,
            "end": end,
            "ms": round((end - start) * 1000),
            "ttft_ms": attrs.get("llm.call.ttft_ms"),
            "mode": attrs.get("llm.request.mode"),
        })

    turn = next((s for s in spans if s["name"] == "agent.turn"), None)
    if turn is None:
        return {"session_id": session_id, "trace_id": trace["id"],
                "missing": False, "root_span": None,
                "note": "无 agent.turn 根 span：时延分母不可测"}

    # 只统计 turn 区间内、且不是根 span 自身的观测
    inner = [s for s in spans if s is not turn and s["start"] >= turn["start"] - 0.01
             and s["end"] <= turn["end"] + 0.01]
    inner.sort(key=lambda s: s["start"])

    covered = []
    for span in inner:
        if covered and span["start"] <= covered[-1][1]:
            covered[-1][1] = max(covered[-1][1], span["end"])
        else:
            covered.append([span["start"], span["end"]])

    gaps = []
    cursor = turn["start"]
    for start, end in covered:
        if start - cursor > 0.2:
            gaps.append((cursor, start))
        cursor = max(cursor, end)
    if turn["end"] - cursor > 0.2:
        gaps.append((cursor, turn["end"]))

    def neighbour(moment, side):
        pool = [s for s in inner if (s["end"] <= moment if side == "before" else s["start"] >= moment)]
        if not pool:
            return None
        picked = (max if side == "before" else min)(pool, key=lambda s: s["end"] if side == "before" else s["start"])
        return f"{picked['type']}:{picked['name']}"

    gap_rows = [{
        "from_s": round(a - turn["start"], 2),
        "to_s": round(b - turn["start"], 2),
        "ms": round((b - a) * 1000),
        "after": neighbour(a, "before"),
        "before": neighbour(b, "after"),
    } for a, b in gaps]

    llm_ms = sum(s["ms"] for s in inner if s["type"] == "GENERATION")
    tool_ms = sum(s["ms"] for s in inner if s["type"] == "TOOL")
    turn_ms = turn["ms"]
    return {
        "session_id": session_id,
        "trace_id": trace["id"],
        "missing": False,
        "root_span": {"ms": turn_ms, "attr_duration_ms": turn.get("ms")},
        "llm_calls": sum(1 for s in inner if s["type"] == "GENERATION"),
        "tool_calls": sum(1 for s in inner if s["type"] == "TOOL"),
        "llm_ms": llm_ms,
        "tool_ms": tool_ms,
        "union_covered_ms": round(sum((b - a) * 1000 for a, b in covered)),
        "gap_total_ms": sum(g["ms"] for g in gap_rows),
        "gap_ratio": round(sum(g["ms"] for g in gap_rows) / turn_ms, 4) if turn_ms else None,
        "top_gaps": sorted(gap_rows, key=lambda g: -g["ms"])[:5],
        "first_call_ttft": next((s["ttft_ms"] for s in inner if s["type"] == "GENERATION"), None),
        "first_call_mode": next((s["mode"] for s in inner if s["type"] == "GENERATION"), None),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--session", action="append", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    rows = [analyze(sid) for sid in args.session]
    for row in rows:
        if row.get("missing"):
            print(f"MISSING {row['session_id']}")
        elif row.get("root_span") is None:
            print(f"NO-ROOT {row['session_id']} {row.get('note')}")
        else:
            print(f"{row['session_id'][:18]} turn={row['root_span']['ms']}ms "
                  f"llm={row['llm_ms']} tool={row['tool_ms']} "
                  f"gap={row['gap_total_ms']}ms ({row['gap_ratio']:.1%}) "
                  f"first_ttft={row['first_call_ttft']} mode={row['first_call_mode']}")
            for gap in row["top_gaps"]:
                print(f"    gap {gap['ms']:>6}ms @{gap['from_s']}s..{gap['to_s']}s "
                      f"after={gap['after']} before={gap['before']}")
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump({"schema": 1, "dimension": 1, "kind": "latency-gaps", "sessions": rows},
                  handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    return 1 if any(r.get("missing") for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
