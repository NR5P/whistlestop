"""Whistlestop "Runaway" intro: an ORIGINAL metal cue for a runaway steam
train. Drop-D gallop riffs, double kick, harmonised lead, accelerating from
150 to 200 bpm. Landing impacts, thunder, and Whistlestop's own CQ whistle
(1000 -> 2000 Hz in 0.2 s and back) as the train whistle.

Writes <out>.wav and <out>-timing.json for video_mountain.py.
"""
import json
import sys
import wave

import numpy as np

SR = 48000
rng = np.random.default_rng(11)

# ---------------------------------------------------------------- tempo map
T0 = 3.5
BARS = 14
BPM0, BPM1 = 150.0, 200.0
grid = np.linspace(0, BARS * 4, 200001)
bpm = BPM0 + (BPM1 - BPM0) * (grid / (BARS * 4)) ** 1.2
tt = T0 + np.concatenate([[0], np.cumsum(60 / bpm[1:] * np.diff(grid))])


def tb(beat):
    return float(np.interp(beat, grid, tt))


def t16(bar, s):
    return tb(bar * 4 + s / 4)


T_HIT = tb(BARS * 4)
DUR = round(T_HIT + 6.5, 2)
N = int(DUR * SR)
dry = np.zeros((2, N))
send = np.zeros((2, N))

LAND_BARS = [4, 8, 11]
LANDINGS = [t16(b, 0) for b in LAND_BARS]
W_INTRO = 1.5
W_CQ = T_HIT + 1.25
LIGHTNING = [0.3, 2.2] + LANDINGS + [t16(6, 0), t16(10, 8), T_HIT]


# ---------------------------------------------------------------- helpers
def tv(d):
    return np.arange(int(d * SR)) / SR


def place(sig, t, gain=1.0, pan=0.0, rev=0.15):
    i = int(round(t * SR))
    if i >= N or len(sig) == 0:
        return
    if i < 0:
        sig, i = sig[-i:], 0
    sig = sig[: N - i] * gain
    a = (pan + 1) * np.pi / 4
    for ch, g in ((0, np.cos(a)), (1, np.sin(a))):
        dry[ch, i : i + len(sig)] += sig * g
        send[ch, i : i + len(sig)] += sig * g * rev


def band(x, lo=0.0, hi=None, order=4):
    X = np.fft.rfft(x)
    f = np.maximum(np.fft.rfftfreq(len(x), 1 / SR), 1e-3)
    m = np.ones_like(f)
    if lo > 0:
        m *= 1 / (1 + (lo / f) ** order)
    if hi:
        m *= 1 / (1 + (f / hi) ** order)
    return np.fft.irfft(X * m, len(x))


def nz(n):
    return rng.standard_normal(n)


def rel(sig, hold, r=0.03):
    t = np.arange(len(sig)) / SR
    return sig * np.clip(1 - (t - hold) / r, 0, 1)


def att(sig, a=0.003):
    t = np.arange(len(sig)) / SR
    return sig * np.clip(t / a, 0, 1)


def midi(m):
    return 440.0 * 2 ** ((m - 69) / 12)


# ---------------------------------------------------------------- drums
def kick(g=1.0, short=False):
    d = 0.16 if short else 0.4
    t = tv(d)
    f = 48 + 140 * np.exp(-t * 40)
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * (22 if short else 8))
    click = band(nz(len(t)), 2500, 10000) * np.exp(-t * 400) * 0.7
    return np.tanh(2.2 * (s + click)) * g


def snare(g=1.0):
    t = tv(0.35)
    tone = (np.sin(2 * np.pi * 200 * t) + 0.6 * np.sin(2 * np.pi * 340 * t)) * np.exp(-t * 18)
    n = band(nz(len(t)), 1200, 10000) * np.exp(-t * 11)
    return np.tanh(1.5 * att(0.6 * tone + n, 0.0008)) * g


def tom(f0, g=1.0):
    t = tv(0.5)
    f = f0 * (1 + 0.5 * np.exp(-t * 18))
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 6)
    return np.tanh(1.4 * (s + band(nz(len(t)), 800, 5000) * np.exp(-t * 60) * 0.3)) * g


def ride(g=1.0):
    t = tv(0.5)
    s = sum(np.sin(2 * np.pi * f * t) for f in (3380, 5120, 6930)) * 0.15 * np.exp(-t * 5)
    return (s + band(nz(len(t)), 6000) * np.exp(-t * 14) * 0.5) * g


def crash(g=1.0, d=3.0):
    t = tv(d)
    n = band(nz(len(t)), 3000, 15000) * np.exp(-t * 1.6)
    return att(n, 0.002) * g


def china(g=1.0, d=1.6):
    t = tv(d)
    n = band(nz(len(t)), 1800, 11000) * np.exp(-t * 2.6)
    n *= 1 + 0.8 * np.sin(2 * np.pi * 640 * t)
    return att(n, 0.001) * g


# ---------------------------------------------------------------- guitars
_cache = {}


def gtr_raw(m, d, cents, seed):
    t = tv(d + 0.08)
    s = np.zeros_like(t)
    for c in (-cents, cents):
        f = midi(m) * 2 ** (c / 1200)
        k = 1
        while k * f < 7000:
            s += np.sin(2 * np.pi * k * f * t + seed * k) / k
            k += 1
    return s


def rhythm(notes, d, mute, side):
    key = (tuple(notes), round(d, 2), mute, side)
    if key in _cache:
        return _cache[key]
    s = sum(gtr_raw(m, d, 7 + 3 * side, 0.7 + side) for m in notes)
    s = np.tanh(9.0 * s / len(notes))
    s = band(s, 90, 6000)
    s = s - 0.35 * band(s, 450, 900, 2)  # mid scoop
    t = np.arange(len(s)) / SR
    if mute:
        s = band(s, 0, 1700) * np.exp(-t * 22) * 1.4
    s = rel(att(s, 0.002), d, 0.035)
    _cache[key] = s
    return s


def lead(m, d, g=1.0, bend=False):
    f0 = midi(m)
    t = tv(d + 0.1)
    pitch = np.ones_like(t)
    if bend:
        pitch *= 2 ** (-np.clip(1 - t / 0.07, 0, 1) / 12)
    pitch *= 1 + 0.009 * np.sin(2 * np.pi * 6.0 * t) * np.clip((t - 0.15) / 0.15, 0, 1)
    ph = 2 * np.pi * np.cumsum(f0 * pitch) / SR
    s = np.zeros_like(t)
    k = 1
    while k * f0 < 8000:
        s += np.sin(k * ph) / k
        k += 1
    s = np.tanh(10 * s)
    s = band(s, 350, 5500)
    return rel(att(s, 0.004), d, 0.05) * g


def bass(m, d, g=1.0):
    f = midi(m)
    t = tv(d + 0.03)
    s = np.zeros_like(t)
    k = 1
    while k * f < 3000:
        s += np.sin(2 * np.pi * k * f * t) / k * np.exp(-t * (1.5 + 0.5 * k))
        k += 1
    s = np.tanh(2.5 * s) + 0.5 * np.sin(2 * np.pi * f * t)
    return rel(att(s, 0.003), d, 0.02) * g


# ---------------------------------------------------------------- fx
def whistle_pair(gain=1.0):
    """Whistlestop whistle: 1000->2000 Hz (0.2 s), 2000->1000 Hz (0.2 s), twice."""
    t1 = tv(0.2)
    up = 1000 + 1000 * t1 / 0.2
    f = np.concatenate([up, up[::-1], up, up[::-1]])
    ph = 2 * np.pi * np.cumsum(f) / SR
    t = np.arange(len(f)) / SR
    taper = np.clip(t / 0.02, 0, 1) * np.clip((t[-1] - t) / 0.03, 0, 1)
    body = 0.35 * np.sin(ph * 2 / 3) + 0.25 * np.sin(ph / 2) + 0.12 * np.sin(2 * ph)
    breath = band(nz(len(f)), 900, 4500) * 0.12
    return (0.75 * np.sin(ph) + body + breath) * taper * gain


def thunder(d=3.5, g=1.0):
    t = tv(d)
    n = band(nz(len(t)), 25, 260)
    env = np.exp(-t * 1.3) * (1 + 0.8 * np.abs(band(nz(len(t)), 0, 6)) * 8)
    crack = band(nz(len(t)), 1500, 9000) * np.exp(-t * 30) * 1.5
    return np.tanh((n * env + crack) * 2) * g


def rain(d):
    return band(nz(int(d * SR)), 2500, 11000) * 0.5 + band(nz(int(d * SR)), 300, 1500) * 0.2


def hiss(d, g=1.0):
    t = tv(d)
    n = band(nz(len(t)), 2000, 9000)
    return n * np.clip(t / 0.03, 0, 1) * np.exp(-t * 2.5) * g


def bell(g=1.0):
    t = tv(2.0)
    s = sum(a * np.sin(2 * np.pi * 620 * r * t) * np.exp(-t * dcy) for r, a, dcy in ((1, 1, 1.8), (2.0, 0.5, 2.5), (2.76, 0.4, 3.5), (5.4, 0.2, 6)))
    return att(s, 0.001) * g


def impact(g=1.0):
    t = tv(2.0)
    boom = np.sin(2 * np.pi * (36 + 70 * np.exp(-t * 9)) * t) * np.exp(-t * 2.4)
    clang = sum(np.sin(2 * np.pi * f * t) for f in (293, 467, 781, 1130)) * 0.25 * np.exp(-t * 5)
    sq = np.sin(2 * np.pi * np.cumsum(3100 + 250 * np.sin(2 * np.pi * 13 * t)) / SR) * 0.25 * np.exp(-t * 3)
    screech = band(nz(len(t)), 2800, 7500) * 0.6 * np.exp(-t * 3.5)
    return np.tanh(1.5 * (boom * 1.4 + clang + sq + screech)) * g


def whoosh(d, g=1.0):
    t = tv(d)
    m = (t / d) ** 1.5
    s = (1 - m) * band(nz(len(t)), 100, 700) + m * band(nz(len(t)), 1200, 8000)
    return s * np.sin(np.pi * np.clip(t / d, 0, 1)) ** 1.2 * g


# ---------------------------------------------------------------- intro
place(rain(DUR), 0, 0.05, 0, 0.3)
for lt in LIGHTNING:
    place(thunder(3.5, 0.55 if lt < T0 else 0.35), lt + 0.08, pan=rng.uniform(-0.5, 0.5), rev=0.4)
idle_t = tv(T0)
place(band(nz(len(idle_t)), 40, 200) * 0.6, 0, 0.12, rev=0.1)  # boiler rumble
# already rolling: steam chuffs and rail clacks across the summit
for i in range(int(T0 / 0.2)):
    tc = 0.05 + i * 0.2
    ct = tv(0.3)
    place(band(nz(len(ct)), 250, 3200) * np.exp(-ct * 9) * np.clip(ct / 0.02, 0, 1), tc, 0.16 if i % 2 == 0 else 0.1, -0.2, 0.3)
for i in range(int(T0 / 0.6)):
    for dt_ in (0.0, 0.11):
        ct = tv(0.09)
        clk = sum(np.sin(2 * np.pi * f * ct) for f in (1830, 2710, 4150)) * np.exp(-ct * 55)
        place(clk, 0.3 + i * 0.6 + dt_, 0.05, 0.3, 0.2)
place(bell(0.22), 0.9, pan=0.25, rev=0.5)
place(bell(0.2), 1.22, pan=0.25, rev=0.5)
place(whistle_pair(0.34), W_INTRO, pan=0.1, rev=0.6)
place(hiss(0.9, 0.35), 2.45, pan=0.1, rev=0.3)          # brakes off
for i, f0 in enumerate((210, 190, 170, 150, 125, 105)):
    place(tom(f0, 0.5 + 0.06 * i), T0 - 6 * 0.1 + i * 0.1, pan=0.4 - i * 0.16, rev=0.15)

# ---------------------------------------------------------------- groove
D, F, G, A, BB, C = 38, 41, 43, 45, 46, 48
CHORDS = [D, D, BB, C,  D, F, G, A,  BB, C, D, D,  D, C]
DMIN = {2, 4, 5, 7, 9, 10, 0}
LEAD = {
    4: [(0, 74, 4), (4, 72, 2), (6, 74, 2), (8, 77, 4), (12, 76, 2), (14, 74, 2)],
    5: [(0, 72, 6), (6, 69, 2), (8, 72, 2), (10, 74, 2), (12, 77, 4)],
    6: [(0, 79, 4), (4, 77, 2), (6, 76, 2), (8, 74, 4), (12, 72, 2), (14, 70, 2)],
    7: [(0, 69, 8), (8, 73, 4), (12, 76, 4)],
    8: [(0, 74, 2), (2, 77, 2), (4, 82, 4), (8, 81, 4), (12, 77, 4)],
    9: [(0, 79, 2), (2, 76, 2), (4, 79, 4), (8, 84, 4), (12, 81, 4)],
    10: [(0, 86, 8), (8, 84, 2), (10, 81, 2), (12, 77, 2), (14, 74, 2)],
    11: [(0, 73, 4), (4, 74, 4), (8, 76, 4), (12, 77, 4)],
}


def third_below(n):
    for d in (3, 4):
        if (n - d) % 12 in DMIN:
            return n - d
    return n - 3


def power(root):
    return [root, root + 7, root + 12]


smoke = []
for bar in range(BARS):
    ch = CHORDS[bar]
    sec = "A" if bar < 4 else "B" if bar < 8 else "C" if bar < 12 else "D"
    if bar in (0, 4, 8, 12) or bar in LAND_BARS:
        place(crash(0.5), t16(bar, 0), pan=-0.4, rev=0.3)
        place(china(0.35), t16(bar, 0), pan=0.4, rev=0.2)
    if bar in LAND_BARS:
        place(impact(0.75), t16(bar, 0), rev=0.35)
        place(kick(1.1), t16(bar, 0))
        tl = t16(bar, 0)
        place(whoosh(0.85, 0.3), tl - 0.85, pan=0.3, rev=0.3)
    for s in range(16):
        t = t16(bar, s)
        d16 = t16(bar, s + 1) - t
        # --- drums
        if sec == "A":
            if s % 4 in (0, 2, 3):
                place(kick(0.75, True), t)
            if s in (4, 12):
                place(snare(0.8), t, pan=0.05, rev=0.2)
            if s % 2 == 0:
                place(ride(0.12), t, pan=0.45, rev=0.1)
        elif sec in ("B", "C"):
            place(kick(0.6, True), t)
            if (sec == "B" and s in (4, 12)) or (sec == "C" and s % 4 == 0 and s > 0) or (sec == "C" and s == 0 and bar not in LAND_BARS):
                place(snare(0.8), t, pan=0.05, rev=0.2)
            if s % 2 == 0:
                place(ride(0.11), t, pan=0.45, rev=0.1)
        else:  # build
            if s % 4 == 0:
                place(kick(0.8, True), t)
            if bar == 12 and s % 2 == 0:
                place(snare(0.45 + 0.02 * s), t, pan=0.05, rev=0.2)
            if bar == 13:
                place(snare(0.5 + 0.03 * s), t, pan=0.05, rev=0.2)
                if s >= 8:
                    place(tom(150 - 5 * (s - 8), 0.5), t, pan=-0.3, rev=0.15)
        if s % 2 == 0:
            smoke.append(t)
        # --- rhythm guitars and bass
        if sec == "A":
            accent = s == 0 or s == 8
            play = s % 4 in (0, 2, 3)
        elif sec == "B":
            accent = s in (0, 3, 6, 10, 12)
            play = True
        elif sec == "C":
            accent = s in (0, 6, 8, 14)
            play = True
        else:
            accent = s in (0, 8) if bar == 12 else s % 4 == 0
            play = True
        if play:
            if accent:
                L = 3 if sec != "D" else 4
                d = t16(bar, min(s + L, 16)) - t
                notes, mute, broot = power(ch), False, ch - 12
            else:
                d = d16 * 0.9
                notes, mute, broot = [D, D + 7], True, D - 12
            for side, pan in ((0, -0.8), (1, 0.8)):
                place(rhythm(notes, d, mute, side), t, 0.12, pan, 0.08)
            place(bass(broot, d * 0.95, 0.42), t, rev=0.02)
    if bar in LEAD:
        for s, m, L in LEAD[bar]:
            d = t16(bar, s + L) - t16(bar, s)
            place(lead(m, d * 0.97, 0.09, bend=L >= 4), t16(bar, s), pan=-0.15, rev=0.4)
            if bar >= 8:
                place(lead(third_below(m), d * 0.97, 0.07, bend=L >= 4), t16(bar, s) + 0.006, pan=0.2, rev=0.4)

# lead-in to the launch
T_LAUNCH = T_HIT - 1.1
place(whoosh(1.15, 0.45), T_LAUNCH, pan=0.2, rev=0.4)
rc = crash(0.5, d=T_HIT - t16(13, 0))[::-1]
place(rc, T_HIT - len(rc) / SR, pan=0.2, rev=0.3)
place(whistle_pair(0.25), T_LAUNCH - 0.9, pan=0.1, rev=0.5)

# ---------------------------------------------------------------- the hit
place(kick(1.3), T_HIT)
place(impact(0.6), T_HIT, rev=0.5)
place(crash(0.8, 5), T_HIT, pan=-0.35, rev=0.5)
place(china(0.6, 5.5), T_HIT, pan=0.35, rev=0.5)
for side, pan in ((0, -0.8), (1, 0.8)):
    ring = rhythm(power(D), 6.0, False, side).copy()
    rt = np.arange(len(ring)) / SR
    ring = ring * np.exp(-rt * 0.9) * np.clip((6.0 - rt) / 2.5, 0, 1)
    place(ring, T_HIT, 0.16, pan, 0.4)
place(bass(D - 12, 4.0, 0.6), T_HIT, rev=0.05)
place(lead(86, 2.2, 0.1, bend=True), T_HIT, pan=-0.15, rev=0.6)
place(lead(81, 2.2, 0.07, bend=True), T_HIT + 0.006, pan=0.2, rev=0.6)
place(whistle_pair(0.38), W_CQ, pan=0.0, rev=0.8)
place(whistle_pair(0.12), W_CQ + 0.9, pan=0.4, rev=1.0)
place(thunder(4.0, 0.3), W_CQ + 1.9, pan=-0.3, rev=0.5)

# ---------------------------------------------------------------- mix
ir_t = tv(2.2)
ir = np.stack([nz(len(ir_t)), nz(len(ir_t))]) * np.exp(-ir_t / 0.4)
ir[:, : int(0.015 * SR)] = 0
ir = np.stack([band(ir[0], 250, 7000), band(ir[1], 250, 7000)])
Lf = 1 << int(np.ceil(np.log2(N + ir.shape[1])))
wet = np.stack([np.fft.irfft(np.fft.rfft(send[c], Lf) * np.fft.rfft(ir[c], Lf), Lf)[:N] for c in range(2)])
wet *= 0.3 * np.abs(dry).max() / max(np.abs(wet).max(), 1e-9)
mix = dry + wet
mix = np.stack([band(mix[c], 28) for c in range(2)])
mix /= np.abs(mix).max()
mix = np.tanh(2.0 * mix) / np.tanh(2.0)
mix *= np.clip((DUR - np.arange(N) / SR) / 1.4, 0, 1) * 0.9

out = sys.argv[1] if len(sys.argv) > 1 else "runaway.wav"
with wave.open(out, "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((np.clip(mix.T, -1, 1) * 32767).astype("<i2").tobytes())

json.dump({
    "dur": DUR, "t0": T0, "t_hit": T_HIT, "t_launch": T_LAUNCH, "w_intro": W_INTRO, "w_cq": W_CQ,
    "landings": LANDINGS, "lightning": LIGHTNING, "smoke": smoke,
    "bars": [t16(b, 0) for b in range(BARS + 1)], "bpm0": BPM0, "bpm1": BPM1,
}, open(out.replace(".wav", "-timing.json"), "w"), indent=1)
print("hit %.2f  dur %.2f" % (T_HIT, DUR))
