"""Radio control through Hamlib's ``rigctld`` (TCP, default port 4532).

rigctld owns the radio's serial port; Mercury (Hamlib model 2, NET rigctl)
and Whistlestop both talk to it, so nobody fights over the port.
"""

from __future__ import annotations

import asyncio


class RigError(RuntimeError):
    pass


class Rig:
    def __init__(self, host: str = "127.0.0.1", port: int = 4532):
        self.host, self.port = host, port
        self._r: asyncio.StreamReader | None = None
        self._w: asyncio.StreamWriter | None = None
        self._lock = asyncio.Lock()

    async def open(self) -> None:
        self._r, self._w = await asyncio.open_connection(self.host, self.port)

    async def close(self) -> None:
        if self._w:
            self._w.close()

    async def _cmd(self, cmd: str) -> str:
        async with self._lock:
            assert self._r and self._w, "not open"
            self._w.write((cmd + "\n").encode())
            await self._w.drain()
            line = (await asyncio.wait_for(self._r.readline(), 5)).decode().strip()
            if line.startswith("RPRT") and line != "RPRT 0":
                raise RigError(f"{cmd!r} -> {line}")
            return line

    async def set_freq(self, hz: float) -> None:
        await self._cmd(f"F {int(round(hz))}")

    async def get_freq(self) -> float:
        return float(await self._cmd("f"))

    async def ptt(self, on: bool) -> None:
        await self._cmd(f"T {1 if on else 0}")


class FakeRig:
    """In-memory rig for tests and the simulator."""

    def __init__(self, freq: float = 14_100_000.0):
        self.freq = freq
        self.keyed = False
        self.history: list[float] = []

    async def open(self) -> None: ...
    async def close(self) -> None: ...

    async def set_freq(self, hz: float) -> None:
        self.freq = float(round(hz))
        self.history.append(self.freq)

    async def get_freq(self) -> float:
        return self.freq

    async def ptt(self, on: bool) -> None:
        self.keyed = on
