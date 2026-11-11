"""Async client for Mercury's VARA-style TNC (see Mercury docs/TNC.md).

Control port (default 8300): CR-terminated ASCII commands and async status.
Data port (8301): the ARQ byte pipe.
Broadcast port (8100): KISS frames, opened separately via ``Broadcast``.
"""

from __future__ import annotations

import asyncio
import logging

from . import protocol

log = logging.getLogger(__name__)

REPLIES = ("OK", "WRONG", "BUSY")


class TncError(RuntimeError):
    pass


class Tnc:
    def __init__(self, host: str = "127.0.0.1", port: int = 8300):
        self.host, self.port = host, port
        self._listeners: list = []  # callables(line) for async status lines
        self._waiters: list[tuple[tuple[str, ...], asyncio.Future[str]]] = []
        self.data: asyncio.Queue[bytes] = asyncio.Queue()  # bytes from the data port
        self.connected_to: str | None = None
        self.ptt = False
        self._ctl_w: asyncio.StreamWriter | None = None
        self._dat_w: asyncio.StreamWriter | None = None
        self._reply: asyncio.Future[str] | None = None
        self._query: tuple[str, asyncio.Future[str]] | None = None
        self._lock = asyncio.Lock()
        self._tasks: list[asyncio.Task] = []

    async def open(self) -> None:
        cr, self._ctl_w = await asyncio.open_connection(self.host, self.port)
        dr, self._dat_w = await asyncio.open_connection(self.host, self.port + 1)
        self._tasks = [asyncio.create_task(self._read_ctl(cr)), asyncio.create_task(self._read_data(dr))]

    async def close(self) -> None:
        for t in self._tasks:
            t.cancel()
        for w in (self._ctl_w, self._dat_w):
            if w:
                w.close()

    # ------------------------------------------------------------ readers --

    async def _read_ctl(self, r: asyncio.StreamReader) -> None:
        buf = b""
        while True:
            chunk = await r.read(4096)
            if not chunk:
                self._dispatch("EOF")
                return
            buf += chunk
            while b"\r" in buf:
                line, buf = buf.split(b"\r", 1)
                text = line.decode("ascii", "replace").strip()
                if text:
                    self._dispatch(text)

    def _dispatch(self, line: str) -> None:
        log.debug("tnc < %s", line)
        if line in REPLIES and self._reply and not self._reply.done():
            self._reply.set_result(line)
            return
        if self._query and line.startswith(self._query[0]) and not self._query[1].done():
            self._query[1].set_result(line)
            return
        word = line.split()[0] if line.split() else ""
        if word == "CONNECTED":
            parts = line.split()
            # CONNECTED <initiator> <called> <bw>; the peer is whichever isn't us.
            self.connected_to = parts[1] if parts[2] == getattr(self, "mycall", None) else parts[2]
        elif word == "DISCONNECTED":
            self.connected_to = None
        elif line == "PTT ON":
            self.ptt = True
        elif line == "PTT OFF":
            self.ptt = False
        if word == "IAMALIVE":
            return
        for prefixes, fut in list(self._waiters):
            if line.startswith(prefixes) and not fut.done():
                fut.set_result(line)
        for cb in list(self._listeners):
            cb(line)

    def subscribe(self, cb) -> None:
        """Call ``cb(line)`` for every async status line from Mercury."""
        self._listeners.append(cb)

    async def _read_data(self, r: asyncio.StreamReader) -> None:
        while True:
            chunk = await r.read(65536)
            if not chunk:
                return
            await self.data.put(chunk)

    # ----------------------------------------------------------- commands --

    async def command(self, cmd: str, timeout: float = 5.0) -> str:
        async with self._lock:
            assert self._ctl_w, "not open"
            self._reply = asyncio.get_running_loop().create_future()
            log.debug("tnc > %s", cmd)
            self._ctl_w.write(cmd.encode("ascii") + b"\r")
            await self._ctl_w.drain()
            try:
                reply = await asyncio.wait_for(self._reply, timeout)
            except asyncio.TimeoutError as e:
                raise TncError(f"no reply to {cmd!r}") from e
            if reply != "OK":
                raise TncError(f"{cmd!r} -> {reply}")
            return reply

    async def query(self, cmd: str, timeout: float = 5.0) -> str:
        async with self._lock:
            assert self._ctl_w, "not open"
            fut = asyncio.get_running_loop().create_future()
            self._query = (cmd.split()[0], fut)
            self._ctl_w.write(cmd.encode("ascii") + b"\r")
            await self._ctl_w.drain()
            return await asyncio.wait_for(fut, timeout)

    async def setup(self, mycall: str, bw: int = 2300) -> None:
        self.mycall = mycall.upper()
        await self.command(f"MYCALL {self.mycall}")
        await self.command(f"BW{bw}")
        await self.command("LISTEN ON")

    async def cq_frame(self, bw: int = 2300) -> None:
        await self.command(f"CQFRAME {self.mycall} {bw}")

    async def connect(self, theircall: str) -> None:
        await self.command(f"CONNECT {self.mycall} {theircall.upper()}")

    async def disconnect(self) -> None:
        await self.command("DISCONNECT")

    async def send(self, data: bytes) -> None:
        assert self._dat_w, "not open"
        self._dat_w.write(data)
        await self._dat_w.drain()

    async def wait_for(self, *prefixes: str, timeout: float | None = None) -> str:
        """Wait for the next async line starting with one of ``prefixes``."""
        fut: asyncio.Future[str] = asyncio.get_running_loop().create_future()
        entry = (tuple(prefixes), fut)
        self._waiters.append(entry)
        try:
            return await asyncio.wait_for(fut, timeout)
        finally:
            self._waiters.remove(entry)


class Broadcast:
    """Mercury's broadcast plane (KISS over TCP), used for nets."""

    def __init__(self, host: str = "127.0.0.1", port: int = 8100):
        self.host, self.port = host, port
        self.frames: asyncio.Queue[bytes] = asyncio.Queue()
        self._w: asyncio.StreamWriter | None = None
        self._task: asyncio.Task | None = None

    async def open(self) -> None:
        r, self._w = await asyncio.open_connection(self.host, self.port)
        self._task = asyncio.create_task(self._read(r))

    async def close(self) -> None:
        if self._task:
            self._task.cancel()
        if self._w:
            self._w.close()

    async def _read(self, r: asyncio.StreamReader) -> None:
        dec = protocol.KissDecoder()
        while True:
            chunk = await r.read(4096)
            if not chunk:
                return
            for cmd, payload in dec.feed(chunk):
                if cmd in (0x00, 0x01, 0x02):
                    await self.frames.put(payload)

    async def send(self, payload: bytes) -> None:
        assert self._w, "not open"
        self._w.write(protocol.kiss_encode(payload, 0x00))
        await self._w.drain()
