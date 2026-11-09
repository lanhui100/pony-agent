#!/usr/bin/env python3
"""r3-dim3 collect：对拍 Langfuse 根结局、headless 封套与结构化事件。

复用 collect_dim1.summarize_session，输出 dim1/dim6 所需字段的超集，保证
verdict.sh 用同一份 metrics 跑历史棘轮时不会因缺字段而假 UNKNOWN。

用法：python3 collect_dim3.py --arm <tag>=<arm-dir> [--arm ...] --out <metrics.json>
退出码：0=证据齐全；1=存在缺 trace/封套/事件流；2=参数或本地 JSON 错误。
"""
import argparse
import json
import os
import sys

from collect_dim1 import fetch_raw, summarize_session


def load_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def host_timeout_spans(trace_id):
    observations = fetch_raw(f"/api/public/observations?traceId={trace_id}&limit=100")
    if not observations or "data" not in observations:
        return None
    spans = []
    for obs in observations["data"]:
        if obs.get("name") != "headless.timeout":
            continue
        attrs = (obs.get("metadata") or {}).get("attributes") or {}
        spans.append({
            "phase": attrs.get("agent.turn.phase"),
            "outcome": attrs.get("agent.turn.outcome"),
            "failed_reason": attrs.get("agent.turn.failed_reason"),
            "turn_id": attrs.get("agent.turn.id"),
            "session_id": attrs.get("langfuse.session.id"),
            "level": obs.get("level"),
        })
    return spans


def event_facts(path):
    terminal_events = []
    fallback_reasons = []
    timeout_markers = 0
    malformed_lines = 0
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                malformed_lines += 1
                continue
            payload = event.get("payload") if isinstance(event.get("payload"), dict) else event
            name = event.get("event") or event.get("name") or payload.get("type") or payload.get("kind")
            if name in {"turn:completed", "turn:failed", "turn:cancelled", "turn:suspended"}:
                terminal_events.append(name)
            if name == "headless:timeout":
                timeout_markers += 1
            reason = payload.get("fallbackReason") or payload.get("fallback_reason")
            if reason and reason not in fallback_reasons:
                fallback_reasons.append(reason)
    return {
        "terminal_events": terminal_events,
        "fallback_reasons": fallback_reasons,
        "provider_fallback": any(
            reason.startswith("provider_followup_failed:") for reason in fallback_reasons
        ),
        "timeout_markers": timeout_markers,
        "malformed_event_lines": malformed_lines,
    }


def collect_arm(tag_prefix, arm_dir):
    sessions_tsv = os.path.join(arm_dir, "sessions.tsv")
    if not os.path.isfile(sessions_tsv):
        raise FileNotFoundError(f"缺 sessions.tsv：{sessions_tsv}")
    arm_meta = load_json(os.path.join(arm_dir, "arm.json"))
    rows = []
    with open(sessions_tsv, encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            tag, session_id = line.rstrip("\n").split("\t")[:2]
            row = summarize_session(f"{tag_prefix}/{tag}", session_id, tag_prefix)
            if row.get("trace_id"):
                host_spans = host_timeout_spans(row["trace_id"])
                row["host_timeout_spans"] = host_spans
                row["host_timeout_fetch_failed"] = host_spans is None
            result_path = os.path.join(arm_dir, f"{tag}.result.json")
            events_path = os.path.join(arm_dir, f"{tag}.jsonl")
            row["arm_exit_all_zero"] = arm_meta.get("exit_all_zero")
            row["result_present"] = os.path.isfile(result_path)
            row["events_present"] = os.path.isfile(events_path)
            if row["result_present"]:
                result = load_json(result_path)
                row.update({
                    "result_schema": result.get("schema"),
                    "result_turn_id": result.get("turn_id"),
                    "result_status": result.get("status"),
                    "result_exit_code": result.get("exit_code"),
                    "result_failure_reason": result.get("failure_reason"),
                    "result_degraded": result.get("degraded"),
                    "result_degraded_reason": result.get("degraded_reason"),
                    "result_tool_error_count": sum(
                        1 for call in result.get("tool_calls") or []
                        if call.get("status") == "error"
                    ),
                })
            if row["events_present"]:
                row.update(event_facts(events_path))
            rows.append(row)
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--arm", action="append", required=True, help="<tag>=<arm-dir>")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    rows = []
    try:
        for spec in args.arm:
            if "=" not in spec:
                print(f"--arm 需 <tag>=<dir>：{spec}", file=sys.stderr)
                return 2
            tag, arm_dir = spec.split("=", 1)
            rows.extend(collect_arm(tag, arm_dir))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"采集输入错误：{error}", file=sys.stderr)
        return 2

    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump({
            "schema": 1,
            "dimension": 3,
            "collected_from": "langfuse+headless-result+jsonl",
            "sessions": rows,
        }, handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    bad = [
        row["tag"] for row in rows
        if row.get("missing") or not row.get("result_present") or not row.get("events_present")
        or row.get("malformed_event_lines") or row.get("host_timeout_fetch_failed")
    ]
    print(f"collected={len(rows)} evidence_incomplete={len(bad)} out={args.out}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
