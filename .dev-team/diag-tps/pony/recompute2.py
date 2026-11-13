#!/usr/bin/env python3
"""只读复算 v2：sessions.db per-call 记录 → 三种口径 + 派生 basis 分布。
用法: python3 recompute2.py [db_path]
不写 DB（mode=ro）。输出可直接粘贴进 report.md。
"""
import json
import sqlite3
import sys

DB = sys.argv[1] if len(sys.argv) > 1 else "/home/dm/.local/share/PonyAgent/sessions.db"
con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
ROWS = con.execute(
    "select n.provider_model, n.turn_id, t.trace_data "
    "from session_turn_traces t join normalized_turn_traces n "
    "on n.session_id=t.session_id and n.turn_id=t.turn_id"
).fetchall()


def floor_ms(d):
    return max(200, d * 5 // 100)


def gen(d, t):
    return d - t if (t is not None and d > t) else d


def elig(d, t, o):
    return bool(d and o and d > 0 and o > 0 and t is not None and d > t and o >= 2
                and (d - t) >= floor_ms(d))


def blank():
    return {k: 0 for k in (
        "turns", "calls", "valid", "elig", "ttft_missing",
        "v_out", "v_dur", "v_gen", "e_out", "e_gen", "e_n")}


def add(a, d, t, o):
    a["calls"] += 1
    if t is None:
        a["ttft_missing"] += 1
    if not (d and o and d > 0 and o > 0):
        return
    a["valid"] += 1
    a["v_out"] += o
    a["v_dur"] += d
    a["v_gen"] += gen(d, t)
    if elig(d, t, o):
        a["elig"] += 1
        a["e_out"] += o
        a["e_gen"] += gen(d, t)
        a["e_n"] += 1


def rates(a, label):
    legacy = a["v_out"] / (a["v_gen"] / 1000) if a["v_gen"] else None
    decode = (a["e_out"] - a["e_n"]) / (a["e_gen"] / 1000) if a["e_gen"] else None
    e2e = a["v_out"] / (a["v_dur"] / 1000) if a["v_dur"] else None
    print(f"{label:44s} turns={a['turns']:4d} calls={a['calls']:6d} valid={a['valid']:6d} "
          f"elig={a['elig']:5d} ttft_missing={a['ttft_missing']:5d} | "
          f"legacy={legacy:7.1f} decode={decode:7.1f} e2e={e2e:6.1f}" if legacy and decode and e2e else
          f"{label:44s} (insufficient)")
    return legacy, decode, e2e


ALL = blank()
BYMODEL = {}
BASIS = {"stream_decode": 0, "end_to_end": 0, "none": 0}
BASIS_GEMINI = {"stream_decode": 0, "end_to_end": 0, "none": 0}
TURN_VALUES = []

for model, turn_id, raw in ROWS:
    try:
        d = json.loads(raw)
    except Exception:
        continue
    recs = d.get("providerCallRecords") or []
    key = model or "(unknown)"
    a = BYMODEL.setdefault(key, blank())
    a["turns"] += 1
    ALL["turns"] += 1
    tacc = blank()
    for r in recs:
        dur = r.get("turnDurationMs") or 0
        out = r.get("outputTokens") or 0
        ttft = r.get("firstTokenLatencyMs")
        add(a, dur, ttft, out)
        add(ALL, dur, ttft, out)
        add(tacc, dur, ttft, out)
    if tacc["elig"] > 0:
        basis = "stream_decode"
    elif tacc["valid"] > 0:
        basis = "end_to_end"
    else:
        basis = "none"
    BASIS[basis] += 1
    if "gemini-3.8-flash" in key:
        BASIS_GEMINI[basis] += 1
    if basis != "none":
        TURN_VALUES.append((basis, tacc))

print("== 全库汇总 ==")
rates(ALL, "ALL")
print("\n== 按 model 分组 ==")
for k, a in sorted(BYMIDEL := BYMODEL.items(), key=lambda kv: -kv[1]["v_out"]) if False else sorted(BYMODEL.items(), key=lambda kv: -kv[1]["v_out"]):
    if a["valid"] == 0:
        continue
    rates(a, k[:44])

print("\n== 派生 basis 分布（按轮；basis 由记录同源推导，DB 未持久化该字段）==")
print("全库  :", BASIS)
print("gemini:", BASIS_GEMINI)

print("\n== 轮级速度分位（按派生 basis，契约 I4/I5 同式）==")
for basis in ("stream_decode", "end_to_end"):
    vals = []
    for b, a in TURN_VALUES:
        if b != basis:
            continue
        if b == "stream_decode":
            vals.append((a["e_out"] - a["e_n"]) / (a["e_gen"] / 1000))
        else:
            vals.append(a["v_out"] / (a["v_dur"] / 1000))
    vals = sorted(v for v in vals if v > 0)
    if not vals:
        continue
    q = lambda p: vals[min(len(vals) - 1, int(len(vals) * p))]
    print(f"{basis:14s} n={len(vals):4d} p10={q(.10):7.1f} p25={q(.25):7.1f} "
          f"p50={q(.50):7.1f} p75={q(.75):7.1f} p90={q(.90):7.1f} max={vals[-1]:8.1f}")
