#!/usr/bin/env python3
"""Post-process probe raw evidence into token-accurate rates.

Splits reasoning vs content tokens using usage.completion_tokens_details.reasoning_tokens
(fallback: char-proportional split). Reports phase windows + p50 instantaneous rate.
"""
import glob, json, os, statistics as st

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "raw")


def load(rid):
    s = json.load(open(os.path.join(RAW, f"{rid}.summary.json")))
    ch = [json.loads(l) for l in open(os.path.join(RAW, f"{rid}.chunks.jsonl"))]
    return s, ch


def p50_instant(chunks, kind):
    ts = [c["t"] for c in chunks if c["kind"] == kind]
    rates = []
    for a, b in zip(ts, ts[1:]):
        dt = b - a
        if dt > 0:
            rates.append(1.0 / dt)  # ~1 token per chunk in phase
    return round(st.median(rates), 1) if len(rates) >= 5 else None


def analyze(s, ch):
    usage = s.get("usage") or {}
    ctok = s.get("completion_tokens")
    rtok = (usage.get("completion_tokens_details") or {}).get("reasoning_tokens")
    ctimes = [c["t"] for c in ch if c["kind"] == "content"]
    rtimes = [c["t"] for c in ch if c["kind"] == "reasoning"]
    out = {
        "run_id": s["run_id"], "model": s["model"], "stream": s["stream"],
        "http": "OK" if not s.get("http_error") else (s["http_error"].get("error", {}) or {}).get("code"),
        "ttft": s.get("ttft_s"), "ttft_content": s.get("ttft_content_s"),
        "total": s.get("total_s"), "ctok": ctok, "rtok": rtok,
        "prompt_tokens": s.get("prompt_tokens"),
        "n_cchunk": len(ctimes), "n_rchunk": len(rtimes),
        "cchars": s.get("content_chars"), "rchars": s.get("reasoning_chars"),
    }
    if ctok:
        content_tokens = (ctok - rtok) if isinstance(rtok, int) else None
        out["content_tok_est"] = content_tokens
        # content phase window
        if len(ctimes) >= 2 and ctimes[-1] > ctimes[0]:
            cw = ctimes[-1] - ctimes[0]
            out["content_window_s"] = round(cw, 3)
            if content_tokens:
                out["content_window_tps"] = round(content_tokens / cw, 1)
        # reasoning phase
        if len(rtimes) >= 2 and rtimes[-1] > rtimes[0]:
            rw = rtimes[-1] - rtimes[0]
            out["reason_window_s"] = round(rw, 3)
            if rtok:
                out["reason_window_tps"] = round(rtok / rw, 1)
        # whole output window
        allt = ctimes + rtimes
        if allt and s.get("ttft_s") is not None:
            first, last = min(allt), max(allt)
            if last > first:
                out["out_window_s"] = round(last - first, 3)
                out["out_window_tps"] = round(ctok / (last - first), 1)
        out["e2e_tps"] = s.get("e2e_tps_by_completion_tokens")
        out["content_p50_inst_tps"] = p50_instant(ch, "content")
        out["tokens_per_cchunk"] = (round(content_tokens / len(ctimes), 2)
                                    if content_tokens and ctimes else None)
        # longest silence gap during output (stall detector)
        if len(allt) >= 2:
            gaps = [b - a for a, b in zip(sorted(allt), sorted(allt)[1:])]
            out["max_gap_s"] = round(max(gaps), 3)
            out["gaps_over_0.5s"] = sum(1 for g in gaps if g > 0.5)
    return out


def main():
    rids = sorted({os.path.basename(f).replace(".summary.json", "")
                   for f in glob.glob(os.path.join(RAW, "*.summary.json"))})
    rows = [analyze(*load(r)) for r in rids]
    with open(os.path.join(HERE, "analysis.jsonl"), "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    cols = ["run_id", "model", "http", "ttft", "total", "ctok", "rtok", "content_tok_est",
            "content_window_s", "content_window_tps", "reason_window_tps", "out_window_s",
            "out_window_tps", "e2e_tps", "content_p50_inst_tps", "tokens_per_cchunk", "max_gap_s"]
    print(" | ".join(c[:13] for c in cols))
    for r in rows:
        print(" | ".join(str(r.get(c, "-"))[:13] for c in cols))


if __name__ == "__main__":
    main()
