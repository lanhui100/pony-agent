#!/usr/bin/env python3
"""r2-dim1 取证：span 保真度（真实调用数 vs 上报 span 数）与根 span 完整性。

口径真值来源（两者独立，互不依赖）：
  - **真实调用数**：replay 产物 `<tag>.stderr` 中 provider 的 usage 行
    （`[pony-provider] usage:<proto> raw={` 每次调用恰好一行，含失败重试后的成功调用）；
  - **上报 span 数**：Langfuse `observations?traceId=` 中 `type=GENERATION` 的条数；
  - **根 span**：`name=agent.turn` 的 SPAN 是否存在。

丢失即 `provider_calls - generation_spans`：负值同样记异常（上报多于真实 = 重复上报）。

用法：
  python3 analyze-span-fidelity.py --arm <tag>=<arm-dir> [--arm ...] --out <report.json>

退出码：0 = 无丢失｜1 = 有会话丢失/根 span 缺失｜2 = 参数错误｜3 = 环境不可用
"""
import argparse
import json
import os
import re
import subprocess
import sys

USAGE_RE = re.compile(r"usage:\w+ raw=\{")


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


def count_provider_calls(stderr_path):
    if not os.path.isfile(stderr_path):
        return None
    with open(stderr_path, encoding="utf-8", errors="ignore") as handle:
        return len(USAGE_RE.findall(handle.read()))


def analyze(arm, tag, session_id, arm_dir):
    provider_calls = count_provider_calls(os.path.join(arm_dir, f"{tag}.stderr"))
    traces = fetch_raw(f"/api/public/traces?limit=1&sessionId={session_id}")
    if not traces or not traces.get("data"):
        return {"arm": arm, "tag": tag, "session_id": session_id, "missing": True}
    trace = traces["data"][0]
    payload = fetch_raw(f"/api/public/observations?traceId={trace['id']}&limit=100")
    if not payload or "data" not in payload:
        return {"arm": arm, "tag": tag, "session_id": session_id,
                "trace_id": trace["id"], "missing": True}
    observations = payload["data"]
    generation_spans = sum(1 for obs in observations if obs.get("type") == "GENERATION")
    turn_spans = sum(1 for obs in observations
                     if obs.get("type") == "SPAN" and obs.get("name") == "agent.turn")
    turn_attrs = {}
    for obs in observations:
        if obs.get("name") == "agent.turn":
            turn_attrs = (obs.get("metadata") or {}).get("attributes") or {}
    declared_calls = turn_attrs.get("agent.turn.calls")
    return {
        "arm": arm,
        "tag": tag,
        "session_id": session_id,
        "trace_id": trace["id"],
        "provider_calls": provider_calls,
        "generation_spans": generation_spans,
        "span_loss": None if provider_calls is None else provider_calls - generation_spans,
        "turn_spans": turn_spans,
        "turn_calls_attr": None if declared_calls is None else int(float(declared_calls)),
        "observation_count": len(observations),
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
        arm, arm_dir = spec.split("=", 1)
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
                rows.append(analyze(arm, tag, session_id, arm_dir))

    total_calls = sum(r["provider_calls"] or 0 for r in rows if not r.get("missing"))
    total_spans = sum(r["generation_spans"] or 0 for r in rows if not r.get("missing"))
    lost = sum(max(0, r["span_loss"] or 0) for r in rows if not r.get("missing"))
    root_missing = sum(1 for r in rows if not r.get("missing") and r.get("turn_spans") == 0)
    for row in rows:
        if row.get("missing"):
            print(f"MISSING {row['arm']}/{row['tag']}")
            continue
        flag = "" if row["span_loss"] == 0 and row["turn_spans"] == 1 else "  <-- BAD"
        print(f"{row['arm']}/{row['tag']:<34} provider={row['provider_calls']:<3} "
              f"spans={row['generation_spans']:<3} loss={row['span_loss']:<3} "
              f"turn_spans={row['turn_spans']} turn.calls={row['turn_calls_attr']}{flag}")
    print(f"totals: provider_calls={total_calls} generation_spans={total_spans} "
          f"lost={lost} loss_ratio={lost / total_calls:.3%} root_missing={root_missing}")
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump({"schema": 1, "dimension": 1, "kind": "span-fidelity",
                   "totals": {"provider_calls": total_calls, "generation_spans": total_spans,
                              "lost": lost, "root_missing": root_missing},
                   "sessions": rows}, handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    bad = any((r.get("span_loss") not in (0, None)) or (r.get("turn_spans") != 1)
              for r in rows)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
