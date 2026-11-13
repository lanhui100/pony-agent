#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
: "${PONYLLM_API_KEY:?need env}"
run() { local model="$1" tag="$2" extra="$3" reps="${4:-3}"; local start="${5:-1}"
  for i in $(seq "$start" $((start+reps-1))); do
    local rid="${tag}_r${i}"
    if [ -n "$extra" ]; then python3 probe.py --model "$model" --run-id "$rid" --extra "$extra" >/dev/null 2>raw/"$rid".err
    else python3 probe.py --model "$model" --run-id "$rid" >/dev/null 2>raw/"$rid".err; fi
    echo "--- $rid rc=$?" >&2; sleep 4
  done; }
run gemini-3.8-flash flash "" 6 4
run antigravity/gemini-3.8-flash agflash "" 6 3
run gemini-3.8-flash-high high "" 3 4
