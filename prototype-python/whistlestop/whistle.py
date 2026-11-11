"""The Whistlestop "whistle": a chirp calling signal that scanners can find
far off frequency, and that tells them exactly how far off they are.

The signal is a train of linear chirp pairs, UP (f_lo -> f_hi) then DOWN
(f_hi -> f_lo), each lasting ``chirp_s`` seconds, so the pattern repeats every
P = 2 * chirp_s seconds.

When the receiver is mistuned so that the audio arrives ``df`` Hz higher than
sent, the matched-filter peak of an up-chirp moves *earlier* by df/k and that
of a down-chirp moves *later* by df/k, where k is the sweep rate in Hz/s.  The
gap between an UP peak and the following DOWN peak is therefore

    gap = T + 2*df/k        ->       df = (gap - T) * k / 2

Because the train is periodic, the gap is only known modulo P, which bounds
the unambiguous range to |df| < k*P/4 (= k*T/2; +/-1000 Hz with the defaults).
See docs/DESIGN.md.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

FS = 8000  # Mercury runs its modem at 8 kHz; so do we.


@dataclass(frozen=True)
class WhistleParams:
    fs: int = FS
    f_lo: float = 500.0
    f_hi: float = 2500.0
    chirp_s: float = 0.5
    taper_s: float = 0.01  # raised-cosine edge on each chirp, limits splatter
    amplitude: float = 0.5

    @property
    def k(self) -> float:
        """Sweep rate in Hz/s."""
        return (self.f_hi - self.f_lo) / self.chirp_s

    @property
    def period_s(self) -> float:
        return 2 * self.chirp_s

    @property
    def n_chirp(self) -> int:
        return int(round(self.chirp_s * self.fs))

    @property
    def max_offset_hz(self) -> float:
        """Largest |offset| that can be measured without ambiguity."""
        return self.k * self.period_s / 4


DEFAULT = WhistleParams()


def _taper(n: int, n_edge: int) -> np.ndarray:
    w = np.ones(n)
    if n_edge > 0:
        ramp = 0.5 - 0.5 * np.cos(np.pi * np.arange(n_edge) / n_edge)
        w[:n_edge] = ramp
        w[-n_edge:] = ramp[::-1]
    return w


def chirp(p: WhistleParams = DEFAULT, up: bool = True, analytic: bool = False) -> np.ndarray:
    """One chirp.  ``analytic=True`` returns the complex template used by the
    detector; otherwise the real audio that goes to the radio."""
    t = np.arange(p.n_chirp) / p.fs
    if up:
        phase = 2 * np.pi * (p.f_lo * t + 0.5 * p.k * t * t)
    else:
        phase = 2 * np.pi * (p.f_hi * t - 0.5 * p.k * t * t)
    w = _taper(p.n_chirp, int(p.taper_s * p.fs))
    if analytic:
        return w * np.exp(1j * phase)
    return (p.amplitude * w * np.cos(phase)).astype(np.float32)


def whistle(duration_s: float, p: WhistleParams = DEFAULT) -> np.ndarray:
    """Audio for a whistle of (at least) ``duration_s`` seconds: whole UP/DOWN pairs."""
    pair = np.concatenate([chirp(p, up=True), chirp(p, up=False)])
    n_pairs = max(1, int(np.ceil(duration_s / p.period_s)))
    return np.tile(pair, n_pairs)


# --------------------------------------------------------------------------
# Detection
# --------------------------------------------------------------------------


@dataclass
class Detection:
    offset_hz: float  # audio arrived this much higher than sent; USB: raise dial by this
    score: float  # mean normalised correlation of the pairs used (0..1)
    n_pairs: int  # how many consistent UP/DOWN pairs supported the estimate
    spread_hz: float  # disagreement between pairs (lower is better)
    up_times_s: list[float] = field(default_factory=list)


def _analytic(x: np.ndarray, nfft: int) -> np.ndarray:
    X = np.fft.fft(x, nfft)
    h = np.zeros(nfft)
    h[0] = 1
    h[1 : nfft // 2] = 2
    h[nfft // 2] = 1
    return X * h  # spectrum of the analytic signal


def _normalised_corr(xa_spec: np.ndarray, x: np.ndarray, tmpl: np.ndarray, nfft: int) -> np.ndarray:
    """|<x[n:n+L], tmpl>| / (|x[n:n+L]| * |tmpl|) for every start n where the
    template fits.  Values lie in 0..1, independent of input gain."""
    L = len(tmpl)
    T = np.fft.fft(tmpl, nfft)
    c = np.fft.ifft(xa_spec * np.conj(T))[: len(x) - L + 1]
    # Energy of the analytic window is 2x the real one; fold that into the norm.
    e = np.concatenate([[0.0], np.cumsum(x.astype(np.float64) ** 2)])
    win = np.sqrt(np.maximum(2 * (e[L:] - e[:-L]), 1e-20))
    return np.abs(c) / (win * np.linalg.norm(tmpl))


def _peaks(r: np.ndarray, threshold: float, min_sep: int) -> list[int]:
    """Local maxima above threshold, strongest first, at least min_sep apart."""
    idx = np.flatnonzero(r > threshold)
    picked: list[int] = []
    for i in idx[np.argsort(r[idx])[::-1]]:
        if all(abs(i - j) >= min_sep for j in picked):
            picked.append(int(i))
    return sorted(picked)


def _refine(r: np.ndarray, i: int) -> float:
    """Parabolic sub-sample peak position."""
    if 0 < i < len(r) - 1:
        a, b, c = r[i - 1], r[i], r[i + 1]
        d = a - 2 * b + c
        if d < 0:
            return i + 0.5 * (a - c) / d
    return float(i)


class Detector:
    """Finds whistles in a block of received audio and measures their offset."""

    def __init__(self, p: WhistleParams = DEFAULT, threshold: float = 0.12):
        self.p = p
        self.threshold = threshold
        self.up = chirp(p, up=True, analytic=True)
        self.down = chirp(p, up=False, analytic=True)

    def correlate(self, x: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        x = np.asarray(x, dtype=np.float64)
        L = self.p.n_chirp
        if len(x) < L:
            z = np.zeros(0)
            return z, z
        nfft = 1 << int(np.ceil(np.log2(len(x) + L)))
        xa = _analytic(x, nfft)
        return (_normalised_corr(xa, x, self.up, nfft), _normalised_corr(xa, x, self.down, nfft))

    def peak_level(self, x: np.ndarray) -> float:
        """Strongest single-chirp correlation in the block.  A scanner can use
        this to *hold* on a channel that looks promising before a full pair
        has arrived."""
        ru, rd = self.correlate(x)
        if len(ru) == 0:
            return 0.0
        return float(max(ru.max(), rd.max()))

    def detect(self, x: np.ndarray) -> Detection | None:
        p = self.p
        ru, rd = self.correlate(x)
        if len(ru) == 0:
            return None
        min_sep = int(0.5 * p.chirp_s * p.fs)
        # Absolute threshold rejects noise; the relative one rejects sidelobes
        # of a strong whistle.
        ups = _peaks(ru, max(self.threshold, 0.5 * ru.max()), min_sep)
        downs = _peaks(rd, max(self.threshold, 0.5 * rd.max()), min_sep)
        if not ups or not downs:
            return None

        P = p.period_s * p.fs
        T = p.chirp_s * p.fs
        lo = T - P / 2  # the gap is folded into (lo, lo + P]
        offsets, scores, up_times = [], [], []
        # The train is periodic, so every UP/DOWN combination gives the same
        # offset once the gap is folded modulo P.  Use them all; the median
        # below throws out any combination involving a noise peak.
        for iu in ups:
            tu = _refine(ru, iu)
            for idn in downs:
                td = _refine(rd, idn)
                gap = lo + (td - tu - lo) % P
                offsets.append((gap - T) / p.fs * p.k / 2)
                scores.append(0.5 * (ru[iu] + rd[idn]))
                up_times.append(tu / p.fs)

        if not offsets:
            return None
        offsets_a = np.array(offsets)
        med = float(np.median(offsets_a))
        # Keep pairs that agree with the median; a real whistle is self-consistent.
        keep = np.abs(offsets_a - med) < 25.0
        if not keep.any():
            return None
        est = float(np.average(offsets_a[keep], weights=np.array(scores)[keep]))
        return Detection(
            offset_hz=est,
            score=float(np.mean(np.array(scores)[keep])),
            n_pairs=len({round(t, 3) for t, k in zip(up_times, keep) if k}),
            spread_hz=float(np.ptp(offsets_a[keep])) if keep.sum() > 1 else 0.0,
            up_times_s=sorted({round(t, 3) for t, k in zip(up_times, keep) if k}),
        )


# --------------------------------------------------------------------------
# A simple channel model, for tests and the demo
# --------------------------------------------------------------------------


def ssb_channel(
    x: np.ndarray,
    offset_hz: float = 0.0,
    snr_db: float | None = None,
    fs: int = FS,
    passband: tuple[float, float] = (300.0, 2700.0),
    rng: np.random.Generator | None = None,
) -> np.ndarray:
    """Shift audio by ``offset_hz`` (a mistuned SSB receiver), add white noise
    at ``snr_db`` measured in a 2500 Hz bandwidth, and apply the receiver's
    audio passband."""
    rng = rng or np.random.default_rng()
    x = np.asarray(x, dtype=np.float64)
    n = len(x)
    nfft = 1 << int(np.ceil(np.log2(n)))
    t = np.arange(nfft) / fs
    xa = np.fft.ifft(_analytic(np.pad(x, (0, nfft - n)), nfft))
    y = np.real(xa * np.exp(2j * np.pi * offset_hz * t))[:n]
    if snr_db is not None:
        active = x[np.abs(x) > 0]
        sig_p = float(np.mean(active**2)) if len(active) else 1e-6
        # White noise over 0..fs/2; in-band (2500 Hz) share is 2500/(fs/2).
        noise_p_total = sig_p / (10 ** (snr_db / 10)) * (fs / 2) / 2500
        y = y + rng.normal(0.0, np.sqrt(noise_p_total), n)
    Y = np.fft.rfft(y, nfft)
    f = np.fft.rfftfreq(nfft, 1 / fs)
    Y[(f < passband[0]) | (f > passband[1])] = 0
    return np.fft.irfft(Y, nfft)[:n]
