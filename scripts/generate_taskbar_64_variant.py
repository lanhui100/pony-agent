from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFilter, ImageOps


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
MASTER = ICON_DIR / "icon-master-approved.png"


def extract_logo(master: Image.Image) -> Image.Image:
    rgb = master.convert("RGB")
    bg = Image.new("RGB", rgb.size, (244, 240, 233))
    diff = ImageChops.difference(rgb, bg).convert("L")
    alpha = diff.filter(ImageFilter.GaussianBlur(0.7))
    alpha = ImageOps.autocontrast(alpha)
    alpha = alpha.point(lambda p: 0 if p < 18 else min(255, int((p - 18) * 1.55)))
    rgba = master.convert("RGBA")
    rgba.putalpha(alpha)
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("Unable to extract logo from master")
    return rgba.crop(bbox)


def render_taskbar_64(master: Image.Image) -> Image.Image:
    logo = extract_logo(master)
    canvas = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)
    draw.rounded_rectangle((2, 2, 61, 61), radius=15, fill=(244, 240, 233, 255))

    logo_w = 46
    ratio = logo_w / logo.size[0]
    logo_h = round(logo.size[1] * ratio)
    resized = logo.resize((logo_w, logo_h), Image.Resampling.LANCZOS)
    x = (64 - resized.size[0]) // 2
    y = (64 - resized.size[1]) // 2 + 1
    canvas.alpha_composite(resized, (x, y))

    canvas = ImageEnhance.Contrast(canvas).enhance(1.08)
    canvas = ImageEnhance.Sharpness(canvas).enhance(1.22)
    canvas = canvas.filter(ImageFilter.UnsharpMask(radius=0.45, percent=145, threshold=2))
    return canvas


def main() -> None:
    master = Image.open(MASTER).convert("RGBA")
    icon = render_taskbar_64(master)
    icon.save(ICON_DIR / "taskbar-64.png")
    print(f"Generated {ICON_DIR / 'taskbar-64.png'}")


if __name__ == "__main__":
    main()
