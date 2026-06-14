from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFilter, ImageOps


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
SOURCE = ICON_DIR / "review" / "logo_3_refined_master_1024.png"
OUT_DIR = ICON_DIR / "review" / "bg-candidates"

SIZE = 1024


def extract_logo(source: Image.Image) -> Image.Image:
    rgb = source.convert("RGB")
    diff = ImageChops.difference(rgb, Image.new("RGB", rgb.size, (255, 255, 255))).convert("L")
    alpha = diff.filter(ImageFilter.GaussianBlur(0.7))
    alpha = ImageOps.autocontrast(alpha)
    alpha = alpha.point(lambda p: 0 if p < 18 else min(255, int((p - 18) * 1.45)))

    rgba = source.convert("RGBA")
    rgba.putalpha(alpha)
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("Cannot extract logo from review master")
    logo = rgba.crop(bbox)
    logo = ImageEnhance.Color(logo).enhance(1.01)
    logo = ImageEnhance.Contrast(logo).enhance(1.01)
    return logo


def fit_logo(logo: Image.Image, width: int) -> Image.Image:
    ratio = width / logo.size[0]
    return logo.resize((width, round(logo.size[1] * ratio)), Image.Resampling.LANCZOS)


def build_candidate(logo: Image.Image) -> Image.Image:
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    card = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(card)

    # 单一暖白容器，避免边框和双层色差干扰主图标。
    draw.rounded_rectangle(
        (44, 44, SIZE - 44, SIZE - 44),
        radius=224,
        fill=(244, 240, 233, 255),
    )

    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    shadow_draw = ImageDraw.Draw(shadow)
    shadow_draw.rounded_rectangle(
        (60, 68, SIZE - 60, SIZE - 36),
        radius=216,
        fill=(72, 48, 20, 14),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(30))
    canvas.alpha_composite(shadow)
    canvas.alpha_composite(card)

    mark = fit_logo(logo, 668)
    x = (SIZE - mark.size[0]) // 2
    y = (SIZE - mark.size[1]) // 2 + 6

    logo_shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    alpha = mark.getchannel("A").point(lambda p: round(p * 0.07))
    shadow_mark = Image.new("RGBA", mark.size, (62, 38, 24, 0))
    shadow_mark.putalpha(alpha)
    logo_shadow.alpha_composite(shadow_mark, (x, y + 12))
    logo_shadow = logo_shadow.filter(ImageFilter.GaussianBlur(14))
    canvas.alpha_composite(logo_shadow)
    canvas.alpha_composite(mark, (x, y))
    return canvas


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    source = Image.open(SOURCE)
    logo = extract_logo(source)
    candidate = build_candidate(logo)
    candidate.save(OUT_DIR / "d-warm-clean-home-tone-v2.png")
    print(f"Generated {OUT_DIR / 'd-warm-clean-home-tone-v2.png'}")


if __name__ == "__main__":
    main()
