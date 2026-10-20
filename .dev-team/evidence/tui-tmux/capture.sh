#!/usr/bin/env bash
# Reproducible tmux capture harness for the pony TUI.
#
# usage: capture.sh <session-name> <shots-dir> [--key KEY]... --shot NAME
#
# Each `--key` sends one key sequence (tmux send-keys syntax) and waits 300ms;
# each `--shot` captures the pane (ANSI + PNG) under <shots-dir>/<NAME>.{ansi,png}.
set -uo pipefail

SESSION="${1:?session name required}"
OUTDIR="${2:?output dir required}"
shift 2

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BIN="$ROOT/target/debug/pony"
COLS=120
ROWS=36
mkdir -p "$OUTDIR"

tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS"
tmux send-keys -t "$SESSION" "$BIN" Enter
sleep 2

while [ $# -gt 0 ]; do
  case "$1" in
    --key)
      tmux send-keys -t "$SESSION" "$2"
      sleep 0.35
      shift 2
      ;;
    --shot)
      tmux capture-pane -t "$SESSION" -p -e > "$OUTDIR/$2.ansi"
      python3 "$ROOT/.dev-team/evidence/tui-tmux/ansi2png.py" \
        "$OUTDIR/$2.ansi" "$OUTDIR/$2.png"
      shift 2
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

tmux kill-session -t "$SESSION" 2>/dev/null || true
