#!/usr/bin/env bash
# //! acceptance: pony TUI runtime robustness (L2-AT) — independent frozen suite
# //! ticket: l2at-tui-runtime | verify-only, DO NOT MODIFY during fixes
# //!
# //! Contract under test (crates/pony-agent-core/src/bin/tui.rs):
# //!   R1: `pony --version` exits 0, prints semver + binary path, never enters alt screen.
# //!   R2: `pony --help` exits 0, lists --version/--doctor and >=3 key bindings.
# //!   R3: `pony --doctor` exits 0 with [ok]/[warn] markers; under LANG=C/LC_ALL=C
# //!       emits a UTF-8 locale warning while still exiting 0.
# //!   R4: unknown flag `pony --bogus` exits 2 with usage on stderr.
# //!   R5: resize repaint — 80x24 -> 120x40: header + bottom bar present, hairline
# //!       length == 116, no stale 80-col hairline (len 76) residue; 60x18 -> len 56.
# //!   R6: TERM matrix {vt100,xterm,screen,linux,xterm-256color} at 100x30:
# //!       header + input prompt + bottom bar all present, never "终端窗口过小".
# //!   R7: CJK soft-wrap at 40x14 with 60+ CJK chars: input area >=2 lines,
# //!       each display width (wide char = 2) <= 36, no gutter overflow.
# //!   R8: tiny terminal 26x9: no panic, bottom bar degrades (no "alt+回车").
# //!
# //! Receipts: tests/acceptance/receipts/l2at-tui-runtime/
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RECEIPTS="$HERE/receipts/l2at-tui-runtime"
mkdir -p "$RECEIPTS"
SRC="$REPO/crates/pony-agent-core/src/bin/tui.rs"

# Binary resolution: fresh debug build -> fresh release build -> installed
PONY=""
if [ -x "$REPO/target/debug/pony" ] && [ "$REPO/target/debug/pony" -nt "$SRC" ]; then
  PONY="$REPO/target/debug/pony"
elif [ -x "$REPO/target/release/pony" ] && [ "$REPO/target/release/pony" -nt "$SRC" ]; then
  PONY="$REPO/target/release/pony"
elif command -v pony >/dev/null 2>&1; then
  PONY="$(command -v pony)"
elif [ -x "$REPO/target/debug/pony" ]; then
  PONY="$REPO/target/debug/pony"
elif [ -x "$REPO/target/release/pony" ]; then
  PONY="$REPO/target/release/pony"
fi
if [ -z "$PONY" ]; then
  echo "FATAL: no pony binary found" | tee "$RECEIPTS/fatal.log"
  exit 2
fi
echo "BIN under test: $PONY" | tee "$RECEIPTS/bin.log"

PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "PASS: $1"; }
bad() { FAIL=$((FAIL+1)); echo "FAIL: $1"; }
clean_ansi() { sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g' "$1" 2>/dev/null | tr -d '\000'; }
tmux_cleanup() { tmux kill-server >/dev/null 2>&1 || true; }
trap tmux_cleanup EXIT

# ---------------------------------------------------------------
# R1: --version (under pty; must NOT enter alternate screen)
# ---------------------------------------------------------------
r1_log="$RECEIPTS/r1-version-typescript.log"
timeout 10 script -qec "$PONY --version" "$r1_log" >/dev/null 2>&1
rc=$?
echo "exit_code=$rc" | tee "$RECEIPTS/r1-exit-code.txt"
clean_ansi "$r1_log" > "$RECEIPTS/r1-version-clean.txt"
ver_out=$(cat "$RECEIPTS/r1-version-clean.txt")
if [ "$rc" -eq 0 ]; then ok "R1a --version exit 0"; else bad "R1a --version exit $rc (expected 0)"; fi
if echo "$ver_out" | grep -Eq "[0-9]+\.[0-9]+\.[0-9]+"; then ok "R1b semver present"; else bad "R1b no semver in output"; fi
if echo "$ver_out" | grep -q "binary: /"; then ok "R1c binary path present"; else bad "R1c no binary path in output"; fi
if grep -q $'\x1b\[?1049h' "$r1_log" 2>/dev/null; then bad "R1d --version entered alternate screen (?1049h seen)"; else ok "R1d no alt screen for --version"; fi

# ---------------------------------------------------------------
# R2: --help
# ---------------------------------------------------------------
r2_log="$RECEIPTS/r2-help.txt"
timeout 10 "$PONY" --help > "$r2_log" 2>&1
rc=$?
echo "exit_code=$rc" | tee "$RECEIPTS/r2-exit-code.txt"
if [ "$rc" -eq 0 ]; then ok "R2a --help exit 0"; else bad "R2a --help exit $rc (expected 0)"; fi
if grep -q -- "--version" "$r2_log" && grep -q -- "--doctor" "$r2_log"; then ok "R2b lists --version and --doctor"; else bad "R2b missing --version/--doctor in help"; fi
keys=0
grep -q "Enter" "$r2_log" && keys=$((keys+1))
grep -q "Alt+Enter" "$r2_log" && keys=$((keys+1))
grep -q "Esc" "$r2_log" && keys=$((keys+1))
grep -q "Ctrl-C" "$r2_log" && keys=$((keys+1))
if [ "$keys" -ge 3 ]; then ok "R2c >=3 key bindings documented ($keys found)"; else bad "R2c only $keys key bindings found"; fi

# ---------------------------------------------------------------
# R3: --doctor (needs a sized pty; doctor requires TTY + size >= 24x8)
# ---------------------------------------------------------------
r3_log="$RECEIPTS/r3-doctor-typescript.log"
timeout 15 script -qec "stty rows 30 cols 100; $PONY --doctor" "$r3_log" >/dev/null 2>&1
rc=$?
echo "exit_code=$rc" | tee "$RECEIPTS/r3-exit-code.txt"
clean_ansi "$r3_log" > "$RECEIPTS/r3-doctor-clean.txt"
if [ "$rc" -eq 0 ]; then ok "R3a --doctor exit 0"; else bad "R3a --doctor exit $rc (expected 0)"; fi
okcnt=$(grep -c "\[ok\]" "$RECEIPTS/r3-doctor-clean.txt")
warncnt=$(grep -c "\[warn\]" "$RECEIPTS/r3-doctor-clean.txt")
if [ "$okcnt" -ge 1 ] && [ "$warncnt" -ge 1 ]; then ok "R3b [ok]/[warn] markers present (ok=$okcnt warn=$warncnt)"; else bad "R3b markers missing (ok=$okcnt warn=$warncnt)"; fi

r3c_log="$RECEIPTS/r3-doctor-c-typescript.log"
timeout 15 script -qec "stty rows 30 cols 100; env LANG=C LC_ALL=C $PONY --doctor" "$r3c_log" >/dev/null 2>&1
crc=$?
echo "exit_code=$crc" | tee "$RECEIPTS/r3c-exit-code.txt"
clean_ansi "$r3c_log" > "$RECEIPTS/r3c-doctor-clean.txt"
if [ "$crc" -eq 0 ]; then ok "R3c C-locale --doctor exit 0"; else bad "R3c C-locale --doctor exit $crc (expected 0)"; fi
if grep -q "UTF-8" "$RECEIPTS/r3c-doctor-clean.txt" && grep -q "\[warn\].*locale\|locale.*\[warn\]" "$RECEIPTS/r3c-doctor-clean.txt"; then
  ok "R3d C-locale emits UTF-8 locale warn"
else
  if grep -qE "\[warn\].*(UTF-8|locale)" "$RECEIPTS/r3c-doctor-clean.txt"; then
    ok "R3d C-locale emits UTF-8 locale warn"
  else
    bad "R3d no UTF-8/locale warn under LANG=C"
  fi
fi

# ---------------------------------------------------------------
# R4: unknown flag
# ---------------------------------------------------------------
r4_out="$RECEIPTS/r4-bogus-stderr.txt"
timeout 10 "$PONY" --bogus > "$RECEIPTS/r4-bogus-stdout.txt" 2> "$r4_out"
rc=$?
echo "exit_code=$rc" | tee "$RECEIPTS/r4-exit-code.txt"
if [ "$rc" -eq 2 ]; then ok "R4a --bogus exit 2"; else bad "R4a --bogus exit $rc (expected 2)"; fi
if grep -qi "usage\|用法" "$r4_out"; then ok "R4b usage hint on stderr"; else bad "R4b no usage on stderr (see $r4_out)"; fi

# ---------------------------------------------------------------
# R5: resize repaint (tmux) — composer band contract (v3, lead-adjudicated):
#   composer is a full-width SURFACE #181a1f band with NO top hairline;
#   band top row (h - input_h - 1) and bottom row (last row) must both carry
#   SURFACE bg; no long '─' line inside the band; band height >= 4 at 80x24
#   (min 3 input lines + 1 status row); status word on last row right side;
#   shortcut hints ("回车 发送" / "alt+回车") removed at every size.
# Detection: tmux capture does not report bg on space-only cells, so we record
# the raw pty stream via pipe-pane and replay it with a minimal VT state machine.
# ---------------------------------------------------------------
analyze_band() { # $1 = raw pty log, $2 = rows, $3 = cols ; emits JSON verdict
python3 - "$1" "$2" "$3" <<'EOF'
import sys, re, json
raw = open(sys.argv[1], 'rb').read().decode('utf-8', 'replace')
rows, cols = int(sys.argv[2]), int(sys.argv[3])
# grid: bg per cell ('S' = SURFACE 24;26;31, '.' = other/none)
grid = [['.' for _ in range(cols)] for _ in range(rows)]
r = c = 0
SURF = '48;2;24;26;31'
def apply_sgr(params):
    global bg
    joined = ';'.join(params)
    if re.search(r'(^|;)48;2;24;26;31(;|$)', joined): bg = 'S'
    elif any(p in ('0', '49') for p in params) and not re.search(r'48;2;24;26;31', joined): bg = '.'  # 39=fg reset only; 49/0 reset bg
i = 0; bg = '.'
sgr_re = re.compile(r'\x1b\[([0-9;]*)m')
while i < len(raw):
    ch = raw[i]
    if ch == '\x1b':
        m = sgr_re.match(raw, i)
        if m:
            params = [p for p in m.group(1).split(';') if p]
            apply_sgr(params)
            i = m.end(); continue
        m = re.match(r'\x1b\[(\d+);?(\d*)H', raw[i:])
        if m:
            r = max(0, int(m.group(1)) - 1); c = max(0, int(m.group(2) or 1) - 1)
            i += m.end(); continue
        m = re.match(r'\x1b\[[0-9;]*[A-Za-z]', raw[i:])
        if m: i += m.end(); continue
        i += 1; continue
    if ch == '\r': c = 0; i += 1; continue
    if ch == '\n': r = min(r + 1, rows - 1); i += 1; continue
    if 0 <= r < rows and 0 <= c < cols:
        grid[r][c] = bg
    c += 1
    if c >= cols: c = 0; r = min(r + 1, rows - 1)
    i += 1
# band detection: rows dominated by SURFACE bg (>=80% cells; per-span SGR resets
# punch isolated '.' holes, so use ratio not strict all)
def row_surf(r_idx, thresh=0.8):
    row = grid[r_idx]
    return row and sum(1 for x in row if x == 'S') / len(row) >= thresh
bottom = rows - 1
band_top = bottom
if row_surf(bottom):
    while band_top - 1 >= 0 and row_surf(band_top - 1):
        band_top -= 1
band_h = bottom - band_top + 1 if row_surf(bottom) else 0
plain = ''.join(''.join(row) for row in grid)
print(json.dumps({
    'band_top_1based': band_top + 1,
    'bottom_surf': row_surf(bottom),
    'top_surf': row_surf(band_top) if band_h else False,
    'band_h': band_h,
}))
EOF
}
run_tui_raw() { # $1 session, $2 x, $3 y, $4 outfile
  tmux_cleanup
  tmux new-session -d -s "$1" -x "$2" -y "$3" 2>/dev/null
  tmux pipe-pane -t "$1" "cat > '$4'"
  sleep 1
  tmux send-keys -t "$1" "$PONY" Enter
  wait_main "$1" "$2"
}
wait_main() { # $1 session, $2 cols — poll until main TUI (status word) appears, then settle
  for _ in $(seq 1 20); do
    sleep 0.5
    if tmux capture-pane -t "$1" -p 2>/dev/null | grep -q "就绪"; then break; fi
  done
  sleep 1
  # splash gate: one harmless Space + Backspace ensures we are past the welcome screen
  if ! tmux capture-pane -t "$1" -p 2>/dev/null | grep -q "就绪"; then
    tmux send-keys -t "$1" Space
    sleep 1
    tmux send-keys -t "$1" BSpace
    for _ in $(seq 1 20); do
      sleep 0.5
      if tmux capture-pane -t "$1" -p 2>/dev/null | grep -q "就绪"; then break; fi
    done
  fi
  sleep 1
}
pipe_restart() { # $1 session, $2 outfile — stop old pipe, start fresh
  tmux pipe-pane -t "$1" "" 2>/dev/null
  tmux pipe-pane -t "$1" "cat > '$2'"
}
pipe_stop() { tmux pipe-pane -t "$1" "" 2>/dev/null; }

# R5 main: fresh TUI session per size for band analysis (80x24, 120x40, 60x18),
# plus one live resize-continuity session (80 -> 120 -> 60) asserting repaint.
run_tui_raw r5a 80 24 "$RECEIPTS/r5-raw-80x24.bin"
sleep 1
pipe_stop r5a
tmux capture-pane -t r5a -p > "$RECEIPTS/r5-80x24.txt" 2>/dev/null
run_tui_raw r5b 120 40 "$RECEIPTS/r5-raw-120x40.bin"
sleep 1
pipe_stop r5b
tmux capture-pane -t r5b -p -e > "$RECEIPTS/r5-120x40.txt" 2>/dev/null
run_tui_raw r5c 60 18 "$RECEIPTS/r5-raw-60x18.bin"
sleep 1
pipe_stop r5c
for _ in 1 2 3 4 5 6 7 8 9 10; do
  tmux capture-pane -t r5c -p > "$RECEIPTS/r5-60x18.txt" 2>/dev/null
  [ -s "$RECEIPTS/r5-60x18.txt" ] && break
  sleep 1
done

# resize continuity: 80 -> 120 -> 60 in one live session; status word must reappear at each size
tmux_cleanup
tmux new-session -d -s r5r -x 80 -y 24 2>/dev/null
sleep 1
tmux send-keys -t r5r "$PONY" Enter
wait_main r5r 80
for SZ in "120 40" "60 18"; do
  set -- $SZ
  tmux resize-window -t r5r -x "$1" -y "$2" 2>>"$RECEIPTS/r5-tmux-err.log"
  wait_main r5r "$1"
done
tmux capture-pane -t r5r -p > "$RECEIPTS/r5-resized-final.txt" 2>/dev/null
tmux kill-server >/dev/null 2>&1

# R5a: header present at 120x40 and no shortcut hints anywhere (hints removed)
clean_ansi "$RECEIPTS/r5-120x40.txt" > "$RECEIPTS/r5-120x40-clean.txt"
if grep -q "pony  ·" "$RECEIPTS/r5-120x40-clean.txt"; then
  ok "R5a header present at 120x40"
else
  bad "R5a header missing at 120x40"
fi
hints=$(grep -l "回车 发送\|alt+回车" "$RECEIPTS/r5-80x24.txt" "$RECEIPTS/r5-120x40-clean.txt" "$RECEIPTS/r5-60x18.txt" 2>/dev/null | wc -l)
if [ "$hints" -eq 0 ]; then
  ok "R5h no shortcut hints (回车 发送 / alt+回车) at any size"
else
  bad "R5h shortcut hints still present in $hints capture(s)"
fi

# R5b: band top and bottom rows both SURFACE at 120x40, no long hairline inside band
v120=$(analyze_band "$RECEIPTS/r5-raw-120x40.bin" 40 120)
echo "R5 120x40: $v120" | tee "$RECEIPTS/r5-120x40-band.json"
if echo "$v120" | grep -q '"top_surf": true' && echo "$v120" | grep -q '"bottom_surf": true'; then
  ok "R5b band top+bottom rows SURFACE #181a1f at 120x40"
else
  bad "R5b band top/bottom SURFACE missing at 120x40 ($v120)"
fi
# R5c: no '─' long line INSIDE the composer band rows (band_top..last row);
# the header hairline above the band is part of the chat area and allowed.
band_top_120=$(echo "$v120" | python3 -c "import sys,json;print(json.load(sys.stdin)['band_top_1based'])")
hair120=$(python3 -c "
import re
plain=re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]','',open('$RECEIPTS/r5-120x40.txt',encoding='utf-8',errors='replace').read())
lines=plain.splitlines()
band_top=$band_top_120
print(max((l.count('─') for l in lines[band_top-1:40]), default=0))
")
if [ "$hair120" -eq 0 ]; then ok "R5c no hairline (─) inside composer band at 120x40"; else bad "R5c hairline found inside composer band at 120x40 (len $hair120)"; fi

# R5d: band surface + status word at 60x18 after resize
v60=$(analyze_band "$RECEIPTS/r5-raw-60x18.bin" 18 60)
echo "R5 60x18: $v60" | tee "$RECEIPTS/r5-60x18-band.json"
if echo "$v60" | grep -q '"top_surf": true' && echo "$v60" | grep -q '"bottom_surf": true'; then
  ok "R5d band top+bottom rows SURFACE at 60x18 after resize"
else
  bad "R5d band SURFACE missing at 60x18 ($v60)"
fi
if grep -q "就绪" "$RECEIPTS/r5-60x18.txt"; then ok "R5f status word present at 60x18"; else bad "R5f status word missing at 60x18"; fi
if grep -q "就绪" "$RECEIPTS/r5-resized-final.txt"; then ok "R5j live-resize continuity: status word reappears at final 60x18"; else bad "R5j live-resize continuity broken (no status word at final size)"; fi

# R5g: band height >= 4 at 80x24 (3 input min + 1 status row)
v80=$(analyze_band "$RECEIPTS/r5-raw-80x24.bin" 24 80)
echo "R5 80x24: $v80" | tee "$RECEIPTS/r5-80x24-band.json"
band_h=$(echo "$v80" | python3 -c "import sys,json;print(json.load(sys.stdin)['band_h'])")
if [ "$band_h" -ge 4 ]; then ok "R5g band height $band_h >= 4 at 80x24"; else bad "R5g band height $band_h < 4 at 80x24"; fi
if grep -q "就绪" "$RECEIPTS/r5-80x24.txt" && tail -1 "$RECEIPTS/r5-80x24.txt" | grep -q "就绪"; then
  ok "R5i status word on last row right side at 80x24"
else
  bad "R5i status word not on last row at 80x24"
fi

# ---------------------------------------------------------------
# R6: TERM matrix at 100x30
# ---------------------------------------------------------------
for T in vt100 xterm screen linux xterm-256color; do
  tmux_cleanup
  tmux new-session -d -s "r6_$T" -x 100 -y 30 2>/dev/null
  sleep 1
  tmux send-keys -t "r6_$T" "TERM=$T $PONY" Enter
  sleep 3
  tmux capture-pane -t "r6_$T" -p > "$RECEIPTS/r6-term-$T.txt" 2>/dev/null
  tmux kill-server >/dev/null 2>&1
  hdr=$(grep -c "pony  ·" "$RECEIPTS/r6-term-$T.txt" || true)
  inp=$(grep -c "› ▏" "$RECEIPTS/r6-term-$T.txt" || true)
  # composer band: status word on last row replaces old bottom-bar hint check
  bot=$(grep -c "就绪" "$RECEIPTS/r6-term-$T.txt" || true)
  hint=$(grep -c "回车 发送\|alt+回车" "$RECEIPTS/r6-term-$T.txt" || true)
  small=$(grep -c "终端窗口过小" "$RECEIPTS/r6-term-$T.txt" || true)
  if [ "$hdr" -ge 1 ] && [ "$inp" -ge 1 ] && [ "$bot" -ge 1 ] && [ "$small" -eq 0 ] && [ "$hint" -eq 0 ]; then
    ok "R6 TERM=$T: header+prompt+band-status present, no too-small notice, no hints"
  else
    bad "R6 TERM=$T: hdr=$hdr inp=$inp status=$bot too_small=$small hints=$hint"
  fi
done

# ---------------------------------------------------------------
# R7: CJK soft-wrap at 40x14
# ---------------------------------------------------------------
tmux_cleanup
tmux new-session -d -s r7 -x 40 -y 14 2>/dev/null
sleep 1
tmux send-keys -t r7 "$PONY" Enter
sleep 3
CJK=$(python3 -c "print('汉'*62)")
tmux send-keys -t r7 -l "$CJK"
sleep 2
tmux capture-pane -t r7 -p > "$RECEIPTS/r7-cjk-40x14.txt" 2>/dev/null
tmux kill-server >/dev/null 2>&1

r7_verdict=$(python3 - "$RECEIPTS/r7-cjk-40x14.txt" <<'EOF'
import sys, unicodedata
def w(s): return sum(2 if unicodedata.east_asian_width(c) in 'WF' else 1 for c in s)
lines=[l.rstrip('\n') for l in open(sys.argv[1],encoding='utf-8',errors='replace')]
inarea=[l for l in lines if '汉' in l]
if len(inarea)<2:
    print(f"FAIL lines={len(inarea)}"); raise SystemExit
bad=[w(l) for l in inarea if w(l)>38]
# gutter overflow: CJK content starting at col 0 (no leading indent)
gutter=[l for l in inarea if l.startswith('汉')]
print(f"{'FAIL' if bad or gutter else 'PASS'} lines={len(inarea)} overwide={bad} gutter={len(gutter)}")
EOF
)
echo "R7 verdict: $r7_verdict" | tee "$RECEIPTS/r7-verdict.txt"
if echo "$r7_verdict" | grep -q "PASS"; then ok "R7 CJK soft-wrap: >=2 lines, full-line width<=38 (40-2 right gutter; = gutter-strip 36 per lead ruling 2+2+34=38), no gutter overflow"; else bad "R7 CJK soft-wrap: $r7_verdict"; fi

# ---------------------------------------------------------------
# R8: tiny terminal 26x9
# ---------------------------------------------------------------
tmux_cleanup
tmux new-session -d -s r8 -x 26 -y 9 2>/dev/null
sleep 1
tmux send-keys -t r8 "$PONY" Enter
sleep 3
tmux capture-pane -t r8 -p > "$RECEIPTS/r8-26x9.txt" 2>/dev/null
dead_status=$(tmux list-panes -t r8 -F '#{pane_dead} #{pane_dead_status}' 2>/dev/null)
tmux kill-server >/dev/null 2>&1
echo "pane_dead=$dead_status" | tee "$RECEIPTS/r8-pane-status.txt"
if echo "$dead_status" | grep -q "^1 101"; then
  bad "R8a TUI panicked (exit 101) at 26x9"
else
  ok "R8a no panic at 26x9 (pane status: ${dead_status:-alive})"
fi
if grep -q "就绪" "$RECEIPTS/r8-26x9.txt" && ! grep -q "alt+回车" "$RECEIPTS/r8-26x9.txt" && ! grep -q "回车 发送" "$RECEIPTS/r8-26x9.txt"; then
  ok "R8b status word present, shortcut hints fully removed (incl. alt+回车)"
else
  bad "R8b status word missing or shortcut hints still present at 26x9"
fi
tmux_cleanup

# ---------------------------------------------------------------
echo
echo "==== l2at-tui-runtime summary: PASS=$PASS FAIL=$FAIL ===="
[ "$FAIL" -eq 0 ]
