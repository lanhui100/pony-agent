#!/usr/bin/env python3
"""Render `tmux capture-pane -p -e` output to a deterministic PNG.

The renderer is a *cell grid* renderer, not a terminal emulator:
  * every cell is `cell_w x cell_h` logical pixels (multiplied by --scale);
  * wide (East-Asian W/F) characters occupy exactly two cells;
  * SGR attributes parsed: truecolor (38;2 / 48;2), 256-color, 16-color,
    bold/dim/italic/underline/reverse, default fg/bg resets (39/49);
  * glyph fallback chain: DejaVu Sans Mono -> Droid Sans Fallback (CJK) ->
    FreeMono (braille) -> DejaVu Sans (misc symbols);
  * box-drawing / block / braille characters are covered by that chain and are
    never substituted by a placeholder.

Output size is stable for a given (cols, rows, font-size, scale):
    width  = cols * cell_w * scale
    height = rows * cell_h * scale

Usage:
    render_ansi.py --input screen.ansi --output screen.png \
                   --cols 120 --rows 34 --scale 2
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import unicodedata
from dataclasses import dataclass, field

from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont

DEFAULT_FG = (232, 229, 224)
DEFAULT_BG = (18, 16, 14)

FONT_PRIMARY = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
FONT_PRIMARY_BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
FONT_CJK = "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf"
FONT_BRAILLE = "/usr/share/fonts/truetype/freefont/FreeMono.ttf"
FONT_SYMBOL = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"


# --------------------------------------------------------------------------
# Character width (unicode-width compatible for the characters a TUI uses)
# --------------------------------------------------------------------------
def char_width(ch: str) -> int:
    if not ch:
        return 0
    cp = ord(ch)
    if cp < 32 or cp == 0x7F:
        return 0
    if unicodedata.combining(ch):
        return 0
    # Zero-width joiner / variation selectors
    if cp in (0x200B, 0x200C, 0x200D, 0xFE0E, 0xFE0F):
        return 0
    if unicodedata.east_asian_width(ch) in ("W", "F"):
        return 2
    return 1


# --------------------------------------------------------------------------
# Font machinery
# --------------------------------------------------------------------------
class FontSet:
    """Font set for pixel-space rendering (all sizes are already scaled)."""

    def __init__(self, size: int):
        self.size = size
        self._cache: dict[tuple[str, int], ImageFont.FreeTypeFont] = {}
        self.coverage: list[set[int]] = []
        self.paths = [FONT_PRIMARY, FONT_CJK, FONT_BRAILLE, FONT_SYMBOL]
        for path in self.paths:
            self.coverage.append(_cmap(path))
        self.primary = self.get(FONT_PRIMARY)
        self.primary_bold = ImageFont.truetype(FONT_PRIMARY_BOLD, size)
        self.ascent, self.descent = self.primary.getmetrics()
        self.cell_w = max(1, int(round(self.primary.getlength("M"))))
        self.cell_h = max(self.cell_w + 2, int(round(size * 1.32)))

    def font_size(self, path: str) -> int:
        # CJK ideographs are 1em-wide: to fill two terminal cells they must be
        # rasterized at 2*cell_w.
        if path == FONT_CJK:
            return max(1, self.cell_w * 2)
        return self.size

    def get(self, path: str) -> ImageFont.FreeTypeFont:
        size = self.font_size(path)
        key = (path, size)
        if key not in self._cache:
            self._cache[key] = ImageFont.truetype(path, size)
        return self._cache[key]

    def pick(self, ch: str) -> tuple[ImageFont.FreeTypeFont, str]:
        """Return (font, font_path) able to draw `ch`."""
        cp = ord(ch)
        for path, cov in zip(self.paths, self.coverage):
            if cp in cov:
                return self.get(path), path
        # Unknown glyph: primary will render .notdef; caller marks it visibly.
        return self.primary, FONT_PRIMARY

    def bold_of(self, path: str) -> ImageFont.FreeTypeFont:
        if path == FONT_PRIMARY:
            return self.primary_bold
        return self.get(path)


# --------------------------------------------------------------------------
# Procedural glyphs: box drawing / block elements / braille.
# Drawing these from rectangles guarantees that strokes touch cell edges (font
# rasterization with a rounded cell width would otherwise leave 1px seams).
# --------------------------------------------------------------------------
BOX_LIGHT = {"─", "│", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼"}
BOX_HEAVY = {"━", "┃", "┏", "┓", "┗", "┛", "┣", "┫", "┳", "┻", "╋"}
BOX_DOUBLE = {"═", "║", "╔", "╗", "╚", "╝", "╠", "╣", "╦", "╩", "╬"}

# direction bits: up, right, down, left
BOX_CONNECTIONS = {
    "─": (0, 1, 0, 1),
    "│": (1, 0, 1, 0),
    "┌": (0, 1, 1, 0),
    "┐": (0, 0, 1, 1),
    "└": (1, 1, 0, 0),
    "┘": (1, 0, 0, 1),
    "├": (1, 1, 1, 0),
    "┤": (1, 0, 1, 1),
    "┬": (0, 1, 1, 1),
    "┴": (1, 1, 0, 1),
    "┼": (1, 1, 1, 1),
    "━": (0, 1, 0, 1),
    "┃": (1, 0, 1, 0),
    "┏": (0, 1, 1, 0),
    "┓": (0, 0, 1, 1),
    "┗": (1, 1, 0, 0),
    "┛": (1, 0, 0, 1),
    "┣": (1, 1, 1, 0),
    "┫": (1, 0, 1, 1),
    "┳": (0, 1, 1, 1),
    "┻": (1, 1, 0, 1),
    "╋": (1, 1, 1, 1),
    "═": (0, 1, 0, 1),
    "║": (1, 0, 1, 0),
    "╔": (0, 1, 1, 0),
    "╗": (0, 0, 1, 1),
    "╚": (1, 1, 0, 0),
    "╝": (1, 0, 0, 1),
    "╠": (1, 1, 1, 0),
    "╣": (1, 0, 1, 1),
    "╦": (0, 1, 1, 1),
    "╩": (1, 1, 0, 1),
    "╬": (1, 1, 1, 1),
}
BOX_HEAVY_CHARS = BOX_HEAVY
BOX_DOUBLE_CHARS = BOX_DOUBLE

# Fractional-width left blocks U+258F..U+2589 (1/8 .. 7/8), plus U+2588 full.
LEFT_BLOCKS = {
    0x258F: 1 / 8, 0x258E: 2 / 8, 0x258D: 3 / 8, 0x258C: 4 / 8,
    0x258B: 5 / 8, 0x258A: 6 / 8, 0x2589: 7 / 8, 0x2588: 1.0,
}
# Lower blocks U+2581..U+2587 (1/8 .. 7/8) and U+2584 half.
LOWER_BLOCKS = {
    0x2581: 1 / 8, 0x2582: 2 / 8, 0x2583: 3 / 8, 0x2584: 0.5,
    0x2585: 5 / 8, 0x2586: 6 / 8, 0x2587: 7 / 8,
}
SHADE_LEVELS = {0x2591: 0.25, 0x2592: 0.5, 0x2593: 0.75}


def draw_box_char(draw, ch: str, x0: int, y0: int, cw: int, chh: int, fg) -> bool:
    conn = BOX_CONNECTIONS.get(ch)
    if conn is None:
        return False
    up, right, down, left = conn
    heavy = ch in BOX_HEAVY_CHARS
    double = ch in BOX_DOUBLE_CHARS
    thick = max(1, round(cw / 9)) if not heavy else max(2, round(cw / 5))
    mid_x = x0 + cw // 2
    mid_y = y0 + chh // 2
    half_t = thick // 2

    def hline(xa: int, xb: int, y: int):
        draw.rectangle([xa, y - half_t, xb, y + half_t], fill=fg)

    def vline(y_a: int, y_b: int, x: int):
        draw.rectangle([x - half_t, y_a, x + half_t, y_b], fill=fg)

    if double:
        gap = max(2, thick + 1)
        if left or right:
            for dy in (-gap // 2, gap // 2 + 1):
                y = mid_y + dy
                xa = x0 if left else mid_x
                xb = x0 + cw - 1 if right else mid_x
                draw.rectangle([xa, y - half_t, xb, y + half_t], fill=fg)
        if up or down:
            for dx in (-gap // 2, gap // 2 + 1):
                x = mid_x + dx
                ya = y0 if up else mid_y
                yb = y0 + chh - 1 if down else mid_y
                draw.rectangle([x - half_t, ya, x + half_t, yb], fill=fg)
        return True

    if left:
        hline(x0, mid_x, mid_y)
    if right:
        hline(mid_x, x0 + cw - 1, mid_y)
    if up:
        vline(y0, mid_y, mid_x)
    if down:
        vline(mid_y, y0 + chh - 1, mid_x)
    return True


def draw_block_char(draw, ch: str, x0: int, y0: int, cw: int, chh: int, fg) -> bool:
    cp = ord(ch)
    if cp in LEFT_BLOCKS:
        w = max(1, round(cw * LEFT_BLOCKS[cp]))
        draw.rectangle([x0, y0, x0 + w - 1, y0 + chh - 1], fill=fg)
        return True
    if cp in LOWER_BLOCKS:
        h = max(1, round(chh * LOWER_BLOCKS[cp]))
        draw.rectangle([x0, y0 + chh - h, x0 + cw - 1, y0 + chh - 1], fill=fg)
        return True
    if cp == 0x2580:  # upper half
        draw.rectangle([x0, y0, x0 + cw - 1, y0 + chh // 2], fill=fg)
        return True
    if cp in SHADE_LEVELS:  # dithered shade
        step = 2 if SHADE_LEVELS[cp] <= 0.25 else 1
        for yy in range(y0, y0 + chh, 2):
            for xx in range(x0 + ((yy - y0) // 2 % 2) * step, x0 + cw, step * 2):
                draw.point((xx, yy), fill=fg)
        return True
    return False


def draw_braille_char(draw, ch: str, x0: int, y0: int, cw: int, chh: int, fg) -> bool:
    cp = ord(ch)
    if not (0x2800 <= cp <= 0x28FF):
        return False
    bits = cp - 0x2800
    if bits == 0:
        return True  # blank braille
    # 2 columns x 4 rows; dot order per Unicode braille pattern.
    positions = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)]
    radius = max(1, round(min(cw, chh) / 10))
    left_x = x0 + cw * 0.32
    right_x = x0 + cw * 0.68
    top_y = y0 + chh * 0.16
    step_y = chh * 0.68 / 3
    for idx, (col, row) in enumerate(positions):
        if bits & (1 << idx):
            cx = left_x if col == 0 else right_x
            cy = top_y + row * step_y
            draw.ellipse([cx - radius, cy - radius, cx + radius, cy + radius], fill=fg)
    return True


def _cmap(path: str) -> set[int]:
    try:
        font = TTFont(path, fontNumber=0, lazy=True)
        cmap: set[int] = set()
        for table in font["cmap"].tables:
            cmap |= set(table.cmap)
        font.close()
        return cmap
    except Exception:  # pragma: no cover - font availability is environment
        return set()


# --------------------------------------------------------------------------
# SGR parsing
# --------------------------------------------------------------------------
@dataclass
class Style:
    fg: tuple[int, int, int] | None = None
    bg: tuple[int, int, int] | None = None
    bold: bool = False
    dim: bool = False
    italic: bool = False
    underline: bool = False
    reverse: bool = False


@dataclass
class Cell:
    ch: str = ""
    style: Style = field(default_factory=Style)
    width: int = 1


BASE16 = [
    (0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16),
    (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
    (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67),
    (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255),
]


def _xterm256(n: int) -> tuple[int, int, int]:
    if n < 16:
        return BASE16[n]
    if n < 232:
        n -= 16
        r, g, b = n // 36, (n % 36) // 6, n % 6
        conv = lambda v: 0 if v == 0 else 55 + v * 40
        return (conv(r), conv(g), conv(b))
    level = 8 + (n - 232) * 10
    return (level, level, level)


def apply_sgr(style: Style, params: list[int]) -> Style:
    out = Style(**style.__dict__)
    i = 0
    while i < len(params):
        p = params[i]
        if p == 0:
            out = Style()
        elif p == 1:
            out.bold = True
        elif p == 2:
            out.dim = True
        elif p == 3:
            out.italic = True
        elif p == 4:
            out.underline = True
        elif p == 7:
            out.reverse = True
        elif p == 22:
            out.bold = False
            out.dim = False
        elif p == 23:
            out.italic = False
        elif p == 24:
            out.underline = False
        elif p == 27:
            out.reverse = False
        elif 30 <= p <= 37:
            out.fg = BASE16[p - 30]
        elif p == 38:
            consumed, color = _parse_extended(params, i)
            if color is not None:
                out.fg = color
            i += consumed
        elif p == 39:
            out.fg = None
        elif 40 <= p <= 47:
            out.bg = BASE16[p - 40]
        elif p == 48:
            consumed, color = _parse_extended(params, i)
            if color is not None:
                out.bg = color
            i += consumed
        elif p == 49:
            out.bg = None
        elif 90 <= p <= 97:
            out.fg = BASE16[p - 90 + 8]
        elif 100 <= p <= 107:
            out.bg = BASE16[p - 100 + 8]
        i += 1
    return out


def _parse_extended(params: list[int], i: int) -> tuple[int, tuple[int, int, int] | None]:
    """params[i] is 38/48. Returns (extra params consumed, color)."""
    if i + 1 >= len(params):
        return 0, None
    mode = params[i + 1]
    if mode == 5 and i + 2 < len(params):
        return 2, _xterm256(params[i + 2] & 0xFF)
    if mode == 2 and i + 4 < len(params):
        return 4, (
            params[i + 2] & 0xFF,
            params[i + 3] & 0xFF,
            params[i + 4] & 0xFF,
        )
    return 1, None


CSI_RE = re.compile(r"\x1b\[([0-9;:?]*)([ -/]*)([@-~])")
OSC_RE = re.compile(r"\x1b\](?:[^\x07\x1b]*)(?:\x07|\x1b\\)")


def _pad_line_tail(grid: list[list[Cell]], row: int, col: int, cols: int, style: Style) -> None:
    """Fill the rest of `row` with spaces carrying the active SGR style."""
    if row < 0 or row >= len(grid):
        return
    for c in range(max(0, col), min(cols, len(grid[row]))):
        cell = grid[row][c]
        if cell.width == 1 and cell.ch == " " and cell.style.bg is None and cell.style.fg is None:
            grid[row][c] = Cell(ch=" ", style=Style(**style.__dict__), width=1)


def parse_ansi(text: str, cols: int, rows: int) -> list[list[Cell]]:
    grid = [[Cell(ch=" ") for _ in range(cols)] for _ in range(rows)]
    style = Style()
    row = col = 0
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == "\x1b":
            m = CSI_RE.match(text, i)
            if m:
                params_raw, _inter, final = m.groups()
                if final == "m":
                    params = []
                    for part in params_raw.replace(":", ";").split(";"):
                        if part == "":
                            params.append(0)
                        else:
                            try:
                                params.append(int(part))
                            except ValueError:
                                params.append(0)
                    style = apply_sgr(style, params or [0])
                i = m.end()
                continue
            m = OSC_RE.match(text, i)
            if m:
                i = m.end()
                continue
            i += 1
            continue
        if ch == "\r":
            col = 0
            i += 1
            continue
        if ch == "\n":
            _pad_line_tail(grid, row, col, cols, style)
            row += 1
            col = 0
            i += 1
            continue
        if ch == "\t":
            col = min(cols - 1, (col // 8 + 1) * 8)
            i += 1
            continue
        width = char_width(ch)
        if width == 0:
            # Combining mark: append to previous cell when possible.
            if col > 0 and row < rows:
                prev_col = max(0, col - 1)
                while prev_col > 0 and grid[row][prev_col].ch == "":
                    prev_col -= 1
                grid[row][prev_col].ch += ch
            i += 1
            continue
        if row >= rows:
            # Terminal would have scrolled; capture-pane normally never does.
            break
        if col >= cols:
            i += 1
            continue
        if width == 2 and col == cols - 1:
            width = 1  # wide char cannot straddle the edge
        grid[row][col] = Cell(ch=ch, style=Style(**style.__dict__), width=width)
        if width == 2 and col + 1 < cols:
            grid[row][col + 1] = Cell(ch="", style=Style(**style.__dict__), width=0)
        col += width
        i += 1
    if row < rows:
        # tmux strips trailing blanks; keep the active SGR background so a
        # composer band / panel fill does not visibly stop mid-row in the PNG.
        _pad_line_tail(grid, row, col, cols, style)
    return grid


# --------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------
def _blend_dim(color: tuple[int, int, int], bg: tuple[int, int, int]) -> tuple[int, int, int]:
    return tuple(int(c * 0.65 + b * 0.35) for c, b in zip(color, bg))


def render(
    ansi_text: str,
    cols: int,
    rows: int,
    scale: int = 2,
    font_size: int = 16,
    default_fg: tuple[int, int, int] = DEFAULT_FG,
    default_bg: tuple[int, int, int] = DEFAULT_BG,
) -> Image.Image:
    fonts = FontSet(font_size * scale)
    grid = parse_ansi(ansi_text, cols, rows)

    cw = fonts.cell_w
    chh = fonts.cell_h
    img = Image.new("RGB", (cols * cw, rows * chh), default_bg)
    draw = ImageDraw.Draw(img)

    for r in range(rows):
        for c in range(cols):
            cell = grid[r][c]
            if not cell.ch:
                # Continuation cell of a wide (CJK) glyph.  The owning cell
                # already painted the full two-cell background rectangle;
                # repainting here would cover the glyph's right half.
                continue

            st = cell.style
            fg = st.fg if st.fg is not None else default_fg
            bg = st.bg if st.bg is not None else default_bg
            if st.reverse:
                fg, bg = bg, fg
            if st.dim:
                fg = _blend_dim(fg, bg)

            width_cells = 2 if cell.width == 2 else 1
            x0 = c * cw
            y0 = r * chh
            x1 = x0 + cw * width_cells
            draw.rectangle([x0, y0, x1 - 1, y0 + chh - 1], fill=bg)

            base = cell.ch[0]
            zwidth = cw * width_cells
            if draw_box_char(draw, base, x0, y0, zwidth, chh, fg):
                pass
            elif draw_block_char(draw, base, x0, y0, zwidth, chh, fg):
                pass
            elif draw_braille_char(draw, base, x0, y0, zwidth, chh, fg):
                pass
            else:
                font, path = fonts.pick(base)
                draw_font = fonts.bold_of(path) if st.bold and path in (
                    FONT_PRIMARY,
                    FONT_PRIMARY_BOLD,
                ) else font
                stroke = 0
                if st.bold and draw_font is font:
                    # Synthetic bold for CJK / symbol fonts that ship no bold face.
                    stroke = max(1, round(scale / 2))

                anchor_y = y0 + chh // 2
                try:
                    draw.text(
                        (x0, anchor_y),
                        cell.ch,
                        font=draw_font,
                        fill=fg,
                        anchor="lm",
                        stroke_width=stroke,
                        stroke_fill=fg,
                    )
                except Exception:
                    draw.text((x0, anchor_y), cell.ch, font=draw_font, fill=fg, anchor="lm")

            if st.underline:
                uy = y0 + chh - max(2, round(fonts.descent / 2))
                draw.line([x0, uy, x1 - 1, uy], fill=fg, width=max(1, scale))

    return img


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", required=True, help="capture-pane -e text file")
    ap.add_argument("--output", required=True, help="PNG output path")
    ap.add_argument("--cols", type=int, required=True)
    ap.add_argument("--rows", type=int, required=True)
    ap.add_argument("--scale", type=int, default=2)
    ap.add_argument("--font-size", type=int, default=16)
    ap.add_argument("--fg", default="232,229,224")
    ap.add_argument("--bg", default="13,14,17")
    args = ap.parse_args(argv)

    with open(args.input, "r", encoding="utf-8", errors="replace") as fh:
        raw = fh.read()
    # tmux capture-pane joins rows with '\n' and appends one final '\n'; do not
    # let that trailing newline push a phantom row.
    text = raw.rstrip("\n")
    rows = [ln for ln in text.split("\n")]
    if len(rows) > args.rows:
        rows = rows[-args.rows:]
    text = "\n".join(rows)

    fg = tuple(int(v) for v in args.fg.split(","))
    bg = tuple(int(v) for v in args.bg.split(","))
    img = render(text, args.cols, args.rows, args.scale, args.font_size, fg, bg)
    os.makedirs(os.path.dirname(os.path.abspath(args.output)) or ".", exist_ok=True)
    img.save(args.output)
    print(
        f"rendered {args.output} {img.width}x{img.height} "
        f"(cols={args.cols} rows={args.rows} scale={args.scale})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
