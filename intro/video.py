"""Whistlestop intro animation (1920x1080, 30 fps), synced to music.py's timing.

  python video.py                -> raw BGRA frames on stdout (pipe to ffmpeg)
  python video.py --still 6.5 x.png   -> one frame for review
"""
import json
import math
import random
import sys

import cairo

W, H, FPS = 1920, 1080, 30
TM = json.load(open("intro-timing.json"))
DUR, T0, T_HIT = TM["dur"], TM["t0"], TM["t_hit"]
W_INTRO, W_CQ = TM["w_intro"], TM["w_cq"]
BARS = TM["bars"]

BRASS = (242 / 255, 169 / 255, 59 / 255)
BRASS_HI = (1.0, 0.86, 0.55)
NAVY_T = (28 / 255, 36 / 255, 50 / 255)
NAVY_B = (8 / 255, 10 / 255, 15 / 255)
IRON = (0.095, 0.11, 0.15)
RAIL_Y = 880
K_SPEED = 7.0


def clamp(x, a=0.0, b=1.0):
    return max(a, min(b, x))


def smooth(x):
    x = clamp(x)
    return x * x * (3 - 2 * x)


def ease_out(x):
    x = clamp(x)
    return 1 - (1 - x) ** 3


def ease_in(x):
    x = clamp(x)
    return x * x


def lerp(a, b, u):
    return a + (b - a) * u


def mix(c1, c2, u):
    return tuple(lerp(a, b, u) for a, b in zip(c1, c2))


# ------------------------------------------------------------------ motion
def bpm_at(t):
    if t < BARS[0]:
        return TM["bpm0"]
    for i in range(len(BARS) - 1):
        if t < BARS[i + 1]:
            return 240 / (BARS[i + 1] - BARS[i])
    return TM["bpm1"]


def cam_speed(t):
    if t < T0:
        return 40.0
    if t < T_HIT:
        return lerp(40, K_SPEED * bpm_at(t), ease_out((t - T0) / 0.8))
    return K_SPEED * TM["bpm1"] * math.exp(-(t - T_HIT) / 0.35)


DT = 1 / 600
_C = [0.0]
for i in range(int((DUR + 1) / DT)):
    _C.append(_C[-1] + cam_speed(i * DT) * DT)


def cam(t):
    i = clamp(t / DT, 0, len(_C) - 2)
    j = int(i)
    return lerp(_C[j], _C[j + 1], i - j)


def scr(t):
    """Screen x of the locomotive's front."""
    if t < T0:
        return lerp(-700, -130, smooth(t / T0))
    if t < T_HIT - 1.3:
        return lerp(-130, 1350, ease_out((t - T0) / 2.0)) + 25 * math.sin((t - T0) * 0.9) * smooth((t - T0 - 1.5))
    return lerp(1350 + 25 * math.sin((T_HIT - 1.3 - T0) * 0.9), 4600, ease_in((t - (T_HIT - 1.3)) / 1.3))


def wtrain(t):
    return cam(t) + scr(t)


def vtrain(t):
    return (wtrain(t + 0.005) - wtrain(t - 0.005)) / 0.01


def sub_beat_kick(t):
    """0..1 punch right after each beat (camera shake / pulses)."""
    best = 0.0
    if T0 <= t < T_HIT:
        for i in range(len(BARS) - 1):
            b0, b1 = BARS[i], BARS[i + 1]
            for k in (0, 2):
                bt = b0 + (b1 - b0) * k / 4
                if 0 <= t - bt < 0.25:
                    best = max(best, math.exp(-(t - bt) * 14))
    return best


# ------------------------------------------------------------------ the mark
def curve(n=160):
    pts = []
    for i in range(n + 1):
        t = i / n
        ph = 2 * math.pi * (1.2 * t + 1.6 * t * t)
        taper = math.sin(math.pi * t) ** 0.6
        pts.append((0.25 + 0.5 * t, 0.5 - 0.15 * taper * math.sin(ph)))
    return pts


CURVE = curve()
LOGO_C = (960, 400)
LOGO_R = 175
LOGO_S = LOGO_R / 0.30


def logo_pt(u, c=LOGO_C, s=LOGO_S):
    i = clamp(u) * (len(CURVE) - 1)
    j = min(int(i), len(CURVE) - 2)
    x = lerp(CURVE[j][0], CURVE[j + 1][0], i - j)
    y = lerp(CURVE[j][1], CURVE[j + 1][1], i - j)
    return (c[0] + (x - 0.5) * s, c[1] + (y - 0.5) * s)


def draw_mark(cr, cx, cy, r, alpha=1.0, ring_u=1.0, wave_u=1.0, glow=0.0, col=BRASS):
    s = r / 0.30
    cr.save()
    # inner glow
    g = cairo.RadialGradient(cx, cy, 0, cx, cy, r * (1.0 + 0.6 * glow))
    g.add_color_stop_rgba(0, *col, (0.20 + 0.35 * glow) * alpha)
    g.add_color_stop_rgba(1, *col, 0)
    cr.set_source(g)
    cr.arc(cx, cy, r * (1.0 + 0.6 * glow), 0, 2 * math.pi)
    cr.fill()
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    if ring_u > 0:
        cr.set_source_rgba(*col, alpha)
        cr.set_line_width(0.042 * s)
        a0 = -math.pi / 2
        cr.arc(cx, cy, r, a0, a0 + 2 * math.pi * clamp(ring_u))
        cr.stroke()
    if wave_u > 0:
        n = max(2, int((len(CURVE) - 1) * clamp(wave_u)) + 1)
        cr.move_to(cx + (CURVE[0][0] - 0.5) * s, cy + (CURVE[0][1] - 0.5) * s)
        for x, y in CURVE[1:n]:
            cr.line_to(cx + (x - 0.5) * s, cy + (y - 0.5) * s)
        cr.set_source_rgba(*mix(col, BRASS_HI, glow), alpha)
        cr.set_line_width(0.048 * s)
        cr.set_line_join(cairo.LINE_JOIN_ROUND)
        cr.stroke()
    cr.restore()


# ------------------------------------------------------------------ scenery
rnd = random.Random(42)
STARS = [(rnd.uniform(0, W * 1.3), rnd.uniform(0, 640) ** 1.0, rnd.uniform(0.6, 2.2), rnd.uniform(0, 6.28), rnd.uniform(0.5, 3)) for _ in range(380)]
MT_FAR = [(rnd.uniform(0.0006, 0.004), rnd.uniform(0, 6.28), rnd.uniform(20, 70)) for _ in range(6)]
MT_NEAR = [(rnd.uniform(0.001, 0.006), rnd.uniform(0, 6.28), rnd.uniform(10, 45)) for _ in range(6)]


def ridge(x, waves, base):
    return base - sum(a * math.sin(f * x + p) for f, p, a in waves)


def draw_sky(cr, t):
    g = cairo.LinearGradient(0, 0, 0, H)
    g.add_color_stop_rgb(0, 0.03, 0.04, 0.07)
    g.add_color_stop_rgb(0.55, *NAVY_T)
    g.add_color_stop_rgb(0.80, 0.16, 0.13, 0.14)
    g.add_color_stop_rgb(1, *NAVY_B)
    cr.set_source(g)
    cr.paint()
    off = cam(t) * 0.01
    for x, y, r, ph, sp in STARS:
        xx = (x - off) % (W * 1.3) - 50
        a = 0.35 + 0.65 * (0.5 + 0.5 * math.sin(t * sp + ph))
        cr.set_source_rgba(1, 0.97, 0.9, a * (1 - y / 700))
        cr.arc(xx, y, r * 0.8, 0, 2 * math.pi)
        cr.fill()
    # moon
    mx, my = 1560 - cam(t) * 0.004, 190
    g = cairo.RadialGradient(mx, my, 20, mx, my, 330)
    g.add_color_stop_rgba(0, 1, 0.92, 0.75, 0.22)
    g.add_color_stop_rgba(1, 1, 0.92, 0.75, 0)
    cr.set_source(g)
    cr.arc(mx, my, 330, 0, 2 * math.pi)
    cr.fill()
    cr.set_source_rgb(0.98, 0.93, 0.80)
    cr.arc(mx, my, 54, 0, 2 * math.pi)
    cr.fill()
    cr.set_source_rgba(0.85, 0.78, 0.62, 0.5)
    for dx, dy, rr in ((-16, -10, 11), (14, 14, 8), (8, -22, 6)):
        cr.arc(mx + dx, my + dy, rr, 0, 2 * math.pi)
        cr.fill()


def draw_ridge(cr, t, par, waves, base, col):
    off = cam(t) * par
    cr.move_to(0, H)
    for x in range(0, W + 9, 8):
        cr.line_to(x, ridge(x + off, waves, base))
    cr.line_to(W, H)
    cr.close_path()
    cr.set_source_rgb(*col)
    cr.fill()


def draw_towers(cr, t):
    """Ham radio towers with Yagis on the near hills (parallax 0.25)."""
    par, sp = 0.25, 1500
    off = cam(t) * par
    k0 = int((off - 200) // sp)
    for k in range(k0, k0 + 3):
        x = k * sp + 700 - off
        if not -200 < x < W + 200:
            continue
        gy = ridge(x + off, MT_NEAR, 700)
        top = gy - 250
        cr.set_source_rgb(0.05, 0.06, 0.09)
        cr.set_line_width(3)
        cr.move_to(x - 22, gy)
        cr.line_to(x, top)
        cr.line_to(x + 22, gy)
        cr.stroke()
        cr.set_line_width(1.5)
        n = 9
        for i in range(n):
            y1 = gy - (gy - top) * i / n
            y2 = gy - (gy - top) * (i + 1) / n
            w1, w2 = 22 * (1 - i / n), 22 * (1 - (i + 1) / n)
            cr.move_to(x - w1, y1)
            cr.line_to(x + w2, y2)
            cr.move_to(x + w1, y1)
            cr.line_to(x - w2, y2)
        cr.stroke()
        # yagi
        cr.set_line_width(3)
        cr.move_to(x, top)
        cr.line_to(x, top - 30)
        cr.move_to(x - 70, top - 22)
        cr.line_to(x + 70, top - 22)
        cr.stroke()
        cr.set_line_width(2)
        for i in range(7):
            ex = x - 60 + i * 20
            h = 26 - abs(i - 2) * 2
            cr.move_to(ex, top - 22 - h)
            cr.line_to(ex, top - 22 + h)
        cr.stroke()
        blink = 0.5 + 0.5 * math.sin(t * 4 + k)
        g = cairo.RadialGradient(x, top - 32, 0, x, top - 32, 18)
        g.add_color_stop_rgba(0, 1, 0.2, 0.15, 0.9 * blink)
        g.add_color_stop_rgba(1, 1, 0.2, 0.15, 0)
        cr.set_source(g)
        cr.arc(x, top - 32, 18, 0, 2 * math.pi)
        cr.fill()


def draw_poles(cr, t):
    par, sp = 0.6, 520
    off = cam(t) * par
    k0 = int(off // sp) - 1
    xs = [k * sp - off for k in range(k0, k0 + 6)]
    cr.set_source_rgb(0.035, 0.04, 0.06)
    for x in xs:
        cr.rectangle(x - 5, 520, 10, 340)
        cr.fill()
        cr.rectangle(x - 46, 540, 92, 7)
        cr.fill()
        cr.rectangle(x - 34, 566, 68, 6)
        cr.fill()
    cr.set_line_width(1.6)
    for i in range(len(xs) - 1):
        for wy, dx in ((540, -40), (540, 40), (566, -28), (566, 28)):
            x1, x2 = xs[i] + dx, xs[i + 1] + dx
            cr.move_to(x1, wy)
            cr.curve_to(x1 + sp * 0.33, wy + 34, x2 - sp * 0.33, wy + 34, x2, wy)
    cr.set_source_rgba(0.12, 0.13, 0.17, 1)
    cr.stroke()


def draw_ground(cr, t):
    g = cairo.LinearGradient(0, 830, 0, H)
    g.add_color_stop_rgb(0, 0.07, 0.075, 0.09)
    g.add_color_stop_rgb(1, 0.01, 0.012, 0.016)
    cr.set_source(g)
    cr.move_to(0, 850)
    cr.curve_to(500, 835, 1400, 838, W, 848)
    cr.line_to(W, H)
    cr.line_to(0, H)
    cr.close_path()
    cr.fill()
    # ballast
    g = cairo.LinearGradient(0, RAIL_Y, 0, RAIL_Y + 60)
    g.add_color_stop_rgb(0, 0.16, 0.155, 0.16)
    g.add_color_stop_rgb(1, 0.05, 0.05, 0.06)
    cr.set_source(g)
    cr.move_to(-10, RAIL_Y + 60)
    cr.line_to(-10, RAIL_Y + 6)
    cr.line_to(W + 10, RAIL_Y + 6)
    cr.line_to(W + 10, RAIL_Y + 60)
    cr.close_path()
    cr.fill()
    # sleepers with motion smear
    v = cam_speed(t)
    smear = v / FPS
    sp = 58
    off = cam(t) % sp
    a = clamp(1.0 - smear / 80, 0.25, 1)
    cr.set_source_rgba(0.26, 0.2, 0.16, a)
    x = -off
    while x < W + sp:
        cr.rectangle(x, RAIL_Y + 6, 18 + smear, 12)
        x += sp
    cr.fill()
    # rail
    cr.set_source_rgb(0.30, 0.32, 0.36)
    cr.rectangle(0, RAIL_Y - 2, W, 8)
    cr.fill()
    cr.set_source_rgba(0.75, 0.78, 0.84, 0.85)
    cr.rectangle(0, RAIL_Y - 2, W, 2)
    cr.fill()


def draw_foreground(cr, t):
    par, sp = 1.8, 1900
    off = cam(t) * par
    k0 = int(off // sp) - 1
    smear = cam_speed(t) * par / FPS
    for k in range(k0, k0 + 3):
        x = k * sp + 300 - off
        w = 70 + smear
        g = cairo.LinearGradient(x, 0, x + w, 0)
        g.add_color_stop_rgba(0, 0, 0, 0, 0)
        g.add_color_stop_rgba(0.5, 0.01, 0.012, 0.015, 0.95)
        g.add_color_stop_rgba(1, 0, 0, 0, 0)
        cr.set_source(g)
        cr.rectangle(x, 930, w, H - 930)
        cr.fill()
    # grass blades (static silhouettes moving fast)
    sp2 = 37
    off2 = (cam(t) * 1.4) % sp2
    cr.set_source_rgb(0.012, 0.014, 0.018)
    x = -off2
    i = int(cam(t) * 1.4 // sp2)
    while x < W + sp2:
        h = 28 + 22 * math.sin(i * 1.7) ** 2
        cr.move_to(x, H)
        cr.line_to(x + 9, H - h - 20 * (i % 3 == 0))
        cr.line_to(x + 18, H)
        x += sp2
        i += 1
    cr.fill()


# ------------------------------------------------------------------ train
def rrect(cr, x, y, w, h, r):
    cr.new_sub_path()
    cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
    cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
    cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
    cr.arc(x + r, y + r, r, math.pi, 1.5 * math.pi)
    cr.close_path()


def metal(cr, y0, y1, base=IRON, rim=0.22):
    g = cairo.LinearGradient(0, y0, 0, y1)
    g.add_color_stop_rgb(0, *mix(base, (0.55, 0.6, 0.72), rim))
    g.add_color_stop_rgb(0.18, *mix(base, (0.35, 0.4, 0.5), rim * 0.6))
    g.add_color_stop_rgb(0.55, *base)
    g.add_color_stop_rgb(1, *mix(base, (0, 0, 0), 0.5))
    return g


def text_spaced(cr, s, x, y, size, spacing, family="Avenir Next", bold=True, center=True):
    cr.select_font_face(family, cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD if bold else cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(size)
    adv = [cr.text_extents(ch).x_advance for ch in s]
    total = sum(adv) + spacing * (len(s) - 1)
    xx = x - total / 2 if center else x
    pos = []
    for ch, a in zip(s, adv):
        pos.append((ch, xx))
        xx += a + spacing
    return pos, total


def wheel(cr, cx, cy, r, ang, dang, spokes=12):
    cr.set_source_rgb(0.06, 0.065, 0.08)
    cr.arc(cx, cy, r, 0, 2 * math.pi)
    cr.fill()
    cr.set_source_rgb(0.42, 0.44, 0.48)
    cr.set_line_width(max(3, r * 0.1))
    cr.arc(cx, cy, r - r * 0.06, 0, 2 * math.pi)
    cr.stroke()
    # motion-blurred spokes: several sub-angles across this frame's rotation
    sub = 6 if abs(dang) > 0.08 else 1
    cr.set_line_width(max(2, r * 0.07))
    for j in range(sub):
        a0 = ang - dang * j / sub
        cr.set_source_rgba(0.30, 0.32, 0.37, 1.0 / sub * (1.6 if sub > 1 else 1))
        for k in range(spokes):
            a = a0 + 2 * math.pi * k / spokes
            cr.move_to(cx + r * 0.18 * math.cos(a), cy + r * 0.18 * math.sin(a))
            cr.line_to(cx + r * 0.86 * math.cos(a), cy + r * 0.86 * math.sin(a))
        cr.stroke()
    cr.set_source_rgb(*BRASS)
    cr.arc(cx, cy, r * 0.17, 0, 2 * math.pi)
    cr.fill()


DRIVERS = [-345, -470, -595]
DR = 60


def draw_train(cr, t):
    x0 = scr(t)
    if x0 < -2400 or x0 - 2300 > W + 200:
        return
    dist = wtrain(t)
    ang = dist / DR
    dang = (wtrain(t) - wtrain(t - 1 / FPS)) / DR
    bob = 1.6 * math.sin(ang * 2)
    cr.save()
    cr.translate(x0, bob)
    Y = RAIL_Y

    # --- coaches (behind loco), right to left
    for ci, (cx1, label) in enumerate(((-1110, "ROUNDTABLE"), (-1700, "FILE EXPRESS"))):
        cx0 = cx1 - 570
        if x0 + cx1 < -50:
            continue
        cr.set_source(metal(cr, Y - 320, Y - 110, (0.12, 0.08, 0.075), 0.25))
        rrect(cr, cx0, Y - 300, 570, 190, 10)
        cr.fill()
        # roof
        cr.set_source(metal(cr, Y - 340, Y - 296, (0.09, 0.09, 0.11), 0.35))
        cr.move_to(cx0 - 8, Y - 298)
        cr.curve_to(cx0 + 20, Y - 330, cx1 - 20, Y - 330, cx1 + 8, Y - 298)
        cr.close_path()
        cr.fill()
        cr.rectangle(cx0 + 60, Y - 345, 450, 18)
        cr.fill()
        # windows (warm, flickering)
        for wi in range(8):
            wx = cx0 + 36 + wi * 64
            fl = 0.85 + 0.15 * math.sin(t * 9 + wi * 2.1 + ci)
            g = cairo.LinearGradient(0, Y - 280, 0, Y - 220)
            g.add_color_stop_rgb(0, 1.0 * fl, 0.80 * fl, 0.45 * fl)
            g.add_color_stop_rgb(1, 0.85 * fl, 0.52 * fl, 0.22 * fl)
            cr.set_source(g)
            rrect(cr, wx, Y - 282, 42, 58, 6)
            cr.fill()
            # passenger silhouette in some windows
            if (wi * 7 + ci * 3) % 3 == 0:
                cr.set_source_rgba(0.18, 0.1, 0.06, 0.8)
                cr.arc(wx + 21, Y - 245, 9, 0, 2 * math.pi)
                cr.fill()
                cr.rectangle(wx + 9, Y - 237, 24, 14)
                cr.fill()
        cr.set_source_rgb(*BRASS)
        cr.rectangle(cx0 + 6, Y - 212, 558, 4)
        cr.fill()
        cr.rectangle(cx0 + 6, Y - 134, 558, 3)
        cr.fill()
        pos, _ = text_spaced(cr, label, cx0 + 285, 0, 26, 9)
        cr.set_source_rgba(*BRASS, 0.95)
        for ch, xx in pos:
            cr.move_to(xx, Y - 162)
            cr.show_text(ch)
        for bx in (cx0 + 70, cx0 + 130, cx1 - 130, cx1 - 70):
            wheel(cr, bx, Y - 26, 26, dist / 26, dang * DR / 26, 8)
        cr.set_source_rgb(0.05, 0.05, 0.06)
        cr.rectangle(cx0 + 40, Y - 112, 120, 30)
        cr.rectangle(cx1 - 160, Y - 112, 120, 30)
        cr.fill()
        cr.rectangle(cx1, Y - 150, 30, 10)
        cr.fill()

    # --- tender
    tx0, tx1 = -1085, -742
    cr.set_source_rgb(0.05, 0.05, 0.06)
    cr.rectangle(tx1 - 5, Y - 150, 30, 10)
    cr.fill()
    # coal
    cr.set_source_rgb(0.04, 0.04, 0.05)
    cr.move_to(tx0 + 20, Y - 282)
    for i in range(12):
        cr.line_to(tx0 + 30 + i * 26, Y - 300 - 10 * math.sin(i * 1.9) ** 2)
    cr.line_to(tx1 - 20, Y - 282)
    cr.fill()
    cr.set_source(metal(cr, Y - 285, Y - 110))
    rrect(cr, tx0, Y - 285, tx1 - tx0, 175, 8)
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(tx0 + 8, Y - 272, tx1 - tx0 - 16, 3)
    cr.rectangle(tx0 + 8, Y - 128, tx1 - tx0 - 16, 3)
    cr.fill()
    pos, _ = text_spaced(cr, "WHISTLESTOP", (tx0 + tx1) / 2, 0, 31, 6)
    for ch, xx in pos:
        cr.move_to(xx, Y - 182)
        cr.text_path(ch)
    cr.set_source(metal(cr, Y - 215, Y - 180, (0.95, 0.66, 0.23), 0.6))
    cr.fill()
    cr.set_source_rgba(*BRASS, 0.85)
    cr.select_font_face("Avenir Next", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(15)
    ex = cr.text_extents("HF  ·  MERCURY  ·  CQ")
    cr.move_to((tx0 + tx1) / 2 - ex.width / 2, Y - 152)
    cr.show_text("HF  ·  MERCURY  ·  CQ")
    for bx in (tx0 + 55, tx0 + 115, tx1 - 115, tx1 - 55):
        wheel(cr, bx, Y - 26, 26, dist / 26, dang * DR / 26, 8)

    # --- locomotive
    by0, by1 = Y - 268, Y - 138
    # frame
    cr.set_source_rgb(0.05, 0.055, 0.07)
    cr.rectangle(-660, Y - 140, 580, 40)
    cr.fill()
    # firebox glow
    fl = 0.7 + 0.3 * math.sin(t * 23) * math.sin(t * 7.3)
    g = cairo.RadialGradient(-560, Y - 110, 5, -560, Y - 110, 120)
    g.add_color_stop_rgba(0, 1, 0.55, 0.15, 0.9 * fl)
    g.add_color_stop_rgba(1, 1, 0.3, 0.05, 0)
    cr.set_source(g)
    cr.arc(-560, Y - 110, 120, 0, 2 * math.pi)
    cr.fill()
    # boiler
    cr.set_source(metal(cr, by0, by1))
    cr.move_to(-600, by0)
    cr.line_to(-140, by0)
    cr.curve_to(-112, by0, -100, by0 + 20, -100, (by0 + by1) / 2)
    cr.curve_to(-100, by1 - 20, -112, by1, -140, by1)
    cr.line_to(-600, by1)
    cr.close_path()
    cr.fill()
    # smokebox darker
    cr.set_source(metal(cr, by0, by1, (0.06, 0.065, 0.08), 0.18))
    cr.move_to(-215, by0)
    cr.line_to(-140, by0)
    cr.curve_to(-112, by0, -100, by0 + 20, -100, (by0 + by1) / 2)
    cr.curve_to(-100, by1 - 20, -112, by1, -140, by1)
    cr.line_to(-215, by1)
    cr.close_path()
    cr.fill()
    # boiler bands
    cr.set_source_rgb(*BRASS)
    for bx in (-215, -330, -450, -575):
        cr.rectangle(bx, by0, 7, by1 - by0)
    cr.fill()
    # stack (flared)
    cr.set_source(metal(cr, Y - 380, by0, (0.07, 0.075, 0.09), 0.3))
    cr.move_to(-190, by0 + 4)
    cr.line_to(-188, Y - 330)
    cr.line_to(-212, Y - 372)
    cr.line_to(-128, Y - 372)
    cr.line_to(-152, Y - 330)
    cr.line_to(-150, by0 + 4)
    cr.close_path()
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(-216, Y - 380, 92, 10)
    cr.fill()
    # domes
    for dx, w_, h_ in ((-300, 46, 34), (-405, 60, 46)):
        cr.set_source(metal(cr, by0 - h_, by0 + 4, (0.75, 0.52, 0.18), 0.5))
        cr.move_to(dx - w_, by0 + 2)
        cr.curve_to(dx - w_ * 0.6, by0 - h_ * 1.1, dx + w_ * 0.6, by0 - h_ * 1.1, dx + w_, by0 + 2)
        cr.close_path()
        cr.fill()
    # bell
    cr.set_source_rgb(*BRASS)
    cr.move_to(-262, by0 - 4)
    cr.curve_to(-262, by0 - 40, -238, by0 - 40, -238, by0 - 4)
    cr.close_path()
    cr.fill()
    # whistle on the cab front, with puff when whistling
    cr.rectangle(-596, by0 - 30, 8, 30)
    cr.fill()
    # headlight
    cr.set_source(metal(cr, Y - 318, by0 + 4, (0.08, 0.085, 0.1), 0.3))
    rrect(cr, -150, Y - 316, 52, 52, 6)
    cr.fill()
    cr.set_source_rgb(1, 0.95, 0.8)
    cr.arc(-102, Y - 290, 14, -math.pi / 2, math.pi / 2)
    cr.fill()
    # cab
    cr.set_source(metal(cr, Y - 360, Y - 140, (0.09, 0.1, 0.13), 0.25))
    rrect(cr, -740, Y - 352, 150, 215, 6)
    cr.fill()
    cr.set_source(metal(cr, Y - 372, Y - 348, (0.06, 0.065, 0.08), 0.4))
    cr.move_to(-756, Y - 350)
    cr.curve_to(-720, Y - 380, -610, Y - 380, -575, Y - 350)
    cr.close_path()
    cr.fill()
    flk = 0.85 + 0.15 * math.sin(t * 17)
    g = cairo.LinearGradient(0, Y - 330, 0, Y - 262)
    g.add_color_stop_rgb(0, 1 * flk, 0.78 * flk, 0.42 * flk)
    g.add_color_stop_rgb(1, 0.95 * flk, 0.48 * flk, 0.15 * flk)
    cr.set_source(g)
    rrect(cr, -716, Y - 330, 98, 66, 6)
    cr.fill()
    # engineer silhouette
    cr.set_source_rgba(0.15, 0.08, 0.04, 0.9)
    cr.arc(-650, Y - 292, 12, 0, 2 * math.pi)
    cr.fill()
    cr.rectangle(-667, Y - 304, 34, 7)
    cr.fill()
    cr.rectangle(-664, Y - 281, 30, 18)
    cr.fill()
    # herald: the Whistlestop mark on the cab side
    draw_mark(cr, -665, Y - 200, 38, 1.0, 1, 1, 0.25 + 0.6 * whistle_level(t))
    # running board
    cr.set_source_rgb(*BRASS)
    cr.rectangle(-600, by1, 510, 5)
    cr.fill()
    # pilot (cowcatcher) + buffer beam
    cr.set_source_rgb(0.55, 0.1, 0.08)
    cr.rectangle(-110, Y - 138, 34, 40)
    cr.fill()
    cr.set_source_rgb(0.07, 0.075, 0.09)
    cr.move_to(-110, Y - 98)
    cr.line_to(-70, Y - 98)
    cr.line_to(8, Y - 12)
    cr.line_to(-110, Y - 12)
    cr.close_path()
    cr.fill()
    cr.set_source_rgb(0.32, 0.34, 0.38)
    cr.set_line_width(3)
    for xx in range(-104, 0, 14):
        top = Y - 98 if xx <= -70 else Y - 98 + (xx + 70) * 86 / 78
        cr.move_to(xx, Y - 14)
        cr.line_to(xx, top + 2)
    cr.stroke()
    cr.set_source_rgb(*BRASS)
    cr.set_line_width(3)
    cr.move_to(-110, Y - 98)
    cr.line_to(-70, Y - 98)
    cr.line_to(8, Y - 12)
    cr.stroke()
    # wheels
    wheel(cr, -140, Y - 30, 30, dist / 30, dang * DR / 30, 8)
    wheel(cr, -712, Y - 32, 32, dist / 32, dang * DR / 32, 8)
    for dx in DRIVERS:
        wheel(cr, dx, Y - DR, DR, ang, dang, 14)
    # cylinder
    cyy = Y - DR
    cr.set_source(metal(cr, cyy - 28, cyy + 28, (0.08, 0.085, 0.1), 0.35))
    rrect(cr, -278, cyy - 28, 100, 56, 8)
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(-274, cyy - 28, 5, 56)
    cr.rectangle(-187, cyy - 28, 5, 56)
    cr.fill()
    # rods
    crank = 24
    pins = [(dx + crank * math.cos(ang), cyy + crank * math.sin(ang)) for dx in DRIVERS]
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_source_rgb(0.62, 0.64, 0.7)
    cr.set_line_width(10)
    cr.move_to(*pins[0])
    for p in pins[1:]:
        cr.line_to(*p)
    cr.stroke()
    px, py = pins[1]
    L = 172
    xh = px + math.sqrt(max(L * L - (py - cyy) ** 2, 1))
    cr.set_source_rgb(0.72, 0.74, 0.8)
    cr.set_line_width(12)
    cr.move_to(px, py)
    cr.line_to(xh, cyy)
    cr.stroke()
    cr.set_line_width(7)
    cr.move_to(xh, cyy)
    cr.line_to(-200, cyy)
    cr.stroke()
    cr.set_source_rgb(0.35, 0.37, 0.42)
    cr.rectangle(xh - 14, cyy - 13, 28, 26)
    cr.fill()
    for p in pins:
        cr.set_source_rgb(*BRASS)
        cr.arc(p[0], p[1], 7, 0, 2 * math.pi)
        cr.fill()
    cr.restore()

    # headlight beam (screen space, additive)
    lx, ly = x0 - 98, RAIL_Y - 290 + bob
    cr.save()
    cr.set_operator(cairo.OPERATOR_ADD)
    g = cairo.LinearGradient(lx, 0, lx + 1500, 0)
    g.add_color_stop_rgba(0, 1, 0.9, 0.65, 0.30)
    g.add_color_stop_rgba(1, 1, 0.9, 0.65, 0)
    cr.set_source(g)
    cr.move_to(lx, ly - 10)
    cr.line_to(lx + 1500, ly - 260)
    cr.line_to(lx + 1500, ly + 330)
    cr.line_to(lx, ly + 12)
    cr.close_path()
    cr.fill()
    g = cairo.RadialGradient(lx, ly, 0, lx, ly, 150)
    g.add_color_stop_rgba(0, 1, 0.95, 0.8, 0.9)
    g.add_color_stop_rgba(0.15, 1, 0.85, 0.55, 0.35)
    g.add_color_stop_rgba(1, 1, 0.8, 0.5, 0)
    cr.set_source(g)
    cr.arc(lx, ly, 150, 0, 2 * math.pi)
    cr.fill()
    cr.restore()


def whistle_level(t):
    lv = 0.0
    for w0, g in ((W_INTRO, 1.0), (W_CQ, 1.0), (W_CQ + 0.9, 0.4)):
        if w0 <= t < w0 + 0.8:
            lv = max(lv, g * (0.6 + 0.4 * math.sin(math.pi * ((t - w0) % 0.4) / 0.4)))
        elif w0 + 0.8 <= t < w0 + 1.3:
            lv = max(lv, g * 0.6 * (1 - (t - w0 - 0.8) / 0.5))
    return lv


# ------------------------------------------------------------------ smoke & sparks
def stack_screen(t):
    return scr(t) - 170, RAIL_Y - 380


PUFFS = []
pr = random.Random(9)
for te in TM["intro_chuffs"]:
    PUFFS.append((te, True, [(pr.uniform(-20, 20), pr.uniform(-10, 10), pr.uniform(0.8, 1.3)) for _ in range(3)], pr.uniform(0, 60)))
_sm = TM["smoke"]
for k, te in enumerate(_sm):
    for tt_ in ([te] + ([(te + _sm[k + 1]) / 2] if k + 1 < len(_sm) else [])):
        if tt_ < T_HIT - 0.15:
            PUFFS.append((tt_, False, [(pr.uniform(-26, 26), pr.uniform(-16, 16), pr.uniform(0.75, 1.3)) for _ in range(4)], 0))


def puff_state(p, t):
    te, intro, blobs, jx = p
    tau = t - te
    if tau < 0:
        return None
    sx, sy = stack_screen(te)
    if intro:
        x0w = cam(te) + max(sx, 70 + jx)
        sy = 860
        v = 60
    else:
        x0w = wtrain(te) - 170
        v = vtrain(te)
    xw = x0w + v * 0.16 * (1 - math.exp(-tau / 0.16)) - 30 * tau
    rise = (360 if intro else 230) * (1 - math.exp(-tau / 1.0)) + 25 * tau
    y = sy - rise
    rad = (20 + 95 * (1 - math.exp(-tau / 0.8))) * (1.7 if intro else 1.0)
    a = (0.4 if intro else 0.42) * math.exp(-tau / (1.5 if intro else 1.1)) * clamp(tau / 0.05)
    return xw, y, rad, a


T_M = T_HIT - 0.35
_morph = None


def morph_set():
    global _morph
    if _morph is None:
        blobs = []
        for p in PUFFS:
            st = puff_state(p, T_M)
            if st and st[3] > 0.05:
                for b in p[2]:
                    blobs.append((p, b, st[0] - cam(T_M) + b[0] * st[2] / 40))
        blobs.sort(key=lambda z: z[2])
        n = len(blobs)
        _morph = [(p, b, logo_pt((i + 0.5) / n)) for i, (p, b, _) in enumerate(blobs)]
    return _morph


def draw_puff_blob(cr, x, y, r, a, col):
    g = cairo.RadialGradient(x, y, 0, x, y, r)
    g.add_color_stop_rgba(0, *col, a)
    g.add_color_stop_rgba(0.6, *col, a * 0.55)
    g.add_color_stop_rgba(1, *col, 0)
    cr.set_source(g)
    cr.arc(x, y, r, 0, 2 * math.pi)
    cr.fill()


def draw_smoke(cr, t):
    c = cam(t)
    morphing = t >= T_M
    mset = morph_set() if morphing else []
    mkeys = {id(p) for p, _, _ in mset}
    u = smooth((t - T_M) / 1.25) if morphing else 0
    for p in PUFFS:
        if morphing and id(p) in mkeys:
            continue
        st = puff_state(p, t)
        if not st or st[3] < 0.005:
            continue
        xw, y, rad, a = st
        if morphing:
            a *= 1 - smooth((t - T_M) / 0.5)
        for bx, by, bs in p[2]:
            col = mix((0.30, 0.31, 0.35), (0.66, 0.68, 0.76), clamp((t - p[0]) / 0.9)) if not p[1] else (0.5, 0.54, 0.64)
            draw_puff_blob(cr, xw - c + bx * rad / 40, y + by * rad / 40, rad * bs, a, col)
    if morphing:
        fade = 1 - smooth((t - (T_HIT + 0.75)) / 0.5)
        for p, b, (tx, ty) in mset:
            st = puff_state(p, min(t, T_M + 0.6))
            xw, y, rad, a = st
            x = lerp(xw - c + b[0] * rad / 40, tx, u)
            yy = lerp(y + b[1] * rad / 40, ty, u)
            r = lerp(rad * b[2], 20, u)
            aa = lerp(a, 0.8, u) * fade
            col = mix((0.6, 0.62, 0.7), BRASS_HI, u)
            if aa > 0.004:
                draw_puff_blob(cr, x, yy, r, aa, col)


SPARKS = []
sr_ = random.Random(3)
tt = T0 + 1.5
while tt < T_HIT - 0.3:
    tt += sr_.expovariate(14 + 26 * (tt - T0) / (T_HIT - T0))
    SPARKS.append((tt, sr_.choice(DRIVERS + [-140]), sr_.uniform(-0.6, 0.2), sr_.uniform(180, 520), sr_.uniform(0.25, 0.45)))


def draw_sparks(cr, t):
    c = cam(t)
    cr.save()
    cr.set_operator(cairo.OPERATOR_ADD)
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_width(2.2)
    for te, dx, vxf, vy, life in SPARKS:
        tau = t - te
        if not 0 <= tau < life:
            continue
        x0 = wtrain(te) + dx + 30
        vx = vtrain(te) * (0.5 + vxf * 0.3)

        def pos(tt_):
            return x0 + vx * tt_ - c, RAIL_Y - vy * tt_ + 900 * tt_ * tt_

        x1, y1 = pos(tau)
        x2, y2 = pos(max(0, tau - 0.025))
        k = 1 - tau / life
        cr.set_source_rgba(1, 0.75 + 0.2 * k, 0.35, 0.9 * k)
        cr.move_to(x2, y2)
        cr.line_to(x1, y1)
        cr.stroke()
    cr.restore()


# ------------------------------------------------------------------ text
def card(cr, text, t0, t1, t, y=190, size=104, sub=None):
    if not t0 <= t < t1:
        return
    u_in = ease_out((t - t0) / 0.18)
    u_out = 1 - smooth((t - (t1 - 0.22)) / 0.22)
    a = u_in * u_out
    sc = lerp(1.25, 1.0, u_in)
    cr.save()
    cr.translate(W / 2, y)
    cr.scale(sc, sc)
    pos, total = text_spaced(cr, text, 0, 0, size, 10, "Avenir Next Condensed")
    for ch, xx in pos:
        cr.move_to(xx + 4, 6)
        cr.text_path(ch)
    cr.set_source_rgba(0, 0, 0, 0.5 * a)
    cr.fill()
    for ch, xx in pos:
        cr.move_to(xx, 0)
        cr.text_path(ch)
    g = cairo.LinearGradient(0, -size * 0.75, 0, 0)
    g.add_color_stop_rgba(0, 1, 0.97, 0.9, a)
    g.add_color_stop_rgba(1, *BRASS_HI, a)
    cr.set_source(g)
    cr.fill()
    w = total * u_in
    cr.set_source_rgba(*BRASS, a)
    cr.rectangle(-w / 2, 22, w, 5)
    cr.fill()
    cr.restore()


def soft_line(cr, text, t0, t1, t, y, size, col=(0.95, 0.92, 0.86), spacing=2, bold=False, family="Avenir Next"):
    a = smooth((t - t0) / 0.6) * (1 - smooth((t - (t1 - 0.6)) / 0.6))
    if a <= 0:
        return
    pos, _ = text_spaced(cr, text, W / 2, 0, size, spacing, family, bold)
    cr.set_source_rgba(*col, a)
    dy = 12 * (1 - smooth((t - t0) / 0.8))
    for ch, xx in pos:
        cr.move_to(xx, y + dy)
        cr.show_text(ch)


# ------------------------------------------------------------------ frame
def frame(cr, t):
    shake = sub_beat_kick(t) * 3.0 * clamp((t - T0) / 1.0)
    if T_HIT <= t < T_HIT + 0.6:
        shake += 14 * math.exp(-(t - T_HIT) * 7)
    sx = shake * math.sin(t * 91.0)
    sy = shake * math.cos(t * 77.0)
    cr.save()
    cr.translate(sx, sy)
    draw_sky(cr, t)
    draw_ridge(cr, t, 0.06, MT_FAR, 620, (0.075, 0.09, 0.125))
    draw_towers(cr, t)
    draw_ridge(cr, t, 0.25, MT_NEAR, 700, (0.045, 0.055, 0.08))
    draw_poles(cr, t)
    draw_ground(cr, t)
    # intro: light from the approaching engine at the left edge
    if t < T0 + 0.5:
        lvl = smooth((t - 1.4) / (T0 - 1.4)) * (1 + 0.6 * whistle_level(t))
        g = cairo.RadialGradient(0, 600, 0, 0, 600, 900)
        g.add_color_stop_rgba(0, 1, 0.8, 0.45, 0.35 * lvl)
        g.add_color_stop_rgba(1, 1, 0.8, 0.45, 0)
        cr.save()
        cr.set_operator(cairo.OPERATOR_ADD)
        cr.set_source(g)
        cr.paint()
        cr.restore()
    draw_train(cr, t)
    draw_sparks(cr, t)
    draw_smoke(cr, t)
    draw_foreground(cr, t)
    cr.restore()

    # dim the world for the logo
    if t >= T_HIT - 0.2:
        cr.set_source_rgba(*NAVY_B, 0.62 * smooth((t - T_HIT + 0.2) / 1.2))
        cr.paint()

    # vignette
    g = cairo.RadialGradient(W / 2, H / 2, H * 0.35, W / 2, H / 2, H * 1.05)
    g.add_color_stop_rgba(0, 0, 0, 0, 0)
    g.add_color_stop_rgba(1, 0, 0, 0, 0.6)
    cr.set_source(g)
    cr.paint()

    # words
    soft_line(cr, "It's been a long road.", 0.5, 2.55, t, 330, 64, family="Avenir Next")
    card(cr, "FULL STEAM AHEAD", BARS[2], BARS[4], t)
    card(cr, "KEYBOARD CHAT", BARS[4], BARS[5], t)
    card(cr, "FILE TRANSFER", BARS[5], BARS[6], t)
    card(cr, "ROUNDTABLES", BARS[6], BARS[7], t)
    card(cr, "ALL OVER HF", BARS[7], T_HIT - 0.1, t)

    # logo
    if t >= T_HIT - 0.2:
        ring_u = ease_out((t - T_HIT - 0.1) / 0.7)
        wave_u = ease_out((t - T_HIT - 0.55) / 0.6)
        wl = whistle_level(t) if t > W_CQ - 0.1 else 0
        draw_mark(cr, *LOGO_C, LOGO_R, 1.0, ring_u, wave_u, wl)
        # traveling highlight along the wave while it whistles (up = left->right)
        for w0, gmax in ((W_CQ, 1.0), (W_CQ + 0.9, 0.45)):
            if w0 <= t < w0 + 0.8:
                ph = ((t - w0) % 0.4) / 0.2
                uu = ph if ph < 1 else 2 - ph
                hx, hy = logo_pt(uu)
                g = cairo.RadialGradient(hx, hy, 0, hx, hy, 70)
                g.add_color_stop_rgba(0, 1, 0.97, 0.85, 0.9 * gmax)
                g.add_color_stop_rgba(1, 1, 0.85, 0.5, 0)
                cr.save()
                cr.set_operator(cairo.OPERATOR_ADD)
                cr.set_source(g)
                cr.arc(hx, hy, 70, 0, 2 * math.pi)
                cr.fill()
                cr.restore()
            # RF rings radiating from the mark on each chirp
            for k in range(4):
                tk = w0 + k * 0.2
                tau = t - tk
                if 0 <= tau < 1.2:
                    rr = LOGO_R + 30 + tau * 260
                    cr.set_source_rgba(*BRASS, 0.5 * gmax * (1 - tau / 1.2) ** 2)
                    cr.set_line_width(4)
                    cr.arc(LOGO_C[0], LOGO_C[1], rr, 0, 2 * math.pi)
                    cr.stroke()
        # wordmark, letter by letter
        pos, total = text_spaced(cr, "WHISTLESTOP", W / 2, 0, 132, 22, "Avenir Next")
        for i, (ch, xx) in enumerate(pos):
            u = ease_out((t - (T_HIT + 0.55 + i * 0.045)) / 0.45)
            if u <= 0:
                continue
            cr.move_to(xx, 745 + 40 * (1 - u))
            cr.text_path(ch)
            g = cairo.LinearGradient(0, 650, 0, 760)
            g.add_color_stop_rgba(0, *BRASS_HI, u)
            g.add_color_stop_rgba(0.55, *BRASS, u)
            g.add_color_stop_rgba(1, 0.7, 0.42, 0.12, u)
            cr.set_source(g)
            cr.fill()
        # rule with a tiny train running along it
        ru = ease_out((t - (T_HIT + 1.0)) / 0.8)
        if ru > 0:
            cr.set_source_rgba(*BRASS, 0.6)
            cr.rectangle(W / 2 - total / 2 * ru, 790, total * ru, 2)
            cr.fill()
            tx = W / 2 - total / 2 + (t - (T_HIT + 1.0)) * 210
            if tx < W / 2 + total / 2:
                cr.set_source_rgba(*BRASS_HI, 0.95 * ru)
                rrect(cr, tx - 26, 776, 26, 12, 3)
                rrect(cr, tx - 56, 778, 27, 10, 3)
                rrect(cr, tx - 86, 778, 27, 10, 3)
                cr.fill()
                cr.rectangle(tx - 10, 769, 5, 8)
                cr.fill()
                for k in range(5):
                    cr.set_source_rgba(0.8, 0.82, 0.9, 0.35 * ru * (1 - k / 5))
                    cr.arc(tx - 8 - k * 18, 762 - k * 4, 4 + k * 2, 0, 2 * math.pi)
                    cr.fill()
        soft_line(cr, "HF keyboard chat  ·  files  ·  Roundtables", T_HIT + 1.3, 99, t, 862, 40)
        soft_line(cr, "over the Mercury modem.  The whistle calls CQ.", T_HIT + 1.8, 99, t, 918, 30, col=(0.75, 0.72, 0.66))

    # flash at the hit
    if T_HIT <= t < T_HIT + 0.6:
        cr.set_source_rgba(1, 0.9, 0.7, 0.5 * math.exp(-(t - T_HIT) * 14))
        cr.set_operator(cairo.OPERATOR_ADD)
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)

    # fade from / to black
    fb = 1 - smooth(t / 0.6)
    fe = smooth((t - (DUR - 1.3)) / 1.2)
    a = max(fb, fe)
    if a > 0:
        cr.set_source_rgba(0, 0, 0, a)
        cr.paint()


def render(t):
    surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, W, H)
    cr = cairo.Context(surf)
    frame(cr, t)
    surf.flush()
    return surf


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[1] == "--still":
        render(float(sys.argv[2])).write_to_png(sys.argv[3])
    else:
        out = sys.stdout.buffer
        for i in range(int(DUR * FPS)):
            out.write(bytes(render(i / FPS).get_data()))
