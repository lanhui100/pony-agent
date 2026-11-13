#!/usr/bin/env python3
"""Upstream TPS probe: raw curl -N streaming, per-chunk timestamps, JSONL evidence.

Usage:
  python3 probe.py --model M --run-id ID [--max-tokens 512] [--stream true|false]
                   [--extra '{"reasoning_effort":"high"}'] [--prompt-file F]
Key is read from PONYLLM_API_KEY env (never written to disk/report).
"""
import argparse, json, os, re, subprocess, sys, time

BASE = "https://tokens.ponyjob.top/v1/chat/completions"
KEY = os.environ.get("PONYLLM_API_KEY", "")
if not KEY:
    sys.exit("PONYLLM_API_KEY not set")
OUT = os.path.dirname(os.path.abspath(__file__))

# Fixed input ~50 tokens: neutral enumeration task that forces long deterministic output.
DEFAULT_PROMPT = (
    "Write a plain list of 40 short factual statements about the solar system, "
    "one per line, numbered 1 to 40. No preamble, no conclusion, no markdown headers."
)


def build_body(args):
    body = {
        "model": args.model,
        "messages": [{"role": "user", "content": args.prompt}],
        "max_tokens": args.max_tokens,
        "stream": args.stream,
    }
    if args.stream:
        body["stream_options"] = {"include_usage": True}
    if args.extra:
        body.update(json.loads(args.extra))
    return body


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--run-id", required=True)
    p.add_argument("--max-tokens", type=int, default=512)
    p.add_argument("--stream", type=lambda s: s.lower() == "true", default=True)
    p.add_argument("--extra", default="")
    p.add_argument("--prompt", default=DEFAULT_PROMPT)
    p.add_argument("--timeout", type=int, default=300)
    args = p.parse_args()

    body = build_body(args)
    payload = json.dumps(body)
    cmd = ["curl", "-N", "-sS", "-m", str(args.timeout),
           "-H", "Authorization: Bearer <redacted>",
           "-H", "Content-Type: application/json",
           "-d", payload, BASE]
    # real command (key injected via header file to avoid argv leakage)
    real_cmd = ["curl", "-N", "-sS", "-m", str(args.timeout),
                "-H", "@" + os.path.join(OUT, "raw", ".hdr"),
                "-H", "Content-Type: application/json",
                "-d", payload, BASE]

    summary = {
        "run_id": args.run_id, "model": args.model, "stream": args.stream,
        "max_tokens": args.max_tokens, "extra": args.extra, "prompt_chars": len(args.prompt),
        "prompt": args.prompt, "request_body": body,
        "cmd": " ".join(cmd), "base_url": BASE, "ts_iso": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }

    t0 = time.perf_counter()
    proc = subprocess.Popen(real_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)

    chunks = []          # per-SSE-event records
    raw_lines = []       # verbatim stdout lines
    content_txt, reasoning_txt = [], []
    usage = None
    first_any = first_content = first_reasoning = None
    finish_reason = None
    err_payload = None
    idx = 0
    for line in proc.stdout:
        now = time.perf_counter() - t0
        raw_lines.append(line)
        s = line.strip()
        if not s or s.startswith(":"):
            continue
        if s.startswith("data:"):
            data = s[5:].strip()
        else:
            data = s
        if data == "[DONE]":
            chunks.append({"i": idx, "t": round(now, 4), "kind": "done"})
            idx += 1
            continue
        try:
            obj = json.loads(data)
        except Exception:
            chunks.append({"i": idx, "t": round(now, 4), "kind": "unparsed", "raw": data[:400]})
            idx += 1
            continue
        if isinstance(obj, dict) and obj.get("error"):
            err_payload = obj
            chunks.append({"i": idx, "t": round(now, 4), "kind": "error", "raw": json.dumps(obj)[:800]})
            idx += 1
            continue
        if isinstance(obj, dict) and obj.get("usage"):
            usage = obj["usage"]
        ch = (obj.get("choices") or [{}])[0] if isinstance(obj, dict) else {}
        delta = ch.get("delta") or (ch.get("message") or {})
        c = delta.get("content") or ""
        r = delta.get("reasoning_content") or delta.get("reasoning") or ""
        if ch.get("finish_reason"):
            finish_reason = ch["finish_reason"]
        if c:
            content_txt.append(c)
            if first_content is None:
                first_content = now
                first_any = first_any if first_any is not None else now
        if r:
            reasoning_txt.append(r)
            if first_reasoning is None:
                first_reasoning = now
                first_any = first_any if first_any is not None else now
        kind = "reasoning" if (r and not c) else ("content" if c else "meta")
        chunks.append({"i": idx, "t": round(now, 4), "kind": kind,
                       "c_len": len(c), "r_len": len(r)})
        idx += 1
    stderr = proc.stderr.read()
    t_end = time.perf_counter() - t0
    rc = proc.wait()

    content_times = [c["t"] for c in chunks if c["kind"] == "content"]
    reasoning_times = [c["t"] for c in chunks if c["kind"] == "reasoning"]
    n_content_chunks = len(content_times)
    n_reason_chunks = len(reasoning_times)

    def rate(times):
        if len(times) < 2:
            return None
        return round((len(times) - 1) / (times[-1] - times[0]), 2) if times[-1] > times[0] else None

    out_times = [c["t"] for c in chunks if c["kind"] in ("content", "reasoning")]
    last_output = out_times[-1] if out_times else None
    decode_window = (last_output - first_any) if (last_output and first_any is not None) else None
    completion_tokens = (usage or {}).get("completion_tokens")
    summary.update({
        "exit_code": rc, "stderr": stderr.strip()[:800],
        "http_error": err_payload,
        "ttft_s": round(first_any, 4) if first_any is not None else None,
        "ttft_content_s": round(first_content, 4) if first_content else None,
        "ttft_reasoning_s": round(first_reasoning, 4) if first_reasoning else None,
        "total_s": round(t_end, 4),
        "n_sse_events": len(chunks),
        "n_content_chunks": n_content_chunks,
        "n_reasoning_chunks": n_reason_chunks,
        "reasoning_chars": sum(len(x) for x in reasoning_txt),
        "content_chars": sum(len(x) for x in content_txt),
        "decode_tps_content": rate(content_times),
        "decode_tps_reasoning": rate(reasoning_times),
        "e2e_tps_by_completion_tokens": (round(completion_tokens / t_end, 2)
                                        if completion_tokens and t_end else None),
        "decode_window_s": round(decode_window, 4) if decode_window else None,
        "decode_window_tps": (round(completion_tokens / decode_window, 2)
                              if completion_tokens and decode_window else None),
        "chunks_per_completion_token": (round(len(out_times) / completion_tokens, 3)
                                        if completion_tokens and out_times else None),
        "completion_tokens": completion_tokens,
        "prompt_tokens": (usage or {}).get("prompt_tokens"),
        "usage": usage,
        "has_reasoning_tokens_in_usage": any(
            k in (usage or {}) for k in ("reasoning_tokens", "completion_tokens_details")),
        "finish_reason": finish_reason,
        "content_head": "".join(content_txt)[:300],
        "reasoning_head": "".join(reasoning_txt)[:300],
    })

    rawdir = os.path.join(OUT, "raw")
    with open(os.path.join(rawdir, f"{args.run_id}.chunks.jsonl"), "w") as f:
        for c in chunks:
            f.write(json.dumps(c, ensure_ascii=False) + "\n")
    with open(os.path.join(rawdir, f"{args.run_id}.sse.txt"), "w") as f:
        f.writelines(raw_lines)
    with open(os.path.join(rawdir, f"{args.run_id}.summary.json"), "w") as f:
        json.dump(summary, f, indent=2, ensure_ascii=False)
    print(json.dumps({k: summary[k] for k in (
        "run_id", "model", "exit_code", "http_error", "ttft_s", "ttft_content_s",
        "ttft_reasoning_s", "total_s", "n_content_chunks", "n_reasoning_chunks",
        "decode_tps_content", "decode_tps_reasoning", "e2e_tps_by_completion_tokens",
        "completion_tokens", "prompt_tokens", "finish_reason")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
