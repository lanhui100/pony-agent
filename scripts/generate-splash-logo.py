#!/usr/bin/env python3
"""由官方母版生成 TUI 启动页品牌标的两份派生物（ADR 0085）。

单一真值源：`src-tauri/icons/icon-master-approved.png`
  ├─> `crates/pony-agent-core/src/bin/tui/splash_coverage.rs`（字符路径：覆盖率表）
  └─> `crates/pony-agent-core/assets/splash-logo.png`（图形协议路径：位图）

字符路径每格固定写一个半块字符：上半格取 `fg`、下半格取 `bg`，两者都是
`mix(CANVAS, gradient(t), coverage)` —— 覆盖率让边缘向画布色渐隐，这是终端里
唯一可用的抗锯齿手段。形状不再手绘，全部来自母版的面积重采样。

用法：
    python3 scripts/generate-splash-logo.py           # 重新生成两份产物
    python3 scripts/generate-splash-logo.py --check   # 只校验漂移（门禁用，非零退出）
"""
import sys

import numpy as np
from PIL import Image

MASTER = "src-tauri/icons/icon-master-approved.png"
OUT_COVERAGE = "crates/pony-agent-core/src/bin/tui/splash_coverage.rs"
OUT_PNG = "crates/pony-agent-core/assets/splash-logo.png"

# 画布底色，必须等于 `mod theme::CANVAS`：位图与周围字符格无缝衔接的前提。
CANVAS = (18, 16, 14)

# C2 克制暖色：暖金 → 赤陶 → 紫，与「单色为骨 + 单一强调色」令牌体系一致。
STOPS = [(0.00, (255, 196, 120)), (0.45, (233, 110, 84)), (1.00, (150, 92, 220))]

# 字符栅格档位：(名字, 列, 子像素行)。子像素行必须为偶数（一个半块字符 = 2 行）。
TIERS = [("RICH", 27, 30), ("COMPACT", 20, 22)]

# 母版 cream 底色 chroma ≈ 11，标身 chroma >= 25，取中间值把标身分离出来。
SEED_CHROMA = 21

CELL_PX = (8, 16)  # 一个字符格的设备像素（位图按此密度输出）
PNG_SS = 4         # 位图超采样倍率


# ── 母版 → 标记遮罩 ────────────────────────────────────────────────────────
def master_rgba() -> np.ndarray:
    return np.asarray(Image.open(MASTER).convert("RGBA")).astype(np.int32)


def mark_mask(rgba: np.ndarray) -> np.ndarray:
    rgb, alpha = rgba[..., :3], rgba[..., 3]
    chroma = rgb.max(-1) - rgb.min(-1)
    mask = (chroma > SEED_CHROMA) & (alpha > 200)
    ys, xs = np.where(mask)
    return mask[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


def mark_rgb(rgba: np.ndarray) -> np.ndarray:
    ys, xs = np.where((rgba[..., :3].max(-1) - rgba[..., :3].min(-1) > SEED_CHROMA) & (rgba[..., 3] > 200))
    return rgba[ys.min():ys.max() + 1, xs.min():xs.max() + 1, :3]


def coverage(mask: np.ndarray, cols: int, rows: int) -> np.ndarray:
    """面积重采样（box filter）：每个子像素取标身的覆盖比例 0..1。"""
    im = Image.fromarray((mask.astype(np.uint8) * 255))
    return np.asarray(im.resize((cols, rows), Image.BOX)).astype(np.float32) / 255.0


# ── 渐变（与 Rust 侧 `splash::gradient_rgb` 同口径：OKLab + smoothstep） ────
def srgb_to_linear(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def linear_to_srgb(v):
    v = max(0.0, min(1.0, v))
    s = v * 12.92 if v <= 0.0031308 else 1.055 * (v ** (1 / 2.4)) - 0.055
    return round(s * 255)


def rgb_to_oklab(rgb):
    r, g, b = [srgb_to_linear(c) for c in rgb]
    l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
    m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
    s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
    l_, m_, s_ = (np.cbrt(v) for v in (l, m, s))
    return [0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
            1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
            0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_]


def oklab_to_rgb(lab):
    L, a, b = lab
    l_ = L + 0.3963377774 * a + 0.2158037573 * b
    m_ = L - 0.1055613458 * a - 0.0638541728 * b
    s_ = L - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_ ** 3, m_ ** 3, s_ ** 3
    r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
    return [linear_to_srgb(v) for v in (r, g, bb)]


_OKLAB_STOPS = [(t, rgb_to_oklab(c)) for t, c in STOPS]


def gradient(t: float):
    t = max(0.0, min(1.0, float(t)))
    idx = 0
    while idx + 2 < len(STOPS) and t > STOPS[idx + 1][0]:
        idx += 1
    (t0, _), (t1, _) = STOPS[idx], STOPS[idx + 1]
    a, b = _OKLAB_STOPS[idx][1], _OKLAB_STOPS[idx + 1][1]
    k = max(0.0, min(1.0, (t - t0) / max(t1 - t0, 1e-6)))
    k = k * k * (3 - 2 * k)
    return oklab_to_rgb([a[i] + (b[i] - a[i]) * k for i in range(3)])


def diagonal_t(x: float, y: float, cols: int, rows: int) -> float:
    """与 Rust `splash::diagonal_t` 完全一致：0 = 暖金高光端，1 = 紫收尾端。"""
    return 0.4 * (x / max(cols - 1, 1)) + 0.6 * (y / max(rows - 1, 1))


# ── 产物 1：覆盖率表 ──────────────────────────────────────────────────────
def emit_coverage(mask: np.ndarray) -> str:
    out = [
        "// 由 `scripts/generate-splash-logo.py` 生成，禁止手改（`--check` 校验漂移）。",
        "// 数据来源：`src-tauri/icons/icon-master-approved.png`（折带标母版，宽高比 0.895）。",
        "// 每字节 = 该子像素被标身覆盖的比例（0..255）；字符路径据此向画布色渐隐做抗锯齿。",
        "",
    ]
    for name, cols, rows in TIERS:
        flat = np.clip(np.round(coverage(mask, cols, rows) * 255.0), 0, 255).astype(np.uint8).reshape(-1)
        out.append(f"pub const {name}_COLS: usize = {cols};")
        out.append(f"pub const {name}_ROWS: usize = {rows};")
        out.append(f"pub const {name}_COVERAGE: [u8; {flat.size}] = [")
        for i in range(0, flat.size, cols):
            out.append("    " + ", ".join(str(int(v)) for v in flat[i:i + cols]) + ",")
        out.append("];")
        out.append("")
    return "\n".join(out)


# ── 产物 2：图形协议位图 ──────────────────────────────────────────────────
def emit_png(mask: np.ndarray, rgb: np.ndarray) -> Image.Image:
    """按 RICH 档的字符矩形铺满，标身 contain 居中，底色为画布色（不透明）。"""
    _, cols, rows = TIERS[0]
    dev_w, dev_h = cols * CELL_PX[0], (rows // 2) * CELL_PX[1]
    ss_w, ss_h = dev_w * PNG_SS, dev_h * PNG_SS
    h, w = mask.shape
    scale = min(dev_w / w, dev_h / h)
    tw, th = max(1, round(w * scale)), max(1, round(h * scale))

    alpha = Image.fromarray((mask.astype(np.float32) * 255).astype(np.uint8)).resize(
        (tw, th), Image.BOX
    ).resize((tw * PNG_SS, th * PNG_SS), Image.BOX)
    body = Image.fromarray(rgb.astype(np.uint8)).resize((tw, th), Image.LANCZOS)

    # 每个像素的渐变色（按标记自身归一化坐标取样，与字符路径同口径）。
    xs = (np.arange(tw) + 0.5) / tw
    ys = (np.arange(th) + 0.5) / th
    grad = np.zeros((th, tw, 3), dtype=np.float32)
    for j, v in enumerate(ys):
        for i, u in enumerate(xs):
            grad[j, i] = gradient(0.4 * u + 0.6 * v)
    grad_img = Image.fromarray(grad.astype(np.uint8)).resize(
        (tw * PNG_SS, th * PNG_SS), Image.BILINEAR
    )

    canvas = Image.new("RGB", (ss_w, ss_h), CANVAS)
    canvas.paste(grad_img, ((ss_w - tw * PNG_SS) // 2, (ss_h - th * PNG_SS) // 2), alpha)
    del body  # 标记自带明暗，不再叠母版原图，避免与渐变双重着色
    return canvas


# ── 入口 ─────────────────────────────────────────────────────────────────
def build():
    rgba = master_rgba()
    mask = mark_mask(rgba)
    return emit_coverage(mask), emit_png(mask, mark_rgb(rgba))


def main() -> int:
    src, png = build()
    if "--check" in sys.argv:
        ok = True
        try:
            if open(OUT_COVERAGE, encoding="utf-8").read() != src:
                print(f"[drift] {OUT_COVERAGE} 与母版不一致：请运行 python3 scripts/generate-splash-logo.py")
                ok = False
        except FileNotFoundError:
            print(f"[drift] 缺少 {OUT_COVERAGE}")
            ok = False
        try:
            on_disk = np.asarray(Image.open(OUT_PNG).convert("RGB"))
            if not np.array_equal(on_disk, np.asarray(png)):
                print(f"[drift] {OUT_PNG} 与母版不一致：请运行 python3 scripts/generate-splash-logo.py")
                ok = False
        except FileNotFoundError:
            print(f"[drift] 缺少 {OUT_PNG}")
            ok = False
        print("splash-logo: 一致" if ok else "splash-logo: 漂移")
        return 0 if ok else 1

    with open(OUT_COVERAGE, "w", encoding="utf-8") as f:
        f.write(src)
    png.save(OUT_PNG)
    print(f"wrote {OUT_COVERAGE}  and  {OUT_PNG} {png.size}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
