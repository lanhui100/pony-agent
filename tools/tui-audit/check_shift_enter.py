#!/usr/bin/env python3
"""wave-40 A2: real-PTY proof that Shift+Enter inserts a newline (kitty keyboard protocol).

Harness overview
----------------
1. Fork a PTY, run the TUI binary with an isolated HOME/XDG and TERM=xterm-256color
   (window size 120x34 so the app does not hit the "terminal too small" fallback).
2. crossterm probes the terminal with `CSI ? u` + `CSI c`; we answer like kitty would:
   `CSI ? 1 u` (keyboard enhancement flags = 1, disambiguate) followed by
   `CSI ? 1 ; 2 c` (primary device attributes RESPONSE).  Order matters: crossterm
   treats the DA1 response as "protocol unsupported" if it arrives before the flags
   response.  NOTE: bare `ESC [ c` is the DA1 *query*, not a response - crossterm's
   parser only accepts `CSI ? ... c`; sending the query back loops/errors.
3. Assert the app pushes keyboard enhancement flags: `CSI > N u` with the
   DISAMBIGUATE bit set.  NOTE: crossterm 0.28.1 serializes
   `KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES` (bits = 1) as `CSI > 1 u`
   (`PushKeyboardEnhancementFlags` in crossterm/src/event.rs), not `CSI > 2 u`;
   the contract's `>2u` literal is off by one flag bit.  We assert the actual
   protocol value (disambiguate bit set) and record the exact bytes.
4. Type `abc`, then send kitty `CSI 13;2u` (Shift+Enter): the composer must still
   hold `abc` and must NOT submit (no thinking/writing state).
5. Send bare CR (plain Enter): the composer must submit/clear (`abc` leaves the
   input row; a provider error is allowed because HOME/XDG are isolated).
6. Ctrl-C twice: the process must exit, print no panic, and leave the alternate
   screen (terminal restored).  If flags were pushed, a pop (`CSI < u`) is required.

Exit codes: 0 = all assertions passed, 1 = at least one assertion failed,
2 = harness error (PTY/startup/evidence problem).

Evidence: .dev-team/evidence/wave40-shift-enter/
            screen-before.txt, screen-after-shift-enter.txt, screen-after-enter.txt,
            report.json
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import pty
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import time
import unicodedata

COLS, ROWS = 120, 34

PROBE = b"\x1b[?u"
# kitty flags response first, then primary device attributes RESPONSE (DA1).
# (bare `ESC [ c` is the DA1 query, not a response; crossterm expects `CSI ? ... c`)
PROBE_REPLY = b"\x1b[?1u\x1b[?1;2c"
PUSH_RE = re.compile(rb"\x1b\[>([0-9]+)u")
# crossterm 0.28.1 serializes PopKeyboardEnhancementFlags as `CSI < 1 u`
# (event.rs:533 `csi!("<1u")`), not a bare `CSI < u`; accept any flag value.
POP_RE = re.compile(rb"\x1b\[<[0-9]*u")
SHIFT_ENTER = b"\x1b[13;2u"  # kitty CSI u: codepoint 13 (Enter), modifier 2 (Shift)
PLAIN_ENTER = b"\r"
CTRL_C = b"\x03"
LEAVE_ALT_SCREEN = b"\x1b[?1049l"
CSI_RE = re.compile(r"\x1b\[([0-9;:?<>!=]*)([ -/]*)([@-~])")
SUBMIT_MARKERS = ("思考中", "生成中", "Thinking", "Responding", "准备就绪中")


# ---------------------------------------------------------------------------
# Minimal ANSI cell-grid emulator (std-lib only)
# ---------------------------------------------------------------------------
class Screen:
    """Enough of a terminal emulator to reconstruct the TUI's visible grid."""

    def __init__(self, cols: int, rows: int):
        self.cols, self.rows = cols, rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.x = self.y = 0

    # -- primitives ---------------------------------------------------------
    def _scroll(self) -> None:
        self.grid.pop(0)
        self.grid.append([" "] * self.cols)
        self.y = self.rows - 1

    def _newline(self) -> None:
        self.y += 1
        if self.y >= self.rows:
            self._scroll()

    def _put(self, ch: str) -> None:
        wide = unicodedata.east_asian_width(ch) in ("W", "F")
        width = 2 if wide else 1
        if self.x + width > self.cols:
            self.x = 0
            self._newline()
        self.grid[self.y][self.x] = ch
        if wide:
            self.grid[self.y][self.x + 1] = ""
        self.x += width

    def _erase_line(self, mode: int) -> None:
        if mode == 2:
            self.grid[self.y] = [" "] * self.cols
        elif mode == 1:
            for x in range(0, min(self.x + 1, self.cols)):
                self.grid[self.y][x] = " "
        else:
            for x in range(self.x, self.cols):
                self.grid[self.y][x] = " "

    def _erase_screen(self, mode: int) -> None:
        if mode in (2, 3):
            self.grid = [[" "] * self.cols for _ in range(self.rows)]
        elif mode == 1:
            self._erase_line(1)
            for y in range(0, self.y):
                self.grid[y] = [" "] * self.cols
        else:
            self._erase_line(0)
            for y in range(self.y + 1, self.rows):
                self.grid[y] = [" "] * self.cols

    def _csi(self, params: str, final: str) -> None:
        raw = params.lstrip("?<>!=")
        nums = [int(p) if p.isdigit() else 0 for p in raw.split(";")] if raw else []

        def arg(index: int, default: int = 1) -> int:
            if index < len(nums) and nums[index]:
                return nums[index]
            return default

        if final in ("H", "f"):
            self.y = min(max(arg(0) - 1, 0), self.rows - 1)
            self.x = min(max(arg(1) - 1, 0), self.cols - 1)
        elif final == "J":
            self._erase_screen(arg(0, 0))
        elif final == "K":
            self._erase_line(arg(0, 0))
        elif final == "G":
            self.x = min(max(arg(0) - 1, 0), self.cols - 1)
        elif final == "d":
            self.y = min(max(arg(0) - 1, 0), self.rows - 1)
        elif final == "A":
            self.y = max(self.y - arg(0), 0)
        elif final == "B":
            self.y = min(self.y + arg(0), self.rows - 1)
        elif final == "C":
            self.x = min(self.x + arg(0), self.cols - 1)
        elif final == "D":
            self.x = max(self.x - arg(0), 0)
        # SGR (m), private modes (h/l), kitty flags (u), scroll regions (r), etc.: ignore.

    # -- driver -------------------------------------------------------------
    def feed(self, text: str) -> None:
        i, n = 0, len(text)
        while i < n:
            ch = text[i]
            if ch == "\x1b":
                m = CSI_RE.match(text, i)
                if m:
                    self._csi(m.group(1), m.group(3))
                    i = m.end()
                    continue
                if i + 1 < n and text[i + 1] == "]":  # OSC ... BEL / ST
                    j = i + 2
                    while j < n:
                        if text[j] == "\x07":
                            j += 1
                            break
                        if text[j] == "\x1b" and j + 1 < n and text[j + 1] == "\\":
                            j += 2
                            break
                        j += 1
                    i = j
                    continue
                if i + 1 < n and text[i + 1] in "()":
                    i += 3  # ESC ( B : charset designation
                    continue
                i += 2  # ESC 7 / ESC 8 / ESC = : short escapes
                continue
            if ch == "\r":
                self.x = 0
            elif ch == "\n":
                self._newline()
            elif ch == "\b":
                self.x = max(0, self.x - 1)
            elif ch == "\x07" or ch < " ":
                pass
            else:
                self._put(ch)
            i += 1

    def lines(self) -> list[str]:
        return ["".join(cell for cell in row).rstrip() for row in self.grid]

    def text(self) -> str:
        return "\n".join(self.lines())


# ---------------------------------------------------------------------------
# PTY session
# ---------------------------------------------------------------------------
class PtySession:
    def __init__(self, argv: list[str], env: dict[str, str], cwd: str):
        pid, fd = pty.fork()
        if pid == 0:  # child
            try:
                os.chdir(cwd)
                os.execvpe(argv[0], argv, env)
            except BaseException:
                os._exit(127)
        self.pid, self.fd = pid, fd
        self.buf = bytearray()
        self.returncode: int | None = None
        self._set_winsize(ROWS, COLS)

    def _set_winsize(self, rows: int, cols: int) -> None:
        try:
            fcntl.ioctl(
                self.fd,
                termios.TIOCSWINSZ,
                struct.pack("HHHH", rows, cols, 0, 0),
            )
        except OSError:
            pass

    def drain(self, timeout: float) -> bytes:
        """Read whatever is available within `timeout` seconds; append to self.buf."""
        chunk = bytearray()
        end = time.time() + timeout
        while True:
            remaining = end - time.time()
            if remaining <= 0:
                break
            ready, _, _ = select.select([self.fd], [], [], remaining)
            if not ready:
                break
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                break
            if not data:
                break
            chunk.extend(data)
            self.buf.extend(data)
            end = min(end, time.time() + 0.02)  # coalesce bursts quickly
        return bytes(chunk)

    def write(self, data: bytes) -> None:
        os.write(self.fd, data)

    def wait_for(self, predicate, timeout: float, poll: float = 0.05) -> bool:
        """Poll the cumulative buffer until predicate() is true or timeout."""
        end = time.time() + timeout
        if predicate():
            return True
        while time.time() < end:
            self.drain(min(poll, max(0.0, end - time.time())))
            if predicate():
                return True
        return False

    def poll_exit(self) -> int | None:
        if self.returncode is not None:
            return self.returncode
        try:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError:
            self.returncode = 0
            return self.returncode
        if pid == 0:
            return None
        if os.WIFEXITED(status):
            self.returncode = os.WEXITSTATUS(status)
        elif os.WIFSIGNALED(status):
            self.returncode = -os.WTERMSIG(status)
        else:
            self.returncode = -1
        return self.returncode

    def screen(self) -> Screen:
        scr = Screen(COLS, ROWS)
        scr.feed(self.buf.decode("utf-8", "replace"))
        return scr

    def close(self) -> None:
        if self.returncode is None:
            try:
                os.kill(self.pid, signal.SIGKILL)
            except OSError:
                pass
            try:
                os.waitpid(self.pid, 0)
            except OSError:
                pass
        try:
            os.close(self.fd)
        except OSError:
            pass


# ---------------------------------------------------------------------------
# Assertion helpers
# ---------------------------------------------------------------------------
def composer_line(screen: Screen) -> str | None:
    """The prompt row of the composer (`›`), if visible."""
    for line in reversed(screen.lines()):
        if "›" in line:
            return line
    return None


def prompt_row(screen: Screen) -> int | None:
    for index in range(len(screen.lines()) - 1, -1, -1):
        if "›" in screen.lines()[index]:
            return index
    return None


def hexs(data: bytes) -> str:
    return data.hex()


def sha256(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


class Report:
    def __init__(self) -> None:
        self.items: list[dict] = []

    def add(self, cid: str, ok: bool, detail: str) -> None:
        self.items.append({"id": cid, "ok": bool(ok), "detail": detail})

    @property
    def failures(self) -> int:
        return sum(1 for item in self.items if not item["ok"])


# ---------------------------------------------------------------------------
# Main flow
# ---------------------------------------------------------------------------
def run(bin_path: str, evidence_dir: str) -> int:
    repo_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    if not os.path.isabs(bin_path):
        bin_path = os.path.join(repo_root, bin_path)
    if not (os.path.isfile(bin_path) and os.access(bin_path, os.X_OK)):
        print(f"FATAL: binary not executable: {bin_path}", file=sys.stderr)
        return 2

    os.makedirs(evidence_dir, exist_ok=True)
    temp_root = tempfile.mkdtemp(prefix="wave40-shift-enter-")
    report = Report()
    sequences: dict[str, str] = {}
    notes: list[str] = []
    session: PtySession | None = None

    iso = {
        "HOME": os.path.join(temp_root, "home"),
        "XDG_CONFIG_HOME": os.path.join(temp_root, "xdg-config"),
        "XDG_DATA_HOME": os.path.join(temp_root, "xdg-data"),
        "XDG_CACHE_HOME": os.path.join(temp_root, "xdg-cache"),
    }
    for directory in iso.values():
        os.makedirs(directory, exist_ok=True)
    os.makedirs(os.path.join(iso["XDG_CONFIG_HOME"], "pony-agent"), exist_ok=True)

    env = {
        key: value
        for key, value in os.environ.items()
        if key.upper()
        not in (
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NODE_USE_ENV_PROXY",
            "PONY_KEYBOARD_ENHANCEMENT",
            "NO_COLOR",
        )
    }
    env.update(iso)
    env["TERM"] = "xterm-256color"
    env.setdefault("LANG", "C.UTF-8")

    exit_code = 2
    try:
        session = PtySession([bin_path], env, repo_root)

        # ---- Assertion 1: probe is answered like kitty, then app pushes flags -----
        probe_ok = session.wait_for(lambda: PROBE in session.buf, timeout=8.0)
        sequences["probe"] = hexs(PROBE) if probe_ok else ""
        if probe_ok:
            session.write(PROBE_REPLY)
            sequences["reply"] = hexs(PROBE_REPLY)
        else:
            notes.append(
                "no CSI ? u probe observed within 8s: the binary does not call "
                "supports_keyboard_enhancement() (R3 not wired)"
            )
        push_ok = session.wait_for(lambda: PUSH_RE.search(bytes(session.buf)) is not None, timeout=4.0)
        push_flags = None
        match = PUSH_RE.search(bytes(session.buf))
        if match:
            push_flags = int(match.group(1))
            sequences["push"] = hexs(match.group(0))
        report.add(
            "a2.1_probe_answered_and_flags_pushed",
            probe_ok and push_ok and push_flags is not None and (push_flags & 0b1) == 1,
            (
                f"probe_seen={probe_ok} push_seen={push_ok} push_flags={push_flags} "
                f"raw_push={match.group(0).hex() if match else ''} "
                f"expected_push_flags_bit=DISAMBIGUATE(1) "
                f"[contract literal '>2u' == REPORT_EVENT_TYPES bit; crossterm writes '>1u' for R2 flags]"
            ),
        )
        if not probe_ok:
            notes.append("assertion 1 failed before the app can receive Shift+Enter (expected red phase)")

        # ---- Wait for the composer to be ready -----------------------------------
        ready = session.wait_for(lambda: "›" in bytes(session.buf).decode("utf-8", "replace"), 10.0)
        if not ready:
            early = session.poll_exit()
            if early is not None:
                raise RuntimeError(f"TUI exited before the composer appeared (exit={early})")
            raise RuntimeError("composer prompt not seen within 10s")
        session.drain(0.4)
        before = session.screen()
        with open(os.path.join(evidence_dir, "screen-before.txt"), "w", encoding="utf-8") as fh:
            fh.write(before.text() + "\n")

        # ---- Type `abc` ----------------------------------------------------------
        session.write(b"abc")
        typed = session.wait_for(
            lambda: (composer_line(session.screen()) or "").find("abc") >= 0, 2.0
        )
        if not typed:
            raise RuntimeError("typed text 'abc' never appeared in the composer prompt row")

        # ---- Assertion 2: Shift+Enter keeps `abc` and does not submit -----------
        session.write(SHIFT_ENTER)
        sequences["shift_enter"] = hexs(SHIFT_ENTER)
        time.sleep(0.3)
        session.drain(0.2)
        after_shift = session.screen()
        with open(
            os.path.join(evidence_dir, "screen-after-shift-enter.txt"),
            "w",
            encoding="utf-8",
        ) as fh:
            fh.write(after_shift.text() + "\n")
        shift_row = composer_line(after_shift)
        shift_text = after_shift.text()
        submit_markers = [marker for marker in SUBMIT_MARKERS if marker in shift_text]
        row_before = prompt_row(before)
        row_after = prompt_row(after_shift)
        composer_grew = (
            row_before is not None and row_after is not None and row_after < row_before
        )
        report.add(
            "a2.2_shift_enter_inserts_newline_without_submit",
            shift_row is not None
            and "abc" in shift_row
            and not submit_markers
            and composer_grew,
            (
                f"composer_row={shift_row!r} submit_markers={submit_markers} "
                f"prompt_row_before={row_before} prompt_row_after={row_after} "
                f"composer_grew(second visual line)={composer_grew}"
            ),
        )

        # ---- Assertion 3: plain Enter submits / clears the composer -------------
        session.write(PLAIN_ENTER)
        sequences["plain_enter"] = hexs(PLAIN_ENTER)
        submitted = session.wait_for(
            lambda: composer_line(session.screen()) is not None
            and "abc" not in (composer_line(session.screen()) or ""),
            3.0,
        )
        time.sleep(0.3)
        session.drain(0.2)
        after_enter = session.screen()
        with open(
            os.path.join(evidence_dir, "screen-after-enter.txt"),
            "w",
            encoding="utf-8",
        ) as fh:
            fh.write(after_enter.text() + "\n")
        enter_row = composer_line(after_enter)
        report.add(
            "a2.3_plain_enter_submits_and_clears_composer",
            submitted and enter_row is not None and "abc" not in enter_row,
            f"submitted={submitted} composer_row={enter_row!r}",
        )

        # ---- Assertion 4: Ctrl-C twice exits cleanly, no panic, terminal restored
        session.write(CTRL_C)
        sequences["ctrl_c"] = hexs(CTRL_C)
        time.sleep(0.2)
        session.write(CTRL_C)
        exited = False
        end = time.time() + 6.0
        while time.time() < end:
            session.drain(0.1)
            if session.poll_exit() is not None:
                exited = True
                break
        raw = bytes(session.buf)
        text = raw.decode("utf-8", "replace")
        panic = "panicked at" in text or "RUST_BACKTRACE" in text
        restored = LEAVE_ALT_SCREEN in raw
        pop_ok = True
        pop_match = POP_RE.search(raw)
        if push_ok and push_flags is not None:
            pop_ok = pop_match is not None
            if pop_match:
                sequences["pop"] = hexs(pop_match.group(0))
        report.add(
            "a2.4_ctrl_c_exits_without_panic_and_restores_terminal",
            exited and not panic and restored and pop_ok,
            (
                f"exited={exited} child_exit={session.returncode} panic={panic} "
                f"leave_alt_screen={restored} pop_seen={pop_match is not None} "
                f"pop_raw={pop_match.group(0).hex() if pop_match else ''} pop_required={push_ok}"
            ),
        )
        if not push_ok:
            notes.append(
                "pop flags not required in this run: no push was observed "
                "(red phase / unsupported terminal path)"
            )

        exit_code = 0 if report.failures == 0 else 1
    except RuntimeError as error:
        print(f"HARNESS ERROR: {error}", file=sys.stderr)
        report.add("harness.startup", False, str(error))
        exit_code = 2
    except Exception as error:  # noqa: BLE001 - harness must report, not crash
        print(f"HARNESS ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        report.add("harness.unexpected", False, f"{type(error).__name__}: {error}")
        exit_code = 2
    finally:
        if session is not None:
            session.close()
        try:
            import shutil

            shutil.rmtree(temp_root, ignore_errors=True)
        except OSError:
            pass

    payload = {
        "scenario": "shift-enter",
        "wave": 40,
        "bin": bin_path,
        "bin_sha256": sha256(bin_path),
        "pty": {"cols": COLS, "rows": ROWS, "term": "xterm-256color", "isolated_home": True},
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "exit_code": exit_code,
        "pass": len(report.items) - report.failures,
        "fail": report.failures,
        "assertions": report.items,
        "sequences": sequences,
        "screens": {
            "before": "screen-before.txt",
            "after_shift_enter": "screen-after-shift-enter.txt",
            "after_enter": "screen-after-enter.txt",
        },
        "notes": notes,
    }
    report_path = os.path.join(evidence_dir, "report.json")
    with open(report_path, "w", encoding="utf-8") as fh:
        json.dump(payload, fh, ensure_ascii=False, indent=2)

    print(
        f"RESULT scenario=shift-enter pass={payload['pass']} fail={payload['fail']} "
        f"exit={exit_code}"
    )
    for item in report.items:
        print(("PASS " if item["ok"] else "FAIL ") + item["id"] + " :: " + item["detail"])
    print(f"report: {report_path}")
    return exit_code


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", default="target/debug/pony", help="TUI binary under test")
    parser.add_argument(
        "--evidence-dir",
        default=".dev-team/evidence/wave40-shift-enter",
        help="directory for screen dumps + report.json",
    )
    args = parser.parse_args(argv)
    evidence_dir = args.evidence_dir
    if not os.path.isabs(evidence_dir):
        repo_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        evidence_dir = os.path.join(repo_root, evidence_dir)
    return run(args.bin, evidence_dir)


if __name__ == "__main__":
    sys.exit(main())
