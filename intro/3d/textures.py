"""Decal textures for the 3D runaway intro (run with the cairo venv)."""
import math
import os
import sys

import cairo

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tex")
os.makedirs(OUT, exist_ok=True)
BRASS = (242 / 255, 169 / 255, 59 / 255)


def curve(n=160):
    pts = []
    for i in range(n + 1):
        u = i / n
        ph = 2 * math.pi * (1.2 * u + 1.6 * u * u)
        pts.append((0.25 + 0.5 * u, 0.5 - 0.15 * math.sin(math.pi * u) ** 0.6 * math.sin(ph)))
    return pts


def mark(cr, cx, cy, r, bg=None):
    s = r / 0.30
    if bg:
        cr.set_source_rgb(*bg)
        cr.arc(cx, cy, r * 1.25, 0, 2 * math.pi)
        cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.set_line_width(0.042 * s)
    cr.arc(cx, cy, r, 0, 2 * math.pi)
    cr.stroke()
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_join(cairo.LINE_JOIN_ROUND)
    pts = curve()
    cr.move_to(cx + (pts[0][0] - 0.5) * s, cy + (pts[0][1] - 0.5) * s)
    for x, y in pts[1:]:
        cr.line_to(cx + (x - 0.5) * s, cy + (y - 0.5) * s)
    cr.set_line_width(0.048 * s)
    cr.stroke()


def spaced(cr, text, cx, y, size, spacing, bold=True, family="Avenir Next"):
    cr.select_font_face(family, cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD if bold else cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(size)
    adv = [cr.text_extents(c).x_advance for c in text]
    x = cx - (sum(adv) + spacing * (len(text) - 1)) / 2
    for c, a in zip(text, adv):
        cr.move_to(x, y)
        cr.show_text(c)
        x += a + spacing


def save(surf, name):
    surf.write_to_png(os.path.join(OUT, name))


# herald (transparent), used on cab sides and the smokebox door
s = cairo.ImageSurface(cairo.FORMAT_ARGB32, 1024, 1024)
mark(cairo.Context(s), 512, 512, 380)
save(s, "herald.png")

# tender lettering
s = cairo.ImageSurface(cairo.FORMAT_ARGB32, 2048, 512)
cr = cairo.Context(s)
cr.set_source_rgb(*BRASS)
cr.rectangle(40, 40, 1968, 12)
cr.rectangle(40, 460, 1968, 12)
cr.fill()
spaced(cr, "WHISTLESTOP", 1024, 300, 230, 34)
spaced(cr, "No. 7  ·  HF  ·  MERCURY  ·  CQ", 1024, 410, 70, 10, bold=False)
save(s, "tender.png")

for name, label in (("coach1.png", "ROUNDTABLE"), ("coach2.png", "FILE EXPRESS")):
    s = cairo.ImageSurface(cairo.FORMAT_ARGB32, 2048, 256)
    cr = cairo.Context(s)
    cr.set_source_rgb(*BRASS)
    spaced(cr, label, 1024, 190, 170, 40)
    save(s, name)

# summit sign: dark weathered wood, brass border
s = cairo.ImageSurface(cairo.FORMAT_RGB24, 1536, 512)
cr = cairo.Context(s)
g = cairo.LinearGradient(0, 0, 0, 512)
g.add_color_stop_rgb(0, 0.11, 0.075, 0.05)
g.add_color_stop_rgb(1, 0.06, 0.04, 0.028)
cr.set_source(g)
cr.paint()
cr.set_source_rgba(0, 0, 0, 0.25)
cr.set_line_width(2)
for i in range(0, 512, 9):
    cr.move_to(0, i + 3 * math.sin(i))
    cr.curve_to(500, i + 6, 1000, i - 6, 1536, i + 2)
cr.stroke()
cr.set_source_rgb(*BRASS)
cr.set_line_width(18)
cr.rectangle(36, 36, 1536 - 72, 512 - 72)
cr.stroke()
spaced(cr, "WHISTLE STOP", 768, 320, 200, 18)
save(s, "sign.png")
print("textures ->", OUT)
