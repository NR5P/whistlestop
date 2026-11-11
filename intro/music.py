"""Whistlestop intro soundtrack: an ORIGINAL driving E-minor train-rock cue
(piano, flute, organ, guitar, bass, drums, steam), accelerating like a train
leaving the station. The train whistle is Whistlestop's own CQ whistle:
1000 -> 2000 Hz in 0.2 s, back down in 0.2 s ("whee-ooo").

Writes intro.wav and timing.json (event times the animation syncs to).
"""
import json
import sys
import numpy as np

SR = 48000
DUR = 24.0
N = int(DUR * SR)
rng = np.random.default_rng(7)

dry = np.zeros((2, N))
send = np.zeros((2, N))


def tv(d):
    return np.arange(int(d * SR)) / SR


def place(sig, t, gain=1.0, pan=0.0, rev=0.2):
    i = int(round(t * SR))
    if i >= N or len(sig) == 0:
        return
    if i < 0:
        sig = sig[-i:]
        i = 0
    sig = sig[: N - i] * gain
    a = (pan + 1) * np.pi / 4
    for ch, g in ((0, np.cos(a)), (1, np.sin(a))):
        dry[ch, i : i + len(sig)] += sig * g
        send[ch, i : i + len(sig)] += sig * g * rev


def band(x, lo=0.0, hi=None, soft=1.15):
    """Brick-ish FFT band filter with soft edges (ratio `soft`)."""
    X = np.fft.rfft(x)
    f = np.fft.rfftfreq(len(x), 1 / SR)
    m = np.ones_like(f)
    if lo > 0:
        m *= 1 / (1 + (lo / np.maximum(f, 1e-3)) ** (2 * np.log(10) / np.log(soft) / 4))
    if hi:
        m *= 1 / (1 + (np.maximum(f, 1e-3) / hi) ** (2 * np.log(10) / np.log(soft) / 4))
    return np.fft.irfft(X * m, len(x))


def noise(d):
    return rng.standard_normal(int(d * SR))


def release(sig, hold, rel=0.03):
    t = np.arange(len(sig)) / SR
    e = np.clip(1 - (t - hold) / rel, 0, 1)
    return sig * e


def attack(sig, a=0.003):
    t = np.arange(len(sig)) / SR
    return sig * np.clip(t / a, 0, 1)


def midi(m):
    return 440.0 * 2 ** ((m - 69) / 12)


# ---------------------------------------------------------------- instruments
def kick(g=1.0):
    t = tv(0.45)
    f = 44 + 120 * np.exp(-t * 30)
    s = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 7.5)
    click = band(noise(0.45), 2000, 9000) * np.exp(-t * 300) * 0.4
    return np.tanh(1.6 * (s + click)) * g


def snare(g=1.0):
    t = tv(0.32)
    tone = (np.sin(2 * np.pi * 188 * t) + 0.5 * np.sin(2 * np.pi * 330 * t)) * np.exp(-t * 22)
    nz = band(noise(0.32), 1400, 9500) * np.exp(-t * 14)
    return attack(0.55 * tone + 0.9 * nz, 0.001) * g


def brush(g=1.0):
    t = tv(0.14)
    nz = band(noise(0.14), 2200, 8000) * np.exp(-t * 32)
    return attack(nz, 0.006) * g


def hat(open_=False, g=1.0):
    d = 0.6 if open_ else 0.08
    t = tv(d)
    nz = band(noise(d), 7000, None) * np.exp(-t * (7 if open_ else 65))
    return attack(nz, 0.0008) * g


def crash(g=1.0, d=3.2):
    t = tv(d)
    nz = band(noise(d), 3500, 15000) * np.exp(-t * 1.5)
    metal = sum(np.sin(2 * np.pi * f * t + rng.uniform(0, 6)) for f in (3120, 4470, 5310, 6880)) * 0.04 * np.exp(-t * 2.5)
    return attack(nz + metal, 0.002) * g


def clack(g=1.0):
    t = tv(0.09)
    s = sum(a * np.sin(2 * np.pi * f * t) for f, a in ((1830, 1), (2710, 0.7), (4150, 0.4)))
    s = s * np.exp(-t * 55) + band(noise(0.09), 1500, 6000) * np.exp(-t * 90) * 0.6
    return s * g


def chuff(g=1.0, d=0.5):
    t = tv(d)
    nz = band(noise(d), 250, 3200) * np.exp(-t * 6.5)
    return attack(nz, 0.03) * g


def bass(f, d, g=1.0):
    t = tv(d + 0.03)
    s = np.zeros_like(t)
    k = 1
    while k * f < 2600:
        s += np.sin(2 * np.pi * k * f * t) / k * np.exp(-t * (2.5 + 0.9 * k))
        k += 1
    s += 0.6 * np.sin(2 * np.pi * f / 2 * t) * np.exp(-t * 2)  # sub
    s = np.tanh(1.8 * s)
    return release(attack(s, 0.004), d, 0.02) * g


def gtr_note(f, d, cents):
    t = tv(d + 0.06)
    s = np.zeros_like(t)
    for c in (-cents, cents):
        ff = f * 2 ** (c / 1200)
        k = 1
        while k * ff < 7000:
            s += np.sin(2 * np.pi * k * ff * t + k * 0.3) / k
            k += 1
    return s


def power(root_m, d, g=1.0, pan=0.0, mute=False):
    notes = [root_m, root_m + 7, root_m + 12]
    s = sum(gtr_note(midi(m), d, rng.uniform(4, 9)) for m in notes)
    s = np.tanh(5.0 * s / 3)
    s = band(s, 90, 5200)
    s = band(s, 0, 2600, soft=1.6) * 0.7 + s * 0.3  # cab-ish
    t = np.arange(len(s)) / SR
    if mute:
        s *= np.exp(-t * 9)
    return release(attack(s, 0.003), d, 0.05) * g


def piano_note(f, d, vel=1.0):
    ring = max(d, 0.05)
    t = tv(ring + 0.12)
    s = np.zeros_like(t)
    B = 0.00035
    for k in range(1, 15):
        fk = k * f * np.sqrt(1 + B * k * k)
        if fk > 15000:
            break
        dec = (0.9 + 0.75 * k) * (f / 261.6) ** 0.5
        for det in (-0.9, 0.9):
            s += np.sin(2 * np.pi * (fk + det) * t + k) * k ** -1.15 * np.exp(-t * dec) * 0.5
    s *= 0.6 + 0.4 * vel
    ham = band(rng.standard_normal(len(t)), 800, 6000) * np.exp(-t * 180) * 0.08
    return release(attack(s + ham, 0.002), ring, 0.09) * vel


def piano(ms, d, vel=1.0):
    out = None
    for m in ms:
        n = piano_note(midi(m), d, vel)
        out = n if out is None else out[: len(n)] + n[: len(out)]
    return out


def flute(m, d, g=1.0):
    f0 = midi(m)
    t = tv(d + 0.08)
    vib = 1 + 0.006 * np.sin(2 * np.pi * 5.3 * t) * np.clip((t - 0.12) / 0.2, 0, 1)
    ph = 2 * np.pi * np.cumsum(f0 * vib) / SR
    s = np.sin(ph) + 0.22 * np.sin(2 * ph) + 0.09 * np.sin(3 * ph) + 0.035 * np.sin(4 * ph)
    env = np.clip(t / 0.045, 0, 1) * (0.9 + 0.1 * np.clip(t / 0.3, 0, 1))
    breath = band(rng.standard_normal(len(t)), 1800, 7000) * 0.05
    chiff = band(rng.standard_normal(len(t)), f0 * 1.5, f0 * 3.5) * np.exp(-t * 60) * 0.35
    return release((s + breath) * env + chiff, d, 0.07) * g


def organ(ms, d, g=1.0, lr=0):
    t = tv(d + 0.2)
    s = np.zeros_like(t)
    for m in ms:
        f = midi(m)
        for h, a in ((0.5, 0.5), (1, 1), (2, 0.55), (3, 0.3), (4, 0.25), (6, 0.12), (8, 0.1)):
            if h * f < 12000:
                s += a * np.sin(2 * np.pi * h * f * t + h)
    s *= 1 + 0.18 * np.sin(2 * np.pi * 6.2 * t + lr * 1.7)
    env = np.clip(t / 0.12, 0, 1)
    return release(s * env, d, 0.2) * g


def whistle_pair(gain=1.0):
    """Whistlestop whistle: 1000->2000 Hz (0.2 s), 2000->1000 Hz (0.2 s), twice."""
    one = 0.2
    t1 = tv(one)
    up = 1000 + 1000 * t1 / one
    f = np.concatenate([up, up[::-1], up, up[::-1]])
    ph = 2 * np.pi * np.cumsum(f) / SR
    t = np.arange(len(f)) / SR
    taper = np.clip(t / 0.02, 0, 1) * np.clip((t[-1] - t) / 0.03, 0, 1)
    lead = np.sin(ph)
    # steam-whistle body under it (a softer chime a fifth and an octave below)
    body = 0.35 * np.sin(ph * 2 / 3) + 0.25 * np.sin(ph / 2) + 0.12 * np.sin(2 * ph)
    breath = band(rng.standard_normal(len(f)), 900, 4500) * 0.12
    return (0.75 * lead + body + breath) * taper * gain


def whoosh(d, g=1.0):
    t = tv(d)
    m = (t / d) ** 2
    lo = band(noise(d), 80, 500)
    hi = band(noise(d), 900, 7000)
    env = np.sin(np.pi * np.clip(t / d, 0, 1)) ** 1.5
    return ((1 - m) * lo + m * hi) * env * g


# ---------------------------------------------------------------- tempo map
T0 = 4.0          # groove starts
BARS = 8
BPM0, BPM1 = 126.0, 170.0
grid = np.linspace(0, BARS * 4, 200001)
bpm = BPM0 + (BPM1 - BPM0) * (grid / (BARS * 4)) ** 1.25
tt = T0 + np.concatenate([[0], np.cumsum(60 / bpm[1:] * np.diff(grid))])


def tb(beat):
    return float(np.interp(beat, grid, tt))


def t16(bar, s):
    return tb(bar * 4 + s / 4)


T_HIT = tb(BARS * 4)

# ---------------------------------------------------------------- intro (0-4 s)
# steam chuffs, accelerating into the groove's quarter notes
ch, iv, t = [], 60 / BPM0, T0 - 60 / BPM0
while t > 0.25:
    ch.append(t)
    iv *= 1.2
    t -= iv
ch = sorted(ch)
for i, c in enumerate(ch):
    place(chuff(0.18 + 0.35 * i / len(ch)), c, pan=-0.2, rev=0.5)
    place(clack(0.05), c + 0.07, pan=0.3, rev=0.4)

# low rumble across everything
rum = band(noise(DUR), 28, 170) * np.clip(np.linspace(0.15, 1.4, N), 0, 1.4)
rum[int(T_HIT * SR):] *= np.exp(-np.arange(N - int(T_HIT * SR)) / SR * 1.2)
place(rum, 0, 0.05, rev=0.1)

# lonely bluesy piano (original figure)
place(piano([28, 40], 3.0, 0.9), 0.30, 0.55, -0.1, 0.6)
for tt_, m, v in ((1.05, 55, 0.5), (1.30, 57, 0.55), (1.52, 58, 0.6), (1.66, 59, 0.75),
                  (1.95, 62, 0.6), (2.15, 59, 0.55), (2.40, 52, 0.7)):
    place(piano([m], 0.9, v), tt_, 0.35, 0.15, 0.6)
place(organ([40, 47, 52], 1.6, 0.05), 2.4, 1, 0, 0.6)

W_INTRO = 2.62
place(whistle_pair(0.32), W_INTRO, 1, 0.05, 0.7)
# snare fill into the groove
for i in range(4):
    place(snare(0.35 + 0.18 * i), T0 - 60 / BPM0 + i * 60 / BPM0 / 4, 1, 0.1, 0.15)

# ---------------------------------------------------------------- groove
CHORD = [40, 40, 36, 38, 40, 40, 36, 38]          # E E C D | E E C D
RIFF = {40: [0, 0, 12, 0, 10, 0, 7, 0], 36: [0, 0, 12, 0, 7, 0, 12, 7], 38: [0, 0, 12, 0, 7, 0, 12, 14]}
PIANO = {40: [64, 67, 71], 36: [64, 67, 72], 38: [62, 66, 69]}
FLUTE = {
    4: [(0, 83, 6), (6, 86, 2), (8, 88, 4), (12, 86, 2), (14, 83, 2)],
    5: [(0, 81, 3), (3, 83, 3), (6, 79, 2), (8, 76, 6), (14, 79, 2)],
    6: [(0, 79, 4), (4, 81, 2), (6, 83, 2), (8, 84, 6), (14, 83, 2)],
    7: [(0, 81, 4), (4, 78, 2), (6, 81, 2), (8, 86, 4), (12, 88, 4)],
}
smoke = []
for bar in range(BARS):
    r = CHORD[bar]
    lvl = 0.8 + 0.2 * bar / BARS
    if bar in (0, 4):
        place(crash(0.5), t16(bar, 0), pan=-0.35, rev=0.3)
    for s in range(16):
        t = t16(bar, s)
        dt16 = t16(bar, s + 1) - t
        if s in (0, 6, 8) or (bar % 4 == 3 and s == 14):
            place(kick(0.9), t, rev=0.05)
        if s in (4, 12):
            place(snare(0.75 * lvl), t, pan=0.05, rev=0.25)
        elif s % 2 == 1:
            place(brush(0.16 + (0.1 if s % 4 == 3 else 0)), t, pan=0.35, rev=0.1)
        if s % 2 == 0:
            op = (s == 14 and bar % 2 == 1)
            place(hat(op, 0.22 if s % 4 else 0.3), t, pan=0.45, rev=0.1)
            smoke.append(t)
        if s in (0, 1):
            place(clack(0.07), t + 0.01, pan=-0.4, rev=0.15)
        if bar == 7 and s >= 8 and s % 2 == 0:
            place(snare(0.3 + 0.05 * s / 2), t, pan=-0.1, rev=0.2)  # build
        # bass eighths
        if s % 2 == 0:
            place(bass(midi(r + RIFF[r][s // 2]), dt16 * 1.8, 0.55), t, pan=0, rev=0.03)
        # piano eighth chords, offbeat accent
        if s % 2 == 0:
            v = 0.85 if s % 4 == 2 else 0.6
            place(piano(PIANO[r], dt16 * 1.7, v), t, 0.17, 0.25, 0.25)
    # guitar pushes: 0 (held), 3, 6, 8 (held), 14
    for s, L, mute in ((0, 3, False), (3, 2, True), (6, 2, True), (8, 5, False), (14, 2, True)):
        d = t16(bar, s + L) - t16(bar, s)
        for pan in (-0.65, 0.65):
            place(power(r - 12 + 12, d * 0.92, 0.10, mute=mute), t16(bar, s), pan=pan, rev=0.15)
    place(organ([r + 12, r + 19, r + 24], t16(bar, 16) - t16(bar, 0), 0.022, lr=bar), t16(bar, 0), 1, -0.25, 0.4)
    if bar in FLUTE:
        for s, m, L in FLUTE[bar]:
            d = t16(bar, s + L) - t16(bar, s)
            place(flute(m, d * 0.95, 0.12), t16(bar, s), pan=-0.15, rev=0.45)

# reverse cymbal + whoosh into the hit
rc = crash(0.45, d=T_HIT - t16(7, 0))[::-1]
place(rc, T_HIT - len(rc) / SR, pan=0.2, rev=0.3)
place(whoosh(1.3, 0.35), T_HIT - 1.15, pan=0.3, rev=0.4)

# ---------------------------------------------------------------- the hit + CQ
place(kick(1.2), T_HIT)
place(crash(0.8, 5), T_HIT, pan=-0.3, rev=0.5)
place(crash(0.6, 5), T_HIT + 0.004, pan=0.3, rev=0.5)
place(bass(midi(28), 3.0, 0.9), T_HIT, rev=0.05)
for pan in (-0.7, 0.7):
    place(power(40, 3.2, 0.13), T_HIT, pan=pan, rev=0.35)
place(piano([28, 40, 47, 52, 55, 59, 64], 4.0, 1.0), T_HIT, 0.3, 0, 0.5)
place(organ([40, 47, 52, 55, 59], 5.0, 0.035), T_HIT, 1, 0, 0.6)
place(flute(88, 1.6, 0.13), T_HIT, pan=-0.15, rev=0.6)
boom_t = tv(2.5)
place(np.sin(2 * np.pi * (38 + 30 * np.exp(-boom_t * 4)) * boom_t) * np.exp(-boom_t * 1.6), T_HIT, 0.5, rev=0.1)

W_CQ = T_HIT + 1.25
place(whistle_pair(0.36), W_CQ, 1, 0.0, 0.8)
place(whistle_pair(0.12), W_CQ + 0.9, 1, 0.4, 1.0)      # far station echo
place(piano([40, 52, 59, 64], 3.0, 0.5), W_CQ + 2.0, 0.22, 0, 0.7)

# ---------------------------------------------------------------- mix
ir_t = tv(2.4)
ir = np.stack([rng.standard_normal(len(ir_t)), rng.standard_normal(len(ir_t))]) * np.exp(-ir_t / 0.45)
ir[:, : int(0.012 * SR)] = 0
ir = np.stack([band(ir[0], 200, 8000), band(ir[1], 200, 8000)])
L = 1 << int(np.ceil(np.log2(N + ir.shape[1])))
wet = np.stack([np.fft.irfft(np.fft.rfft(send[c], L) * np.fft.rfft(ir[c], L), L)[:N] for c in range(2)])
wet *= 0.9 * np.abs(dry).max() / max(np.abs(wet).max(), 1e-9) * 0.35
mix = dry + wet
mix = np.stack([band(mix[c], 30) for c in range(2)])
mix /= np.abs(mix).max()
mix = np.tanh(1.5 * mix) / np.tanh(1.5)
fade = np.clip((DUR - np.arange(N) / SR) / 1.2, 0, 1)
mix *= fade * 0.9

out = sys.argv[1] if len(sys.argv) > 1 else "intro.wav"
pcm = (np.clip(mix.T, -1, 1) * 32767).astype("<i2")
import wave
with wave.open(out, "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes(pcm.tobytes())

timing = {
    "dur": DUR, "t0": T0, "t_hit": T_HIT, "w_intro": W_INTRO, "w_cq": W_CQ,
    "intro_chuffs": ch, "smoke": smoke,
    "bars": [t16(b, 0) for b in range(BARS + 1)],
    "bpm0": BPM0, "bpm1": BPM1,
}
json.dump(timing, open(out.replace(".wav", "-timing.json"), "w"), indent=1)
print("hit at %.3f s, peak ok" % T_HIT)
