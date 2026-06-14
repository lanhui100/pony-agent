from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageEnhance, ImageFilter, ImageFont


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
SOURCE = ICON_DIR / "logo_3.png"
REVIEW_DIR = ICON_DIR / "review"


def resize_logo(source: Image.Image, size: int) -> Image.Image:
    # Conservative refinement: keep the original canvas and geometry intact.
    image = source.convert("RGB")
    image = image.filter(ImageFilter.MedianFilter(size=3))
    image = ImageEnhance.Color(image).enhance(1.02)
    image = ImageEnhance.Contrast(image).enhance(1.025)
    image = image.resize((size, size), Image.Resampling.LANCZOS)
    image = image.filter(ImageFilter.UnsharpMask(radius=0.8, percent=85, threshold=4))
    return image


def make_compare(original: Image.Image, refined: Image.Image) -> Image.Image:
    tile = 512
    gap = 40
    top = 72
    bottom = 32
    width = tile * 2 + gap * 3
    height = tile + top + bottom

    canvas = Image.new("RGB", (width, height), (246, 247, 250))
    draw = ImageDraw.Draw(canvas)
    font = ImageFont.load_default()

    original_tile = original.convert("RGB").resize((tile, tile), Image.Resampling.LANCZOS)
    refined_tile = refined.resize((tile, tile), Image.Resampling.LANCZOS)

    x1 = gap
    x2 = gap * 2 + tile
    y = top
    canvas.paste(original_tile, (x1, y))
    canvas.paste(refined_tile, (x2, y))

    draw.text((x1, 28), "logo_3 original", fill=(32, 36, 44), font=font)
    draw.text((x2, 28), "refined review - structure locked", fill=(32, 36, 44), font=font)
    return canvas


def main() -> None:
    REVIEW_DIR.mkdir(parents=True, exist_ok=True)
    original = Image.open(SOURCE)

    refined_1024 = resize_logo(original, 1024)
    refined_512 = resize_logo(original, 512)
    refined_128 = resize_logo(original, 128)
    refined_32 = resize_logo(original, 32)

    refined_1024.save(REVIEW_DIR / "logo_3_refined_master_1024.png")
    refined_512.save(REVIEW_DIR / "logo_3_refined_512.png")
    refined_128.save(REVIEW_DIR / "logo_3_refined_128.png")
    refined_32.save(REVIEW_DIR / "logo_3_refined_32.png")
    make_compare(original, refined_1024).save(REVIEW_DIR / "logo_3_compare.png")

    print(f"Review assets generated in {REVIEW_DIR}")


if __name__ == "__main__":
    main()
