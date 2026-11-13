#!/usr/bin/env bash
# Matrix driver: N repeats per (model, extra) cell, JSONL summaries appended.
set -u
cd "$(dirname "$0")"
: "${PONYLLM_API_KEY:?need env}"
OUT=raw/matrix.jsonl
: > "$OUT"

run() { # model tag extra repeats
  local model="$1" tag="$2" extra="$3" reps="${4:-3}"
  for i in $(seq 1 "$reps"); do
    local rid="${tag}_r${i}"
    if [ -n "$extra" ]; then
      python3 probe.py --model "$model" --run-id "$rid" --extra "$extra" >> "$OUT" 2>raw/"$rid".err
    else
      python3 probe.py --model "$model" --run-id "$rid" >> "$OUT" 2>raw/"$rid".err
    fi
    echo "--- $rid done rc=$?" >&2
    sleep 4
  done
}

run gemini-3.8-flash flash "" 3
run antigravity/gemini-3.8-flash agflash "" 3
run gemini-3.8-flash-high high "" 3
run gemini-3.8-flash-medium medium "" 2
