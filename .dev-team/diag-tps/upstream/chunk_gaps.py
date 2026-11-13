#!/usr/bin/env python3
"""Content-chunk inter-arrival stats (for throttle-bite analysis)."""
import json, sys, statistics as st

def stats(rid):
    ch = [json.loads(l) for l in open(f"raw/{rid}.chunks.jsonl")]
    ct = [c["t"] for c in ch if c["kind"] == "content"]
    at = [c["t"] for c in ch if c["kind"] in ("content", "reasoning")]
    def gap(times):
        g = [(b - a) * 1000 for a, b in zip(times, times[1:])]
        if not g:
            return None
        g_s = sorted(g)
        p = lambda q: round(g_s[min(len(g_s) - 1, int(q * (len(g_s) - 1)))], 1)
        return {"n": len(g), "med_ms": round(st.median(g), 1), "p10_ms": p(.10),
                "p90_ms": p(.90), "max_ms": round(max(g), 1)}
    return {"run_id": rid, "content": gap(ct), "all_output": gap(at),
            "n_content": len(ct)}

for rid in sys.argv[1:]:
    print(json.dumps(stats(rid), ensure_ascii=False))
