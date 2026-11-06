#!/usr/bin/env python3
"""Generate the TUI splash logo PNG shown through terminal image protocols.

The 20x16 pixel-art mask is traced as the union of pixel squares, its contour is
simplified (Douglas-Peucker) into straight edges, then rasterized with 4x
supersampling and filled with the splash gradient (OKLab interpolation) on a
canvas-colored background. Run this after changing the gradient stops or mask:

    python3 scripts/generate-splash-logo.py
"""
import math
import numpy as np
from PIL import Image

MASK = [
    ".....####...........",
    "....######..........",
    "...########.........",
    "..###..#####........",
    "..##....####........",
    ".###.....###........",
    ".##.......###.......",
    "###.......####......",
    "......####.......###",
    ".......###.......##.",
    "........###.....###.",
    "........####....##..",
    "........#####..###..",
    ".........########...",
    "..........######....",
    "...........####.....",
]

STOPS = [(0.00, (255, 214, 103)), (0.12, (255, 165, 65)), (0.28, (255, 104, 83)),
         (0.44, (242, 62, 161)), (0.60, (223, 59, 209)), (0.76, (182, 103, 255)),
         (0.90, (132, 114, 254)), (1.00, (91, 91, 240))]
T_MIN, T_SPAN = 2 / 19, 15 / 19


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
    l_, m_, s_ = (math.copysign(abs(v) ** (1 / 3), v) for v in (l, m, s))
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
    b = 2.0 * 0 + (-0.0041960863) * l - 0.7034186147 * m + 1.7076147010 * s
    return tuple(linear_to_srgb(v) for v in (r, g, b))


def gradient(t):
    t = max(0.0, min(1.0, t))
    for (t0, c0), (t1, c1) in zip(STOPS, STOPS[1:]):
        if t0 <= t <= t1:
            k = (t - t0) / (t1 - t0)
            k = k * k * (3 - 2 * k)
            a, b = rgb_to_oklab(c0), rgb_to_oklab(c1)
            return oklab_to_rgb([a[i] + (b[i] - a[i]) * k for i in range(3)])
    return STOPS[-1][1]


def trace_boundary(mask):
    edges = {}
    def add(p, q):
        edges.setdefault(p, []).append(q)
    ys, xs = np.where(mask)
    h, w = mask.shape
    for y, x in zip(ys.tolist(), xs.tolist()):
        if y == 0 or not mask[y - 1, x]:
            add((x, y), (x + 1, y))
        if x == w - 1 or not mask[y, x + 1]:
            add((x + 1, y), (x + 1, y + 1))
        if y == h - 1 or not mask[y + 1, x]:
            add((x + 1, y + 1), (x, y + 1))
        if x == 0 or not mask[y, x - 1]:
            add((x, y + 1), (x, y))
    loops = []
    while edges:
        start = next(iter(edges))
        loop = [start]
        cur = start
        while True:
            nxts = edges.get(cur)
            if not nxts:
                break
            nxt = nxts.pop()
            if not nxts:
                del edges[cur]
            loop.append(nxt)
            cur = nxt
            if cur == start:
                break
        loops.append(loop)
    return loops


def chaikin(points, iterations=2):
    pts = points
    for _ in range(iterations):
        new = []
        n = len(pts)
        for i in range(n):
            p, q = pts[i], pts[(i + 1) % n]
            new.append((0.75 * p[0] + 0.25 * q[0], 0.75 * p[1] + 0.25 * q[1]))
            new.append((0.25 * p[0] + 0.75 * q[0], 0.25 * p[1] + 0.75 * q[1]))
        pts = new
    return pts


def inside(polys, x, y):
    hit = False
    for poly in polys:
        n = len(poly)
        for i in range(n):
            x0, y0 = poly[i]
            x1, y1 = poly[(i + 1) % n]
            if (y0 > y) != (y1 > y):
                xin = (x1 - x0) * (y - y0) / (y1 - y0) + x0
                if x < xin:
                    hit = not hit
    return hit


def rdp(points, eps=0.9):
    if len(points) < 3:
        return points
    (x0, y0), (x1, y1) = points[0], points[-1]
    dmax, idx = 0.0, 0
    for i in range(1, len(points) - 1):
        x, y = points[i]
        num = abs((y1 - y0) * x - (x1 - x0) * y + x1 * y0 - y1 * x0)
        den = ((y1 - y0) ** 2 + (x1 - x0) ** 2) ** 0.5 or 1.0
        d = num / den
        if d > dmax:
            dmax, idx = d, i
    if dmax > eps:
        return rdp(points[:idx + 1], eps)[:-1] + rdp(points[idx:], eps)
    return [points[0], points[-1]]


def simplify_loop(loop, eps=0.9):
    """Douglas-Peucker for a closed loop: split at the farthest point, simplify halves."""
    if len(loop) < 4:
        return loop
    x0, y0 = loop[0]
    far_idx = max(range(len(loop)), key=lambda i: (loop[i][0] - x0) ** 2 + (loop[i][1] - y0) ** 2)
    half_a = loop[: far_idx + 1]
    half_b = loop[far_idx:] + [loop[0]]
    return rdp(half_a, eps)[:-1] + rdp(half_b, eps)[:-1]


CANVAS = (18, 16, 14)


def render(width, height, ss=4):
    mask = np.array([[ch == '#' for ch in row] for row in MASK])
    loops = [simplify_loop(l, 0.9) for l in trace_boundary(mask)]
    scale_x, scale_y = width / 20.0, height / 16.0
    polys = [[(px * scale_x, py * scale_y) for px, py in loop] for loop in loops]
    img = Image.new("RGB", (width, height), CANVAS)
    px = img.load()
    for y in range(height):
        for x in range(width):
            hit = 0
            for sy in range(ss):
                for sx in range(ss):
                    xx = x + (sx + 0.5) / ss
                    yy = y + (sy + 0.5) / ss
                    if inside(polys, xx, yy):
                        hit += 1
            alpha = hit / (ss * ss)
            if alpha <= 0:
                continue
            t = (0.4 * (x + 0.5) / width + 0.6 * (y + 0.5) / height - T_MIN) / T_SPAN
            r, g, b = gradient(t)
            px[x, y] = (
                round(r * alpha + CANVAS[0] * (1 - alpha)),
                round(g * alpha + CANVAS[1] * (1 - alpha)),
                round(b * alpha + CANVAS[2] * (1 - alpha)),
            )
    return img


if __name__ == "__main__":
    img = render(616, 484)
    img.save("/tmp/splash-verify/splash-logo.png")
    print("saved", img.size)
