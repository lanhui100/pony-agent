#!/usr/bin/env python3
"""由官方母版生成 TUI 启动页品牌标的两份派生物（ADR 0098，取代 ADR 0085）。

单一真值源：`src-tauri/icons/icon-master-approved.png`
  ├─> `crates/pony-agent-core/src/bin/tui/splash_coverage.rs`（字符路径：覆盖率表 + 母版真色表）
  └─> `crates/pony-agent-core/assets/splash-logo.png`（图形协议路径：位图）

两条路径共用同一份几何与配色：形状来自母版标身的面积重采样，颜色直接取母版 RGB
（不再自造渐变），因此 `PONY_TUI_SPLASH_IMAGE_PROTOCOL` 开关前后观感一致。

字符路径每格固定写一个半块字符：上半格取 `fg`、下半格取 `bg`，两者都是
`mix(CANVAS, master_rgb, coverage)` —— 覆盖率让边缘向画布色渐隐，这是终端里
唯一可用的抗锯齿手段。

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

# 字符栅格档位：(名字, 列, 子像素行)。子像素行必须为偶数（一个半块字符 = 2 行）。
# ADR 0105：两档按原尺寸 1/3 收敛（Rich 26×30→9×10、Compact 20×22→7×8），
# 仍取母版宽高比（0.895）附近的整数栅格：9 列 × 5 行 = 72×80 设备像素，比例 0.900。
# Rich 9 列 = 图形协议位图的字符矩形。
TIERS = [("RICH", 9, 10), ("COMPACT", 7, 8)]

# 母版 cream 底色 chroma ≈ 11，标身 chroma >= 25，取中间值把标身分离出来。
SEED_CHROMA = 21

CELL_PX = (8, 16)  # 一个字符格的设备像素（位图按此密度输出）
PNG_SS = 4         # 位图超采样倍率


# ── 母版 → 标记遮罩 ────────────────────────────────────────────────────────
def master_rgba() -> np.ndarray:
    return np.asarray(Image.open(MASTER).convert("RGBA")).astype(np.int32)


def mark_bbox(rgba: np.ndarray) -> tuple:
    rgb, alpha = rgba[..., :3], rgba[..., 3]
    chroma = rgb.max(-1) - rgb.min(-1)
    ys, xs = np.where((chroma > SEED_CHROMA) & (alpha > 200))
    return ys.min(), ys.max() + 1, xs.min(), xs.max() + 1


def mark_mask(rgba: np.ndarray) -> np.ndarray:
    y0, y1, x0, x1 = mark_bbox(rgba)
    rgb = rgba[..., :3]
    mask = (rgb.max(-1) - rgb.min(-1) > SEED_CHROMA) & (rgba[..., 3] > 200)
    return mask[y0:y1, x0:x1]


def mark_rgb(rgba: np.ndarray) -> np.ndarray:
    y0, y1, x0, x1 = mark_bbox(rgba)
    return rgba[y0:y1, x0:x1, :3]


# ── 面积重采样（与字符栅格同口径） ─────────────────────────────────────────
def area_weights(n: int, m: int) -> np.ndarray:
    """(m, n) 权重矩阵：把 n 个母版像素按面积摊到 m 个子像素。"""
    weights = np.zeros((m, n), dtype=np.float64)
    edges = np.linspace(0.0, float(n), m + 1)
    for i in range(m):
        lo, hi = edges[i], edges[i + 1]
        for j in range(int(np.floor(lo)), min(int(np.ceil(hi)), n)):
            overlap = min(hi, j + 1.0) - max(lo, float(j))
            if overlap > 0.0:
                weights[i, j] = overlap / (hi - lo)
    return weights


def resample(plane: np.ndarray, cols: int, rows: int) -> np.ndarray:
    """把 (h, w) 或 (h, w, c) 的母版平面按面积重采样到 (rows, cols) / (rows, cols, c)。"""
    wy, wx = area_weights(plane.shape[0], rows), area_weights(plane.shape[1], cols)
    return np.einsum("ij,jkc,lk->ilc", wy, plane, wx) if plane.ndim == 3 else wy @ plane @ wx.T


def coverage(mask: np.ndarray, cols: int, rows: int) -> np.ndarray:
    """每个子像素被标身覆盖的比例 0..1。"""
    return resample(mask.astype(np.float64), cols, rows)


def subpixel_rgb(rgb: np.ndarray, mask: np.ndarray, cols: int, rows: int) -> np.ndarray:
    """每个子像素的母版真色：只对遮罩内的像素做加权平均，避免 cream 底色渗色。"""
    weighted = rgb.astype(np.float64) * mask[..., None].astype(np.float64)
    sums = resample(weighted, cols, rows)
    weight = coverage(mask, cols, rows)[..., None]
    return np.clip(np.round(sums / np.maximum(weight, 1e-6)), 0, 255)


# ── 产物 1：覆盖率表 + 母版真色表 ─────────────────────────────────────────
def emit_tables(rgba: np.ndarray) -> str:
    mask, rgb = mark_mask(rgba), mark_rgb(rgba)
    out = [
        "// 由 `scripts/generate-splash-logo.py` 生成，禁止手改（`--check` 校验漂移）。",
        "// 数据来源：`src-tauri/icons/icon-master-approved.png`（折带标母版，宽高比 0.895）。",
        "// COVERAGE 每字节 = 该子像素被标身覆盖的比例（0..255）；字符路径据此向画布色渐隐做抗锯齿。",
        "// RGB 每三字节 = 该子像素的母版真色（遮罩内加权平均），字符路径直接取用，不再自造渐变。",
        "",
    ]
    for name, cols, rows in TIERS:
        cov = np.clip(np.round(coverage(mask, cols, rows) * 255.0), 0, 255).astype(np.uint8)
        colour = subpixel_rgb(rgb, mask, cols, rows).astype(np.uint8)
        # 覆盖率为 0 的子像素不参与绘制，真色归零：避免「空白格却带彩度」的错位歧义，
        # 也让漂移检查能把「表错位」和「数值噪声」区分开。
        colour[cov == 0] = 0
        out.append(f"pub const {name}_COLS: usize = {cols};")
        out.append(f"pub const {name}_ROWS: usize = {rows};")
        out.append(f"pub const {name}_COVERAGE: [u8; {cov.size}] = [")
        flat_cov = cov.reshape(-1)
        for i in range(0, flat_cov.size, cols):
            out.append("    " + ", ".join(str(int(v)) for v in flat_cov[i:i + cols]) + ",")
        out.append("];")
        out.append(f"pub const {name}_RGB: [u8; {colour.size}] = [")
        flat_rgb = colour.reshape(-1)
        for i in range(0, flat_rgb.size, cols * 3):
            out.append("    " + ", ".join(str(int(v)) for v in flat_rgb[i:i + cols * 3]) + ",")
        out.append("];")
        out.append("")
    return "\n".join(out)


# ── 产物 2：图形协议位图 ──────────────────────────────────────────────────
def emit_png(rgba: np.ndarray) -> Image.Image:
    """按 Rich 档字符矩形铺满，标身取母版真色、宽度占满、垂直居中，底色为画布色（不透明）。"""
    mask, rgb = mark_mask(rgba), mark_rgb(rgba)
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
    body = body.resize((tw * PNG_SS, th * PNG_SS), Image.LANCZOS)

    canvas = Image.new("RGB", (ss_w, ss_h), CANVAS)
    canvas.paste(body, ((ss_w - tw * PNG_SS) // 2, (ss_h - th * PNG_SS) // 2), alpha)
    return canvas


# ── 入口 ─────────────────────────────────────────────────────────────────
def build():
    rgba = master_rgba()
    return emit_tables(rgba), emit_png(rgba)


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
            current = np.asarray(Image.open(OUT_PNG).convert("RGB"))
            if current.shape != (png.size[1], png.size[0], 3) or not np.array_equal(
                current, np.asarray(png)
            ):
                print(f"[drift] {OUT_PNG} 与母版不一致：请运行 python3 scripts/generate-splash-logo.py")
                ok = False
        except FileNotFoundError:
            print(f"[drift] 缺少 {OUT_PNG}")
            ok = False
        print("splash-logo: 一致" if ok else "splash-logo: 漂移")
        return 0 if ok else 1

    with open(OUT_COVERAGE, "w", encoding="utf-8") as handle:
        handle.write(src)
    png.save(OUT_PNG)
    print(f"wrote {OUT_COVERAGE} ({len(src)} bytes)")
    print(f"wrote {OUT_PNG} {png.size}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
