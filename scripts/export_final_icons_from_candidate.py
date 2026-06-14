from __future__ import annotations

from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
SOURCE = ICON_DIR / "review" / "bg-candidates" / "d-warm-clean-home-tone-v2.png"

EXPORT_SIZES = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "256x256.png": 256,
    "512x512.png": 512,
    "icon.png": 512,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "StoreLogo.png": 50,
}


def export_pngs(master: Image.Image) -> None:
    master.save(ICON_DIR / "icon-master-approved.png")
    for filename, size in EXPORT_SIZES.items():
        resized = master.resize((size, size), Image.Resampling.LANCZOS)
        resized.save(ICON_DIR / filename)


def export_ico(master: Image.Image) -> None:
    master.save(
        ICON_DIR / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )


def main() -> None:
    if not SOURCE.exists():
        raise FileNotFoundError(f"Approved icon candidate not found: {SOURCE}")
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    master = Image.open(SOURCE).convert("RGBA")
    export_pngs(master)
    export_ico(master)
    print(f"Exported final icon set from {SOURCE}")


if __name__ == "__main__":
    main()
