"""Whistlestop "Runaway" intro (1920x1080, 30 fps): a no-brakes steam train
screaming down a mountain in a storm, jumping off the rails over crests,
crossing trestles, and launching off a broken trestle into the logo.
Synced to music_metal.py's runaway-timing.json.

  python video_mountain.py                     -> BGRA frames on stdout
  python video_mountain.py --still 9.2 x.png   -> one frame
"""
import bisect
import json
import math
import random
import sys

import cairo

W, H, FPS = 1920, 1080, 30
TM = json.load(open("runaway-timing.json"))
DUR, T0, T_HIT, T_LAUNCH = TM["dur"], TM["t0"], TM["t_hit"], TM["t_launch"]
W_INTRO, W_CQ = TM["w_intro"], TM["w_cq"]
BARS, LANDINGS, LIGHTNING = TM["bars"], TM["landings"], TM["lightning"]

BRASS = (242 / 255, 169 / 255, 59 / 255)
BRASS_HI = (1.0, 0.86, 0.55)
NAVY_B = (8 / 255, 10 / 255, 15 / 255)
IRON = (0.095, 0.105, 0.14)
RED = (0.42, 0.08, 0.06)
K_SPEED = 9.0
AIR = 0.85
LOCO_SX = 1150          # loco front on screen


def clamp(x, a=0.0, b=1.0):
    return max(a, min(b, x))


def smooth(x):
    x = clamp(x)
    return x * x * (3 - 2 * x)


def ease_out(x):
    x = clamp(x)
    return 1 - (1 - x) ** 3


def lerp(a, b, u):
    return a + (b - a) * u


def mix(c1, c2, u):
    return tuple(lerp(a, b, u) for a, b in zip(c1, c2))


# ------------------------------------------------------------------ train motion
def bpm_at(t):
    for i in range(len(BARS) - 1):
        if t < BARS[i + 1]:
            return 240 / (BARS[i + 1] - BARS[i])
    return TM["bpm1"]


def train_speed(t):
    cruise = 0.7 * K_SPEED * TM["bpm0"]
    if t < T0:
        return cruise
    return lerp(cruise, K_SPEED * bpm_at(min(t, T_HIT - 0.01)), ease_out((t - T0) / 1.2))


DT = 1 / 600
NT = int((DUR + 1) / DT)
XF = [0.0]
for i in range(NT):
    boost = 1 + 3.2 * clamp((i * DT - T_LAUNCH) / 1.1) ** 2
    XF.append(XF[-1] + train_speed(i * DT) * boost * DT)


def table(tab, t):
    i = clamp(t / DT, 0, len(tab) - 2)
    j = int(i)
    return lerp(tab[j], tab[j + 1], i - j)


def xf(t):
    return table(XF, t)


def t_at_x(x):
    j = bisect.bisect_left(XF, x)
    return clamp(j * DT, 0, DUR)


T_CS = T_LAUNCH + 0.3     # camera lets the train go
CX = [0.0]
for i in range(NT):
    t = i * DT
    v = train_speed(t) if t < T_CS else train_speed(T_CS) * math.exp(-(t - T_CS) / 0.22)
    CX.append(CX[-1] + v * DT)


# ------------------------------------------------------------------ the track
XS0, XS1, XSTEP = -4000.0, xf(DUR) + 6000, 4.0
DROP_X = xf(T0) - 250
SIGN_X = xf(1.8) + 400
BASE = []
y = 0.0
x = XS0
while x <= XS1:
    if x > DROP_X:
        s = 0.30 * smooth((x - DROP_X) / 900) + (0.07 * math.sin(x / 1300) + 0.05 * math.sin(x / 530 + 1)) * smooth((x - DROP_X - 900) / 1500)
        y += s * XSTEP
    BASE.append(y)
    x += XSTEP


def base(x):
    i = clamp((x - XS0) / XSTEP, 0, len(BASE) - 2)
    j = int(i)
    return lerp(BASE[j], BASE[j + 1], i - j)


HUMP_A, HUMP_W = 150, 200
JUMPS = []  # (xc, xa, xl, slope_a, k, y_a)
X_END = xf(T_LAUNCH)
RAMP = 700


def track_raw(x, jumps):
    y = base(x)
    for xc, *_ in jumps:
        y -= HUMP_A * math.exp(-((x - xc) / HUMP_W) ** 2)
    if x > X_END - RAMP:
        u = clamp((x - (X_END - RAMP)) / RAMP)
        y -= 260 * u * u
    return y


_hc = []
for tl in LANDINGS:
    xa, xl = xf(tl - AIR), xf(tl)
    _hc.append((xa + 0.3 * HUMP_W,))
for (xc,), tl in zip(_hc, LANDINGS):
    xa, xl = xf(tl - AIR), xf(tl)
    ya = track_raw(xa, _hc)
    sa = (track_raw(xa + 2, _hc) - track_raw(xa - 2, _hc)) / 4
    D = xl - xa
    k = (track_raw(xl, _hc) - ya - sa * D) / (D * D)
    JUMPS.append((xc, xa, xl, sa, k, ya))


def track(x):
    return track_raw(x, JUMPS)


Y_END = track(X_END)
S_END = (track(X_END) - track(X_END - 4)) / 4
K_END = 0.00004


def car_y(x):
    if x > X_END:
        d = x - X_END
        return Y_END + S_END * d + K_END * d * d
    y = track(x)
    for xc, xa, xl, sa, k, ya in JUMPS:
        if xa <= x <= xl:
            d = x - xa
            y = min(y, ya + sa * d + k * d * d)
    return y


TRESTLES = [(JUMPS[0][2] + 1500, JUMPS[0][2] + 3100), (X_END - 2800, X_END)]


def on_trestle(x):
    return any(a <= x <= b for a, b in TRESTLES) or x > X_END


def camx(t):
    return table(CX, t) + xf(0) - LOCO_SX


def camy(t):
    x = camx(t) + LOCO_SX - 150
    u = clamp((x - (X_END - RAMP)) / RAMP)
    return base(x) - 260 * u * u - 800


def tilt(t):
    x = camx(t) + LOCO_SX
    s = (base(x + 200) - base(x - 200)) / 400
    a = -0.22 * math.atan(s)
    for tl in LANDINGS:
        if 0 <= t - tl < 1.2:
            a += 0.035 * math.exp(-(t - tl) * 4) * math.sin((t - tl) * 25)
    return a


# ------------------------------------------------------------------ the mark
def curve(n=160):
    pts = []
    for i in range(n + 1):
        u = i / n
        ph = 2 * math.pi * (1.2 * u + 1.6 * u * u)
        taper = math.sin(math.pi * u) ** 0.6
        pts.append((0.25 + 0.5 * u, 0.5 - 0.15 * taper * math.sin(ph)))
    return pts


CURVE = curve()
LOGO_C = (960, 400)
LOGO_R = 175
LOGO_S = LOGO_R / 0.30


def logo_pt(u):
    i = clamp(u) * (len(CURVE) - 1)
    j = min(int(i), len(CURVE) - 2)
    x = lerp(CURVE[j][0], CURVE[j + 1][0], i - j)
    y = lerp(CURVE[j][1], CURVE[j + 1][1], i - j)
    return (LOGO_C[0] + (x - 0.5) * LOGO_S, LOGO_C[1] + (y - 0.5) * LOGO_S)


def draw_mark(cr, cx, cy, r, alpha=1.0, ring_u=1.0, wave_u=1.0, glow=0.0, col=BRASS):
    s = r / 0.30
    cr.save()
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
        cr.arc(cx, cy, r, -math.pi / 2, -math.pi / 2 + 2 * math.pi * clamp(ring_u))
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


def whistle_level(t):
    lv = 0.0
    for w0, g in ((W_INTRO, 1.0), (T_LAUNCH - 0.9, 0.8), (W_CQ, 1.0), (W_CQ + 0.9, 0.4)):
        if w0 <= t < w0 + 0.8:
            lv = max(lv, g * (0.6 + 0.4 * math.sin(math.pi * ((t - w0) % 0.4) / 0.4)))
        elif w0 + 0.8 <= t < w0 + 1.3:
            lv = max(lv, g * 0.6 * (1 - (t - w0 - 0.8) / 0.5))
    return lv


# ------------------------------------------------------------------ sky & storm
rnd = random.Random(42)
STARS = [(rnd.uniform(0, W * 1.3), rnd.uniform(0, 500), rnd.uniform(0.6, 2.0), rnd.uniform(0, 6.28), rnd.uniform(0.5, 3)) for _ in range(220)]
CLOUDS = [(rnd.uniform(-300, W * 1.6), rnd.uniform(-80, 330), rnd.uniform(160, 380), rnd.uniform(0.6, 1.0)) for _ in range(34)]
FAR = [(rnd.uniform(0.002, 0.009), rnd.uniform(0, 6.28), rnd.uniform(30, 120)) for _ in range(6)]
MID = [(rnd.uniform(0.002, 0.008), rnd.uniform(0, 6.28), rnd.uniform(20, 70)) for _ in range(6)]
RAIN = [(rnd.uniform(0, W + 400), rnd.uniform(0, H), rnd.uniform(0.7, 1.3)) for _ in range(420)]


def flash_level(t):
    lv = 0.0
    for lt in LIGHTNING:
        d = t - lt
        if 0 <= d < 0.6:
            lv = max(lv, math.exp(-d * 9) + 0.6 * math.exp(-max(0, d - 0.12) * 14) * (d > 0.12))
    return clamp(lv, 0, 1.2)


def ridge(x, waves, b, sharp=False):
    if sharp:
        return b - sum(a * (1 - abs(math.sin(f * x + p))) ** 2 * 1.6 for f, p, a in waves)
    return b - sum(a * math.sin(f * x + p) for f, p, a in waves)


def draw_sky(cr, t, fl):
    g = cairo.LinearGradient(0, -200, 0, H)
    g.add_color_stop_rgb(0, *mix((0.02, 0.025, 0.045), (0.55, 0.6, 0.8), fl * 0.6))
    g.add_color_stop_rgb(0.6, *mix((0.09, 0.11, 0.16), (0.6, 0.65, 0.85), fl * 0.5))
    g.add_color_stop_rgb(1, *mix((0.05, 0.05, 0.07), (0.3, 0.32, 0.45), fl * 0.4))
    cr.set_source(g)
    cr.paint()
    off = camx(t) * 0.004
    for x, yy, r, ph, sp in STARS:
        a = (0.25 + 0.5 * (0.5 + 0.5 * math.sin(t * sp + ph))) * (1 - yy / 520)
        cr.set_source_rgba(1, 0.97, 0.9, a)
        cr.arc((x - off) % (W * 1.3) - 100, yy, r * 0.8, 0, 2 * math.pi)
        cr.fill()
    mx, my = 1540, 170
    g = cairo.RadialGradient(mx, my, 20, mx, my, 300)
    g.add_color_stop_rgba(0, 1, 0.92, 0.78, 0.2)
    g.add_color_stop_rgba(1, 1, 0.92, 0.78, 0)
    cr.set_source(g)
    cr.arc(mx, my, 300, 0, 2 * math.pi)
    cr.fill()
    cr.set_source_rgb(0.95, 0.92, 0.82)
    cr.arc(mx, my, 48, 0, 2 * math.pi)
    cr.fill()


def draw_clouds(cr, t, fl):
    off = camx(t) * 0.03 + t * 25
    for x, yy, r, d in CLOUDS:
        xx = (x - off) % (W * 1.6 + 600) - 400
        col = mix((0.07, 0.08, 0.11), (0.75, 0.78, 0.92), fl * 0.7)
        g = cairo.RadialGradient(xx, yy, 0, xx, yy, r)
        g.add_color_stop_rgba(0, *col, 0.85 * d)
        g.add_color_stop_rgba(0.6, *col, 0.55 * d)
        g.add_color_stop_rgba(1, *col, 0)
        cr.set_source(g)
        cr.arc(xx, yy, r, 0, 2 * math.pi)
        cr.fill()


def draw_bolt(cr, t):
    for i, lt in enumerate(LIGHTNING):
        d = t - lt
        if not 0 <= d < 0.22 or (0.06 < d < 0.12):
            continue
        r = random.Random(100 + i)
        x = r.uniform(250, 1650)
        y = -20
        pts = [(x, y)]
        while y < r.uniform(380, 560):
            x += r.uniform(-60, 60)
            y += r.uniform(25, 60)
            pts.append((x, y))
        cr.save()
        cr.set_operator(cairo.OPERATOR_ADD)
        for wdt, a in ((22, 0.12), (9, 0.35), (3, 1.0)):
            cr.set_line_width(wdt)
            cr.set_source_rgba(0.8, 0.85, 1.0, a)
            cr.move_to(*pts[0])
            for p in pts[1:]:
                cr.line_to(*p)
            cr.stroke()
        # a branch
        bx, by = pts[len(pts) // 2]
        cr.set_line_width(2)
        cr.set_source_rgba(0.8, 0.85, 1.0, 0.7)
        cr.move_to(bx, by)
        for _ in range(4):
            bx += r.uniform(10, 50) * (1 if i % 2 else -1)
            by += r.uniform(20, 45)
            cr.line_to(bx, by)
        cr.stroke()
        cr.restore()


def draw_ridges(cr, t, fl):
    for par, waves, b, sharp, col, snow in ((0.03, FAR, 470, True, (0.09, 0.1, 0.14), True),
                                           (0.12, MID, 600, False, (0.035, 0.045, 0.06), False)):
        off = camx(t) * par
        pts = [(x, ridge(x + off, waves, b, sharp)) for x in range(-300, W + 309, 10)]
        top = min(p[1] for p in pts)
        cr.move_to(-300, H + 600)
        for p in pts:
            cr.line_to(*p)
        cr.line_to(W + 300, H + 600)
        cr.close_path()
        if snow:
            g = cairo.LinearGradient(0, top, 0, top + 170)
            g.add_color_stop_rgb(0, *mix((0.55, 0.58, 0.66), (0.85, 0.88, 1.0), fl))
            g.add_color_stop_rgb(0.45, *mix(col, (0.4, 0.42, 0.55), fl * 0.5))
            g.add_color_stop_rgb(1, *col)
            cr.set_source(g)
        else:
            cr.set_source_rgb(*mix(col, (0.15, 0.17, 0.25), fl * 0.6))
        cr.fill()
        if not snow:  # pine fringe on the mid ridge
            cr.set_source_rgb(*mix((0.02, 0.028, 0.038), (0.1, 0.12, 0.18), fl * 0.5))
            k0 = int((off - 300) // 26)
            for k in range(k0, k0 + int((W + 600) / 26) + 2):
                x = k * 26 - off
                yy = ridge(x + off, waves, b)
                h = 30 + 26 * ((k * 7919) % 13) / 13
                cr.move_to(x - 9, yy + 4)
                cr.line_to(x, yy - h)
                cr.line_to(x + 9, yy + 4)
            cr.fill()


# ------------------------------------------------------------------ world: ground, trestles, track
def w2s(x, y, cx, cy):
    return x - cx, y - cy


def draw_world(cr, t, fl):
    cx, cy = camx(t), camy(t)
    x0, x1 = cx - 400, cx + W + 400
    xs = [x0 + i * 12 for i in range(int((x1 - x0) / 12) + 1)]
    # ground under the track (skip trestles and the canyon)
    runs, cur = [], []
    for x in xs:
        if on_trestle(x):
            if cur:
                runs.append(cur)
                cur = []
        else:
            cur.append(x)
    if cur:
        runs.append(cur)
    rock = mix((0.05, 0.055, 0.07), (0.25, 0.27, 0.35), fl * 0.5)
    for run in runs:
        cr.move_to(run[0] - cx, H + 800)
        for x in run:
            cr.line_to(x - cx, track(x) - cy + 22)
        # cliff faces at trestle ends
        cr.line_to(run[-1] - cx + 40, track(run[-1]) - cy + 300)
        cr.line_to(run[-1] - cx + 10, H + 800)
        cr.close_path()
        g = cairo.LinearGradient(0, track(run[0]) - cy, 0, track(run[0]) - cy + 900)
        g.add_color_stop_rgb(0, *mix(rock, (0.16, 0.17, 0.22), 0.6))
        g.add_color_stop_rgb(0.35, *rock)
        g.add_color_stop_rgb(1, 0.01, 0.012, 0.016)
        cr.set_source(g)
        cr.fill()
        # strata and boulders following the slope
        cr.set_line_width(2)
        cr.set_source_rgba(*mix((0.12, 0.13, 0.17), (0.4, 0.42, 0.52), fl * 0.5), 0.7)
        for depth in (70, 150, 260, 400):
            first = True
            for x in run[::3]:
                yy = track(x) - cy + depth + 18 * math.sin(x / 210 + depth)
                if first:
                    cr.move_to(x - cx, yy)
                    first = False
                else:
                    cr.line_to(x - cx, yy)
            cr.stroke()
        for x in run[::9]:
            hsh = ((int(x // 108) * 2654435761) >> 9) % 1000 / 1000
            if hsh > 0.55:
                bx = int(x // 108) * 108 + 40
                if run[0] <= bx <= run[-1]:
                    by = track(bx) - cy + 60 + 260 * hsh
                    cr.set_source_rgba(*mix((0.1, 0.11, 0.14), (0.35, 0.37, 0.46), fl * 0.5), 0.9)
                    cr.arc(bx - cx, by, 14 + 30 * hsh, math.pi, 2 * math.pi)
                    cr.close_path()
                    cr.fill()
    # pines along the slope (behind the train)
    sp = 120
    k0 = int(x0 // sp)
    for k in range(k0, int(x1 // sp) + 1):
        h = ((k * 2654435761) >> 7) % 1000 / 1000
        if h < 0.35:
            continue
        x = k * sp + h * 60
        if on_trestle(x):
            continue
        gy = track(x) - cy - 8
        hh = 110 + 180 * h
        sx = x - cx
        cr.set_source_rgb(*mix((0.015, 0.025, 0.03), (0.12, 0.15, 0.2), fl * 0.5))
        for tier in range(4):
            ty = gy - hh * tier / 4.5
            wdt = hh * 0.32 * (1 - tier / 5)
            cr.move_to(sx - wdt, ty)
            cr.line_to(sx, ty - hh * 0.42)
            cr.line_to(sx + wdt, ty)
        cr.fill()
    # trestles: timber bents down into the canyon
    for a, b in TRESTLES:
        if b < x0 or a > x1:
            continue
        cr.set_line_width(7)
        wood = mix((0.16, 0.1, 0.06), (0.5, 0.42, 0.35), fl * 0.5)
        cr.set_source_rgb(*wood)
        bx = a - (a % 90)
        while bx <= b:
            if x0 < bx < x1:
                ty = track(bx) - cy + 20
                cr.move_to(bx - cx - 14, ty)
                cr.line_to(bx - cx - 70, ty + 2200)
                cr.move_to(bx - cx + 14, ty)
                cr.line_to(bx - cx + 70, ty + 2200)
            bx += 90
        cr.stroke()
        cr.set_line_width(4)
        bx = a - (a % 90)
        while bx + 90 <= b:
            if x0 < bx < x1:
                ty1, ty2 = track(bx) - cy + 20, track(bx + 90) - cy + 20
                for lvl in range(0, 2100, 150):
                    cr.move_to(bx - cx, ty1 + lvl)
                    cr.line_to(bx + 90 - cx, ty2 + lvl + 150)
                    cr.move_to(bx - cx, ty1 + lvl + 150)
                    cr.line_to(bx + 90 - cx, ty2 + lvl)
            bx += 90
        cr.stroke()
        # canyon mist
        g = cairo.LinearGradient(0, track((a + b) / 2) - cy + 300, 0, track((a + b) / 2) - cy + 900)
        g.add_color_stop_rgba(0, 0.3, 0.35, 0.45, 0)
        g.add_color_stop_rgba(1, 0.3, 0.35, 0.45, 0.35)
        cr.set_source(g)
        cr.rectangle(max(a, x0) - cx, -2000, min(b, x1) - max(a, x0), 6000)
        cr.fill()
    # summit sign
    sxs = SIGN_X - cx
    if -300 < sxs < W + 300:
        gy = track(SIGN_X) - cy
        cr.set_source_rgb(0.12, 0.08, 0.05)
        cr.rectangle(sxs - 110, gy - 150, 10, 150)
        cr.rectangle(sxs + 100, gy - 150, 10, 150)
        cr.fill()
        cr.set_source_rgb(0.08, 0.06, 0.045)
        rrect(cr, sxs - 150, gy - 230, 300, 92, 8)
        cr.fill()
        cr.set_source_rgb(*BRASS)
        cr.set_line_width(3)
        rrect(cr, sxs - 144, gy - 224, 288, 80, 6)
        cr.stroke()
        pos, _ = text_spaced(cr, "WHISTLE STOP", sxs, 0, 29, 3)
        for ch, xx in pos:
            cr.move_to(xx, gy - 174)
            cr.show_text(ch)
    # ties and rail
    v = train_speed(t) if t < T_CS else 0
    smear = v / FPS
    tie_a = clamp(1 - smear / 90, 0.3, 1)
    sp = 52
    k0 = int(x0 // sp)
    cr.set_source_rgba(0.24, 0.17, 0.12, tie_a)
    for k in range(k0, int(min(x1, X_END) // sp) + 1):
        x = k * sp
        if x > X_END - 120 and (k % 3 == 0):
            continue  # broken ties at the end
        y1 = track(x) - cy
        y2 = track(x + 18 + smear) - cy
        cr.move_to(x - cx, y1 + 4)
        cr.line_to(x + 18 + smear - cx, y2 + 4)
        cr.line_to(x + 18 + smear - cx, y2 + 16)
        cr.line_to(x - cx, y1 + 16)
        cr.close_path()
    cr.fill()
    cr.set_line_join(cairo.LINE_JOIN_ROUND)
    rail = [x for x in xs if x <= X_END]
    if rail:
        cr.move_to(rail[0] - cx, track(rail[0]) - cy)
        for x in rail[1:] + [min(X_END, x1)]:
            cr.line_to(x - cx, track(x) - cy)
        cr.set_source_rgb(0.3, 0.32, 0.36)
        cr.set_line_width(8)
        cr.stroke_preserve()
        cr.set_source_rgba(*mix((0.7, 0.74, 0.82), (1, 1, 1), fl), 0.9)
        cr.set_line_width(2)
        cr.stroke()
        # broken rail end, twisted up
        if x0 < X_END < x1:
            ex, ey = X_END - cx, Y_END - cy
            cr.set_source_rgb(0.5, 0.52, 0.58)
            cr.set_line_width(5)
            cr.move_to(ex, ey)
            cr.curve_to(ex + 40, ey - 30, ex + 60, ey - 90, ex + 45, ey - 130)
            cr.stroke()


# ------------------------------------------------------------------ train drawing
def rrect(cr, x, y, w, h, r):
    cr.new_sub_path()
    cr.arc(x + w - r, y + r, r, -math.pi / 2, 0)
    cr.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
    cr.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
    cr.arc(x + r, y + r, r, math.pi, 1.5 * math.pi)
    cr.close_path()


def metal(y0, y1, base=IRON, rim=0.22):
    g = cairo.LinearGradient(0, y0, 0, y1)
    g.add_color_stop_rgb(0, *mix(base, (0.6, 0.65, 0.78), rim))
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


def wheel(cr, cx, cy, r, ang, dang, spokes=12, hub=BRASS):
    cr.set_source_rgb(0.06, 0.065, 0.08)
    cr.arc(cx, cy, r, 0, 2 * math.pi)
    cr.fill()
    cr.set_source_rgb(0.45, 0.47, 0.52)
    cr.set_line_width(max(3, r * 0.1))
    cr.arc(cx, cy, r - r * 0.06, 0, 2 * math.pi)
    cr.stroke()
    sub = 6 if abs(dang) > 0.08 else 1
    cr.set_line_width(max(2, r * 0.075))
    for j in range(sub):
        a0 = ang - dang * j / sub
        cr.set_source_rgba(0.55, 0.12, 0.08, (1.6 / sub) if sub > 1 else 1)
        for k in range(spokes):
            a = a0 + 2 * math.pi * k / spokes
            cr.move_to(cx + r * 0.18 * math.cos(a), cy + r * 0.18 * math.sin(a))
            cr.line_to(cx + r * 0.86 * math.cos(a), cy + r * 0.86 * math.sin(a))
        cr.stroke()
    cr.set_source_rgb(*hub)
    cr.arc(cx, cy, r * 0.17, 0, 2 * math.pi)
    cr.fill()


DRIVERS = [-330, -455, -580]
DR = 58
CARS = [  # (front pivot, rear pivot, kind)
    (-150, -580, "loco"),
    (-800, -1040, "tender"),
    (-1180, -1620, "ROUNDTABLE"),
    (-1770, -2210, "FILE EXPRESS"),
]


def car_tf(t, p1, p2):
    x = xf(t)
    x1, x2 = x + p1, x + p2
    y1, y2 = car_y(x1), car_y(x2)
    return x1, y1, math.atan2(y1 - y2, x1 - x2)


def push_car(cr, t, p1, p2, cx, cy):
    x1, y1, a = car_tf(t, p1, p2)
    cr.save()
    cr.translate(x1 - cx, y1 - cy)
    cr.rotate(a)
    cr.translate(-p1, 0)
    return a


def local_to_screen(t, p1, p2, lx, ly, cx, cy):
    x1, y1, a = car_tf(t, p1, p2)
    dx, dy = lx - p1, ly
    return x1 - cx + dx * math.cos(a) - dy * math.sin(a), y1 - cy + dx * math.sin(a) + dy * math.cos(a)


def draw_coach(cr, t, cx0, cx1, label, ci, dist, dang):
    Y = 0
    cr.set_source(metal(Y - 320, Y - 110, (0.3, 0.07, 0.05), 0.25))
    rrect(cr, cx0, Y - 300, cx1 - cx0, 190, 10)
    cr.fill()
    cr.set_source(metal(Y - 345, Y - 296, (0.09, 0.09, 0.11), 0.35))
    cr.move_to(cx0 - 8, Y - 298)
    cr.curve_to(cx0 + 20, Y - 330, cx1 - 20, Y - 330, cx1 + 8, Y - 298)
    cr.close_path()
    cr.fill()
    cr.rectangle(cx0 + 60, Y - 345, cx1 - cx0 - 120, 18)
    cr.fill()
    for wi in range(8):
        wx = cx0 + 36 + wi * 64
        fl = 0.85 + 0.15 * math.sin(t * 9 + wi * 2.1 + ci)
        g = cairo.LinearGradient(0, Y - 280, 0, Y - 220)
        g.add_color_stop_rgb(0, 1.0 * fl, 0.8 * fl, 0.45 * fl)
        g.add_color_stop_rgb(1, 0.85 * fl, 0.52 * fl, 0.22 * fl)
        cr.set_source(g)
        rrect(cr, wx, Y - 282, 42, 58, 6)
        cr.fill()
        if (wi * 7 + ci * 3) % 3 == 0:  # passengers, arms up
            cr.set_source_rgba(0.18, 0.1, 0.06, 0.85)
            cr.arc(wx + 21, Y - 245, 9, 0, 2 * math.pi)
            cr.fill()
            cr.rectangle(wx + 9, Y - 237, 24, 14)
            cr.fill()
            if t > T0 + 2:
                cr.set_line_width(4)
                cr.move_to(wx + 11, Y - 240)
                cr.line_to(wx + 6, Y - 270 + 4 * math.sin(t * 20 + wi))
                cr.move_to(wx + 31, Y - 240)
                cr.line_to(wx + 36, Y - 270 + 4 * math.sin(t * 20 + wi + 1))
                cr.stroke()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(cx0 + 6, Y - 212, cx1 - cx0 - 12, 4)
    cr.rectangle(cx0 + 6, Y - 134, cx1 - cx0 - 12, 3)
    cr.fill()
    pos, _ = text_spaced(cr, label, (cx0 + cx1) / 2, 0, 26, 9)
    for ch, xx in pos:
        cr.move_to(xx, Y - 162)
        cr.show_text(ch)
    for bx in (cx0 + 70, cx0 + 130, cx1 - 130, cx1 - 70):
        wheel(cr, bx, Y - 26, 26, dist / 26, dang * DR / 26, 8)
    cr.set_source_rgb(0.05, 0.05, 0.06)
    cr.rectangle(cx0 + 40, Y - 112, 120, 30)
    cr.rectangle(cx1 - 160, Y - 112, 120, 30)
    cr.rectangle(cx1, Y - 150, 34, 10)
    cr.fill()


def draw_tender(cr, t, dist, dang):
    Y = 0
    tx0, tx1 = -1100, -742
    cr.set_source_rgb(0.05, 0.05, 0.06)
    cr.rectangle(tx1 - 5, Y - 150, 34, 10)
    cr.fill()
    cr.set_source_rgb(0.04, 0.04, 0.05)
    cr.move_to(tx0 + 20, Y - 282)
    for i in range(13):
        cr.line_to(tx0 + 30 + i * 25, Y - 302 - 10 * math.sin(i * 1.9) ** 2)
    cr.line_to(tx1 - 20, Y - 282)
    cr.fill()
    cr.set_source(metal(Y - 285, Y - 110))
    rrect(cr, tx0, Y - 285, tx1 - tx0, 175, 8)
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(tx0 + 8, Y - 272, tx1 - tx0 - 16, 3)
    cr.rectangle(tx0 + 8, Y - 128, tx1 - tx0 - 16, 3)
    cr.fill()
    pos, _ = text_spaced(cr, "WHISTLESTOP", (tx0 + tx1) / 2, 0, 32, 6)
    for ch, xx in pos:
        cr.move_to(xx, Y - 182)
        cr.text_path(ch)
    cr.set_source(metal(Y - 215, Y - 180, (0.95, 0.66, 0.23), 0.6))
    cr.fill()
    cr.set_source_rgba(*BRASS, 0.85)
    cr.select_font_face("Avenir Next", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(15)
    e = cr.text_extents("No. 7  ·  HF  ·  MERCURY  ·  CQ")
    cr.move_to((tx0 + tx1) / 2 - e.width / 2, Y - 152)
    cr.show_text("No. 7  ·  HF  ·  MERCURY  ·  CQ")
    for bx in (tx0 + 55, tx0 + 115, tx1 - 115, tx1 - 55):
        wheel(cr, bx, Y - 26, 26, dist / 26, dang * DR / 26, 8)


def draw_loco(cr, t, dist, dang):
    """A 4-6-0 ten-wheeler with a diamond stack and a big box headlamp."""
    Y = 0
    ang = dist / DR
    by0, by1 = Y - 268, Y - 140
    cr.set_source_rgb(0.05, 0.055, 0.07)
    cr.rectangle(-660, Y - 142, 580, 40)
    cr.fill()
    fl = 0.7 + 0.3 * math.sin(t * 23) * math.sin(t * 7.3)
    g = cairo.RadialGradient(-560, Y - 110, 5, -560, Y - 110, 130)
    g.add_color_stop_rgba(0, 1, 0.55, 0.15, 0.95 * fl)
    g.add_color_stop_rgba(1, 1, 0.3, 0.05, 0)
    cr.set_source(g)
    cr.arc(-560, Y - 110, 130, 0, 2 * math.pi)
    cr.fill()
    # boiler + smokebox
    for x_from, col, rim in ((-600, IRON, 0.22), (-220, (0.06, 0.065, 0.08), 0.18)):
        cr.set_source(metal(by0, by1, col, rim))
        cr.move_to(x_from, by0)
        cr.line_to(-140, by0)
        cr.curve_to(-112, by0, -100, by0 + 20, -100, (by0 + by1) / 2)
        cr.curve_to(-100, by1 - 20, -112, by1, -140, by1)
        cr.line_to(x_from, by1)
        cr.close_path()
        cr.fill()
    cr.set_source_rgb(*BRASS)
    for bx in (-220, -335, -450, -575):
        cr.rectangle(bx, by0, 7, by1 - by0)
    cr.fill()
    # diamond stack
    cr.set_source(metal(Y - 400, by0, (0.07, 0.075, 0.09), 0.35))
    cr.move_to(-186, by0 + 4)
    cr.line_to(-184, Y - 318)
    cr.line_to(-236, Y - 366)
    cr.line_to(-205, Y - 400)
    cr.line_to(-135, Y - 400)
    cr.line_to(-104, Y - 366)
    cr.line_to(-156, Y - 318)
    cr.line_to(-154, by0 + 4)
    cr.close_path()
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(-208, Y - 406, 76, 8)
    cr.rectangle(-188, Y - 322, 36, 6)
    cr.fill()
    # domes, bell, whistle
    for dx, w_, h_ in ((-300, 44, 34), (-410, 58, 46)):
        cr.set_source(metal(by0 - h_, by0 + 4, (0.75, 0.52, 0.18), 0.5))
        cr.move_to(dx - w_, by0 + 2)
        cr.curve_to(dx - w_ * 0.6, by0 - h_ * 1.1, dx + w_ * 0.6, by0 - h_ * 1.1, dx + w_, by0 + 2)
        cr.close_path()
        cr.fill()
    cr.set_source_rgb(*BRASS)
    swing = 0.35 * math.sin(t * 9) if t > T0 or 0.8 < t < 1.6 else 0
    cr.save()
    cr.translate(-250, by0 - 46)
    cr.rotate(swing)
    cr.move_to(-13, 40)
    cr.curve_to(-13, 6, 13, 6, 13, 40)
    cr.close_path()
    cr.fill()
    cr.restore()
    cr.rectangle(-253, by0 - 48, 6, 10)
    cr.rectangle(-596, by0 - 34, 8, 34)
    cr.fill()
    # box headlamp
    cr.set_source(metal(Y - 340, by0 + 4, (0.12, 0.1, 0.08), 0.3))
    rrect(cr, -160, Y - 338, 66, 64, 6)
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.set_line_width(3)
    rrect(cr, -160, Y - 338, 66, 64, 6)
    cr.stroke()
    cr.rectangle(-140, Y - 352, 26, 14)
    cr.fill()
    cr.set_source_rgb(1, 0.95, 0.8)
    cr.arc(-96, Y - 306, 20, -math.pi / 2, math.pi / 2)
    cr.fill()
    # cab (red)
    cr.set_source(metal(Y - 360, Y - 140, RED, 0.25))
    rrect(cr, -745, Y - 352, 152, 214, 6)
    cr.fill()
    cr.set_source(metal(Y - 378, Y - 348, (0.06, 0.065, 0.08), 0.4))
    cr.move_to(-762, Y - 350)
    cr.curve_to(-724, Y - 382, -612, Y - 382, -576, Y - 350)
    cr.close_path()
    cr.fill()
    flk = 0.85 + 0.15 * math.sin(t * 17)
    g = cairo.LinearGradient(0, Y - 330, 0, Y - 262)
    g.add_color_stop_rgb(0, 1 * flk, 0.78 * flk, 0.42 * flk)
    g.add_color_stop_rgb(1, 0.95 * flk, 0.48 * flk, 0.15 * flk)
    cr.set_source(g)
    rrect(cr, -722, Y - 330, 100, 66, 6)
    cr.fill()
    # engineer leaning out, cap on, one hand on the whistle cord
    lean = 16 * smooth((t - T0) / 1.5)
    cr.set_source_rgba(0.13, 0.07, 0.035, 0.95)
    cr.arc(-640 + lean, Y - 296, 13, 0, 2 * math.pi)
    cr.fill()
    cr.move_to(-660 + lean, Y - 304)
    cr.line_to(-618 + lean, Y - 304)
    cr.line_to(-622 + lean, Y - 316)
    cr.line_to(-655 + lean, Y - 318)
    cr.close_path()
    cr.fill()
    cr.rectangle(-656 + lean * 0.6, Y - 283, 30, 20)
    cr.fill()
    cr.set_line_width(5)
    cr.move_to(-632 + lean * 0.6, Y - 282)
    cr.line_to(-606 + lean * 0.4, Y - 330 + 10 * whistle_level(t))
    cr.stroke()
    draw_mark(cr, -669, Y - 198, 38, 1.0, 1, 1, 0.25 + 0.6 * whistle_level(t))
    cr.set_source_rgb(*BRASS)
    cr.rectangle(-600, by1, 510, 5)
    cr.fill()
    # pilot
    cr.set_source_rgb(*RED)
    cr.rectangle(-110, Y - 140, 34, 42)
    cr.fill()
    cr.set_source_rgb(0.07, 0.075, 0.09)
    cr.move_to(-110, Y - 98)
    cr.line_to(-70, Y - 98)
    cr.line_to(22, Y - 10)
    cr.line_to(-110, Y - 10)
    cr.close_path()
    cr.fill()
    cr.set_source_rgb(0.34, 0.36, 0.4)
    cr.set_line_width(3)
    for xx in range(-104, 14, 13):
        top = Y - 98 if xx <= -70 else Y - 98 + (xx + 70) * 88 / 92
        cr.move_to(xx, Y - 12)
        cr.line_to(xx, top + 2)
    cr.stroke()
    cr.set_source_rgb(*BRASS)
    cr.move_to(-110, Y - 98)
    cr.line_to(-70, Y - 98)
    cr.line_to(22, Y - 10)
    cr.stroke()
    # wheels: 4 leading (2 visible), 6 drivers (3 visible)
    for lx in (-120, -205):
        wheel(cr, lx, Y - 28, 28, dist / 28, dang * DR / 28, 8)
    for dx in DRIVERS:
        wheel(cr, dx, Y - DR, DR, ang, dang, 14)
    cyy = Y - DR
    cr.set_source(metal(cyy - 30, cyy + 30, (0.08, 0.085, 0.1), 0.35))
    rrect(cr, -268, cyy - 30, 96, 60, 8)
    cr.fill()
    cr.set_source_rgb(*BRASS)
    cr.rectangle(-264, cyy - 30, 5, 60)
    cr.rectangle(-181, cyy - 30, 5, 60)
    cr.fill()
    crank = 24
    pins = [(dx + crank * math.cos(ang), cyy + crank * math.sin(ang)) for dx in DRIVERS]
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_source_rgb(0.65, 0.67, 0.73)
    cr.set_line_width(10)
    cr.move_to(*pins[0])
    for p in pins[1:]:
        cr.line_to(*p)
    cr.stroke()
    px, py = pins[1]
    L = 180
    xh = px + math.sqrt(max(L * L - (py - cyy) ** 2, 1))
    cr.set_source_rgb(0.75, 0.77, 0.83)
    cr.set_line_width(12)
    cr.move_to(px, py)
    cr.line_to(xh, cyy)
    cr.stroke()
    cr.set_line_width(7)
    cr.move_to(xh, cyy)
    cr.line_to(-195, cyy)
    cr.stroke()
    cr.set_source_rgb(0.35, 0.37, 0.42)
    cr.rectangle(xh - 14, cyy - 13, 28, 26)
    cr.fill()
    cr.set_source_rgb(*BRASS)
    for p in pins:
        cr.arc(p[0], p[1], 7, 0, 2 * math.pi)
        cr.fill()


def draw_train(cr, t, cx, cy):
    dist = xf(t)
    dang = (xf(t) - xf(t - 1 / FPS)) / DR
    for p1, p2, kind in reversed(CARS):
        sx = xf(t) + p1 - cx
        if sx < -900 or sx > W + 2400:
            continue
        push_car(cr, t, p1, p2, cx, cy)
        if kind == "loco":
            draw_loco(cr, t, dist, dang)
        elif kind == "tender":
            draw_tender(cr, t, dist, dang)
        else:
            ci = 0 if kind == "ROUNDTABLE" else 1
            x1 = -1110 if ci == 0 else -1700
            draw_coach(cr, t, x1 - 570, x1, kind, ci, dist, dang)
        cr.restore()
    # headlamp beam
    lx, ly = local_to_screen(t, -150, -580, -96, -306, cx, cy)
    _, _, a = car_tf(t, -150, -580)
    cr.save()
    cr.translate(lx, ly)
    cr.rotate(a)
    cr.set_operator(cairo.OPERATOR_ADD)
    g = cairo.LinearGradient(0, 0, 1600, 0)
    g.add_color_stop_rgba(0, 1, 0.9, 0.65, 0.32)
    g.add_color_stop_rgba(1, 1, 0.9, 0.65, 0)
    cr.set_source(g)
    cr.move_to(0, -14)
    cr.line_to(1600, -300)
    cr.line_to(1600, 380)
    cr.line_to(0, 16)
    cr.close_path()
    cr.fill()
    g = cairo.RadialGradient(0, 0, 0, 0, 0, 170)
    g.add_color_stop_rgba(0, 1, 0.95, 0.8, 0.9)
    g.add_color_stop_rgba(0.15, 1, 0.85, 0.55, 0.35)
    g.add_color_stop_rgba(1, 1, 0.8, 0.5, 0)
    cr.set_source(g)
    cr.arc(0, 0, 170, 0, 2 * math.pi)
    cr.fill()
    cr.restore()


# ------------------------------------------------------------------ smoke & sparks
pr = random.Random(9)
PUFFS = []
t_ = 0.2
while t_ < T0:  # rolling across the summit
    PUFFS.append((t_, 0.9, [(pr.uniform(-22, 22), pr.uniform(-14, 14), pr.uniform(0.8, 1.25)) for _ in range(4)]))
    t_ += 0.2
_sm = TM["smoke"]
for k, te in enumerate(_sm):
    for tt_ in [te] + ([(te + _sm[k + 1]) / 2] if k + 1 < len(_sm) else []):
        if tt_ < T_HIT - 0.15:
            PUFFS.append((tt_, 1.0, [(pr.uniform(-26, 26), pr.uniform(-16, 16), pr.uniform(0.75, 1.3)) for _ in range(4)]))
STEAM = [(2.45 + i * 0.05, i) for i in range(8)]  # brake release, low white steam


def stack_world(te):
    x1, y1, a = car_tf(te, -150, -580)
    dx, dy = -170 - (-150), -405
    return x1 + dx * math.cos(a) - dy * math.sin(a), y1 + dx * math.sin(a) + dy * math.cos(a)


def puff_state(p, t):
    te, size, _ = p
    tau = t - te
    if tau < 0:
        return None
    x0, y0 = stack_world(te)
    v = train_speed(te)
    x = x0 + v * 0.15 * (1 - math.exp(-tau / 0.15)) - 40 * tau
    y = y0 - (200 * size) * (1 - math.exp(-tau / 0.9)) - 25 * tau
    rad = (20 + 100 * (1 - math.exp(-tau / 0.8))) * size
    a = 0.45 * math.exp(-tau / 1.1) * clamp(tau / 0.05)
    return x, y, rad, a


def blob(cr, x, y, r, a, col):
    g = cairo.RadialGradient(x, y, 0, x, y, r)
    g.add_color_stop_rgba(0, *col, a)
    g.add_color_stop_rgba(0.6, *col, a * 0.55)
    g.add_color_stop_rgba(1, *col, 0)
    cr.set_source(g)
    cr.arc(x, y, r, 0, 2 * math.pi)
    cr.fill()


T_M = T_HIT - 0.35
_morph = None


def morph_set():
    global _morph
    if _morph is None:
        cx = camx(T_M)
        blobs = []
        for p in PUFFS:
            st = puff_state(p, T_M)
            if st and st[3] > 0.05:
                for b in p[2]:
                    blobs.append((p, b, st[0] - cx + b[0] * st[2] / 40))
        blobs.sort(key=lambda z: z[2])
        n = max(1, len(blobs))
        _morph = [(p, b, logo_pt((i + 0.5) / n)) for i, (p, b, _) in enumerate(blobs)]
    return _morph


def draw_smoke(cr, t, cx, cy, fl):
    morphing = t >= T_M
    mset = morph_set() if morphing else []
    mkeys = {id(p) for p, _, _ in mset}
    u = smooth((t - T_M) / 1.25) if morphing else 0
    lit = (0.5, 0.52, 0.6)
    for p in PUFFS:
        if morphing and id(p) in mkeys:
            continue
        st = puff_state(p, t)
        if not st or st[3] < 0.005:
            continue
        x, y, rad, a = st
        if morphing:
            a *= 1 - smooth((t - T_M) / 0.5)
        col = mix(mix((0.28, 0.29, 0.33), (0.66, 0.68, 0.76), clamp((t - p[0]) / 0.9)), (0.9, 0.92, 1.0), fl * 0.5)
        for bx, by, bs in p[2]:
            blob(cr, x - cx + bx * rad / 40, y - cy + by * rad / 40, rad * bs, a, col)
    # brake-release steam from the cylinders
    for te, i in STEAM:
        tau = t - te
        if 0 <= tau < 1.6:
            sx, sy = local_to_screen(t, -150, -580, -230 + i * 6, -40, cx, cy)
            blob(cr, sx + 60 * tau + i * 10, sy - 50 * tau, 30 + 90 * tau, 0.5 * (1 - tau / 1.6), (0.85, 0.87, 0.92))
    if morphing:
        fade = 1 - smooth((t - (T_HIT + 0.75)) / 0.5)
        cxm = camx(T_M)
        for p, b, (tx, ty) in mset:
            x, y, rad, a = puff_state(p, min(t, T_M + 0.6))
            x0 = x - cx + b[0] * rad / 40
            y0 = y - cy + b[1] * rad / 40
            r = lerp(rad * b[2], 20, u)
            aa = lerp(a, 0.8, u) * fade
            col = mix(lit, BRASS_HI, u)
            if aa > 0.004:
                blob(cr, lerp(x0, tx, u), lerp(y0, ty, u), r, aa, col)


SPARKS = []
sr_ = random.Random(3)
tt = T0 + 1.5
while tt < T_LAUNCH:
    tt += sr_.expovariate(10 + 24 * (tt - T0) / (T_HIT - T0))
    SPARKS.append((tt, sr_.choice(DRIVERS), sr_.uniform(0.3, 0.8), sr_.uniform(150, 480), sr_.uniform(0.25, 0.45)))
# landing bursts: each car's wheels hit the rail as it reaches the landing point
for xc, xa, xl, *_ in JUMPS:
    for p1, p2, kind in CARS:
        for px in (p1, p2):
            tl = t_at_x(xl - px)
            for _ in range(30 if kind == "loco" else 16):
                SPARKS.append((tl + sr_.uniform(0, 0.08), px, sr_.uniform(0.1, 0.9), sr_.uniform(250, 900), sr_.uniform(0.35, 0.7)))


def draw_sparks(cr, t, cx, cy):
    cr.save()
    cr.set_operator(cairo.OPERATOR_ADD)
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_width(2.4)
    for te, dx, vxf, vy, life in SPARKS:
        tau = t - te
        if not 0 <= tau < life:
            continue
        x0 = xf(te) + dx
        y0 = car_y(x0)
        vx = train_speed(te) * vxf

        def pos(q):
            return x0 + vx * q - cx, y0 - vy * q + 1400 * q * q - cy

        x1, y1 = pos(tau)
        x2, y2 = pos(max(0, tau - 0.025))
        k = 1 - tau / life
        cr.set_source_rgba(1, 0.72 + 0.25 * k, 0.3, 0.95 * k)
        cr.move_to(x2, y2)
        cr.line_to(x1, y1)
        cr.stroke()
    cr.restore()


def draw_rain(cr, t, fl):
    cr.set_line_width(1.4)
    cr.set_source_rgba(*mix((0.55, 0.6, 0.72), (0.9, 0.93, 1.0), fl), 0.22 + 0.2 * fl)
    v = train_speed(t) if t < T_CS else 0
    for x, y, s in RAIN:
        yy = (y + t * 1700 * s) % (H + 100) - 50
        xx = (x - t * (260 + v * 0.35) * s) % (W + 400) - 200
        cr.move_to(xx, yy)
        cr.line_to(xx - 10 - v * 0.012, yy + 38 * s)
    cr.stroke()


# ------------------------------------------------------------------ text
def card(cr, text, t0, t1, t, y=985, size=100):
    if not t0 <= t < t1:
        return
    u_in = ease_out((t - t0) / 0.16)
    a = u_in * (1 - smooth((t - (t1 - 0.2)) / 0.2))
    sc = lerp(1.35, 1.0, u_in)
    jit = 3 * math.sin(t * 61) * (1 - u_in)
    cr.save()
    cr.translate(W / 2 + jit, y)
    cr.rotate(-0.025)
    cr.scale(sc, sc)
    pos, total = text_spaced(cr, text, 0, 0, size, 10, "Avenir Next Condensed")
    for ch, xx in pos:
        cr.move_to(xx + 5, 7)
        cr.text_path(ch)
    cr.set_source_rgba(0, 0, 0, 0.6 * a)
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
    cr.rectangle(-w / 2, 22, w, 6)
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


CARDS = [("FULL STEAM AHEAD", 0, 4), ("KEYBOARD CHAT", 4, 6), ("FILE TRANSFER", 6, 8),
         ("ROUNDTABLES", 8, 10), ("ALL OVER HF", 10, 12), ("HOLD ON", 12, 14)]


# ------------------------------------------------------------------ frame
def frame(cr, t):
    fl = flash_level(t)
    shake = 0.0
    for tl in LANDINGS:
        if 0 <= t - tl < 0.8:
            shake += 22 * math.exp(-(t - tl) * 6)
    if T0 < t < T_HIT:
        shake += 2.5 + 2.0 * (t - T0) / (T_HIT - T0)
    if T_HIT <= t < T_HIT + 0.6:
        shake += 14 * math.exp(-(t - T_HIT) * 7)
    sx, sy = shake * math.sin(t * 91.0), shake * math.cos(t * 77.0)
    cx, cy = camx(t), camy(t)

    cr.save()
    cr.translate(W / 2 + sx, H / 2 + sy)
    cr.rotate(tilt(t))
    cr.scale(1.06, 1.06)
    cr.translate(-W / 2, -H / 2)
    draw_sky(cr, t, fl)
    draw_clouds(cr, t, fl)
    draw_bolt(cr, t)
    draw_ridges(cr, t, fl)
    draw_world(cr, t, fl)
    draw_train(cr, t, cx, cy)
    draw_sparks(cr, t, cx, cy)
    draw_smoke(cr, t, cx, cy, fl)
    cr.restore()
    draw_rain(cr, t, fl)

    if fl > 0:
        cr.set_source_rgba(0.75, 0.8, 1.0, 0.18 * fl)
        cr.paint()
    if t >= T_HIT - 0.2:
        cr.set_source_rgba(*NAVY_B, 0.66 * smooth((t - T_HIT + 0.2) / 1.2))
        cr.paint()
    g = cairo.RadialGradient(W / 2, H / 2, H * 0.35, W / 2, H / 2, H * 1.05)
    g.add_color_stop_rgba(0, 0, 0, 0, 0)
    g.add_color_stop_rgba(1, 0, 0, 0, 0.65)
    cr.set_source(g)
    cr.paint()

    soft_line(cr, "There's only one way down.", 2.0, T0 + 0.1, t, 215, 40, col=(0.8, 0.77, 0.7))
    for text, b0, b1 in CARDS:
        card(cr, text, BARS[b0], BARS[b1] - 0.05 if b1 < len(BARS) - 1 else T_HIT - 0.05, t)

    if t >= T_HIT - 0.2:
        ring_u = ease_out((t - T_HIT - 0.1) / 0.7)
        wave_u = ease_out((t - T_HIT - 0.55) / 0.6)
        wl = whistle_level(t) if t > W_CQ - 0.1 else 0
        draw_mark(cr, *LOGO_C, LOGO_R, 1.0, ring_u, wave_u, wl)
        for w0, gmax in ((W_CQ, 1.0), (W_CQ + 0.9, 0.45)):
            if w0 <= t < w0 + 0.8:
                ph = ((t - w0) % 0.4) / 0.2
                hx, hy = logo_pt(ph if ph < 1 else 2 - ph)
                g = cairo.RadialGradient(hx, hy, 0, hx, hy, 70)
                g.add_color_stop_rgba(0, 1, 0.97, 0.85, 0.9 * gmax)
                g.add_color_stop_rgba(1, 1, 0.85, 0.5, 0)
                cr.save()
                cr.set_operator(cairo.OPERATOR_ADD)
                cr.set_source(g)
                cr.arc(hx, hy, 70, 0, 2 * math.pi)
                cr.fill()
                cr.restore()
            for k in range(4):
                tau = t - (w0 + k * 0.2)
                if 0 <= tau < 1.2:
                    cr.set_source_rgba(*BRASS, 0.5 * gmax * (1 - tau / 1.2) ** 2)
                    cr.set_line_width(4)
                    cr.arc(LOGO_C[0], LOGO_C[1], LOGO_R + 30 + tau * 260, 0, 2 * math.pi)
                    cr.stroke()
        pos, total = text_spaced(cr, "WHISTLESTOP", W / 2, 0, 132, 22)
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
        ru = ease_out((t - (T_HIT + 1.0)) / 0.8)
        if ru > 0:
            cr.set_source_rgba(*BRASS, 0.6)
            cr.rectangle(W / 2 - total / 2 * ru, 790, total * ru, 2)
            cr.fill()
            tx = W / 2 - total / 2 + (t - (T_HIT + 1.0)) * 260
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
        soft_line(cr, "USING THE MERCURY MODEM", T_HIT + 1.8, 99, t, 930, 32, col=BRASS, spacing=9, bold=True)
        soft_line(cr, "The whistle calls CQ.", T_HIT + 2.4, 99, t, 984, 26, col=(0.72, 0.69, 0.63))

    if T_HIT <= t < T_HIT + 0.6:
        cr.set_operator(cairo.OPERATOR_ADD)
        cr.set_source_rgba(1, 0.9, 0.7, 0.55 * math.exp(-(t - T_HIT) * 12))
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)
    a = max(1 - smooth(t / 0.5), smooth((t - (DUR - 1.4)) / 1.3))
    if a > 0:
        cr.set_source_rgba(0, 0, 0, a)
        cr.paint()


def render(t):
    surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, W, H)
    frame(cairo.Context(surf), t)
    surf.flush()
    return surf


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[1] == "--still":
        render(float(sys.argv[2])).write_to_png(sys.argv[3])
    else:
        out = sys.stdout.buffer
        for i in range(int(DUR * FPS)):
            out.write(bytes(render(i / FPS).get_data()))
