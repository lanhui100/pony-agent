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
# R5: resize repaint (tmux)
# ---------------------------------------------------------------
tmux_cleanup
tmux new-session -d -s r5 -x 80 -y 24 2>"$RECEIPTS/r5-tmux-err.log"
sleep 1
tmux send-keys -t r5 "$PONY" Enter
sleep 3
tmux capture-pane -t r5 -p > "$RECEIPTS/r5-80x24.txt" 2>/dev/null
tmux resize-window -t r5 -x 120 -y 40 2>>"$RECEIPTS/r5-tmux-err.log"
sleep 3
tmux capture-pane -t r5 -p > "$RECEIPTS/r5-120x40.txt" 2>/dev/null
tmux resize-window -t r5 -x 60 -y 18 2>>"$RECEIPTS/r5-tmux-err.log"
sleep 3
tmux capture-pane -t r5 -p > "$RECEIPTS/r5-60x18.txt" 2>/dev/null
tmux kill-server >/dev/null 2>&1

if grep -q "pony  ·" "$RECEIPTS/r5-120x40.txt" && grep -q "回车 发送" "$RECEIPTS/r5-120x40.txt"; then
  ok "R5a header + bottom bar present at 120x40"
else
  bad "R5a header/bottom bar missing at 120x40"
fi
hair120=$(python3 - "$RECEIPTS/r5-120x40.txt" <<'EOF'
import sys
mx=0
for l in open(sys.argv[1],encoding='utf-8',errors='replace'):
    mx=max(mx,l.rstrip('\n').count('─'))
print(mx)
EOF
)
if [ "$hair120" -eq 116 ]; then ok "R5b hairline length 116 at 120x40 (got $hair120)"; else bad "R5b hairline at 120x40 is $hair120 (expected 116)"; fi
if grep -q "────────────\|────────────" "$RECEIPTS/r5-120x40.txt"; then :; fi
stale76=$(python3 - "$RECEIPTS/r5-120x40.txt" <<'EOF'
import sys
n=0
for l in open(sys.argv[1],encoding='utf-8',errors='replace'):
    if l.rstrip('\n').count('─')==76: n+=1
print(n)
EOF
)
if [ "$stale76" -eq 0 ]; then ok "R5c no stale 80-col hairline (len 76) residue"; else bad "R5c stale 76-char hairline residue found ($stale76 lines)"; fi
hair60=$(python3 - "$RECEIPTS/r5-60x18.txt" <<'EOF'
import sys
mx=0
for l in open(sys.argv[1],encoding='utf-8',errors='replace'):
    mx=max(mx,l.rstrip('\n').count('─'))
print(mx)
EOF
)
if [ "$hair60" -eq 56 ]; then ok "R5d hairline length 56 at 60x18 (got $hair60)"; else bad "R5d hairline at 60x18 is $hair60 (expected 56)"; fi

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
  bot=$(grep -c "回车 发送" "$RECEIPTS/r6-term-$T.txt" || true)
  small=$(grep -c "终端窗口过小" "$RECEIPTS/r6-term-$T.txt" || true)
  if [ "$hdr" -ge 1 ] && [ "$inp" -ge 1 ] && [ "$bot" -ge 1 ] && [ "$small" -eq 0 ]; then
    ok "R6 TERM=$T: header+prompt+bottom present, no too-small notice"
  else
    bad "R6 TERM=$T: hdr=$hdr inp=$inp bot=$bot too_small=$small"
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
if grep -q "回车 发送" "$RECEIPTS/r8-26x9.txt" && ! grep -q "alt+回车" "$RECEIPTS/r8-26x9.txt"; then
  ok "R8b bottom bar degraded (present without alt+回车)"
else
  bad "R8b bottom bar not properly degraded (alt+回车 present or bar missing)"
fi
tmux_cleanup

# ---------------------------------------------------------------
echo
echo "==== l2at-tui-runtime summary: PASS=$PASS FAIL=$FAIL ===="
[ "$FAIL" -eq 0 ]
