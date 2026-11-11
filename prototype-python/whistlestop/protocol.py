"""Wire formats that Whistlestop puts on top of Mercury.

* Direct (ARQ) messages: ``type(1) | length(4, big-endian) | payload``
* Net messages over the broadcast plane: ``WS1|from|net|msgid|i/n|text``
* KISS framing for Mercury's broadcast TCP port (8100)
"""

from __future__ import annotations

import hashlib
import json
import os
import struct
from dataclasses import dataclass

# ---------------------------------------------------------------- direct ----

CHAT = b"C"
FILE_HEADER = b"F"
FILE_DATA = b"D"
FILE_END = b"E"
BYE = b"B"

MAX_PAYLOAD = 1 << 24
FILE_CHUNK = 4096


@dataclass
class Message:
    kind: bytes
    payload: bytes

    @property
    def text(self) -> str:
        return self.payload.decode("utf-8", errors="replace")


def encode(kind: bytes, payload: bytes = b"") -> bytes:
    if len(payload) > MAX_PAYLOAD:
        raise ValueError("payload too large")
    return kind + struct.pack(">I", len(payload)) + payload


def encode_chat(text: str) -> bytes:
    return encode(CHAT, text.encode("utf-8"))


class StreamDecoder:
    """Turns the raw byte stream from Mercury's data port back into messages."""

    def __init__(self) -> None:
        self._buf = bytearray()

    def feed(self, data: bytes) -> list[Message]:
        self._buf += data
        out: list[Message] = []
        while len(self._buf) >= 5:
            n = struct.unpack(">I", self._buf[1:5])[0]
            if n > MAX_PAYLOAD:
                raise ValueError("corrupt stream: length %d" % n)
            if len(self._buf) < 5 + n:
                break
            out.append(Message(bytes(self._buf[:1]), bytes(self._buf[5 : 5 + n])))
            del self._buf[: 5 + n]
        return out


def file_messages(path: str):
    """Yield the encoded messages that send one file."""
    size = os.path.getsize(path)
    h = hashlib.sha256()
    with open(path, "rb") as f:
        data = f.read()
    h.update(data)
    header = {"name": os.path.basename(path), "size": size, "sha256": h.hexdigest()}
    yield encode(FILE_HEADER, json.dumps(header).encode())
    for i in range(0, size, FILE_CHUNK):
        yield encode(FILE_DATA, data[i : i + FILE_CHUNK])
    yield encode(FILE_END, h.hexdigest().encode())


class FileReceiver:
    """Collects FILE_HEADER / FILE_DATA / FILE_END into a file on disk."""

    def __init__(self, directory: str) -> None:
        self.directory = directory
        self._hdr: dict | None = None
        self._data = bytearray()

    @property
    def active(self) -> bool:
        return self._hdr is not None

    @property
    def progress(self) -> tuple[int, int]:
        return len(self._data), (self._hdr or {}).get("size", 0)

    def handle(self, msg: Message) -> str | None:
        """Returns the saved path when a file completes; raises on a bad file."""
        if msg.kind == FILE_HEADER:
            self._hdr = json.loads(msg.payload)
            self._data = bytearray()
        elif msg.kind == FILE_DATA and self._hdr is not None:
            self._data += msg.payload
        elif msg.kind == FILE_END and self._hdr is not None:
            hdr, data = self._hdr, bytes(self._data)
            self._hdr, self._data = None, bytearray()
            if hashlib.sha256(data).hexdigest() != hdr["sha256"]:
                raise ValueError(f"checksum mismatch on {hdr['name']}")
            os.makedirs(self.directory, exist_ok=True)
            name = os.path.basename(hdr["name"]) or "received.bin"  # no path tricks
            path = os.path.join(self.directory, name)
            base, ext = os.path.splitext(path)
            i = 1
            while os.path.exists(path):
                path = f"{base}-{i}{ext}"
                i += 1
            with open(path, "wb") as f:
                f.write(data)
            return path
        return None


# ------------------------------------------------------------------ net -----

NET_TAG = "WS1"


@dataclass
class NetPart:
    sender: str
    net: str
    msgid: int
    part: int
    total: int
    text: str


def net_split(sender: str, net: str, msgid: int, text: str, frame_bytes: int) -> list[bytes]:
    """Split a net message into broadcast payloads that each fit one modem
    frame (``frame_bytes`` = the listen mode's payload minus Mercury's 3)."""
    raw = text.encode("utf-8")
    # Header size with worst-case part numbers; leave room for it.
    hdr_room = len(f"{NET_TAG}|{sender}|{net}|{msgid}|999/999|".encode())
    room = frame_bytes - hdr_room
    if room < 8:
        raise ValueError("frame too small for net messages")
    chunks: list[bytes] = []
    while raw:
        cut = min(room, len(raw))
        while cut < len(raw) and cut > 0 and (raw[cut] & 0xC0) == 0x80:
            cut -= 1  # never split a UTF-8 character
        chunks.append(raw[:cut])
        raw = raw[cut:]
    chunks = chunks or [b""]
    n = len(chunks)
    return [f"{NET_TAG}|{sender}|{net}|{msgid}|{i + 1}/{n}|".encode() + c for i, c in enumerate(chunks)]


def net_parse(payload: bytes) -> NetPart | None:
    try:
        tag, sender, net, msgid, pn, text = payload.decode("utf-8").split("|", 5)
        part, total = pn.split("/")
        if tag != NET_TAG:
            return None
        return NetPart(sender, net, int(msgid), int(part), int(total), text)
    except (UnicodeDecodeError, ValueError):
        return None


class NetAssembler:
    def __init__(self) -> None:
        self._parts: dict[tuple[str, int], dict[int, str]] = {}

    def add(self, p: NetPart) -> str | None:
        key = (p.sender, p.msgid)
        got = self._parts.setdefault(key, {})
        got[p.part] = p.text
        if len(got) == p.total:
            del self._parts[key]
            return "".join(got[i] for i in range(1, p.total + 1))
        return None


# ----------------------------------------------------------------- KISS -----

FEND, FESC, TFEND, TFESC = 0xC0, 0xDB, 0xDC, 0xDD


def kiss_encode(payload: bytes, cmd: int = 0x00) -> bytes:
    out = bytearray([FEND, cmd])
    for b in payload:
        if b == FEND:
            out += bytes([FESC, TFEND])
        elif b == FESC:
            out += bytes([FESC, TFESC])
        else:
            out.append(b)
    out.append(FEND)
    return bytes(out)


class KissDecoder:
    def __init__(self) -> None:
        self._buf = bytearray()
        self._in = False
        self._esc = False

    def feed(self, data: bytes) -> list[tuple[int, bytes]]:
        out: list[tuple[int, bytes]] = []
        for b in data:
            if b == FEND:
                if self._in and len(self._buf) >= 1:
                    out.append((self._buf[0], bytes(self._buf[1:])))
                self._buf = bytearray()
                self._in = True
                self._esc = False
                continue
            if not self._in:
                continue
            if self._esc:
                self._buf.append(FEND if b == TFEND else FESC if b == TFESC else b)
                self._esc = False
            elif b == FESC:
                self._esc = True
            else:
                self._buf.append(b)
        return out
