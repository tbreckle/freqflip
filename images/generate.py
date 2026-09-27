#!/usr/bin/env python3
"""Generates the FreqFlip pixel-art icons and logo (needs Pillow).

    python3 images/generate.py

Writes icon.png (512 px app icon), freqflip.ico (Windows, 16-256 px), tray.png (64 px tray
icon, 8-bit RGBA as the app expects) and logo.png (README banner) next to this script.
Everything is drawn on small pixel grids and scaled up with nearest neighbour, so the pixels
stay crisp, in the retro arcade style of B.L.A.S.T.
"""

import math
from pathlib import Path

from PIL import Image

OUT = Path(__file__).resolve().parent

# Arcade palette (the B.L.A.S.T. colors plus a CRT glow).
BLACK = (8, 8, 16, 255)
TILE = (18, 16, 34, 255)
RED = (255, 30, 40, 255)
DARK_RED = (150, 10, 24, 255)
YELLOW = (255, 214, 0, 255)
BLUE = (22, 130, 255, 255)
CYAN = (40, 230, 255, 255)
GREEN = (30, 220, 70, 255)
WHITE = (255, 255, 255, 255)
BEZEL = (206, 208, 222, 255)
BEZEL_DARK = (118, 120, 142, 255)
BEZEL_LIGHT = (246, 246, 255, 255)
SCREEN = (10, 18, 52, 255)
SCANLINE = (16, 28, 72, 255)
CLEAR = (0, 0, 0, 0)


class Grid:
    def __init__(self, w, h, fill=CLEAR):
        self.w, self.h = w, h
        self.img = Image.new("RGBA", (w, h), fill)

    def px(self, x, y, c):
        if 0 <= x < self.w and 0 <= y < self.h:
            self.img.putpixel((x, y), c)

    def rect(self, x0, y0, x1, y1, c):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                self.px(x, y, c)

    def round_rect(self, x0, y0, x1, y1, c, r=1):
        """Filled rectangle with rounded corners of radius r (pixel-art rounding)."""
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                dx = max(x0 + r - x, x - (x1 - r), 0)
                dy = max(y0 + r - y, y - (y1 - r), 0)
                if dx * dx + dy * dy <= r * r + r * 0.6:
                    self.px(x, y, c)

    def sprite(self, x0, y0, rows, colors):
        for y, row in enumerate(rows):
            for x, ch in enumerate(row):
                if ch in colors:
                    self.px(x0 + x, y0 + y, colors[ch])

    def scaled(self, size):
        return self.img.resize((size, size * self.h // self.w), Image.NEAREST)


def wave_points(x0, x1, cy, amp, period, square):
    """y per x for a square or sine wave."""
    pts = []
    for x in range(x0, x1 + 1):
        t = (x - x0) / period
        if square:
            y = cy - amp if int(t * 2) % 2 == 0 else cy + amp
        else:
            y = cy - round(amp * math.sin(t * 2 * math.pi))
        pts.append((x, y))
    return pts


def draw_wave(g, pts, c, square):
    prev = None
    for x, y in pts:
        g.px(x, y, c)
        if prev is not None and prev[1] != y:
            # Connect jumps with a vertical line so the wave stays continuous.
            lo, hi = sorted((prev[1], y))
            vx = x if square else prev[0]
            for yy in range(lo, hi + 1):
                g.px(vx, yy, c)
        prev = (x, y)


# Round red badge with two white arrows chasing each other: the "flip".
BADGE = [
    "..KKKKKKK..",
    ".KRRRRRRRK.",
    "KRRRRRRWRRK",
    "KRWWWWWWWRK",
    "KRRRRRRWRRK",
    "KRRRRRRRRRK",
    "KRRWRRRRRRK",
    "KRWWWWWWWRK",
    "KRRWRRRRRRK",
    ".KRRRRRRRK.",
    "..KKKKKKK..",
]


def draw_flip_badge(g, x, y):
    g.sprite(x, y, BADGE, {"K": BLACK, "R": RED, "W": WHITE})


def draw_monitor(g, x0, y0, x1, y1, amp, fast_period, thick=1):
    """CRT monitor on a stand with a wave that flips from fast (yellow) to slow (cyan)."""
    # Black outline, bezel with light top-left and dark bottom-right edges.
    g.round_rect(x0 - 1, y0 - 1, x1 + 1, y1 + 1, BLACK, 3)
    g.round_rect(x0, y0, x1, y1, BEZEL, 2)
    g.rect(x0 + 2, y0, x1 - 2, y0, BEZEL_LIGHT)
    g.rect(x0, y0 + 2, x0, y1 - 2, BEZEL_LIGHT)
    g.rect(x0 + 2, y1, x1 - 2, y1, BEZEL_DARK)
    g.rect(x1, y0 + 2, x1, y1 - 2, BEZEL_DARK)
    # Screen with scanlines.
    sx0, sy0, sx1, sy1 = x0 + 2, y0 + 2, x1 - 2, y1 - 3
    g.round_rect(sx0, sy0, sx1, sy1, SCREEN, 1)
    if thick == 1:
        for y in range(sy0 + 1, sy1, 2):
            g.rect(sx0 + 1, y, sx1 - 1, y, SCANLINE)
    # Power LED.
    g.rect(x1 - 3, y1 - 1, x1 - 4 + thick, y1 - 1, GREEN)
    # Stand.
    mid = (x0 + x1) // 2
    g.rect(mid - 3, y1 + 1, mid + 3, y1 + 2, BLACK)
    g.rect(mid - 2, y1 + 1, mid + 2, y1 + 1, BEZEL_DARK)
    g.round_rect(mid - 7, y1 + 2, mid + 7, y1 + 4, BLACK, 1)
    g.rect(mid - 6, y1 + 3, mid + 6, y1 + 3, BEZEL)

    cy = (sy0 + sy1) // 2
    split = (sx0 + sx1) // 2
    fast = wave_points(sx0 + 2, split, cy, amp, fast_period, True)
    slow = wave_points(split, sx1 - 2, cy, amp, sx1 - 2 - split + 1, False)
    for d in range(thick):
        draw_wave(g, [(x, y + d) for x, y in fast], YELLOW, True)
        draw_wave(g, [(x, y + d) for x, y in slow], CYAN, False)
    # Screen glare.
    g.sprite(sx0 + 1, sy0 + 1, ["WW", "W."], {"W": WHITE})


def app_icon():
    g = Grid(32, 32)
    # Rounded tile with the B.L.A.S.T. red rim and its tri-color stripes.
    g.round_rect(0, 0, 31, 31, RED, 7)
    g.round_rect(1, 1, 30, 30, TILE, 6)
    for i, c in enumerate((RED, YELLOW, BLUE)):
        g.rect(5, 27 + i, 26, 27 + i, c)
    draw_monitor(g, 3, 3, 26, 18, 3, 4)
    draw_flip_badge(g, 20, 16)
    return g


def tray_icon():
    # 32 px grid with 2 px lines, so it stays readable when the OS scales it to 16-24 px.
    g = Grid(32, 32)
    x0, y0, x1, y1 = 1, 2, 30, 23
    g.round_rect(x0 - 1, y0 - 1, x1 + 1, y1 + 1, BLACK, 4)
    g.round_rect(x0, y0, x1, y1, BEZEL, 3)
    g.rect(x0 + 3, y1 - 1, x1 - 3, y1, BEZEL_DARK)
    g.round_rect(x0 + 2, y0 + 2, x1 - 2, y1 - 3, SCREEN, 2)
    cy = 11
    fast = wave_points(5, 15, cy, 4, 5, True)
    slow = wave_points(15, 27, cy, 4, 13, False)
    for d in (0, 1):
        draw_wave(g, [(x, y + d) for x, y in fast], YELLOW, True)
        draw_wave(g, [(x + d, y) for x, y in fast], YELLOW, True)
        draw_wave(g, [(x, y + d) for x, y in slow], CYAN, False)
    g.rect(12, 24, 19, 26, BLACK)
    g.rect(13, 24, 18, 25, BEZEL_DARK)
    g.round_rect(5, 26, 26, 31, BLACK, 2)
    g.rect(6, 27, 25, 30, RED)
    g.rect(6, 29, 25, 30, DARK_RED)
    return g


# 5x7 pixel font for the logo lettering.
FONT = {
    "A": ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
    "B": ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
    "C": ["01110", "10001", "10000", "10000", "10000", "10001", "01110"],
    "D": ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
    "E": ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
    "F": ["11111", "10000", "10000", "11110", "10000", "10000", "10000"],
    "G": ["01110", "10001", "10000", "10111", "10001", "10001", "01111"],
    "H": ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
    "I": ["01110", "00100", "00100", "00100", "00100", "00100", "01110"],
    "L": ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
    "M": ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
    "N": ["10001", "11001", "10101", "10011", "10001", "10001", "10001"],
    "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    "P": ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
    "Q": ["01110", "10001", "10001", "10001", "10101", "10010", "01101"],
    "R": ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
    "S": ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
    "T": ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
    "U": ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
    "W": ["10001", "10001", "10001", "10101", "10101", "10101", "01010"],
    "Y": ["10001", "10001", "01010", "00100", "00100", "00100", "00100"],
    "Z": ["11111", "00001", "00010", "00100", "01000", "10000", "11111"],
    "0": ["01110", "10011", "10101", "10101", "11001", "10001", "01110"],
    "6": ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
    "·": ["00000", "00000", "00000", "00100", "00000", "00000", "00000"],
    "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
    ">": ["01000", "00100", "00010", "00001", "00010", "00100", "01000"],
    " ": ["00000"] * 7,
}


def text_width(text, scale, gap=1):
    return len(text) * (5 + gap) * scale - gap * scale


def draw_text(g, x, y, text, scale, fill, outline=None, shadow=None, gap=1):
    """Draws pixel text; each font pixel becomes a scale x scale block."""
    def blocks(ch):
        for row, bits in enumerate(FONT[ch]):
            for col, bit in enumerate(bits):
                if bit == "1":
                    yield col, row

    layers = []
    if shadow:
        layers.append((shadow, scale, scale, 0))
    if outline:
        layers.append((outline, 0, 0, max(1, scale // 3)))
    layers.append((fill, 0, 0, 0))
    for color, ox, oy, grow in layers:
        cx = x
        for ch in text:
            for col, row in blocks(ch):
                px, py = cx + col * scale + ox, y + row * scale + oy
                g.rect(px - grow, py - grow, px + scale - 1 + grow, py + scale - 1 + grow, color)
            cx += (5 + gap) * scale


def logo():
    # Drawn at 2x logical pixels on a 480x160 grid, scaled 4x to 1920x640.
    g = Grid(480, 160, BLACK)
    icon = app_icon().scaled(128)
    g.img.alpha_composite(icon, (16, 16))

    title = "FREQFLIP"
    draw_text(g, 170, 28, title, 6, YELLOW, outline=RED, shadow=DARK_RED)
    tagline = "REFRESH RATE FLIPPER"
    draw_text(g, 172, 90, tagline, 2, WHITE, gap=1)

    # Wave strip that flips from fast to slow, then the B.L.A.S.T. stripes.
    y = 124
    fast = wave_points(172, 300, y, 5, 10, True)
    slow = wave_points(300, 460, y, 5, 54, False)
    for dy in (0, 1):
        draw_wave(g, [(x, yy + dy) for x, yy in fast], YELLOW, True)
        draw_wave(g, [(x, yy + dy) for x, yy in slow], CYAN, False)
    for i, c in enumerate((RED, YELLOW, BLUE)):
        g.rect(16, 148 + i * 3, 463, 149 + i * 3, c)
    return g.img.resize((1920, 640), Image.NEAREST)


def main():
    icon = app_icon()
    icon.scaled(512).save(OUT / "icon.png")
    sizes = [16, 24, 32, 48, 64, 128, 256]
    big = icon.scaled(256)
    big.save(OUT / "freqflip.ico", sizes=[(s, s) for s in sizes])
    tray = tray_icon().scaled(64)
    tray.save(OUT / "tray.png")
    logo().save(OUT / "logo.png")
    # Small preview of the tray icon on light and dark panels, for the README.
    preview = Image.new("RGBA", (160, 80), (236, 236, 240, 255))
    preview.paste((40, 40, 48, 255), (80, 0, 160, 80))
    for size, x in ((16, 4), (24, 22), (32, 46)):
        t = tray_icon().img.resize((size, size), Image.LANCZOS)
        preview.alpha_composite(t, (x, 24))
        preview.alpha_composite(t, (x + 80, 24))
    preview.save(OUT / "tray_preview.png")
    print("written:", ", ".join(p.name for p in sorted(OUT.glob("*.[pi][nc][go]"))))


if __name__ == "__main__":
    main()
