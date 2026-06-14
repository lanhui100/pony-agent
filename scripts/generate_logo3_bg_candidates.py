from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFilter, ImageOps


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
SOURCE = ICON_DIR / "review" / "logo_3_refined_master_1024.png"
OUT_DIR = ICON_DIR / "review" / "bg-candidates"

SIZE = 1024


def mix(a: tuple[int, int, int], b: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def radial_gradient(size: int, inner: tuple[int, int, int], outer: tuple[int, int, int]) -> Image.Image:
    small = 256
    img = Image.new("RGB", (small, small))
    px = img.load()
    cx, cy = small * 0.42, small * 0.34
    max_d = ((max(cx, small - cx) ** 2 + max(cy, small - cy) ** 2) ** 0.5)
    for y in range(small):
        for x in range(small):
            d = (((x - cx) ** 2 + (y - cy) ** 2) ** 0.5) / max_d
            px[x, y] = mix(inner, outer, min(1.0, max(0.0, d)))
    return img.resize((size, size), Image.Resampling.BICUBIC).convert("RGBA")


def rounded_mask(size: int, radius: int, inset: int = 0) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    draw.rounded_rectangle(
        (inset, inset, size - inset - 1, size - inset - 1),
        radius=radius,
        fill=255,
    )
    return mask


def extract_logo(source: Image.Image) -> Image.Image:
    rgb = source.convert("RGB")
    diff = ImageChops.difference(rgb, Image.new("RGB", rgb.size, (255, 255, 255))).convert("L")
    alpha = diff.filter(ImageFilter.GaussianBlur(0.8))
    alpha = ImageOps.autocontrast(alpha)
    alpha = alpha.point(lambda p: 0 if p < 16 else min(255, int((p - 16) * 1.55)))

    # Keep the original logo colors and use a softened matte. This is for visual
    # candidates only; a final transparent master should be reviewed separately.
    rgba = source.convert("RGBA")
    rgba.putalpha(alpha)
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("Cannot extract logo from source")
    logo = rgba.crop(bbox)
    logo = ImageEnhance.Color(logo).enhance(1.03)
    logo = ImageEnhance.Contrast(logo).enhance(1.02)
    return logo


def fit_logo(logo: Image.Image, width: int) -> Image.Image:
    ratio = width / logo.size[0]
    return logo.resize((width, round(logo.size[1] * ratio)), Image.Resampling.LANCZOS)


def add_shadow(base: Image.Image, logo: Image.Image, xy: tuple[int, int], blur: int, offset: tuple[int, int], opacity: int) -> None:
    shadow = Image.new("RGBA", base.size, (0, 0, 0, 0))
    shadow_logo = Image.new("RGBA", logo.size, (0, 0, 0, 0))
    shadow_logo.putalpha(logo.getchannel("A").point(lambda p: min(opacity, p)))
    shadow.alpha_composite(shadow_logo, (xy[0] + offset[0], xy[1] + offset[1]))
    shadow = shadow.filter(ImageFilter.GaussianBlur(blur))
    base.alpha_composite(shadow)


def render_clean(logo: Image.Image) -> Image.Image:
    bg = radial_gradient(SIZE, (236, 255, 245), (169, 230, 205))
    mask = rounded_mask(SIZE, 218)
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    canvas.alpha_composite(bg)
    canvas.putalpha(mask)

    draw = ImageDraw.Draw(canvas)
    draw.rounded_rectangle((28, 28, 995, 995), radius=205, outline=(255, 255, 255, 150), width=5)
    mark = fit_logo(logo, 740)
    xy = ((SIZE - mark.size[0]) // 2, (SIZE - mark.size[1]) // 2 + 8)
    add_shadow(canvas, mark, xy, blur=18, offset=(0, 18), opacity=54)
    canvas.alpha_composite(mark, xy)
    return canvas


def render_soft_3d(logo: Image.Image) -> Image.Image:
    bg = radial_gradient(SIZE, (245, 255, 250), (112, 215, 178))
    mask = rounded_mask(SIZE, 230)
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    canvas.alpha_composite(bg)
    canvas.putalpha(mask)

    shine = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(shine)
    draw.ellipse((-120, -180, 820, 560), fill=(255, 255, 255, 88))
    draw.rounded_rectangle((38, 38, 985, 985), radius=212, outline=(255, 255, 255, 178), width=6)
    draw.rounded_rectangle((64, 70, 960, 958), radius=196, outline=(33, 128, 96, 34), width=4)
    canvas.alpha_composite(shine)

    mark = fit_logo(logo, 700)
    xy = ((SIZE - mark.size[0]) // 2, (SIZE - mark.size[1]) // 2 + 12)
    add_shadow(canvas, mark, xy, blur=28, offset=(0, 28), opacity=82)
    add_shadow(canvas, mark, xy, blur=5, offset=(0, 4), opacity=36)
    canvas.alpha_composite(mark, xy)
    return canvas


def render_dark_tech(logo: Image.Image) -> Image.Image:
    bg = radial_gradient(SIZE, (23, 79, 72), (5, 16, 32))
    mask = rounded_mask(SIZE, 218)
    canvas = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    canvas.alpha_composite(bg)
    canvas.putalpha(mask)

    draw = ImageDraw.Draw(canvas)
    for i, color in [(0, (102, 255, 205, 44)), (42, (255, 255, 255, 26)), (84, (102, 255, 205, 18))]:
        draw.arc((120 + i, 122 + i, 910 - i, 904 - i), 205, 338, fill=color, width=4)
    draw.rounded_rectangle((34, 34, 989, 989), radius=206, outline=(118, 255, 210, 90), width=4)

    mark = fit_logo(logo, 710)
    xy = ((SIZE - mark.size[0]) // 2, (SIZE - mark.size[1]) // 2 + 10)
    add_shadow(canvas, mark, xy, blur=34, offset=(0, 16), opacity=125)
    glow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    glow_logo = Image.new("RGBA", mark.size, (255, 255, 255, 0))
    glow_logo.putalpha(mark.getchannel("A").point(lambda p: round(p * 0.18)))
    glow.alpha_composite(glow_logo, xy)
    glow = glow.filter(ImageFilter.GaussianBlur(18))
    canvas.alpha_composite(glow)
    canvas.alpha_composite(mark, xy)
    return canvas


def make_contact_sheet(images: list[tuple[str, Image.Image]]) -> Image.Image:
    tile = 360
    gap = 36
    title_h = 56
    canvas = Image.new("RGB", (gap + len(images) * (tile + gap), tile + title_h + gap), (244, 246, 250))
    draw = ImageDraw.Draw(canvas)
    for idx, (title, image) in enumerate(images):
        x = gap + idx * (tile + gap)
        draw.text((x, 20), title, fill=(30, 36, 44))
        canvas.paste(image.convert("RGB").resize((tile, tile), Image.Resampling.LANCZOS), (x, title_h))
    return canvas


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    source = Image.open(SOURCE)
    logo = extract_logo(source)

    candidates = [
        ("A clean mint", render_clean(logo)),
        ("B soft 3D", render_soft_3d(logo)),
        ("C dark tech", render_dark_tech(logo)),
    ]

    for title, image in candidates:
        slug = title.lower().replace(" ", "-")
        image.save(OUT_DIR / f"{slug}.png")

    make_contact_sheet(candidates).save(OUT_DIR / "contact-sheet.png")
    print(f"Generated background candidates in {OUT_DIR}")


if __name__ == "__main__":
    main()
