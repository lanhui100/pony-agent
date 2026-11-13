#!/usr/bin/env python3
"""只读复算：从 sessions.db 的 per-call providerCallRecords 复算三种口径。
用法: python3 recompute.py [db_path]
不写入 DB（mode=ro），仅在 stdout 输出。
"""
import json
import sqlite3
import sys

DB = sys.argv[1] if len(sys.argv) > 1 else "/home/dm/.local/share/PonyAgent/sessions.db"
con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)

rows = con.execute(
    "select n.provider_model, t.trace_data "
    "from session_turn_traces t join normalized_turn_traces n "
    "on n.session_id=t.session_id and n.turn_id=t.turn_id"
).fetchall()


def floor_ms(dur):
    return max(200, dur * 5 // 100)


def eligible(dur, ttft, out):
    if not (dur and out and dur > 0 and out > 0):
        return False
    if ttft is None or dur <= ttft or out < 2:
        return False
    return (dur - ttft) >= floor_ms(dur)


def gen(dur, ttft):
    if ttft is not None and dur > ttft:
        return dur - ttft
    return dur


def new_acc():
    return dict(calls=0, valid=0, elig=0, sum_out=0, sum_dur=0, sum_gen=0,
                elig_out=0, elig_gen=0, ttft_missing=0, turns=0,
                e2e_out=0, e2e_dur=0, turn_wall_ms=0, turn_out=0, turn_with_ttft=0)


acc = {}
for model, raw in rows:
    try:
        d = json.loads(raw)
    except Exception:
        continue
    key = model or "(unknown)"
    a = acc.setdefault(key, new_acc())
    a["turns"] += 1
    # 轮级墙钟（含 TTFT/排队）：normalized_turns 有 start/complete，这里用 trace 自带 turn 字段
    # turnDurationMs 是轮级总墙钟（首个调用起至末个调用结束，见 report 附录）
    t_out = d.get("outputTokens") or 0
    t_dur = d.get("turnDurationMs") or 0
    if t_dur > 0 and t_out > 0:
        a["turn_wall_ms"] += t_dur
        a["turn_out"] += t_out
        a["turn_with_ttft"] += 1
    for r in d.get("providerCallRecords") or []:
        dur = r.get("turnDurationMs") or 0
        out = r.get("outputTokens") or 0
        ttft = r.get("firstTokenLatencyMs")
        a["calls"] += 1
        if ttft is None:
            a["ttft_missing"] += 1
        if not (dur > 0 and out > 0):
            continue
        a["valid"] += 1
        a["sum_out"] += out
        a["sum_dur"] += dur
        a["e2e_out"] += out
        a["e2e_dur"] += dur
        if eligible(dur, ttft, out):
            a["elig"] += 1
            a["elig_out"] += out
            a["elig_gen"] += gen(dur, ttft)


def fmt(v):
    return "n/a" if v is None else f"{v:.1f}"


print(f"db={DB}")
print(f"turns_with_trace={sum(a['turns'] for a in acc.values())}")
print(f"{'model':46s} {'turns':>5s} {'calls':>6s} {'valid':>6s} {'elig':>5s} {'ttft_miss':>9s} "
      f"{'decode':>8s} {'e2e':>8s} {'turn_wall':>10s}")
for key, a in sorted(acc.items(), key=lambda kv: -kv[1]["sum_out"]):
    if a["valid"] == 0:
        continue
    decode = (a["elig_out"] - a["elig"]) / (a["elig_gen"] / 1000) if a["elig"] and a["elig_gen"] else None
    e2e = a["e2e_out"] / (a["e2e_dur"] / 1000) if a["e2e_dur"] else None
    wall = a["turn_out"] / (a["turn_wall_ms"] / 1000) if a["turn_wall_ms"] else None
    print(f"{key[:46]:46s} {a['turns']:5d} {a['calls']:6d} {a['valid']:6d} {a['elig']:5d} "
          f"{a['ttft_missing']:9d} {fmt(decode):>8s} {fmt(e2e):>8s} {fmt(wall):>10s}")

# ---- gemini-3.8-flash 变体单独输出 ----
print("\n=== gemini-3.8-flash 变体（model like '%gemini-3.8-flash%'）===")
g = new_acc()
for key, a in acc.items():
    if "gemini-3.8-flash" in key:
        for k, v in a.items():
            g[k] += v
print(f"turns={g['turns']} calls={g['calls']} valid={g['valid']} elig={g['elig']} ttft_missing={g['ttft_missing']}")
if g["elig_gen"]:
    print(f"decode = (Σout_elig - elig) / (Σgen/1000) = ({g['elig_out']} - {g['elig']}) / "
          f"({g['elig_gen']}/1000) = {fmt((g['elig_out'] - g['elig']) / (g['elig_gen'] / 1000))} t/s")
if g["e2e_dur"]:
    print(f"end_to_end = Σout/Σdur = {g['e2e_out']} / ({g['e2e_dur']}/1000) = "
          f"{fmt(g['e2e_out'] / (g['e2e_dur'] / 1000))} t/s")
if g["turn_wall_ms"]:
    print(f"turn_wall = Σturn_out/Σturn_dur = {g['turn_out']} / ({g['turn_wall_ms']}/1000) = "
          f"{fmt(g['turn_out'] / (g['turn_wall_ms'] / 1000))} t/s")
