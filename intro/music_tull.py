"""Whistlestop "Runaway" intro, Tull-style: an ORIGINAL 70s prog/blues-rock
cue (no melody or riff from any existing song). Free bluesy piano over the
rolling train, then a driving band (bass pulse, crunch guitar, piano stabs,
organ) with a breathy flute lead, accelerating 150 -> 200 bpm on the same
beat grid as music_metal.py, so the 3D render's cuts and landings still hit.

Writes <out>.wav (default runaway-tull.wav) and <out>-timing.json.
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




# ---------------------------------------------------------------- band instruments
def piano(m, d, g=1.0):
    """Struck string: inharmonic partials, per-partial decay, hammer thump."""
    key = ("p", m, round(d, 2))
    if key not in _cache:
        f0 = midi(m)
        B = 0.0004
        t = tv(min(d, 3.5) + 0.25)
        s = np.zeros_like(t)
        k = 1
        while k * f0 < 9000 and k <= 24:
            fk = k * f0 * np.sqrt(1 + B * k * k)
            amp = (1 / k ** 1.1) * (1 if k > 1 else 1.2)
            s += amp * np.sin(2 * np.pi * fk * t + k) * np.exp(-t * (0.6 + 0.35 * k + f0 / 900))
            k += 1
        ham = band(nz(len(t)), 200, 3000) * np.exp(-t * 120) * 0.15
        s = att(s + ham, 0.002)
        s = rel(s, d, 0.12)
        _cache[key] = np.tanh(1.2 * s)
    return _cache[key] * g


def chord_piano(notes, t, d, g, pan=0.0, roll=0.0):
    for i, m in enumerate(notes):
        place(piano(m, d, g), t + i * roll, pan=pan + 0.1 * (i - len(notes) / 2), rev=0.3)


def organ(notes, d, g=1.0):
    """Drawbar organ with a rotary-speaker wobble and a little overdrive."""
    t = tv(d + 0.1)
    s = np.zeros_like(t)
    lfo = np.sin(2 * np.pi * 6.3 * t)
    for m in notes:
        f = midi(m) * (1 + 0.0025 * lfo)
        ph = 2 * np.pi * np.cumsum(f) / SR
        for mult, a in ((0.5, 0.6), (1, 1.0), (1.5, 0.5), (2, 0.6), (3, 0.25), (4, 0.3)):
            s += a * np.sin(ph * mult)
    s = np.tanh(1.6 * s / len(notes)) * (1 + 0.18 * lfo)
    s = band(s, 70, 6000)
    return rel(att(s, 0.02), d, 0.08) * g


def flute(m, d, g=1.0, flutter=False, slide=0):
    """Breathy flute: sine-rich tone, chiff on attack, delayed vibrato,
    breath noise riding the pitch, optional flutter-tongue growl."""
    f0 = midi(m)
    t = tv(d + 0.12)
    pitch = np.ones_like(t)
    if slide:
        pitch *= 2 ** (slide * np.clip(1 - t / 0.06, 0, 1) / 12)
    vib = np.clip((t - 0.18) / 0.25, 0, 1)
    pitch *= 1 + 0.011 * np.sin(2 * np.pi * 5.4 * t) * vib
    ph = 2 * np.pi * np.cumsum(f0 * pitch) / SR
    tone = np.sin(ph) + 0.28 * np.sin(2 * ph) + 0.12 * np.sin(3 * ph) + 0.05 * np.sin(4 * ph)
    amp = 1 + 0.12 * np.sin(2 * np.pi * 5.4 * t) * vib
    if flutter:
        amp *= 1 + 0.55 * np.sin(2 * np.pi * 27 * t)
        tone = np.tanh(2.2 * tone)
    breath = band(nz(len(t)), f0 * 0.8, f0 * 3.5, 2) * 0.22
    chiff = band(nz(len(t)), 1500, 8000) * np.exp(-t * 60) * 0.5
    s = (tone * amp + breath * (1.5 if flutter else 1)) * np.clip(t / 0.035, 0, 1) + chiff
    return rel(s, d, 0.06) * g


def crunch(notes, d, mute, side):
    """Classic-rock overdrive rhythm guitar (much less gain than the metal cue)."""
    key = ("c", tuple(notes), round(d, 2), mute, side)
    if key in _cache:
        return _cache[key]
    s = sum(gtr_raw(m, d, 5 + 3 * side, 0.4 + side) for m in notes)
    s = np.tanh(3.2 * s / len(notes))
    s = band(s, 110, 4800)
    t = np.arange(len(s)) / SR
    if mute:
        s = band(s, 0, 1500) * np.exp(-t * 26) * 1.5
    s = rel(att(s, 0.002), d, 0.03)
    _cache[key] = s
    return s


def hat(g=1.0, open_=False):
    t = tv(0.4 if open_ else 0.07)
    n = band(nz(len(t)), 7000, 16000) * np.exp(-t * (9 if open_ else 70))
    return att(n, 0.0005) * g


def blues_bass(m, d, g=1.0):
    f = midi(m)
    t = tv(d + 0.03)
    s = np.sin(2 * np.pi * f * t) + 0.45 * np.sin(4 * np.pi * f * t) * np.exp(-t * 4) + 0.2 * np.sin(6 * np.pi * f * t) * np.exp(-t * 7)
    s = np.tanh(1.8 * s) * np.exp(-t * 1.2)
    return rel(att(s, 0.004), d, 0.025) * g


# ---------------------------------------------------------------- intro: rolling train + free piano
place(rain(DUR), 0, 0.045, 0, 0.3)
for lt in LIGHTNING:
    place(thunder(3.5, 0.45 if lt < T0 else 0.3), lt + 0.08, pan=rng.uniform(-0.5, 0.5), rev=0.4)
idle_t = tv(T0)
place(band(nz(len(idle_t)), 40, 200) * 0.6, 0, 0.1, rev=0.1)
for i in range(int(T0 / 0.2)):
    ct = tv(0.3)
    place(band(nz(len(ct)), 250, 3200) * np.exp(-ct * 9) * np.clip(ct / 0.02, 0, 1), 0.05 + i * 0.2, 0.13 if i % 2 == 0 else 0.08, -0.2, 0.3)
for i in range(int(T0 / 0.6)):
    for dt_ in (0.0, 0.11):
        ct = tv(0.09)
        clk = sum(np.sin(2 * np.pi * f * ct) for f in (1830, 2710, 4150)) * np.exp(-ct * 55)
        place(clk, 0.3 + i * 0.6 + dt_, 0.04, 0.3, 0.2)
place(bell(0.16), 0.9, pan=0.25, rev=0.5)
place(whistle_pair(0.3), W_INTRO, pan=0.1, rev=0.6)
place(hiss(0.9, 0.28), 2.45, pan=0.1, rev=0.3)

# left hand: low D tremolo rumbling like the wheels
for i in range(int((T0 - 0.15) / 0.115)):
    tl = 0.12 + i * 0.115
    m = (38, 50)[i % 2]
    place(piano(m, 0.2, 0.06 + 0.06 * (tl / T0)), tl, pan=-0.25, rev=0.3)
# right hand: a free bluesy run in D minor, rubato, leaving room for the whistle
RH = [(0.25, 74, 0.5), (0.55, 77, 0.3), (0.75, 79, 0.25), (0.92, 80, 0.12), (1.02, 81, 0.6),
      (2.3, 84, 0.25), (2.42, 81, 0.2), (2.55, 79, 0.2), (2.68, 77, 0.2), (2.8, 74, 0.2),
      (2.93, 72, 0.2), (3.05, 69, 0.4)]
for tn, m, d in RH:
    place(piano(m, d, 0.16), tn, pan=0.2, rev=0.35)
chord_piano([62, 65, 69, 74], 1.02, 1.2, 0.09, pan=0.1, roll=0.02)
# pickup into the band: A7 stab with a snare drag
chord_piano([45, 57, 61, 64, 67], T0 - 0.32, 0.3, 0.3, roll=0.012)
for i in range(4):
    place(snare(0.25 + 0.12 * i), T0 - 0.24 + i * 0.06, pan=0.05, rev=0.2)

# ---------------------------------------------------------------- groove
D2, C2, BB1, A1, G1 = 38, 36, 34, 33, 31
CHORDS = [  # (bass root, piano/organ voicing)
    (D2, [62, 65, 69]), (C2, [60, 64, 67]), (BB1, [58, 62, 65]), (A1, [61, 64, 69]),
    (D2, [62, 65, 69]), (C2, [60, 64, 67]), (BB1, [58, 62, 65]), (A1, [61, 64, 69]),
    (G1, [62, 67, 70]), (BB1, [62, 65, 70]), (C2, [64, 67, 72]), (A1, [64, 69, 73]),
    (D2, [62, 65, 69]), (A1, [61, 64, 69]),
]
FLUTE = {
    4: [(0, 74, 6, 0), (6, 77, 2, 0), (8, 76, 2, 0), (10, 74, 2, 0), (12, 72, 4, -1)],
    5: [(0, 76, 4, -1), (4, 79, 2, 0), (6, 76, 2, 0), (8, 72, 6, 0), (14, 74, 2, 0)],
    6: [(0, 77, 2, 0), (2, 79, 2, 0), (4, 81, 6, -2), (10, 79, 2, 0), (12, 77, 2, 0), (14, 74, 2, 0)],
    7: [(0, 76, 8, -1), (8, 73, 4, 0), (12, 76, 2, 0), (14, 79, 2, 0)],
    8: [(0, 81, 4, -2), (4, 82, 2, 0), (6, 81, 2, 0), (8, 79, 4, 0), (12, 77, 4, 0)],
    9: [(0, 82, 2, 0), (2, 84, 2, 0), (4, 86, 8, -2), (12, 84, 2, 0), (14, 82, 2, 0)],
    10: [(0, 84, 4, 0), (4, 81, 4, 0), (8, 79, 2, 0), (10, 81, 2, 0), (12, 84, 4, -1)],
    11: [(0, 85, 8, -1), (8, 88, 8, -2)],
}
FLUTTER = {(7, 0), (9, 4), (11, 8)}
# bar 13: a racing D-minor arpeggio up into the launch
RUN = [62, 65, 69, 74, 77, 81, 86, 89, 74, 77, 81, 86, 89, 93, 89, 93]

smoke = []
for bar in range(BARS):
    root, voice = CHORDS[bar]
    sec = "A" if bar < 4 else "B" if bar < 8 else "C" if bar < 12 else "D"
    if bar in (0, 4, 8, 12) or bar in LAND_BARS:
        place(crash(0.42), t16(bar, 0), pan=-0.4, rev=0.3)
    if bar in LAND_BARS:
        place(impact(0.6), t16(bar, 0), rev=0.35)
        place(kick(1.0), t16(bar, 0))
        place(whoosh(0.85, 0.25), t16(bar, 0) - 0.85, pan=0.3, rev=0.3)
    # sustained organ from the second half on
    if sec in ("C", "D"):
        place(organ(voice, t16(bar, 16) - t16(bar, 0), 0.09), t16(bar, 0), pan=0.3, rev=0.25)
    for s in range(16):
        t = t16(bar, s)
        d16 = t16(bar, s + 1) - t
        # --- drums: straight driving rock, the hat ticking like rail joints
        if s % 2 == 0:
            place(hat(0.16 if s % 4 == 0 else 0.1, open_=(sec == "C" and s % 4 == 2)), t, pan=0.35)
        if sec != "D":
            if s in (0, 8) or (sec != "A" and s in (10,)) or (sec == "A" and s == 6):
                place(kick(0.75, True), t)
            if s in (4, 12):
                place(snare(0.7), t, pan=0.05, rev=0.22)
        else:
            if s % 4 == 0:
                place(kick(0.8, True), t)
            if bar == 12 and s % 2 == 0:
                place(snare(0.4 + 0.02 * s), t, pan=0.05, rev=0.2)
            if bar == 13:
                place(snare(0.45 + 0.03 * s), t, pan=0.05, rev=0.2)
        if s % 2 == 0:
            smoke.append(t)
        # --- bass: eighth-note root/octave walk, the locomotive pulse
        if s % 2 == 0:
            m = root + (12 if s % 4 == 2 else 0)
            if sec != "A" and s == 14:
                m = root + 10
            place(blues_bass(m, d16 * 1.8, 0.38), t, rev=0.03)
        # --- guitar: muted chug on eighths, open chord on the accents
        if s % 2 == 0 or (sec in ("C", "D") and s % 2 == 1 and s > 8):
            accent = s in (0, 6) if sec != "D" else s % 4 == 0
            g0 = root + 12
            if accent:
                notes, mute, d = [g0, g0 + 7, g0 + 12], False, t16(bar, min(s + 3, 16)) - t
            else:
                notes, mute, d = [g0, g0 + 7], True, d16 * 0.9
            for side, pan in ((0, -0.7), (1, 0.7)):
                place(crunch(notes, d, mute, side), t, 0.075, pan, 0.08)
        # --- piano: chord stabs that push the beat
        if s in (0, 6, 10) and sec != "D":
            pl = t16(bar, s + (6 if s == 0 else 4)) - t
            chord_piano(voice, t, pl, 0.12 if sec == "A" else 0.09, pan=-0.1)
            place(piano(voice[0] - 12, pl, 0.1), t, pan=-0.3, rev=0.3)
        if sec == "D" and s % 2 == 0:
            chord_piano(voice, t, d16 * 1.8, 0.06 + 0.004 * s, pan=-0.1)
    if bar in FLUTE:
        for s, m, L, sl in FLUTE[bar]:
            d = t16(bar, s + L) - t16(bar, s)
            place(flute(m, d * 0.96, 0.21, flutter=(bar, s) in FLUTTER, slide=sl), t16(bar, s), pan=0.05, rev=0.45)
    if bar == 13:
        for s, m in enumerate(RUN):
            d = t16(bar, s + 1) - t16(bar, s)
            place(flute(m, d * 0.95, 0.12), t16(bar, s), pan=0.05, rev=0.4)

# lead-in to the launch
T_LAUNCH = T_HIT - 1.1
place(whoosh(1.15, 0.38), T_LAUNCH, pan=0.2, rev=0.4)
rc = crash(0.4, d=T_HIT - t16(13, 0))[::-1]
place(rc, T_HIT - len(rc) / SR, pan=0.2, rev=0.3)
place(whistle_pair(0.22), T_LAUNCH - 0.9, pan=0.1, rev=0.5)

# ---------------------------------------------------------------- the hit
place(kick(1.2), T_HIT)
place(impact(0.5), T_HIT, rev=0.5)
place(crash(0.7, 5), T_HIT, pan=-0.35, rev=0.5)
place(crash(0.5, 5), T_HIT + 0.01, pan=0.35, rev=0.5)
for side, pan in ((0, -0.7), (1, 0.7)):
    ring = crunch([50, 57, 62], 6.0, False, side).copy()
    rt = np.arange(len(ring)) / SR
    place(ring * np.exp(-rt * 0.8) * np.clip((6.0 - rt) / 2.5, 0, 1), T_HIT, 0.12, pan, 0.4)
place(organ([50, 57, 62, 65, 69], 5.5, 0.12) * np.clip((5.5 + 0.1 - tv(5.6)) / 2.5, 0, 1), T_HIT, pan=0.2, rev=0.5)
chord_piano([26, 38, 50, 62, 65, 69, 74], T_HIT, 4.0, 0.2, roll=0.008)
place(blues_bass(26, 4.0, 0.55), T_HIT, rev=0.05)
place(flute(86, 1.6, 0.2, flutter=True, slide=-2), T_HIT + 0.02, pan=0.05, rev=0.6)
# piano answers the CQ whistle, then a last low note under the end card
for tn, m in ((W_CQ + 1.75, 81), (W_CQ + 1.9, 77), (W_CQ + 2.05, 74)):
    place(piano(m, 1.4, 0.2), tn, pan=0.2, rev=0.5)
place(whistle_pair(0.36), W_CQ, pan=0.0, rev=0.8)
place(whistle_pair(0.11), W_CQ + 0.9, pan=0.4, rev=1.0)
place(thunder(4.0, 0.25), W_CQ + 2.6, pan=-0.3, rev=0.5)

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

out = sys.argv[1] if len(sys.argv) > 1 else "runaway-tull.wav"
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
