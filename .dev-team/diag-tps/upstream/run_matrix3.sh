#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
: "${PONYLLM_API_KEY:?need env}"
run() { local model="$1" tag="$2" extra="$3" reps="${4:-2}" start="${5:-1}"
  for i in $(seq "$start" $((start+reps-1))); do
    local rid="${tag}_r${i}"
    if [ -n "$extra" ]; then python3 probe.py --model "$model" --run-id "$rid" --extra "$extra" >/dev/null 2>raw/"$rid".err
    else python3 probe.py --model "$model" --run-id "$rid" >/dev/null 2>raw/"$rid".err; fi
    echo "--- $rid rc=$?" >&2; sleep 4
  done; }
runstream_false() { local model="$1" tag="$2" reps="${3:-3}"
  for i in $(seq 1 "$reps"); do
    python3 probe.py --model "$model" --run-id "$tag"_r$i --stream false >/dev/null 2>raw/"$tag"_r$i.err
    echo "--- ${tag}_r$i rc=$?" >&2; sleep 4
  done; }
# 1) more -high attempts (scarce: quota)
run gemini-3.8-flash-high high "" 6 7
# 2) param ablations on gemini-3.8-flash
run gemini-3.8-flash efflow '{"reasoning_effort":"low"}' 2
run gemini-3.8-flash effhigh '{"reasoning_effort":"high"}' 2
run gemini-3.8-flash thinkdis '{"thinking":{"type":"disabled"}}' 2
run gemini-3.8-flash reffort '{"reasoning":{"effort":"low"}}' 1
# 3) non-stream end-to-end control
runstream_false gemini-3.8-flash nostream 3
runstream_false antigravity/gemini-3.8-flash agnostream 2
