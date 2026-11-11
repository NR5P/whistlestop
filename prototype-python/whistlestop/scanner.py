"""Scan a band segment for whistles; stop, retune onto the caller.

The scanner is driven by *audio samples*, not wall-clock time: it asks the
audio source for "dwell" seconds of samples on each channel.  With a live
sound card that takes real time; with the simulator it runs instantly, which
is what makes it testable.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Awaitable, Callable, Protocol

import numpy as np

from .whistle import DEFAULT, Detection, Detector, WhistleParams

log = logging.getLogger(__name__)


class AudioSource(Protocol):
    fs: int

    async def read(self, n: int) -> np.ndarray: ...


class RigLike(Protocol):
    async def set_freq(self, hz: float) -> None: ...


@dataclass
class ScanPlan:
    start_hz: float
    stop_hz: float
    step_hz: float = 1800.0
    dwell_s: float = 2.2
    settle_s: float = 0.15  # discarded after each retune (relays, AGC)
    hold_s: float = 1.5  # extra listening when one chirp is heard but no pair yet

    def channels(self) -> list[float]:
        n = int(np.floor((self.stop_hz - self.start_hz) / self.step_hz)) + 1
        return [self.start_hz + i * self.step_hz for i in range(max(n, 1))]

    @property
    def cycle_s(self) -> float:
        """Time to visit every channel once (no holds). A caller's whistle
        should be at least this long."""
        return len(self.channels()) * (self.dwell_s + self.settle_s)


@dataclass
class Found:
    dial_hz: float  # where the radio is now, on the caller
    scanned_hz: float  # the channel the whistle was first heard on
    first: Detection
    verify: Detection | None


class Scanner:
    def __init__(
        self,
        rig: RigLike,
        audio: AudioSource,
        plan: ScanPlan,
        params: WhistleParams = DEFAULT,
        on_status: Callable[[str], None] | None = None,
        verify_tolerance_hz: float = 50.0,
    ):
        self.rig, self.audio, self.plan = rig, audio, plan
        self.detector = Detector(params)
        self.on_status = on_status or (lambda s: None)
        self.verify_tolerance_hz = verify_tolerance_hz
        self.running = False

    async def _listen(self, seconds: float) -> np.ndarray:
        return await self.audio.read(int(seconds * self.audio.fs))

    async def _tune(self, hz: float) -> None:
        await self.rig.set_freq(hz)
        await self._listen(self.plan.settle_s)

    async def check_channel(self, hz: float) -> Detection | None:
        await self._tune(hz)
        buf = await self._listen(self.plan.dwell_s)
        det = self.detector.detect(buf)
        if det is None and self.detector.peak_level(buf) >= self.detector.threshold:
            # One chirp heard but not a full pair yet: hold a little longer.
            self.on_status(f"{hz / 1e3:.1f} kHz: something whistle-like, holding")
            more = await self._listen(self.plan.hold_s)
            det = self.detector.detect(np.concatenate([buf, more]))
        return det

    async def scan(self, max_cycles: int | None = None) -> Found | None:
        """Scan until a whistle is found (returns Found) or ``max_cycles``
        complete passes find nothing (returns None) or stop() is called."""
        self.running = True
        cycles = 0
        while self.running and (max_cycles is None or cycles < max_cycles):
            for ch in self.plan.channels():
                if not self.running:
                    return None
                self.on_status(f"scanning {ch / 1e3:.1f} kHz")
                det = await self.check_channel(ch)
                if det is None:
                    continue
                self.on_status(
                    f"WHISTLE on {ch / 1e3:.1f} kHz, offset {det.offset_hz:+.1f} Hz "
                    f"(score {det.score:.2f}, {det.n_pairs} pairs); retuning"
                )
                found = await self._lock_on(ch, det)
                if found:
                    self.running = False
                    return found
            cycles += 1
        self.running = False
        return None

    async def _lock_on(self, ch: float, det: Detection) -> Found | None:
        target = ch + det.offset_hz
        await self._tune(target)
        buf = await self._listen(self.plan.dwell_s)
        verify = self.detector.detect(buf)
        if verify is None:
            # Whistle ended while we retuned; the first estimate is good to <1 Hz.
            return Found(target, ch, det, None)
        if abs(verify.offset_hz) > self.verify_tolerance_hz:
            # A caller more than the unambiguous range away aliased; resume.
            self.on_status(f"verify off by {verify.offset_hz:+.0f} Hz, resuming scan")
            return None
        if abs(verify.offset_hz) >= 2:
            target += verify.offset_hz
            await self.rig.set_freq(target)
        return Found(target, ch, det, verify)

    def stop(self) -> None:
        self.running = False


class AudioSink(Protocol):
    fs: int

    async def play(self, samples: np.ndarray) -> None: ...


async def blow_whistle(
    rig, sink: AudioSink, seconds: float, params: WhistleParams = DEFAULT
) -> None:
    """Key the radio and send a whistle of ``seconds``."""
    from .whistle import whistle

    p = WhistleParams(**{**params.__dict__, "fs": sink.fs})
    audio = whistle(seconds, p)
    await rig.ptt(True)
    try:
        await sink.play(audio)
    finally:
        await rig.ptt(False)


async def call_cq(
    rig,
    sink: AudioSink,
    tnc,
    whistle_s: float,
    listen_s: float = 15.0,
    cycles: int = 5,
    bw: int = 2300,
    on_status: Callable[[str], None] | None = None,
    should_continue: Callable[[], bool] = lambda: True,
) -> str | None:
    """The CQ cycle: whistle, Mercury CQFRAME, listen for a CONNECT.
    Returns the CONNECTED line or None if nobody answered."""
    import asyncio

    say = on_status or (lambda s: None)
    for i in range(cycles):
        if not should_continue():
            return None
        say(f"CQ {i + 1}/{cycles}: whistling {whistle_s:.0f} s")
        await blow_whistle(rig, sink, whistle_s)
        say("sending CQFRAME")
        await tnc.cq_frame(bw)
        try:
            await tnc.wait_for("CANCELPENDING", timeout=20)
            line = await tnc.wait_for("CONNECTED", timeout=listen_s)
            return line
        except asyncio.TimeoutError:
            continue
    return None
