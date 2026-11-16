#!/usr/bin/env python3
"""Render a `tmux capture-pane -e -N -p` dump to a PNG that looks like a terminal window.

Usage: ansi2png.py CAPTURE.txt OUT.png [--cols N --rows N]
       ansi2png.py --gif OUT.gif FRAME.txt... [--delay MS]

Needs Pillow. The font comes from `fc-match monospace`.
"""

import re
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont

BG = (26, 27, 38)
FG = (192, 202, 245)
CHROME = (36, 37, 51)
PALETTE = [
    (21, 22, 30), (247, 118, 142), (158, 206, 106), (224, 175, 104),
    (122, 162, 247), (187, 154, 247), (125, 207, 255), (169, 177, 214),
    (86, 95, 137), (255, 137, 157), (175, 225, 125), (255, 199, 119),
    (141, 176, 255), (199, 167, 255), (164, 218, 255), (192, 202, 245),
]
FONT_SIZE = 18
PAD = 28
BAR = 34
SGR = re.compile(r"\x1b\[([0-9;:]*)m")


def xterm256(n):
    if n < 16:
        return PALETTE[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return (steps[n // 36], steps[n // 6 % 6], steps[n % 6])
    v = 8 + (n - 232) * 10
    return (v, v, v)


def parse(text):
    """Turns the dump into rows of (char, fg, bg, bold)."""
    rows = []
    # tmux only writes a code when the style changes, so it carries over lines.
    fg, bg, bold, rev = None, None, False, False
    for raw in text.split("\n"):
        cells = []
        pos = 0
        for m in list(SGR.finditer(raw)) + [None]:
            end = m.start() if m else len(raw)
            for ch in raw[pos:end]:
                f, b = (bg or BG, fg or FG) if rev else (fg, bg)
                cells.append((ch, f, b, bold))
            if not m:
                break
            pos = m.end()
            codes = [int(c) if c else 0 for c in m.group(1).replace(":", ";").split(";")]
            i = 0
            while i < len(codes):
                c = codes[i]
                if c == 0:
                    fg, bg, bold, rev = None, None, False, False
                elif c == 1:
                    bold = True
                elif c == 22:
                    bold = False
                elif c == 7:
                    rev = True
                elif c == 27:
                    rev = False
                elif 30 <= c <= 37:
                    fg = PALETTE[c - 30]
                elif 90 <= c <= 97:
                    fg = PALETTE[c - 90 + 8]
                elif c == 39:
                    fg = None
                elif 40 <= c <= 47:
                    bg = PALETTE[c - 40]
                elif 100 <= c <= 107:
                    bg = PALETTE[c - 100 + 8]
                elif c == 49:
                    bg = None
                elif c in (38, 48) and i + 1 < len(codes):
                    if codes[i + 1] == 5 and i + 2 < len(codes):
                        col = xterm256(codes[i + 2])
                        i += 2
                    elif codes[i + 1] == 2 and i + 4 < len(codes):
                        col = tuple(codes[i + 2 : i + 5])
                        i += 4
                    else:
                        col = None
                    if c == 38:
                        fg = col
                    else:
                        bg = col
                i += 1
        rows.append(cells)
    return rows


def font_path(bold=False):
    pattern = "monospace:bold" if bold else "monospace"
    return subprocess.run(
        ["fc-match", "-f", "%{file}", pattern], capture_output=True, text=True, check=True
    ).stdout


def render(text, cols, rows):
    font = ImageFont.truetype(font_path(), FONT_SIZE)
    bold = ImageFont.truetype(font_path(True), FONT_SIZE)
    cw = round(font.getlength("M"))
    ch = cw * 2
    grid = parse(text)[:rows]
    w, h = cols * cw + 2 * PAD, rows * ch + 2 * PAD + BAR
    img = Image.new("RGB", (w, h), (0, 0, 0, 0))
    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w - 1, h - 1], 14, fill=255)
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, w, h], fill=BG)
    d.rectangle([0, 0, w, BAR], fill=CHROME)
    for i, dot in enumerate([(247, 118, 142), (224, 175, 104), (158, 206, 106)]):
        x = 20 + i * 22
        d.ellipse([x - 6, BAR // 2 - 6, x + 6, BAR // 2 + 6], fill=dot)

    # Paint the whole screen background first when the capture fills it.
    for y, cells in enumerate(grid):
        for x, (c, fg, bg, b) in enumerate(cells[:cols]):
            px, py = PAD + x * cw, PAD + BAR + y * ch
            if bg:
                d.rectangle([px, py, px + cw - 1, py + ch - 1], fill=bg)
            colour = fg or FG
            if c == "█":
                # Draw full blocks as solid cells, the way terminals do.
                d.rectangle([px, py, px + cw - 1, py + ch - 1], fill=colour)
            elif c.strip():
                f = bold if b else font
                d.text((px, py + (ch - FONT_SIZE) // 2 - 2), c, font=f, fill=colour)
        # A flashing screen paints past the last written cell too.
        if cells and cells[-1][2] and len(cells) < cols:
            bg = cells[-1][2]
            px = PAD + len(cells) * cw
            d.rectangle([px, PAD + BAR + y * ch, PAD + cols * cw - 1, PAD + BAR + (y + 1) * ch - 1], fill=bg)
    out = Image.new("RGBA", (w, h))
    out.paste(img, (0, 0), mask)
    return out


def main(argv):
    cols, rows, delay = 100, 26, 250
    if "--cols" in argv:
        cols = int(argv[argv.index("--cols") + 1])
    if "--rows" in argv:
        rows = int(argv[argv.index("--rows") + 1])
    if "--delay" in argv:
        delay = int(argv[argv.index("--delay") + 1])
    args = [a for i, a in enumerate(argv) if not a.startswith("--") and not (i and argv[i - 1] in ("--cols", "--rows", "--delay", "--gif"))]
    if "--gif" in argv:
        out = argv[argv.index("--gif") + 1]
        frames = [render(open(p, encoding="utf-8").read(), cols, rows).convert("RGB") for p in args]
        frames[0].save(out, save_all=True, append_images=frames[1:], duration=delay, loop=0, optimize=True)
        return
    capture, out = args
    render(open(capture, encoding="utf-8").read(), cols, rows).save(out)


if __name__ == "__main__":
    main(sys.argv[1:])
