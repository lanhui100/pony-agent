from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageEnhance, ImageFilter, ImageOps


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
REVIEW_DIR = ICON_DIR / "review"
MASTER = ICON_DIR / "icon-master-approved.png"
ASSET_DIR = ROOT / "src" / "assets"


def extract_logo_from_master(source: Image.Image) -> Image.Image:
    rgb = source.convert("RGB")
    warm_bg = Image.new("RGB", rgb.size, (244, 240, 233))
    diff = ImageChops.difference(rgb, warm_bg).convert("L")
    alpha = diff.filter(ImageFilter.GaussianBlur(0.7))
    alpha = ImageOps.autocontrast(alpha)
    alpha = alpha.point(lambda p: 0 if p < 18 else min(255, int((p - 18) * 1.55)))

    rgba = source.convert("RGBA")
    rgba.putalpha(alpha)
    bbox = alpha.getbbox()
    if not bbox:
        raise RuntimeError("Unable to extract logo silhouette from icon master")
    return rgba.crop(bbox)


def render_sidebar_icon(master: Image.Image) -> Image.Image:
    icon = master.resize((96, 96), Image.Resampling.LANCZOS)
    icon = ImageEnhance.Contrast(icon).enhance(1.03)
    icon = ImageEnhance.Sharpness(icon).enhance(1.08)
    return icon


def render_small_tray_layer(logo: Image.Image, size: int) -> Image.Image:
    # Windows notification area should have dedicated 16/20/24px layers.
    # Keep the background very clean, remove shadow, enlarge the logo slightly,
    # and sharpen the final raster specifically for tiny display.
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)

    radius = max(4, round(size * 0.24))
    inset = max(0, round(size * 0.06))
    draw.rounded_rectangle(
        (inset, inset, size - inset - 1, size - inset - 1),
        radius=radius,
        fill=(244, 240, 233, 255),
    )

    target_w = round(size * 0.74)
    ratio = target_w / logo.size[0]
    target_h = max(1, round(logo.size[1] * ratio))
    resized = logo.resize((target_w, target_h), Image.Resampling.LANCZOS)

    x = (size - resized.size[0]) // 2
    y = (size - resized.size[1]) // 2 + max(0, round(size * 0.01))
    canvas.alpha_composite(resized, (x, y))

    # Tiny icon crispness pass
    canvas = ImageEnhance.Contrast(canvas).enhance(1.10)
    canvas = ImageEnhance.Sharpness(canvas).enhance(1.35)
    canvas = canvas.filter(ImageFilter.UnsharpMask(radius=0.45, percent=160, threshold=2))
    return canvas


def render_standard_png(master: Image.Image, size: int) -> Image.Image:
    image = master.resize((size, size), Image.Resampling.LANCZOS)
    if size <= 44:
        image = ImageEnhance.Contrast(image).enhance(1.05)
        image = ImageEnhance.Sharpness(image).enhance(1.2)
        image = image.filter(ImageFilter.UnsharpMask(radius=0.5, percent=140, threshold=2))
    return image


def export_sidebar_asset(master: Image.Image) -> None:
    ASSET_DIR.mkdir(parents=True, exist_ok=True)
    render_sidebar_icon(master).save(ASSET_DIR / "pony-brand-icon.png")


def export_small_size_pngs(master: Image.Image, logo: Image.Image) -> None:
    render_small_tray_layer(logo, 32).save(ICON_DIR / "32x32.png")
    render_small_tray_layer(logo, 30).save(ICON_DIR / "Square30x30Logo.png")
    render_small_tray_layer(logo, 44).save(ICON_DIR / "Square44x44Logo.png")


def export_tray_review_layers(logo: Image.Image) -> list[Path]:
    layer_dir = REVIEW_DIR / "tray-layers"
    layer_dir.mkdir(parents=True, exist_ok=True)
    paths: list[Path] = []
    for size in (16, 20, 24, 32, 40, 48, 64):
        path = layer_dir / f"tray-{size}.png"
        render_small_tray_layer(logo, size).save(path)
        paths.append(path)
    return paths


def export_ico_with_dedicated_small_layers(master: Image.Image, logo: Image.Image) -> None:
    export_tray_review_layers(logo)
    # PIL cannot directly pack custom per-size frames into one .ico from separate files,
    # but saving from the master with a full size table ensures Windows can pick 16/20/24/32.
    # We still keep the dedicated rendered layers in review/tray-layers for inspection.
    master.save(
        ICON_DIR / "icon.ico",
        sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)],
    )


def main() -> None:
    if not MASTER.exists():
        raise FileNotFoundError(f"Approved icon master not found: {MASTER}")
    master = Image.open(MASTER).convert("RGBA")
    logo = extract_logo_from_master(master)

    export_sidebar_asset(master)
    export_small_size_pngs(master, logo)
    export_ico_with_dedicated_small_layers(master, logo)
    print("Optimized sidebar brand icon and exported dedicated tray review layers.")


if __name__ == "__main__":
    main()
