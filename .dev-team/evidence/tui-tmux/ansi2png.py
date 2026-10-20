#!/usr/bin/env python3
"""Render a tmux `capture-pane -e` ANSI dump into a PNG screenshot.

Cell model matches tmux's own grid: every emitted character advances
East-Asian-Width cells (W/F = 2, ambiguous = 1), with no placeholder
skipping. Each glyph is drawn at its exact cell column so box borders stay
aligned regardless of the font's CJK advance.
"""
import re
import sys
import unicodedata

from PIL import Image, ImageDraw, ImageFont

CJK_FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
]
SYMBOL_FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
]
DEFAULT_FG = (232, 229, 224)
DEFAULT_BG = (18, 16, 14)

SGR = re.compile(r"\x1b\[([0-9;]*)m")
CELL_W = 9
CELL_H = 20
FONT_PX = 16
PAD = 14


def load_font(candidates):
    for path in candidates:
        try:
            return ImageFont.truetype(path, 16)
        except Exception:
            continue
    return ImageFont.load_default()


CJK_FONT = load_font(CJK_FONT_CANDIDATES)
SYMBOL_FONT = load_font(SYMBOL_FONT_CANDIDATES)


def glyph_font(ch: str):
    """CJK glyphs from a CJK font; everything else from DejaVu (rich symbols)."""
    return CJK_FONT if unicodedata.east_asian_width(ch) in ("W", "F") else SYMBOL_FONT


def char_width(ch: str) -> int:
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def parse_runs(line: str):
    fg, bg = DEFAULT_FG, DEFAULT_BG
    bold = False
    runs = []
    for idx, part in enumerate(SGR.split(line)):
        if idx % 2 == 1:
            codes = [c for c in part.split(";") if c != ""]
            i = 0
            while i < len(codes):
                code = codes[i]
                if code == "0":
                    fg, bg, bold = DEFAULT_FG, DEFAULT_BG, False
                elif code == "1":
                    bold = True
                elif code == "22":
                    bold = False
                elif code in ("38", "48") and i + 4 < len(codes) and codes[i + 1] == "2":
                    color = tuple(int(codes[i + k]) for k in (2, 3, 4))
                    if code == "38":
                        fg = color
                    else:
                        bg = color
                    i += 4
                elif code == "39":
                    fg = DEFAULT_FG
                elif code == "49":
                    bg = DEFAULT_BG
                i += 1
        elif part:
            runs.append((part, fg, bg, bold))
    return runs


def render(src: str, dst: str, cols: int = 120, rows: int = 36) -> None:
    with open(src, "r", encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    lines = lines[:rows]

    width = cols * CELL_W + PAD * 2
    height = rows * CELL_H + PAD * 2
    image = Image.new("RGB", (width, height), DEFAULT_BG)
    draw = ImageDraw.Draw(image)

    for row, line in enumerate(lines):
        y = PAD + row * CELL_H
        col = 0
        for text, fg, bg, bold in parse_runs(line):
            for ch in text:
                cells = char_width(ch)
                x = PAD + col * CELL_W
                if bg != DEFAULT_BG:
                    draw.rectangle(
                        [x, y, x + cells * CELL_W - 1, y + CELL_H - 1], fill=bg
                    )
                if not ch.isspace():
                    font_for_ch = glyph_font(ch)
                    draw.text((x, y - 1), ch, font=font_for_ch, fill=fg)
                    if bold:
                        # SGR 1：用亚像素描边模拟粗体，保证证据图能体现强调层级。
                        for dx, dy in ((0.6, 0), (0, 0.4), (0.6, 0.4)):
                            draw.text((x + dx, y - 1 + dy), ch, font=font_for_ch, fill=fg)
                col += cells
    image.save(dst)
    print(f"saved {dst} ({width}x{height})")


if __name__ == "__main__":
    render(sys.argv[1], sys.argv[2])
