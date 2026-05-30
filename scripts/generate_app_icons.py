from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
CANDIDATE_DIR = ROOT / "docs" / "design" / "icon-candidates"
MASTER_SIZE = 1024
UPSCALE = 4


def rounded_rect_mask(size: int, radius: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(mask)
    inset = size * 36 // 1024
    draw.rounded_rectangle((inset, inset, size - inset, size - inset), radius=radius, fill=255)
    return mask


def vertical_gradient(size: int, top: tuple[int, ...], bottom: tuple[int, ...]) -> Image.Image:
    image = Image.new("RGBA", (size, size))
    pixels = image.load()
    for y in range(size):
        t = y / (size - 1)
        color = tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(4))
        for x in range(size):
            pixels[x, y] = color
    return image


def tint_mask(mask: Image.Image, fill: tuple[int, ...]) -> Image.Image:
    image = Image.new("RGBA", mask.size, fill)
    image.putalpha(mask)
    return image


def add_glow(base: Image.Image, center: tuple[int, int], radius: int, color: tuple[int, ...]) -> None:
    glow = Image.new("RGBA", base.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(glow)
    x, y = center
    draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=color)
    glow = glow.filter(ImageFilter.GaussianBlur(radius // 2))
    base.alpha_composite(glow)


def draw_background(
    size: int,
    top: tuple[int, ...],
    bottom: tuple[int, ...],
    edge: tuple[int, ...],
    glow: tuple[int, ...],
) -> Image.Image:
    canvas = vertical_gradient(size, top, bottom)
    add_glow(canvas, (size * 770 // 1024, size * 210 // 1024), size * 180 // 1024, glow)

    border = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    border_draw = ImageDraw.Draw(border)
    inset = size * 36 // 1024
    border_draw.rounded_rectangle(
        (inset, inset, size - inset, size - inset),
        radius=size * 228 // 1024,
        outline=edge,
        width=max(2, size * 8 // 1024),
    )
    canvas.alpha_composite(border)

    mask = rounded_rect_mask(size, size * 228 // 1024)
    result = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    result.paste(canvas, mask=mask)
    return result


def draw_horse_head_mask(size: int, variant: str) -> Image.Image:
    scale = size / 1024
    layer = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(layer)

    if variant == "a":
        draw.ellipse((260 * scale, 240 * scale, 660 * scale, 620 * scale), fill=255)
        draw.rounded_rectangle((580 * scale, 372 * scale, 820 * scale, 560 * scale), radius=90 * scale, fill=255)
        draw.polygon([(396 * scale, 252 * scale), (490 * scale, 92 * scale), (566 * scale, 286 * scale)], fill=255)
        draw.polygon([(312 * scale, 310 * scale), (398 * scale, 176 * scale), (430 * scale, 350 * scale)], fill=255)
        draw.polygon(
            [(296 * scale, 528 * scale), (472 * scale, 450 * scale), (612 * scale, 572 * scale), (550 * scale, 846 * scale), (346 * scale, 846 * scale), (256 * scale, 652 * scale)],
            fill=255,
        )
        erase = ImageDraw.Draw(layer)
        erase.polygon([(288 * scale, 542 * scale), (464 * scale, 446 * scale), (542 * scale, 518 * scale), (366 * scale, 734 * scale)], fill=0)
        erase.polygon([(438 * scale, 842 * scale), (542 * scale, 686 * scale), (516 * scale, 848 * scale)], fill=0)
    elif variant == "b":
        draw.ellipse((282 * scale, 252 * scale, 640 * scale, 598 * scale), fill=255)
        draw.rounded_rectangle((562 * scale, 364 * scale, 796 * scale, 548 * scale), radius=88 * scale, fill=255)
        draw.polygon([(422 * scale, 242 * scale), (498 * scale, 108 * scale), (564 * scale, 280 * scale)], fill=255)
        draw.polygon([(340 * scale, 284 * scale), (412 * scale, 176 * scale), (442 * scale, 330 * scale)], fill=255)
        draw.rounded_rectangle((368 * scale, 558 * scale, 582 * scale, 844 * scale), radius=48 * scale, fill=255)
        erase = ImageDraw.Draw(layer)
        erase.polygon([(314 * scale, 548 * scale), (474 * scale, 464 * scale), (546 * scale, 534 * scale), (388 * scale, 742 * scale)], fill=0)
        erase.polygon([(456 * scale, 840 * scale), (548 * scale, 712 * scale), (526 * scale, 842 * scale)], fill=0)
    else:
        draw.ellipse((250 * scale, 236 * scale, 654 * scale, 624 * scale), fill=255)
        draw.rounded_rectangle((574 * scale, 384 * scale, 818 * scale, 566 * scale), radius=88 * scale, fill=255)
        draw.polygon([(404 * scale, 238 * scale), (492 * scale, 88 * scale), (564 * scale, 288 * scale)], fill=255)
        draw.polygon([(322 * scale, 296 * scale), (396 * scale, 180 * scale), (434 * scale, 350 * scale)], fill=255)
        draw.polygon(
            [(304 * scale, 548 * scale), (480 * scale, 456 * scale), (636 * scale, 586 * scale), (564 * scale, 850 * scale), (340 * scale, 850 * scale), (248 * scale, 666 * scale)],
            fill=255,
        )
        erase = ImageDraw.Draw(layer)
        erase.polygon([(290 * scale, 564 * scale), (468 * scale, 456 * scale), (558 * scale, 536 * scale), (366 * scale, 748 * scale)], fill=0)
        erase.polygon([(450 * scale, 846 * scale), (560 * scale, 704 * scale), (526 * scale, 850 * scale)], fill=0)

    return layer


def draw_accent(layer: Image.Image, variant: str, accent: tuple[int, ...], accent_soft: tuple[int, ...]) -> None:
    size = layer.size[0]
    draw = ImageDraw.Draw(layer)
    if variant == "a":
        width = max(10, size * 34 // 1024)
        box = (size * 454 // 1024, size * 168 // 1024, size * 898 // 1024, size * 612 // 1024)
        draw.arc(box, start=236, end=334, fill=accent_soft, width=width + max(4, size * 10 // 1024))
        draw.arc(box, start=240, end=332, fill=accent, width=width)
        r = size * 40 // 1024
        x = size * 804 // 1024
        y = size * 250 // 1024
        draw.ellipse((x - r, y - r, x + r, y + r), fill=accent)
    elif variant == "b":
        width = max(10, size * 30 // 1024)
        box = (size * 468 // 1024, size * 176 // 1024, size * 870 // 1024, size * 578 // 1024)
        draw.arc(box, start=246, end=330, fill=accent_soft, width=width + max(4, size * 10 // 1024))
        draw.arc(box, start=250, end=328, fill=accent, width=width)
        draw.rounded_rectangle((size * 718 // 1024, size * 212 // 1024, size * 824 // 1024, size * 270 // 1024), radius=size * 20 // 1024, fill=accent)
    else:
        draw.rounded_rectangle((size * 680 // 1024, size * 210 // 1024, size * 850 // 1024, size * 258 // 1024), radius=size * 24 // 1024, fill=accent)
        draw.rounded_rectangle((size * 760 // 1024, size * 154 // 1024, size * 810 // 1024, size * 308 // 1024), radius=size * 24 // 1024, fill=accent_soft)
        draw.ellipse((size * 736 // 1024, size * 188 // 1024, size * 834 // 1024, size * 286 // 1024), fill=accent)


def create_icon(spec: dict[str, object]) -> Image.Image:
    working_size = MASTER_SIZE * UPSCALE
    result = Image.new("RGBA", (working_size, working_size), (0, 0, 0, 0))
    result.alpha_composite(
        draw_background(
            working_size,
            spec["bg_top"],
            spec["bg_bottom"],
            spec["bg_edge"],
            spec["bg_glow"],
        )
    )

    accent_layer = Image.new("RGBA", result.size, (0, 0, 0, 0))
    draw_accent(accent_layer, spec["variant"], spec["accent"], spec["accent_soft"])
    result.alpha_composite(accent_layer)

    horse_mask = draw_horse_head_mask(working_size, spec["variant"])
    shadow = tint_mask(
        horse_mask.filter(ImageFilter.GaussianBlur(max(4, working_size // 130))),
        spec["shadow"],
    )
    shadow_layer = Image.new("RGBA", result.size, (0, 0, 0, 0))
    shadow_layer.alpha_composite(shadow, dest=(working_size // 220, working_size // 220))
    result.alpha_composite(shadow_layer)
    result.alpha_composite(tint_mask(horse_mask, spec["horse"]))

    details = Image.new("RGBA", result.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(details)
    draw.ellipse(
        (
            working_size * 664 // 1024,
            working_size * 400 // 1024,
            working_size * 704 // 1024,
            working_size * 440 // 1024,
        ),
        fill=spec["eye"],
    )
    result.alpha_composite(details)

    border = Image.new("RGBA", result.size, (0, 0, 0, 0))
    border_draw = ImageDraw.Draw(border)
    inset = working_size * 36 // 1024
    border_draw.rounded_rectangle(
        (inset, inset, working_size - inset, working_size - inset),
        radius=working_size * 228 // 1024,
        outline=spec["border"],
        width=max(2, working_size * 7 // 1024),
    )
    result.alpha_composite(border)

    mask = rounded_rect_mask(working_size, working_size * 228 // 1024)
    clipped = Image.new("RGBA", result.size, (0, 0, 0, 0))
    clipped.paste(result, mask=mask)
    return clipped.resize((MASTER_SIZE, MASTER_SIZE), Image.Resampling.LANCZOS)


def save_square(image: Image.Image, filename: str, size: int, target_dir: Path) -> None:
    image.resize((size, size), Image.Resampling.LANCZOS).save(target_dir / filename)


def export_icon_set(image: Image.Image, target_dir: Path) -> None:
    target_dir.mkdir(parents=True, exist_ok=True)
    image.save(target_dir / "icon.png")
    image.save(
        target_dir / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    exports = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "StoreLogo.png": 50,
    }
    for filename, size in exports.items():
        save_square(image, filename, size, target_dir)


def main() -> None:
    specs = {
        "candidate-a": {
            "variant": "a",
            "bg_top": (252, 247, 239, 255),
            "bg_bottom": (242, 233, 221, 255),
            "bg_edge": (224, 211, 195, 255),
            "bg_glow": (241, 202, 149, 80),
            "horse": (101, 75, 54, 255),
            "eye": (248, 244, 238, 255),
            "accent": (214, 135, 68, 255),
            "accent_soft": (232, 176, 113, 120),
            "shadow": (92, 69, 48, 18),
            "border": (255, 255, 255, 70),
        },
        "candidate-b": {
            "variant": "b",
            "bg_top": (248, 243, 234, 255),
            "bg_bottom": (236, 228, 214, 255),
            "bg_edge": (215, 202, 186, 255),
            "bg_glow": (255, 236, 198, 78),
            "horse": (78, 61, 48, 255),
            "eye": (248, 244, 238, 255),
            "accent": (201, 127, 62, 255),
            "accent_soft": (228, 173, 110, 120),
            "shadow": (80, 61, 45, 16),
            "border": (255, 255, 255, 72),
        },
        "candidate-c": {
            "variant": "c",
            "bg_top": (251, 248, 241, 255),
            "bg_bottom": (240, 234, 223, 255),
            "bg_edge": (223, 212, 198, 255),
            "bg_glow": (244, 211, 165, 74),
            "horse": (86, 66, 51, 255),
            "eye": (248, 244, 238, 255),
            "accent": (211, 133, 65, 255),
            "accent_soft": (237, 183, 119, 122),
            "shadow": (88, 67, 49, 16),
            "border": (255, 255, 255, 72),
        },
    }

    images: dict[str, Image.Image] = {}
    for name, spec in specs.items():
        image = create_icon(spec)
        images[name] = image
        export_icon_set(image, CANDIDATE_DIR / name)

    export_icon_set(images["candidate-b"], ICON_DIR)


if __name__ == "__main__":
    main()
