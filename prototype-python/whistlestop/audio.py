"""Audio: the live sound card (sounddevice / PortAudio) and a simulated band.

The live path runs the card at 48 kHz (what radio USB codecs use) and
converts to/from the 8 kHz the whistle detector works at.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field

import numpy as np

from .whistle import DEFAULT, FS, WhistleParams, ssb_channel, whistle

CARD_FS = 48000
DECIM = CARD_FS // FS


def _lowpass(taps: int = 121, cutoff: float = 3400.0, fs: int = CARD_FS) -> np.ndarray:
    n = np.arange(taps) - (taps - 1) / 2
    h = np.sinc(2 * cutoff / fs * n) * np.hamming(taps)
    return h / h.sum()


def list_devices() -> str:
    import sounddevice as sd

    return str(sd.query_devices())


def _find_device(name: str | None, kind: str):
    import sounddevice as sd

    if not name:
        return None
    for i, d in enumerate(sd.query_devices()):
        if name.lower() in d["name"].lower() and d[f"max_{kind}_channels"] > 0:
            return i
    raise ValueError(f"no {kind} audio device matching {name!r}")


class LiveAudio:
    """Capture from (and play to) the radio's sound card."""

    fs = FS

    def __init__(self, input_name: str | None = None, output_name: str | None = None):
        self.in_dev = _find_device(input_name, "input")
        self.out_dev = _find_device(output_name, "output")
        self._h = _lowpass()
        self._tail = np.zeros(len(self._h) - 1)
        self._phase = 0
        self._buf = np.zeros(0)
        self._event = asyncio.Event()
        self._stream = None
        self.level = 0.0

    async def open(self) -> None:
        import sounddevice as sd

        loop = asyncio.get_running_loop()

        def cb(indata, frames, t, status):
            x = indata[:, 0].copy()
            loop.call_soon_threadsafe(self._push, x)

        self._stream = sd.InputStream(
            samplerate=CARD_FS, channels=1, dtype="float32", device=self.in_dev, callback=cb, blocksize=4800
        )
        self._stream.start()

    def _push(self, x: np.ndarray) -> None:
        self.level = float(np.sqrt(np.mean(x**2)))
        y = np.convolve(np.concatenate([self._tail, x]), self._h, mode="valid")
        self._tail = np.concatenate([self._tail, x])[-(len(self._h) - 1) :]
        y = y[self._phase :: DECIM]
        self._phase = (self._phase - len(x)) % DECIM
        self._buf = np.concatenate([self._buf, y])
        if len(self._buf) > FS * 60:  # never hold more than a minute
            self._buf = self._buf[-FS * 60 :]
        self._event.set()

    async def read(self, n: int) -> np.ndarray:
        """Next ``n`` samples at 8 kHz, starting from *now* (older audio,
        e.g. from before a retune, is discarded)."""
        self._buf = np.zeros(0)
        while len(self._buf) < n:
            self._event.clear()
            await self._event.wait()
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    async def close(self) -> None:
        if self._stream:
            self._stream.stop()
            self._stream.close()


class LiveSink:
    """Plays audio (the whistle) to the radio."""

    fs = CARD_FS

    def __init__(self, output_name: str | None = None):
        self.dev = _find_device(output_name, "output")

    async def play(self, samples: np.ndarray) -> None:
        import sounddevice as sd

        def _go():
            sd.play(samples.astype(np.float32), CARD_FS, device=self.dev, blocking=True)

        await asyncio.to_thread(_go)


# --------------------------------------------------------------- simulator --


@dataclass
class SimCaller:
    """A simulated station whistling on ``dial_hz``.  Its whistle runs for
    ``whistle_s`` out of every ``cycle_s`` seconds, starting at ``start_s``."""

    dial_hz: float
    snr_db: float = 0.0
    whistle_s: float = 25.0
    cycle_s: float = 40.0
    start_s: float = 0.0
    params: WhistleParams = DEFAULT
    _audio: np.ndarray = field(init=False, repr=False)

    def __post_init__(self) -> None:
        self._audio = whistle(self.whistle_s, self.params)

    def segment(self, t0: float, n: int, fs: int) -> np.ndarray:
        out = np.zeros(n)
        t = t0 + np.arange(n) / fs - self.start_s
        phase = np.mod(t, self.cycle_s)
        idx = np.round(phase * fs).astype(int)
        on = (t >= 0) & (idx < len(self._audio))
        out[on] = self._audio[idx[on]]
        return out


class SimBand:
    """A simulated HF band: noise plus any number of whistling callers, heard
    through a receiver tuned by ``rig`` (a FakeRig).  Virtual time advances
    only as samples are read, so scanning runs as fast as the CPU allows."""

    fs = FS

    def __init__(self, rig, callers: list[SimCaller], noise_rms: float = 0.05, seed: int = 0):
        self.rig = rig
        self.callers = callers
        self.noise_rms = noise_rms
        self.t = 0.0
        self.rng = np.random.default_rng(seed)

    async def read(self, n: int) -> np.ndarray:
        y = np.zeros(n)
        for c in self.callers:
            off = c.dial_hz - self.rig.freq  # USB: caller above us -> higher audio
            if abs(off) > 3000:
                continue
            seg = c.segment(self.t, n, self.fs)
            if not seg.any():
                continue
            # Scale so this caller has its SNR against our noise in 2500 Hz.
            sig_rms = c.params.amplitude / np.sqrt(2)
            noise_inband = self.noise_rms * np.sqrt(2500 / (self.fs / 2))
            gain = noise_inband * 10 ** (c.snr_db / 20) / sig_rms
            y += ssb_channel(seg * gain, off, None, self.fs)
        y += ssb_channel(self.rng.normal(0, self.noise_rms, n), 0, None, self.fs)
        self.t += n / self.fs
        return y


class NullSink:
    fs = FS

    def __init__(self) -> None:
        self.played = 0

    async def play(self, samples: np.ndarray) -> None:
        self.played += len(samples)
