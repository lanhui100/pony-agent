#!/usr/bin/env bash
set -u
cd "$(dirname "$0")"
: "${PONYLLM_API_KEY:?need env}"
for i in $(seq 10 19); do
  python3 probe.py --model gemini-3.8-flash --run-id flash_r$i >/dev/null 2>raw/flash_r$i.err
  echo "--- flash_r$i rc=$?" >&2; sleep 4
done
for i in $(seq 5 8); do
  python3 probe.py --model gemini-3.8-flash-high --run-id high_r$i >/dev/null 2>raw/high_r$i.err 2>&1
  echo "--- high_r$i rc=$?" >&2; sleep 4
done
