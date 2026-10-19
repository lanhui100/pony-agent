#!/usr/bin/env python3
"""Text assertions and report generation for the pony TUI audit harness.

The shell driver (`run_audit.sh`) owns tmux, the mock LLM and screenshots.  This
module only looks at the physical evidence the driver captured:

  <scenario>/meta.json        phase / scenario / actual pane size / binary id
  <scenario>/extra.json       optional: mock_log, exit_code, cmd_runs, ...
  <scenario>/<name>.txt       `tmux capture-pane -p`   (plain text)
  <scenario>/<name>.ansi      `tmux capture-pane -p -e` (SGR, rendered to PNG)
  <scenario>/<name>.png       rendered screenshot

Assertions are deliberately strict and content-based.  They never grade
intent: every verdict quotes the captured line(s) that produced it.

Usage (called by run_audit.sh):
    checks.py check     --dir <scenario dir>
    checks.py aggregate --phase-dir <phase dir>
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import unicodedata
from datetime import datetime, timezone

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import render_ansi as ansi_render  # noqa: E402

# ---------------------------------------------------------------------------
# terminal text helpers (display width, matching the CJK-aware renderer)
# ---------------------------------------------------------------------------


def cell_width(ch: str) -> int:
    if unicodedata.combining(ch):
        return 0
    if unicodedata.east_asian_width(ch) in ("W", "F"):
        return 2
    return 1


def dw(text: str) -> int:
    return sum(cell_width(ch) for ch in text)


def read_lines(path: str) -> list[str]:
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        raw = fh.read()
    rows = raw.split("\n")
    if rows and rows[-1] == "":
        rows.pop()
    return rows


def stripped(lines: list[str]) -> list[str]:
    return [ln.rstrip() for ln in lines]


def col_of(line: str, sub: str) -> int | None:
    idx = line.find(sub)
    return None if idx < 0 else dw(line[:idx])


def first_line(lines: list[str], sub: str) -> tuple[int, str] | tuple[None, None]:
    for i, line in enumerate(lines):
        if sub in line:
            return i, line
    return None, None


def line_indices(lines: list[str], sub: str) -> list[int]:
    return [i for i, line in enumerate(lines) if sub in line]


def clip(text: str, limit: int = 240) -> str:
    text = " ".join(text.split())
    return text if len(text) <= limit else text[: limit - 1] + "…"


# ---------------------------------------------------------------------------
# wave-39 R7/R11/R13: the row directly below the composer band is the metrics
# line (`↑in ↓out · 缓存 NN% · 上下文 NN%`), not a status/hint phrase.  v4 (R13)
# removed the standalone spacing row: the band's own last (breathing) row is the
# only gap.  Metrics row is CANVAS/GHOST and content-column wide (gutters stay
# CANVAS - ADR 0038), and only segments with a value are rendered
# (`in>0` / `window>0`, no `—` placeholder).
# ---------------------------------------------------------------------------
METRICS_TOKENS = ("↑", "↓", "缓存", "上下文")


def is_metrics_line(line: str) -> bool:
    """Full cluster (post-turn): ↑ ↓ 缓存 上下文 all present."""
    return all(token in line for token in METRICS_TOKENS)


def metrics_segments(line: str) -> list[str]:
    """Segments actually rendered (v3 R11: no value -> no segment, no '—')."""
    return [token for token in METRICS_TOKENS if token in line]


def metrics_row_index(cap: "Capture") -> int | None:
    for i in range(len(cap.lines) - 1, -1, -1):
        if metrics_segments(cap.lines[i]):
            return i
    return None


class Checks:
    def __init__(self, scenario: str):
        self.scenario = scenario
        self.items: list[dict] = []

    def add(self, cid: str, desc: str, ok: bool, evidence: str) -> None:
        self.items.append(
            {"id": cid, "desc": desc, "ok": bool(ok), "evidence": clip(evidence)}
        )

    def failures(self) -> int:
        return sum(1 for item in self.items if not item["ok"])

    def as_dict(self) -> dict:
        return {
            "scenario": self.scenario,
            "pass": len(self.items) - self.failures(),
            "fail": self.failures(),
            "checks": self.items,
        }


# ---------------------------------------------------------------------------
# scenario context
# ---------------------------------------------------------------------------
class Capture:
    def __init__(self, name: str, directory: str):
        self.name = name
        self.text_path = os.path.join(directory, name + ".txt")
        self.ansi_path = os.path.join(directory, name + ".ansi")
        self.png_path = os.path.join(directory, name + ".png")
        self.lines = stripped(read_lines(self.text_path))
        self.raw = self.lines
        self.cols = None  # filled from meta
        self.rows = None

    def text(self) -> str:
        return "\n".join(self.lines)


class Ctx:
    def __init__(self, directory: str):
        self.dir = os.path.abspath(directory)
        with open(os.path.join(self.dir, "meta.json"), encoding="utf-8") as fh:
            self.meta = json.load(fh)
        extra_path = os.path.join(self.dir, "extra.json")
        self.extra = {}
        if os.path.exists(extra_path):
            with open(extra_path, encoding="utf-8") as fh:
                self.extra = json.load(fh)
        self.cols = int(self.meta["cols"])
        self.rows = int(self.meta["rows"])
        self.captures: dict[str, Capture] = {}
        for name in sorted(os.listdir(self.dir)):
            if name.endswith(".txt"):
                base = name[:-4]
                if base == "assertions":
                    continue
                cap = Capture(base, self.dir)
                cap.cols, cap.rows = self.cols, self.rows
                self.captures[base] = cap


# ---------------------------------------------------------------------------
# generic assertions
# ---------------------------------------------------------------------------
PANIC_RE = re.compile(r"panicked at|thread '.*' panicked|RUST_BACKTRACE|stack backtrace")


def common_checks(c: Checks, cap: Capture, *, allow_ni: bool = False) -> None:
    text = cap.text()

    m = PANIC_RE.search(text)
    c.add(
        f"{cap.name}.no_panic",
        "capture contains no Rust panic/backtrace",
        m is None,
        f"matched {m.group(0)!r}" if m else "no panic tokens in capture",
    )

    if not allow_ni:
        ni_lines = [(i, ln) for i, ln in enumerate(cap.lines) if ln.strip() == "你"]
        c.add(
            f"{cap.name}.no_standalone_role_label",
            "no standalone role label line \"你\" (role shown by material, not label)",
            not ni_lines,
            f"row {ni_lines[0][0]} = {ni_lines[0][1]!r}" if ni_lines else "no line equals \"你\"",
        )

    widths = [(i, dw(ln), ln) for i, ln in enumerate(cap.lines)]
    widest = max(widths, key=lambda item: item[1]) if widths else (0, 0, "")
    c.add(
        f"{cap.name}.within_terminal_width",
        f"every captured row display width <= {cap.cols} cols",
        widest[1] <= cap.cols,
        f"widest row {widest[0]} width={widest[1]} cols={cap.cols} :: {clip(widest[2])}",
    )

    c.add(
        f"{cap.name}.capture_rows",
        f"capture has exactly pane height rows ({cap.rows})",
        len(cap.lines) == cap.rows,
        f"rows={len(cap.lines)} pane_rows={cap.rows}",
    )

    for suffix, path in (("ansi", cap.ansi_path), ("png", cap.png_path)):
        exists = os.path.exists(path) and os.path.getsize(path) > 0
        if os.path.exists(cap.text_path):
            c.add(
                f"{cap.name}.artifact_{suffix}",
                f"{suffix} artifact exists for screenshot review",
                exists,
                f"{path} size={os.path.getsize(path) if os.path.exists(path) else 0}",
            )


def input_line(cap: Capture) -> tuple[int, str] | tuple[None, None]:
    for i in range(len(cap.lines) - 1, -1, -1):
        if "›" in cap.lines[i]:
            return i, cap.lines[i]
    return None, None


def input_checks(c: Checks, cap: Capture, *, require_placeholder: bool = True) -> None:
    idx, line = input_line(cap)
    if idx is None:
        c.add(
            f"{cap.name}.input_prompt_present",
            "input prompt line is visible",
            False,
            "no line containing '›'",
        )
        return
    inset = col_of(line, "›")
    c.add(
        f"{cap.name}.input_padded_from_left_edge",
        "input prompt has left padding (>0 cols from terminal edge)",
        inset is not None and inset >= 2,
        f"row {idx} prompt_col={inset} :: {clip(line)}",
    )
    if require_placeholder:
        placeholder = "写下任务" in line or "输入任务" in line
        c.add(
            f"{cap.name}.input_placeholder",
            "composer shows placeholder guidance when empty",
            placeholder,
            f"row {idx} :: {clip(line)}",
        )


def drop_display_prefix(text: str, cols: int) -> str:
    """Drop characters until `cols` terminal columns have been consumed."""
    used = 0
    idx = 0
    for ch in text:
        if used >= cols:
            break
        used += cell_width(ch)
        idx += 1
    return text[idx:]


def bottom_chrome_start(cap: Capture) -> int:
    """First row below the composer band: the metrics row (wave-39 v4 R13).

    v4 removed the standalone CANVAS spacing row: the blank breathing row above
    the metrics row is the band's last row (SURFACE), i.e. it belongs to the
    composer band, so only the metrics row itself is excluded from the
    reconstructed composer buffer (blank rows are filtered anyway).  Falls back
    to the last row when the capture has no metrics segment at all (empty state
    before any value, or legacy captures).
    """
    metrics_idx = metrics_row_index(cap)
    return metrics_idx if metrics_idx is not None else len(cap.lines) - 1


def composer_rows(cap: Capture) -> list[str]:
    """Non-empty composer rows (inside the band, above the metrics row) in screen order."""
    prompt_idx = composer_prompt_index(cap)
    if prompt_idx is None:
        return []
    end = bottom_chrome_start(cap)
    if end <= prompt_idx:
        end = len(cap.lines) - 1
    return [ln for ln in cap.lines[prompt_idx:end] if ln.strip()]


def input_text(cap: Capture) -> str | None:
    """Reconstruct the composer buffer from the captured composer rows.

    The first row carries `› ` and continuation rows are aligned to the same
    text column.  The caret glyph is removed, so the result is exactly what the
    user typed.  wave-39 v4 R13: only the metrics row below the band is excluded;
    the band's own breathing row is blank and filtered out (see `bottom_chrome_start`).
    """
    rows = composer_rows(cap)
    prompt_row = next((ln for ln in rows if "›" in ln), None)
    if prompt_row is None:
        return None
    pcol = col_of(prompt_row, "›")
    content_col = (pcol or 0) + 2  # '› ' is two columns
    parts: list[str] = []
    for row in rows:
        row = row.replace("▏", "")
        text = drop_display_prefix(row, content_col).rstrip()
        if not parts and ("写下任务" in text or "输入任务" in text):
            return ""
        parts.append(text)
    return "\n".join(parts)


def input_region_line_count(cap: Capture) -> int:
    return len(composer_rows(cap))


def header_checks(c: Checks, cap: Capture) -> None:
    line = cap.lines[0] if cap.lines else ""
    ok = "pony" in line and "pony-agent" in line
    c.add(
        f"{cap.name}.header",
        "header shows brand + workspace",
        ok,
        f"row 0 :: {clip(line)}",
    )


def composer_prompt_index(cap: Capture) -> int | None:
    """Row of the bottom composer prompt (`›`), if the composer is on screen."""
    for i in range(len(cap.lines) - 1, -1, -1):
        if "›" in cap.lines[i]:
            return i
    return None


def composer_boundary(cap: Capture) -> int | None:
    """Top row of the composer block, or None when a modal hides the composer.

    wave-39 R4 removed the `▁` separator above the composer, so the prompt row is
    the only reliable anchor; the legacy `▁` fallback is kept for old captures.
    """
    prompt = composer_prompt_index(cap)
    if prompt is not None:
        return prompt
    bands = [
        i
        for i, line in enumerate(cap.lines)
        if i >= 3 and len(line.strip()) >= 20 and set(line.strip()) <= {"▁"}
    ]
    return bands[-1] if bands else None


def band_index(cap: Capture) -> int | None:
    for i, line in enumerate(cap.lines):
        if re.fullmatch(r"[▁\s]{20,}", line.strip()) and "▁" in line:
            return i
    return None


def panel_geometry(c: Checks, cap: Capture, tag: str, expected_width: int | None = None) -> None:
    tops = line_indices(cap.lines, "┌")
    bots = line_indices(cap.lines, "└")
    ok = len(tops) == 1 and len(bots) == 1 and tops[0] < bots[0]
    c.add(
        f"{tag}.border_box",
        "exactly one top and one bottom panel border, top above bottom",
        ok,
        f"top_rows={tops} bottom_rows={bots}",
    )
    if not ok:
        return
    top, bot = tops[0], bots[0]
    band = composer_boundary(cap)
    c.add(
        f"{tag}.panel_height_fits_screen",
        "panel top/bottom borders are both on screen and the panel height fits",
        top >= 2 and bot <= cap.rows - 2 and (bot - top + 1) <= cap.rows - 4,
        f"top={top} bottom={bot} height={bot - top + 1} pane_rows={cap.rows}",
    )
    # A modal that owns the prompt row (add-provider form) puts its own input
    # inside the panel; only a composer *below* the panel can be occluded.
    if band is not None and band > bot:
        c.add(
            f"{tag}.panel_above_composer",
            "panel bottom border sits above the composer prompt",
            bot < band,
            f"bottom={bot} composer_top_row={band}",
        )

    interior = [
        (i, cap.lines[i]) for i in range(top, bot + 1) if "│" in cap.lines[i]
    ]
    lefts = [col_of(ln, "│") for _, ln in interior]
    rights = [dw(ln[: ln.rfind("│")]) for _, ln in interior]
    aligned = (
        len(interior) >= 2
        and len(set(lefts)) == 1
        and len(set(rights)) == 1
        and lefts[0] < rights[0]
    )
    c.add(
        f"{tag}.panel_vertical_borders_aligned",
        "every panel interior row shares the same left/right border column",
        aligned,
        f"left_cols={sorted(set(lefts))} right_cols={sorted(set(rights))} "
        f"rows={len(interior)}",
    )
    if aligned:
        width = rights[0] - lefts[0] + 1
        c.add(
            f"{tag}.panel_width_fits",
            "panel width fits inside the terminal",
            width <= cap.cols,
            f"panel_cols={lefts[0]}..{rights[0]} width={width} terminal_cols={cap.cols}",
        )
        if expected_width is not None:
            centered = abs(lefts[0] - (cap.cols - rights[0] - 1)) <= 1
            c.add(
                f"{tag}.panel_width_expected",
                f"panel width matches the design token ({expected_width}) and is centered",
                width == expected_width and centered,
                f"width={width} expected={expected_width} left={lefts[0]} "
                f"right_margin={cap.cols - rights[0] - 1}",
            )
        row_ids = [i for i, _ in interior]
        consecutive = row_ids == list(range(row_ids[0], row_ids[0] + len(row_ids)))
        c.add(
            f"{tag}.panel_rows_not_overlapping",
            "panel rows are consecutive (no overlapping/duplicated rows)",
            consecutive,
            f"interior_rows={row_ids}",
        )
    max_borders = max((ln.count("│") for ln in cap.lines), default=0)
    c.add(
        f"{tag}.no_double_border_overlap",
        "no row paints more than one panel's worth of vertical borders",
        max_borders <= 2,
        f"max '│' per row={max_borders}",
    )


def char_at_display_col(line: str, col: int) -> str:
    used = 0
    for ch in line:
        if used >= col:
            return ch
        used += cell_width(ch)
    return ""


def rule_rows(cap: Capture) -> list[int]:
    return [
        i
        for i, line in enumerate(cap.lines)
        if len(line.strip()) >= 20 and set(line.strip()) == {"▁"}
    ]


def separator_checks(c: Checks, cap: Capture) -> None:
    """wave-39 R4: only the header hairline remains (composer top rule removed)."""
    rows = rule_rows(cap)
    ok = len(rows) == 1
    c.add(
        f"{cap.name}.separator_single_header_rule",
        "exactly one full ▁ rule remains: the header hairline (composer top rule removed)",
        ok,
        f"rule_rows={rows}",
    )
    if not ok:
        return
    header = cap.lines[rows[0]].strip()
    content_w = min(cap.cols - 6, 120)
    expected_start = (cap.cols - content_w) // 2
    start = col_of(cap.lines[rows[0]], header[0])
    c.add(
        f"{cap.name}.separator_header_rule_geometry",
        f"header rule lives on row 1, starts at content column {expected_start} "
        f"and spans {content_w} cols",
        rows[0] == 1 and start == expected_start and dw(header) == content_w,
        f"row={rows[0]} start={start} expected_start={expected_start} "
        f"width={dw(header)} expected_width={content_w}",
    )


DESC_PAIRS = [
    ("/resume", "选择并恢复历史会话"),
    ("/provider", "管理与切换模型提供商"),
    ("/models", "切换当前活跃模型"),
    ("/thinking", "设置思考强度 (Reasoning Effort)"),
    ("/clear", "清空当前消息上下文"),
    ("/cost", "查看当前模型与网关配额信息"),
    ("/help", "查看所有支持的快捷指令"),
    ("/quit", "安全退出 TUI 界面"),
]


def desc_columns(cap: Capture) -> dict[str, int | None]:
    out: dict[str, int | None] = {}
    for cmd, desc in DESC_PAIRS:
        col = None
        for line in cap.lines:
            if cmd in line and desc in line:
                col = col_of(line, desc)
                break
        out[cmd] = col
    return out


def command_table_checks(
    c: Checks,
    cap: Capture,
    prefix: str,
    expected_col: int = 16,
    required: list[str] | None = None,
) -> None:
    cols = desc_columns(cap)
    required = required if required is not None else list(cols)
    missing = [cmd for cmd in required if cols.get(cmd) is None]
    wrong = {cmd: col for cmd, col in cols.items() if col is not None and col != expected_col}
    c.add(
        f"{prefix}.command_desc_column",
        f"command descriptions start at column {expected_col} (same table geometry everywhere)",
        not missing and not wrong,
        f"cols={cols} expected={expected_col} missing={missing}",
    )


def composer_cell(cap: Capture, needle: str):
    """Return (row, col, cell) for the first cell of `needle` in composer row."""
    with open(cap.ansi_path, encoding="utf-8", errors="replace") as fh:
        ansi = fh.read()
    grid = ansi_render.parse_ansi(ansi, cap.cols, cap.rows)
    for r, row in enumerate(grid):
        text = "".join(cell.ch for cell in row)
        pos = text.find(needle)
        if pos >= 0:
            return r, pos, row[pos]
    return None, None, None


def composer_cursor_checks(c: Checks, cap: Capture) -> None:
    row, col, cell = composer_cell(cap, "写下任务")
    if cell is None:
        c.add(f"{cap.name}.composer_cursor_block", "placeholder cursor cell found", False, "placeholder not found")
        return
    canvas = ansi_render.DEFAULT_BG
    ink = ansi_render.DEFAULT_FG
    block = (
        cell.style.bg is not None
        and tuple(cell.style.bg) != tuple(canvas)
        and tuple(cell.style.fg or ()) == tuple(canvas)
    )
    c.add(
        f"{cap.name}.composer_cursor_block",
        "first placeholder glyph is a reverse-video block (accent bg / canvas-colored glyph)",
        block,
        f"row={row} col={col} fg={cell.style.fg} bg={cell.style.bg} canvas={canvas} ink={ink}",
    )


def composer_text_column(cap: Capture, needle: str) -> int | None:
    row = None
    for i in range(len(cap.lines) - 1, -1, -1):
        if "›" in cap.lines[i]:
            row = i
            break
    if row is None:
        return None
    return col_of(cap.lines[row], needle)


def focus_mode_checks(c: Checks, cap: Capture, tag: str) -> None:
    text = cap.text()
    tops = line_indices(cap.lines, "┌")
    bots = line_indices(cap.lines, "└")
    top = tops[0] if tops else None
    bot = bots[0] if bots else None
    outside_prompts = []
    if top is None or bot is None:
        outside_prompts = [i for i, ln in enumerate(cap.lines) if "›" in ln]
    else:
        outside_prompts = [
            i
            for i, ln in enumerate(cap.lines)
            if "›" in ln and not (top <= i <= bot)
        ]
    c.add(
        f"{tag}.focus_composer_hidden",
        "focus mode hides the global composer (prompt may only live inside the modal)",
        not outside_prompts,
        f"prompt_rows_outside_panel={outside_prompts}",
    )
    c.add(
        f"{tag}.focus_chat_hidden",
        "focus mode hides chat and splash (no transcript/logo residue)",
        "描述一个任务开始" not in text
        and "终端里的自主智能体" not in text
        and "▀" not in text
        and "▄" not in text,
        f"guidance={'描述一个任务开始' in text} logo_tag={'终端里的自主智能体' in text} "
        f"block_glyphs={('▀' in text) or ('▄' in text)}",
    )
    if top is None or bot is None:
        c.add(f"{tag}.focus_background_clean", "rows outside the panel are blank", False, "panel bounds missing")
        return
    outside = [
        i
        for i in range(len(cap.lines))
        if i not in (0, 1) and not (top <= i <= bot)
    ]
    dirty = [(i, cap.lines[i]) for i in outside if cap.lines[i].strip()]
    c.add(
        f"{tag}.focus_background_clean",
        "rows outside header + panel are completely blank (no half-cell residue)",
        not dirty,
        f"dirty_rows={[(i, clip(ln)) for i, ln in dirty[:4]]}",
    )


def focus_border_checks(c: Checks, cap: Capture, tag: str) -> None:
    tops = line_indices(cap.lines, "┌")
    bots = line_indices(cap.lines, "└")
    if not tops or not bots:
        c.add(f"{tag}.border_rows_complete", "every panel row carries both border columns", False, "missing top/bottom border")
        return
    top, bot = tops[0], bots[0]
    interior = [i for i in range(top, bot + 1) if "│" in cap.lines[i]]
    if not interior:
        c.add(f"{tag}.border_rows_complete", "every panel row carries both border columns", False, "no interior rows")
        return
    left = col_of(cap.lines[interior[0]], "│")
    right = dw(cap.lines[interior[0]][: cap.lines[interior[0]].rfind("│")])
    expected_left = "┌│└"
    expected_right = "┐│┘"
    broken = []
    for r in range(top, bot + 1):
        line = cap.lines[r]
        lc = char_at_display_col(line, left)
        rc = char_at_display_col(line, right)
        if lc not in expected_left or rc not in expected_right:
            broken.append((r, lc, rc))
    c.add(
        f"{tag}.border_rows_complete",
        "every panel row carries both border columns at the same display columns",
        not broken,
        f"left={left} right={right} broken_rows={broken[:5]}",
    )


def footer_text(cap: Capture) -> str:
    return cap.lines[-1] if cap.lines else ""


# ---------------------------------------------------------------------------
# per-scenario assertions
# ---------------------------------------------------------------------------
def check_empty(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    header_checks(c, cap)
    input_checks(c, cap)
    text = cap.text()
    c.add(
        "empty.guidance",
        "empty state shows onboarding guidance",
        "描述一个任务开始" in text,
        next((clip(ln) for ln in cap.lines if "描述一个任务开始" in ln), "missing"),
    )
    c.add(
        "empty.footer_workspace_only",
        "empty footer shows the workspace only: no metrics segments before the first turn "
        "(v3 R11: no value -> no segment, and no '—' placeholder)",
        bool(footer_text(cap).strip())
        and "pony-agent" in footer_text(cap)
        and not metrics_segments(footer_text(cap))
        and "—" not in footer_text(cap),
        f"footer :: {clip(footer_text(cap))}",
    )
    separator_checks(c, cap)
    composer_cursor_checks(c, cap)
    placeholder_col = composer_text_column(cap, "写")
    c.add(
        "empty.placeholder_column",
        "placeholder starts at the shared text column 5",
        placeholder_col == 5,
        f"placeholder_col={placeholder_col}",
    )
    command_table_checks(
        c,
        cap,
        "empty",
        required=["/resume", "/provider", "/models", "/thinking"],
    )


def check_conversation(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    header_checks(c, cap)
    input_checks(c, cap)
    text = cap.text()

    user = "读取 package.json 并总结"
    user_row, user_line = first_line(cap.lines, user)
    c.add(
        "conversation.user_message_visible",
        "user prompt is rendered in the transcript",
        user_row is not None,
        f"row {user_row} :: {clip(user_line)}" if user_line else "missing user prompt",
    )
    c.add(
        "conversation.user_role_rail",
        "user message is distinguished by a rail/block, not a text label",
        any("▌" in ln for ln in cap.lines),
        next((f"row {i} :: {clip(ln)}" for i, ln in enumerate(cap.lines) if "▌" in ln), "no '▌' rail found"),
    )
    tool_row, tool_line = first_line(cap.lines, "读取 (Read)")
    c.add(
        "conversation.tool_line_visible",
        "tool activity line (Read) is visible with a status glyph",
        tool_row is not None and re.search(r"[✓◦✕]\s*读取 \(Read\)", tool_line or ""),
        f"row {tool_row} :: {clip(tool_line)}" if tool_line else "missing tool line",
    )
    c.add(
        "conversation.final_answer_visible",
        "final assistant answer (mock round 2) is visible",
        "结论" in text and "0.1.125" in text,
        next((f"row {i} :: {clip(ln)}" for i, ln in enumerate(cap.lines) if "结论" in ln), "missing 结论"),
    )

    if user_line and tool_line:
        assistant_row, assistant_line = first_line(cap.lines, "结论")
        user_col = col_of(user_line, user)
        tool_col = col_of(tool_line, "读取 (Read)")
        assistant_col = col_of(assistant_line, "结论") if assistant_line else None
        cols = [user_col, assistant_col, tool_col]
        equal = None not in cols and len(set(cols)) == 1
        c.add(
            "conversation.text_columns_aligned",
            "user body / assistant body / tool text start on the same column",
            equal,
            f"user_col={user_col} assistant_col={assistant_col} tool_col={tool_col}",
        )
        c.add(
            "conversation.aligned_columns_padded",
            "shared text column is inset from the terminal edge",
            equal and user_col is not None and user_col >= 2,
            f"shared_col={user_col}",
        )

    no_error = not re.search(r"(?m)^\s*[✕·]?\s*(Error:|错误|失败)", text) and "Error:" not in text
    c.add(
        "conversation.no_error_rendered",
        "successful turn shows no error message",
        no_error,
        "no Error:/失败 rows in transcript" if no_error else "error row present",
    )

    # Physical receipt: the TUI runtime plans workspace_gather_context itself and
    # then makes one follow-up model round that carries the tool result.  Either
    # shape (two rounds, or one follow-up round with role=tool) proves that a tool
    # round happened before the final answer.
    chat = mock_chat_requests(ctx.extra.get("mock_log"))
    matched = [r for r in chat if user in (r.get("last_user") or "")]
    with_tool = [r for r in matched if int(r.get("n_tool_messages") or 0) >= 1]
    c.add(
        "conversation.mock_followup_has_tool_round",
        "mock receipt proves the final model round carried a tool result",
        len(matched) >= 1 and len(with_tool) >= 1,
        f"chat_requests={len(matched)} tool_round_requests={len(with_tool)} "
        f"roles={[r.get('msg_roles') for r in matched][-2:]} "
        f"scenarios={[r.get('scenario') for r in matched]}",
    )
    separator_checks(c, cap)
    c.add(
        "conversation.footer_metrics",
        "footer is the composer metrics line after the turn (status phrase removed by wave-39)",
        is_metrics_line(footer_text(cap)) and "完成" not in footer_text(cap),
        f"footer :: {clip(footer_text(cap))}",
    )


def mock_chat_requests(log_path: str | None) -> list[dict]:
    if not log_path or not os.path.exists(log_path):
        return []
    out = []
    with open(log_path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            path = (rec.get("path") or "").rstrip("/")
            if path.endswith("/chat/completions"):
                out.append(rec)
    return out


def check_slash(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    input_checks(c, cap, require_placeholder=False)
    text = cap.text()
    commands = ["/resume", "/provider", "/models", "/thinking", "/clear", "/cost", "/help", "/quit"]
    missing = [cmd for cmd in commands if cmd not in text]
    c.add(
        "slash.all_commands_listed",
        "slash menu lists the full command table",
        not missing,
        f"menu_commands={commands} missing={missing}",
    )

    idx, line = input_line(cap)
    rules = [
        i
        for i, ln in enumerate(cap.lines)
        if i > 2 and len(ln.strip()) >= 20 and set(ln.strip()) <= {"─"}
    ]
    c.add(
        "slash.menu_has_top_and_bottom_rules",
        "menu is framed by two hairline rules",
        len(rules) >= 2,
        f"rule_rows={rules}",
    )
    if len(rules) < 2:
        c.add("slash.menu_above_composer", "menu rows sit above the composer input row", False, "menu rules missing")
        c.add("slash.each_command_once", "each command appears exactly once inside the menu region", False, "menu rules missing")
        c.add("slash.selected_row_marker", "selected command row carries the selection marker", False, "menu rules missing")
        return
    top_rule, bottom_rule = rules[-2], rules[-1]
    menu_rows = list(range(top_rule + 1, bottom_rule))
    menu_text = "\n".join(cap.lines[i] for i in menu_rows)
    above = idx is not None and menu_rows and bottom_rule < idx
    c.add(
        "slash.menu_above_composer",
        "menu rows sit above the composer input row",
        above,
        f"menu_rows={menu_rows[:2]}..{menu_rows[-1] if menu_rows else None} "
        f"bottom_rule={bottom_rule} input_row={idx}",
    )
    duplicates = [cmd for cmd in commands if menu_text.count(cmd) != 1]
    c.add(
        "slash.each_command_once",
        "each command appears exactly once inside the menu region",
        not duplicates,
        f"duplicate/missing commands={duplicates} menu_rows={len(menu_rows)}",
    )
    command_table_checks(c, cap, "slash")
    resume_count = cap.text().count("/resume")
    c.add(
        "slash.resume_once_whole_screen",
        "'/resume' appears exactly once on the whole screen while the menu is open",
        resume_count == 1,
        f"/resume occurrences={resume_count}",
    )
    c.add(
        "slash.selected_row_marker",
        "selected command row carries the selection marker",
        any("▏" in cap.lines[i] for i in menu_rows),
        next((f"row {i} :: {clip(cap.lines[i])}" for i in menu_rows if "▏" in cap.lines[i]), "no selection marker"),
    )


def check_models_panel(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    text = cap.text()
    title = next((ln for ln in cap.lines if "模型" in ln and "↑↓" in ln), "")
    counter = re.search(r"模型 · (\d+)/(\d+)", title)
    c.add(
        "models.title_counter",
        "model panel title carries the '模型 · 1/N' position counter",
        bool(counter) and counter.group(1) == "1" and int(counter.group(2)) >= 13,
        f"title_row :: {clip(title)}",
    )
    panel_geometry(c, cap, "models", expected_width=84)
    focus_mode_checks(c, cap, "models")
    focus_border_checks(c, cap, "models")

    tops = line_indices(cap.lines, "┌")
    bots = line_indices(cap.lines, "└")
    if tops and bots:
        top, bot = tops[0], bots[0]
        rows = []
        ids = []
        for i in range(top + 1, bot):
            ln = cap.lines[i]
            if "│" not in ln:
                continue
            segment = ln[ln.find("│") + 1 : ln.rfind("│")]
            m = re.search(r"(mock-[a-z0-9-]+)", segment)
            if m:
                rows.append((i, ln))
                ids.append(m.group(1))
        c.add(
            "models.rows_rendered",
            "panel renders at least the capped 12 model rows",
            len(rows) >= 12,
            f"model_rows={len(rows)} first={ids[:2]} last={ids[-1:]}",
        )
        c.add(
            "models.rows_unique_and_consecutive",
            "model rows are unique and occupy consecutive screen rows",
            len(set(ids)) == len(ids) and [i for i, _ in rows] == list(range(rows[0][0], rows[0][0] + len(rows))) if rows else False,
            f"ids={len(ids)} unique={len(set(ids))} rows={[i for i,_ in rows]}",
        )
        c.add(
            "models.no_fetch_failure",
            "remote /models fetch succeeded (no failure row)",
            "拉取失败" not in text,
            next((clip(ln) for ln in cap.lines if "拉取失败" in ln), "no fetch failure row"),
        )
    c.add(
        "models.selected_marker",
        "selected model row carries the selection marker",
        any("▏" in ln for ln in cap.lines),
        "selection marker '▏' present" if any("▏" in ln for ln in cap.lines) else "no selection marker",
    )


def check_provider_panel(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    text = cap.text()
    c.add(
        "provider.title",
        "provider panel title is visible",
        any("提供商" in ln and "↑↓" in ln for ln in cap.lines),
        next((clip(ln) for ln in cap.lines if "提供商" in ln and "↑↓" in ln), "missing provider title"),
    )
    c.add(
        "provider.fixture_listed",
        "fixture provider is listed",
        "FakeProvider" in text,
        next((clip(ln) for ln in cap.lines if "FakeProvider" in ln), "FakeProvider missing"),
    )
    c.add(
        "provider.add_hint",
        "panel advertises the 'a 新增' shortcut",
        "a 新增" in text,
        next((clip(ln) for ln in cap.lines if "a 新增" in ln), "missing add hint"),
    )
    panel_geometry(c, cap, "provider", expected_width=84)
    focus_mode_checks(c, cap, "provider")
    focus_border_checks(c, cap, "provider")
    c.add(
        "provider.current_marker",
        "active provider row is marked as current",
        any("当前" in ln and "FakeProvider" in ln for ln in cap.lines),
        next((clip(ln) for ln in cap.lines if "当前" in ln), "no 当前 marker"),
    )


def check_sessions_panel(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    text = cap.text()
    c.add(
        "sessions.title",
        "session panel title is visible",
        any("会话" in ln and "↑↓" in ln for ln in cap.lines),
        next((clip(ln) for ln in cap.lines if "会话" in ln and "↑↓" in ln), "missing session title"),
    )
    c.add(
        "sessions.prior_conversation_listed",
        "the conversation just completed is offered for resume",
        "读取 package" in text,
        next((clip(ln) for ln in cap.lines if "读取 package" in ln), "prior conversation missing"),
    )
    c.add(
        "sessions.session_id_visible",
        "session row carries a short session id",
        re.search(r"\b[0-9a-f]{8}\b", text) is not None,
        next((clip(ln) for ln in cap.lines if re.search(r"\b[0-9a-f]{8}\b", ln)), "no 8-hex id"),
    )
    c.add(
        "sessions.not_empty",
        "panel does not show the empty-sessions placeholder",
        "暂无历史会话" not in text,
        "no '暂无历史会话' placeholder",
    )
    panel_geometry(c, cap, "sessions", expected_width=84)
    focus_mode_checks(c, cap, "sessions")
    focus_border_checks(c, cap, "sessions")
    c.add(
        "sessions.current_marker",
        "sessions panel marks the active session",
        any("当前" in ln and "读取 package" in ln for ln in cap.lines),
        next((clip(ln) for ln in cap.lines if "当前" in ln), "no 当前 marker"),
    )


def check_add_provider(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    text = cap.text()
    c.add(
        "add_provider.title",
        "add-provider modal title is visible",
        "新增提供商" in text,
        next((clip(ln) for ln in cap.lines if "新增提供商" in ln), "missing modal title"),
    )
    c.add(
        "add_provider.step_indicator",
        "3-step wizard shows the name step",
        "1 / 3  name" in text and "Provider 名称" in text,
        next((clip(ln) for ln in cap.lines if "1 / 3" in ln), "missing step indicator"),
    )
    idx, line = input_line(cap)
    c.add(
        "add_provider.input_focused",
        "modal input row renders the caret prompt",
        idx is not None and "›" in (line or ""),
        f"row {idx} :: {clip(line or '')}",
    )
    panel_geometry(c, cap, "add_provider", expected_width=64)
    focus_mode_checks(c, cap, "add_provider")
    focus_border_checks(c, cap, "add_provider")


def check_streaming(c: Checks, ctx: Ctx) -> None:
    mid = ctx.captures["mid"]
    final = ctx.captures["final"]
    common_checks(c, mid)
    common_checks(c, final)

    mid_text = mid.text()
    # wave-39 R6/R9/R11: the generating state lives in the transcript active line only;
    # the footer is the metrics row (v3: out of the band, CANVAS background).  Mid-turn
    # has no usage yet, so the only segment it can carry is 上下文 (window resolved at send).
    # 状态三态契约（思考/生成/执行）：流式正文阶段 = 生成中，不再出现「写入中」。
    active = next((ln for ln in mid.lines if re.search(r"生成中\s+\d+秒", ln)), "")
    mid_footer = footer_text(mid)
    mid_has_context = "上下文" in mid_footer
    c.add(
        "streaming.mid_status_generating",
        "streaming active line shows the generating state while the footer is the metrics row",
        "生成中" in mid_text and mid_has_context and "完成" not in mid_footer,
        f"active_row :: {clip(active)} | footer :: {clip(mid_footer)}",
    )
    c.add(
        "streaming.mid_active_elapsed",
        "active line shows elapsed time next to the generating state",
        bool(active),
        f"active_row :: {clip(active)}" if active else "no elapsed line",
    )
    c.add(
        "streaming.mid_partial_body",
        "partial streamed markdown body is already rendered",
        "正在分析" in mid_text and "第一部分" in mid_text,
        next((clip(ln) for ln in mid.lines if "正在分析" in ln), "missing partial body"),
    )
    c.add(
        "streaming.mid_not_finished",
        "mid-stream capture is not the completed answer yet (footer is the metrics row)",
        "最后一行：流式结束" not in mid_text
        and mid_has_context
        and "完成" not in mid_footer,
        f"footer :: {clip(mid_footer)}",
    )
    c.add(
        "streaming.final_complete",
        "final capture contains the terminal answer and the metrics footer",
        "最后一行：流式结束" in final.text() and is_metrics_line(footer_text(final)),
        f"footer :: {clip(footer_text(final))}",
    )
    c.add(
        "streaming.final_not_generating",
        "transient generating status disappeared once the stream finished",
        "生成中" not in final.text(),
        f"footer :: {clip(footer_text(final))}",
    )


def check_error(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    input_checks(c, cap)
    text = cap.text()
    err_row, err_line = first_line(cap.lines, "Error:")
    c.add(
        "error.message_visible",
        "provider 401 is surfaced as a visible error message",
        err_row is not None,
        f"row {err_row} :: {clip(err_line)}" if err_line else "no Error: row",
    )
    c.add(
        "error.error_glyph",
        "error row carries the failure glyph",
        any("✕" in ln for ln in cap.lines),
        next((f"row {i} :: {clip(ln)}" for i, ln in enumerate(cap.lines) if "✕" in ln), "no ✕ glyph"),
    )
    c.add(
        "error.reason_included",
        "error text names the 401/unauthorized cause",
        "401" in text and "unauthorized" in text,
        next((clip(ln) for ln in cap.lines if "401" in ln), "401 missing"),
    )
    c.add(
        "error.footer_metrics",
        "footer stays the metrics row during a failed turn (no usage -> 上下文 only; failure in transcript)",
        "上下文" in footer_text(cap)
        and "失败" not in footer_text(cap)
        and "Turn failed" not in footer_text(cap),
        f"footer :: {clip(footer_text(cap))}",
    )
    c.add(
        "error.no_fake_answer",
        "no assistant answer is fabricated for the failed turn",
        "结论" not in text,
        "no 结论 row in failed capture",
    )
    chat = mock_chat_requests(ctx.extra.get("mock_log"))
    c.add(
        "error.mock_401_receipt",
        "mock receipt proves the 401 scenario was requested",
        any(r.get("scenario") == "401" for r in chat),
        f"scenarios={[r.get('scenario') for r in chat][-3:]}",
    )


def check_narrow(c: Checks, ctx: Ctx) -> None:
    empty = ctx.captures["empty"]
    conv = ctx.captures["conversation"]
    for cap in (empty, conv):
        common_checks(c, cap)
        input_checks(c, cap)
    header_checks(c, empty)
    separator_checks(c, empty)
    c.add(
        "narrow.empty_guidance",
        "narrow empty state keeps onboarding guidance on screen",
        "描述一个任务开始" in empty.text(),
        next((clip(ln) for ln in empty.lines if "描述一个任务开始" in ln), "missing guidance"),
    )
    c.add(
        "narrow.conversation_visible",
        "narrow conversation still shows the tail of the final answer",
        "0.1.125" in conv.text()
        and ("结论" in conv.text() or "如果需要" in conv.text()),
        next(
            (clip(ln) for ln in conv.lines if "0.1.125" in ln or "如果需要" in ln),
            "missing final answer tail",
        ),
    )
    user_row, user_line = first_line(conv.lines, "读取 package.json 并总结")
    tool_row, tool_line = first_line(conv.lines, "读取 (Read)")
    assistant_row, assistant_line = first_line(conv.lines, "结论")
    if user_line and tool_line and assistant_line:
        cols = [
            col_of(user_line, "读取 package.json 并总结"),
            col_of(assistant_line, "结论"),
            col_of(tool_line, "读取 (Read)"),
        ]
        c.add(
            "narrow.text_columns_aligned",
            "at 60x20 the three text streams still share one column",
            len(set(cols)) == 1,
            f"user_col={cols[0]} assistant_col={cols[1]} tool_col={cols[2]}",
        )


def check_wide(c: Checks, ctx: Ctx) -> None:
    empty = ctx.captures["empty"]
    conv = ctx.captures["conversation"]
    for cap in (empty, conv):
        common_checks(c, cap)
        input_checks(c, cap)
    header_checks(c, empty)
    separator_checks(c, empty)
    c.add(
        "wide.header_full_metadata",
        "wide header keeps workspace and model metadata on one line",
        "pony-agent" in empty.lines[0] and "mock-alpha" in empty.lines[0] if empty.lines else False,
        f"row 0 :: {clip(empty.lines[0]) if empty.lines else ''}",
    )
    c.add(
        "wide.conversation_visible",
        "wide conversation shows the final answer",
        "结论" in conv.text(),
        next((clip(ln) for ln in conv.lines if "结论" in ln), "missing 结论"),
    )
    user_row, user_line = first_line(conv.lines, "读取 package.json 并总结")
    tool_row, tool_line = first_line(conv.lines, "读取 (Read)")
    assistant_row, assistant_line = first_line(conv.lines, "结论")
    if user_line and tool_line and assistant_line:
        cols = [
            col_of(user_line, "读取 package.json 并总结"),
            col_of(assistant_line, "结论"),
            col_of(tool_line, "读取 (Read)"),
        ]
        c.add(
            "wide.text_columns_aligned",
            "at 160x40 the three text streams still share one column",
            len(set(cols)) == 1,
            f"user_col={cols[0]} assistant_col={cols[1]} tool_col={cols[2]}",
        )


def check_scrolled(c: Checks, ctx: Ctx) -> None:
    before = ctx.captures["before"]
    after = ctx.captures["after"]
    common_checks(c, before)
    common_checks(c, after)
    input_checks(c, after)

    c.add(
        "scrolled.bottom_before",
        "before scrolling the transcript is pinned to the newest line",
        "这是最后一行" in before.text() and "第 40 行" in before.text(),
        next((clip(ln) for ln in before.lines if "这是最后一行" in ln), "newest line missing"),
    )
    c.add(
        "scrolled.metrics_footer_before",
        "footer is the metrics line while pinned to bottom (scroll-up hint removed by wave-39)",
        is_metrics_line(footer_text(before)) and "已上翻" not in before.text(),
        f"footer :: {clip(footer_text(before))}",
    )
    c.add(
        "scrolled.after_shows_earlier_rows",
        "PageUp reveals earlier lines and hides the newest ones",
        "第 14 行" in after.text()
        and "这是最后一行" not in after.text(),
        next((clip(ln) for ln in after.lines if "第 14 行" in ln), "第 14 行 missing"),
    )
    c.add(
        "scrolled.metrics_footer_after",
        "footer stays the metrics line after scrolling up (the old '已上翻' hint row is gone)",
        is_metrics_line(footer_text(after)) and "已上翻" not in after.text(),
        f"footer :: {clip(footer_text(after))}",
    )

    def first_row_number(cap: Capture) -> int | None:
        for line in cap.lines:
            m = re.search(r"第 (\d+) 行", line)
            if m:
                return int(m.group(1))
        return None

    b, a = first_row_number(before), first_row_number(after)
    c.add(
        "scrolled.offset_moved_up",
        "first visible content row moved up by at least 8 terminal rows",
        b is not None and a is not None and b - a >= 8,
        f"first_visible_before=第{b}行 first_visible_after=第{a}行",
    )


def check_help(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    input_checks(c, cap)
    text = cap.text()
    row, line = first_line(cap.lines, "命令列表")
    if row is None:
        row, line = first_line(cap.lines, "Commands:")
    commands = ["/resume", "/provider", "/models", "/thinking", "/clear", "/cost", "/help", "/quit"]
    missing = [cmd for cmd in commands if cmd not in text]
    c.add(
        "help.commands_listed",
        "/help renders the command reference in the transcript",
        row is not None and not missing,
        (f"row {row} :: {clip(line)}" if line else "missing help heading")
        + f" missing={missing}",
    )
    separator_checks(c, cap)
    command_table_checks(c, cap, "help")
    c.add(
        "help.system_message_glyph",
        "help output is a system row (bullet glyph), not a user message",
        line is not None and "·" in line,
        f"row {row} :: {clip(line or '')}",
    )


def check_cost(c: Checks, ctx: Ctx) -> None:
    cap = ctx.captures["main"]
    common_checks(c, cap)
    input_checks(c, cap)
    text = cap.text()
    model_line = next(
        (ln for ln in cap.lines if re.search(r"(Model:|模型)", ln)),
        "",
    )
    gateway_line = next(
        (ln for ln in cap.lines if re.search(r"(Gateway:|网关)", ln)),
        "",
    )
    c.add(
        "cost.model_line",
        "/cost shows the active model",
        bool(model_line) and "mock-alpha" in model_line,
        f"row :: {clip(model_line)}" if model_line else "missing model row",
    )
    separator_checks(c, cap)
    c.add(
        "cost.gateway_line",
        "/cost shows the gateway endpoint",
        bool(gateway_line) and "http" in gateway_line,
        f"row :: {clip(gateway_line)}" if gateway_line else "missing gateway row",
    )


def check_input_edit(c: Checks, ctx: Ctx) -> None:
    steps = [
        ("abc", "abc", "typed text lands in the composer"),
        ("inserted", "abXc", "Left + X inserts in the middle without losing the tail"),
        ("backspace", "abX", "Right + Backspace deletes the character at the end"),
        ("word_cleared", "", "Ctrl+W deletes the whole word"),
        ("tab_complete", "/models", "Tab completes '/mo' to '/models'"),
        ("ctrl_u_cleared", "", "Ctrl+U clears the composer"),
        ("multiline", "line1\nline2", "Ctrl+J inserts a newline (two visual rows)"),
        ("esc_cleared", "", "Esc clears the composer"),
        ("draft", "draft", "draft text is present before Ctrl-C"),
    ]
    for name, expected, desc in steps:
        cap = ctx.captures[name]
        common_checks(c, cap)
        got = input_text(cap)
        c.add(
            f"input_edit.{name}",
            desc,
            got == expected,
            f"buffer={got!r} expected={expected!r}",
        )
    abc = ctx.captures["abc"]
    typed_col = composer_text_column(abc, "abc")
    c.add(
        "input_edit.typed_text_column",
        "typed text starts at the same column as the empty-state placeholder (5)",
        typed_col == 5,
        f"typed_col={typed_col}",
    )
    multiline = ctx.captures["multiline"]
    visuals = input_region_line_count(multiline)
    c.add(
        "input_edit.multiline_visual_rows",
        "multi-line draft occupies two distinct composer rows",
        visuals == 2,
        f"composer_rows={visuals} buffer={input_text(multiline)!r}",
    )

    draft_cleared = ctx.captures["draft_cleared"]
    common_checks(c, draft_cleared)
    got = input_text(draft_cleared)
    c.add(
        "input_edit.ctrl_c_with_draft_clears_only",
        "Ctrl-C with a draft clears the composer instead of quitting",
        got == "" and "AUDIT_EXIT" not in draft_cleared.text(),
        f"buffer={got!r} exit_marker={'AUDIT_EXIT' in draft_cleared.text()}",
    )
    c.add(
        "input_edit.ctrl_c_draft_still_running",
        "TUI is still painting its composer after the first Ctrl-C",
        "写下任务" in draft_cleared.text() or "输入任务" in draft_cleared.text(),
        next((clip(ln) for ln in draft_cleared.lines if "写下任务" in ln or "输入任务" in ln), "composer missing"),
    )

    exit_cap = ctx.captures["exit"]
    shell_cap = ctx.captures["shell"]
    common_checks(c, exit_cap, allow_ni=True)
    common_checks(c, shell_cap, allow_ni=True)
    m = re.search(r"AUDIT_EXIT=(\d+)", exit_cap.text())
    code = int(m.group(1)) if m else None
    c.add(
        "input_edit.ctrl_c_empty_exits",
        "a second Ctrl-C on the empty composer exits the process normally",
        code is not None and code in (0, 130, 143),
        f"exit_code={code} row :: {clip(next((ln for ln in exit_cap.lines if 'AUDIT_EXIT=' in ln), 'no marker'))}",
    )
    c.add(
        "input_edit.terminal_restored",
        "alternate screen is left after exit",
        "▁▁▁" not in exit_cap.text() and "写下任务" not in exit_cap.text(),
        f"tui_band={'▁▁▁' in exit_cap.text()}",
    )
    c.add(
        "input_edit.shell_alive",
        "the pane still runs commands after exit",
        "AUDIT_SHELL_ALIVE=ok" in shell_cap.text(),
        next((clip(ln) for ln in shell_cap.lines if "AUDIT_SHELL_ALIVE" in ln), "marker missing"),
    )
    pane_cmd = (ctx.extra.get("pane_command_after") or "").strip()
    c.add(
        "input_edit.pane_command_is_shell",
        "pane foreground command is a shell again",
        bool(pane_cmd) and not pane_cmd.endswith("pony"),
        f"pane_current_command={pane_cmd!r}",
    )


def check_ctrl_c(c: Checks, ctx: Ctx) -> None:
    exit_cap = ctx.captures["exit"]
    shell_cap = ctx.captures["shell"]
    common_checks(c, exit_cap, allow_ni=True)
    common_checks(c, shell_cap, allow_ni=True)
    m = re.search(r"AUDIT_EXIT=(\d+)", exit_cap.text())
    code = int(m.group(1)) if m else None
    c.add(
        "ctrl_c.process_exited",
        "Ctrl-C terminates the TUI process with a normal exit status",
        code is not None and code in (0, 130, 143),
        f"row :: {clip(next((ln for ln in exit_cap.lines if 'AUDIT_EXIT=' in ln), 'no AUDIT_EXIT line'))} code={code}",
    )
    c.add(
        "ctrl_c.terminal_restored",
        "alternate screen is left and the pane stops painting the TUI",
        "▁▁▁" not in exit_cap.text() and "写下任务" not in exit_cap.text(),
        f"tui_band_present={'▁▁▁' in exit_cap.text()} placeholder_present={'写下任务' in exit_cap.text()}",
    )
    c.add(
        "ctrl_c.shell_alive",
        "the same tmux pane still runs commands after exit",
        "AUDIT_SHELL_ALIVE=ok" in shell_cap.text(),
        next((clip(ln) for ln in shell_cap.lines if "AUDIT_SHELL_ALIVE" in ln), "marker missing"),
    )
    pane_cmd = (ctx.extra.get("pane_command_after") or "").strip()
    c.add(
        "ctrl_c.pane_command_is_shell",
        "pane foreground command is a shell again, not pony",
        bool(pane_cmd) and not pane_cmd.endswith("pony"),
        f"pane_current_command={pane_cmd!r}",
    )
    c.add(
        "ctrl_c.no_panic",
        "shutdown path prints no panic",
        not PANIC_RE.search(exit_cap.text() + "\n" + shell_cap.text()),
        "no panic tokens in shutdown captures",
    )


def check_non_tty(c: Checks, ctx: Ctx) -> None:
    out_file = ctx.extra.get("output_file")
    out_path = os.path.join(ctx.dir, out_file) if out_file else ""
    out = ""
    if out_path and os.path.exists(out_path):
        with open(out_path, encoding="utf-8", errors="replace") as fh:
            out = fh.read()
    code = ctx.extra.get("exit_code")
    c.add(
        "non_tty.diagnostic",
        "non-TTY stdin prints an explicit interactive-terminal diagnostic",
        "不是交互式终端" in out,
        clip(next((ln for ln in out.splitlines() if "不是交互式终端" in ln), "diagnostic missing")),
    )
    c.add(
        "non_tty.no_panic",
        "non-TTY launch does not panic",
        not PANIC_RE.search(out),
        "no panic tokens in stdout/stderr",
    )
    c.add(
        "non_tty.exit_code_recorded",
        "exit code is a real process status (not a timeout)",
        isinstance(code, int) and code != 124,
        f"exit_code={code}",
    )
    c.add(
        "non_tty.tty_hint",
        "diagnostic points the user at a real terminal / ponycli",
        "ponycli" in out or "真实终端" in out,
        clip(next((ln for ln in out.splitlines() if "ponycli" in ln or "真实终端" in ln), "no hint")),
    )


def check_cli_modes(c: Checks, ctx: Ctx) -> None:
    runs = ctx.extra.get("cmd_runs") or []
    by_name = {run.get("name"): run for run in runs}
    expected = {
        "version": [r"pony \d+\.\d+\.\d+", r"binary:"],
        "help": [r"用法:", r"--doctor"],
        "doctor": [r"\[ok\]", r"TTY"],
    }
    for name, patterns in expected.items():
        run = by_name.get(name)
        if not run:
            c.add(f"cli.{name}_run", f"--{name} run is recorded", False, "run missing")
            continue
        out_path = os.path.join(ctx.dir, run.get("output_file", ""))
        out = ""
        if os.path.exists(out_path):
            with open(out_path, encoding="utf-8", errors="replace") as fh:
                out = fh.read()
        missing = [p for p in patterns if re.search(p, out) is None]
        allowed = (0, 1) if name == "doctor" else (0,)
        c.add(
            f"cli.{name}_exit_status",
            f"--{name} exits with a documented status (0"
            + (", or 1 when the non-TTY self-check reports a hard failure)" if name == "doctor" else ")"),
            run.get("exit_code") in allowed,
            f"exit_code={run.get('exit_code')} allowed={allowed}",
        )
        c.add(
            f"cli.{name}_output",
            f"--{name} prints its documented payload",
            not missing,
            next((clip(ln) for ln in out.splitlines() if "pony " in ln or "用法" in ln or "[ok]" in ln), "output missing") + f" missing={missing}",
        )
        c.add(
            f"cli.{name}_no_panic",
            f"--{name} does not panic",
            not PANIC_RE.search(out),
            "no panic tokens in output",
        )


SCENARIOS = {
    "empty": check_empty,
    "conversation": check_conversation,
    "slash-menu": check_slash,
    "models-panel": check_models_panel,
    "provider-panel": check_provider_panel,
    "sessions-panel": check_sessions_panel,
    "add-provider": check_add_provider,
    "streaming": check_streaming,
    "error": check_error,
    "narrow": check_narrow,
    "wide": check_wide,
    "scrolled": check_scrolled,
    "help": check_help,
    "cost": check_cost,
    "ctrl-c": check_ctrl_c,
    "input-edit": check_input_edit,
    "non-tty": check_non_tty,
    "cli-modes": check_cli_modes,
}


# ---------------------------------------------------------------------------
# report aggregation
# ---------------------------------------------------------------------------
def load_report(path: str) -> dict | None:
    if not path or not os.path.exists(path):
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (json.JSONDecodeError, OSError):
        return None


def build_comparison(before: dict, after: dict) -> dict:
    """Per-scenario / per-check baseline -> after status transitions."""
    before_scen = {s["name"]: s for s in before.get("scenarios", [])}
    after_scen = {s["name"]: s for s in after.get("scenarios", [])}
    fixed: dict[str, list[str]] = {}
    regressed: dict[str, list[str]] = {}
    only_after: dict[str, list[str]] = {}
    only_before: dict[str, list[str]] = {}
    scenario_rows = []
    for name in sorted(set(before_scen) | set(after_scen), key=lambda n: (after_scen.get(n, {}).get("index", 999), n)):
        b = before_scen.get(name, {})
        a = after_scen.get(name, {})
        bchecks = {chk["id"]: chk for chk in b.get("checks", [])}
        achecks = {chk["id"]: chk for chk in a.get("checks", [])}
        for cid, chk in achecks.items():
            if cid in bchecks:
                if not bchecks[cid]["ok"] and chk["ok"]:
                    fixed.setdefault(name, []).append(cid)
                elif bchecks[cid]["ok"] and not chk["ok"]:
                    regressed.setdefault(name, []).append(cid)
            else:
                only_after.setdefault(name, []).append(cid)
        for cid in bchecks:
            if cid not in achecks:
                only_before.setdefault(name, []).append(cid)
        scenario_rows.append(
            {
                "scenario": name,
                "baseline": f"{b.get('pass', 0)}/{b.get('pass', 0) + b.get('fail', 0)}",
                "after": f"{a.get('pass', 0)}/{a.get('pass', 0) + a.get('fail', 0)}",
                "fixed": len(fixed.get(name, [])),
                "regressed": len(regressed.get(name, [])),
            }
        )
    return {
        "baseline": before.get("summary", {}),
        "after": after.get("summary", {}),
        "fixed": fixed,
        "regressed": regressed,
        "only_after": only_after,
        "only_before": only_before,
        "scenarios": scenario_rows,
    }


def comparison_markdown(comp: dict) -> list[str]:
    b = comp["baseline"]
    a = comp["after"]
    fixed_total = sum(len(v) for v in comp["fixed"].values())
    regressed_total = sum(len(v) for v in comp["regressed"].values())
    only_after_total = sum(len(v) for v in comp["only_after"].values())
    only_before_total = sum(len(v) for v in comp["only_before"].values())
    lines = ["## baseline → after 差异摘要", ""]
    lines.append(
        f"- 断言：baseline {b.get('checks', 0)} / after {a.get('checks', 0)}；"
        f"失败 {b.get('fail', 0)} → {a.get('fail', 0)}；"
        f"场景失败 {len(b.get('failed_scenarios', []))} → {len(a.get('failed_scenarios', []))}。"
    )
    lines.append(f"- FAIL→PASS（已消除）：{fixed_total} 项。")
    if comp["fixed"]:
        for name, ids in comp["fixed"].items():
            lines.append(f"  - {name}：{', '.join(f'`{i}`' for i in ids)}")
    else:
        lines.append("  - 无")
    lines.append(f"- PASS→FAIL（回归）：{regressed_total} 项。")
    if comp["regressed"]:
        for name, ids in comp["regressed"].items():
            lines.append(f"  - {name}：{', '.join(f'`{i}`' for i in ids)}")
    else:
        lines.append("  - 无")
    if only_after_total:
        lines.append(
            f"- 仅 after 求值的断言（baseline 因边框/证据缺失提前返回，或本相位新增）：{only_after_total} 项。"
        )
        for name, ids in comp["only_after"].items():
            lines.append(f"  - {name}：{', '.join(f'`{i}`' for i in ids)}")
    if only_before_total:
        lines.append(f"- 仅 baseline 求值的断言：{only_before_total} 项。")
        for name, ids in comp["only_before"].items():
            lines.append(f"  - {name}：{', '.join(f'`{i}`' for i in ids)}")
    lines.append("")
    lines.append("| scenario | baseline pass/total | after pass/total | fixed | regressed |")
    lines.append("| --- | --- | --- | --- | --- |")
    for row in comp["scenarios"]:
        lines.append(
            f"| {row['scenario']} | {row['baseline']} | {row['after']} | {row['fixed']} | {row['regressed']} |"
        )
    lines.append("")
    return lines


def aggregate(phase_dir: str, compare_dir: str | None = None) -> int:
    phase_dir = os.path.abspath(phase_dir)
    with open(os.path.join(phase_dir, "bin-info.json"), encoding="utf-8") as fh:
        bin_info = json.load(fh)
    scenarios = []
    for name in sorted(os.listdir(phase_dir)):
        sdir = os.path.join(phase_dir, name)
        a_path = os.path.join(sdir, "assertions.json")
        m_path = os.path.join(sdir, "meta.json")
        if not (os.path.isdir(sdir) and os.path.exists(a_path) and os.path.exists(m_path)):
            continue
        with open(a_path, encoding="utf-8") as fh:
            assertions = json.load(fh)
        with open(m_path, encoding="utf-8") as fh:
            meta = json.load(fh)
        scenarios.append(
            {
                "name": name,
                "index": meta.get("index", 0),
                "cols": meta.get("cols"),
                "rows": meta.get("rows"),
                "duration_ms": meta.get("duration_ms"),
                "started_at": meta.get("started_at"),
                "evidence_dir": os.path.relpath(sdir, phase_dir),
                "captures": sorted(
                    f[:-4] for f in os.listdir(sdir) if f.endswith(".txt") and f != "assertions.txt"
                ),
                "pass": assertions.get("pass", 0),
                "fail": assertions.get("fail", 0),
                "checks": assertions.get("checks", []),
            }
        )
    scenarios.sort(key=lambda item: item["index"])
    total_pass = sum(s["pass"] for s in scenarios)
    total_fail = sum(s["fail"] for s in scenarios)
    failed_scenarios = [s["name"] for s in scenarios if s["fail"]]
    report = {
        "phase": bin_info.get("phase"),
        "binary": bin_info,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "summary": {
            "scenarios": len(scenarios),
            "checks": total_pass + total_fail,
            "pass": total_pass,
            "fail": total_fail,
            "failed_scenarios": failed_scenarios,
        },
        "scenarios": scenarios,
    }
    comparison = None
    if compare_dir:
        before_report = load_report(os.path.join(os.path.abspath(compare_dir), "report.json"))
        if before_report:
            comparison = build_comparison(before_report, report)
            report["baseline_comparison"] = comparison
    json_path = os.path.join(phase_dir, "report.json")
    with open(json_path, "w", encoding="utf-8") as fh:
        json.dump(report, fh, ensure_ascii=False, indent=2)

    md = []
    md.append(f"# TUI audit report — {bin_info.get('phase')}")
    md.append("")
    md.append("| field | value |")
    md.append("| --- | --- |")
    md.append(f"| binary | `{bin_info.get('bin')}` |")
    md.append(f"| sha256 | `{bin_info.get('sha256')}` |")
    md.append(f"| mtime | `{bin_info.get('mtime')}` |")
    md.append(f"| version | `{bin_info.get('version')}` |")
    md.append(f"| generated_at | `{report['generated_at']}` |")
    md.append(
        f"| result | **{total_pass} pass / {total_fail} fail** across "
        f"{len(scenarios)} scenarios |"
    )
    md.append("")
    if bin_info.get("phase") == "baseline":
        md.append(
            "> Baseline is the pre-refactor release binary. Its failing checks are the "
            "expected contrast evidence for the UI refactor; they are reported verbatim, "
            "never relaxed."
        )
        md.append("")
    if failed_scenarios:
        md.append("## Failed checks")
        md.append("")
        md.append("| scenario | check | evidence |")
        md.append("| --- | --- | --- |")
        for scen in scenarios:
            for chk in scen["checks"]:
                if not chk["ok"]:
                    ev = chk["evidence"].replace("|", "\\|")
                    md.append(f"| {scen['name']} | `{chk['id']}` | {ev} |")
        md.append("")
    if comparison:
        md.extend(comparison_markdown(comparison))
    md.append("## Scenarios")
    md.append("")
    for scen in scenarios:
        md.append(f"### {scen['name']} ({scen['cols']}x{scen['rows']})")
        md.append("")
        md.append(f"- evidence: `{scen['evidence_dir']}/`")
        if scen["captures"]:
            links = ", ".join(
                f"[{cap}.png]({scen['evidence_dir']}/{cap}.png)" for cap in scen["captures"]
            )
            md.append(f"- screenshots: {links}")
        md.append(f"- checks: {scen['pass']} pass / {scen['fail']} fail")
        md.append("")
        md.append("| result | check | evidence |")
        md.append("| --- | --- | --- |")
        for chk in scen["checks"]:
            ev = chk["evidence"].replace("|", "\\|")
            md.append(
                f"| {'PASS' if chk['ok'] else 'FAIL'} | `{chk['id']}` — {chk['desc']} | {ev} |"
            )
        md.append("")
    md_path = os.path.join(phase_dir, "report.md")
    with open(md_path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(md) + "\n")

    print(json.dumps({"report_json": json_path, "report_md": md_path,
                      "pass": total_pass, "fail": total_fail,
                      "failed_scenarios": failed_scenarios}, ensure_ascii=False))
    return 0


# ---------------------------------------------------------------------------
# entry point
# ---------------------------------------------------------------------------
def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="command", required=True)

    p_check = sub.add_parser("check", help="assert one scenario evidence directory")
    p_check.add_argument("--dir", required=True)

    p_agg = sub.add_parser("aggregate", help="build report.json/report.md for a phase")
    p_agg.add_argument("--phase-dir", required=True)
    p_agg.add_argument(
        "--compare",
        default=None,
        help="baseline phase dir; appends a baseline -> after diff section",
    )

    args = ap.parse_args(argv)

    if args.command == "check":
        ctx = Ctx(args.dir)
        scenario = ctx.meta["scenario"]
        fn = SCENARIOS.get(scenario)
        if fn is None:
            print(f"FATAL: unknown scenario {scenario!r}", file=sys.stderr)
            return 2
        checks = Checks(scenario)
        fn(checks, ctx)
        payload = checks.as_dict()
        with open(os.path.join(ctx.dir, "assertions.json"), "w", encoding="utf-8") as fh:
            json.dump(payload, fh, ensure_ascii=False, indent=2)
        print(
            f"RESULT scenario={scenario} pass={payload['pass']} fail={payload['fail']}"
        )
        return 0

    return aggregate(args.phase_dir, args.compare)


if __name__ == "__main__":
    sys.exit(main())
