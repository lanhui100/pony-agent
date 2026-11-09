#!/usr/bin/env python3
"""重放隔离性取证（只读）——PA-169 C3 A1 断言的实现体。

归属：由 `scripts/audit/assertions/dim6.sh` 调用；输入 replay/<arm>/*.jsonl +
*.result.json，输出 JSON 到 stdout。退出码语义：

  0 = 所有会话满足隔离断言（env cwd == 臂 worktree）
  1 = 存在隔离失配会话（阻断性发现）
  2 = 输入缺失（无法取证，UNKNOWN 不得当 PASS）

判定口径（每会话）：
  - env_cwd：首个携带 buildContextObservation.requestMessagesText 的事件里
    `Environment: cwd=<path>` 的取值（模型实际看到的 cwd 真值）；
  - tool_error_activities：toolActivities 中 status ∉ {done, running} 的条数
    （隔离失配时的伴生症状，不单独判红）。
"""
import glob
import json
import os
import re
import sys

ENV_RE = re.compile(r"Environment: cwd=([^,\s]+)")


def session_facts(jsonl_path):
    env_cwd = None
    tool_total = 0
    tool_bad = 0
    trace_ids = set()
    with open(jsonl_path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if env_cwd is None:
                observation = event.get("buildContextObservation") or {}
                match = ENV_RE.search(observation.get("requestMessagesText") or "")
                if match:
                    env_cwd = match.group(1)
            if event.get("traceId"):
                trace_ids.add(event["traceId"])
            for activity in event.get("toolActivities") or []:
                status = activity.get("status")
                if status is None:
                    continue
                tool_total += 1
                if status not in ("done", "running"):
                    tool_bad += 1
    return env_cwd, tool_total, tool_bad, sorted(trace_ids)


def main():
    arm_dir = sys.argv[1] if len(sys.argv) > 1 else "docs/audit/rounds/r1-dim6/replay/base"
    expected_cwd = sys.argv[2] if len(sys.argv) > 2 else None

    files = sorted(glob.glob(os.path.join(arm_dir, "*.jsonl")))
    if not files:
        print(json.dumps({"ok": False, "reason": "no jsonl in arm dir", "arm": arm_dir}))
        return 2

    sessions = []
    mismatched = 0
    for jsonl in files:
        tag = os.path.basename(jsonl)[: -len(".jsonl")]
        env_cwd, tool_total, tool_bad, trace_ids = session_facts(jsonl)
        result_path = os.path.join(arm_dir, tag + ".result.json")
        result = {}
        if os.path.exists(result_path):
            with open(result_path, encoding="utf-8") as handle:
                result = json.load(handle)
        isolated = (env_cwd is not None) and (
            expected_cwd is None or os.path.realpath(env_cwd) == os.path.realpath(expected_cwd)
        )
        if expected_cwd is not None and not isolated:
            mismatched += 1
        sessions.append(
            {
                "tag": tag,
                "session_id": result.get("session_id"),
                "status": result.get("status"),
                "exit_code": result.get("exit_code"),
                "env_cwd": env_cwd,
                "isolated": isolated,
                "tool_activities": tool_total,
                "tool_error_activities": tool_bad,
                "trace_ids": trace_ids,
                "input_tokens": (result.get("usage") or {}).get("input_tokens"),
            }
        )

    report = {
        "ok": mismatched == 0,
        "arm": arm_dir,
        "expected_cwd": expected_cwd,
        "session_count": len(sessions),
        "isolation_mismatch_count": mismatched,
        "sessions": sessions,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if mismatched == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
