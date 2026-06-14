from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
REVIEW_DIR = ICON_DIR / "review"
OUT = REVIEW_DIR / "icon-audit-board.png"


def load(path: Path) -> Image.Image:
    return Image.open(path).convert("RGBA")


def paste_center(canvas: Image.Image, image: Image.Image, box: tuple[int, int, int, int], bg=None) -> None:
    x0, y0, x1, y1 = box
    if bg is not None:
        draw = ImageDraw.Draw(canvas)
        draw.rounded_rectangle(box, radius=18, fill=bg)
    avail_w = x1 - x0
    avail_h = y1 - y0
    ratio = min(avail_w / image.width, avail_h / image.height)
    resized = image.resize((max(1, round(image.width * ratio)), max(1, round(image.height * ratio))), Image.Resampling.NEAREST if ratio >= 4 else Image.Resampling.LANCZOS)
    x = x0 + (avail_w - resized.width) // 2
    y = y0 + (avail_h - resized.height) // 2
    canvas.alpha_composite(resized, (x, y))


def main() -> None:
    font = ImageFont.load_default()
    board = Image.new("RGBA", (1500, 1120), (244, 246, 250, 255))
    draw = ImageDraw.Draw(board)

    draw.text((40, 24), "Pony Agent Icon Audit Board", fill=(24, 28, 34), font=font)
    draw.text((40, 48), "Read-image review for sidebar / tray / titlebar / taskbar sizes", fill=(92, 98, 108), font=font)

    assets = [
        ("Sidebar 96", ROOT / "src" / "assets" / "pony-brand-icon.png", (40, 100, 340, 400), (255, 255, 255, 255)),
        ("Taskbar 64", ICON_DIR / "taskbar-64.png", (380, 100, 680, 400), (30, 31, 34, 255)),
        ("32 PNG", ICON_DIR / "32x32.png", (720, 100, 1020, 400), (30, 31, 34, 255)),
        ("128 App", ICON_DIR / "128x128.png", (1060, 100, 1360, 400), (255, 255, 255, 255)),
        ("Tray 16", REVIEW_DIR / "tray-layers" / "tray-16.png", (40, 460, 340, 760), (30, 31, 34, 255)),
        ("Tray 20", REVIEW_DIR / "tray-layers" / "tray-20.png", (380, 460, 680, 760), (30, 31, 34, 255)),
        ("Tray 24", REVIEW_DIR / "tray-layers" / "tray-24.png", (720, 460, 1020, 760), (30, 31, 34, 255)),
        ("Tray 32", REVIEW_DIR / "tray-layers" / "tray-32.png", (1060, 460, 1360, 760), (30, 31, 34, 255)),
    ]

    for title, path, box, bg in assets:
        draw.rounded_rectangle(box, radius=20, fill=(232, 235, 240, 255))
        inner = (box[0] + 16, box[1] + 36, box[2] - 16, box[3] - 16)
        draw.text((box[0] + 16, box[1] + 12), title, fill=(24, 28, 34), font=font)
        paste_center(board, load(path), inner, bg=bg)

    notes_box = (40, 820, 1460, 1060)
    draw.rounded_rectangle(notes_box, radius=20, fill=(255, 255, 255, 255))
    notes = [
        "Audit focus:",
        "1. Tiny-size logo readability at 16/20/24px",
        "2. Warm background edge clarity on dark surfaces",
        "3. White inner negative space visibility after downscaling",
        "4. Whether logo occupies too little or too much area in small icons",
    ]
    y = 848
    for line in notes:
        draw.text((64, y), line, fill=(36, 40, 46), font=font)
        y += 28

    OUT.parent.mkdir(parents=True, exist_ok=True)
    board.save(OUT)
    print(f"Generated {OUT}")


if __name__ == "__main__":
    main()
