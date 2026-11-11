# WSP — the Whistlestop Protocol (version 1)

WSP is the application layer Whistlestop runs on top of the Mercury modem.
It is built for HF:

* **Every byte costs airtime.** Packets are binary, integers are varints, and
  strings are length-prefixed. A chat line costs its text plus about 4 bytes.
* **Talking never waits for a file.** Files move in small chunks, and a chat
  line is queued ahead of any chunk not yet handed to the modem.
* **Ask before you send.** A file is *offered* with its size, a zip flag and a
  checksum. The receiver sees the predicted time and accepts or declines. An
  accept can ask to **resume** from an offset, so a transfer cut by fading
  picks up where it stopped.
* **Both ends know the link.** Stations exchange SNR reports automatically, so
  each side sees how it is being heard.
* **Roundtables are run, not just broadcast.** The host sets the speed. Members
  report how they hear the host. Anyone can ask for missing pieces again, and
  the floor (the "talking stick") can be passed.

WSP has two planes, matching Mercury's two data paths.

| Plane | Mercury path | Delivery | Used for |
|---|---|---|---|
| **Link** | ARQ, TCP 8300/8301 | reliable, ordered, 1:1 | chat, files, reports |
| **Table** | broadcast, TCP 8100 (KISS) | best effort, one frame each, many | Roundtable, CQ info |

---

## Encoding

| Type | Encoding |
|---|---|
| `u8`, `i8` | one byte |
| `uv` | unsigned LEB128 varint (1 byte below 128) |
| `str` | `uv` length + UTF-8 bytes |
| `bytes` | `uv` length + raw bytes |
| `call` | `str`, upper case, at most 12 bytes |
| `hash32` | 32 bytes, SHA-256 |

## Link plane

Mercury's ARQ delivers a byte stream, and WSP splits it into packets:

```
packet := uv(length of body) body
body   := type(u8) fields...
```

| Type | Name | Fields | Meaning |
|---|---|---|---|
| `0x01` | HELLO | `u8 version`, `u8 caps`, `str name`, `str grid`, `str app` | First thing each side sends after `CONNECTED`. `caps` bit 0 = resume, bit 1 = zip. |
| `0x02` | CHAT | `uv id`, `str text` | A chat line. `id` counts up per sender. |
| `0x03` | RECEIVED | `uv id` | The CHAT `id` arrived. Sent automatically on receipt, so the sender sees "✓ delivered". (No read receipts and no typing indicators: they cost airtime.) |
| `0x04` | OFFER | `uv fid`, `str name`, `uv size`, `hash32 sha`, `u8 flags`, `uv files` | A file is offered. `flags` bit 0 = zip made by Whistlestop. `files` = entries inside the zip. |
| `0x05` | ANSWER | `uv fid`, `u8 accept`, `uv resume_from` | Accept (`1`) or decline (`0`). `resume_from` = bytes already held. |
| `0x06` | CHUNK | `uv fid`, `uv offset`, `bytes data` | Piece of an accepted file, at most 1024 bytes. |
| `0x07` | DONE | `uv fid`, `u8 ok` | The receiver checked the SHA-256: `1` good, `0` bad (sender may offer again). |
| `0x08` | CANCEL | `uv fid` | Either side stops a transfer. |
| `0x09` | REPORT | `i8 snr_db`, `u8 mode_index` | How this station hears the other, sent at most every 30 s while linked. |
| `0x0A` | BYE | `str text` | Leaving (e.g. `73`). Then the sender disconnects. |

**Scheduling.** The sender keeps at most `2 × 1024` bytes of CHUNKs inside
Mercury's TX buffer (it watches Mercury's `BUFFER` reports), and nothing else
is queued behind them. A CHAT therefore waits for at most two chunks, even
while a multi-megabyte file is going.

**Resume.** The receiver keeps a partial file under its SHA-256. When the same
`sha` is offered again, ANSWER carries the length it already has, and the
sender starts there.

## Table plane

Each packet is exactly one Mercury broadcast frame, sent with KISS command
`0x00`. It must fit the listen mode's usable frame (frame − 3 bytes, e.g. 123
bytes on DATAC3). Every frame starts with the magic `0x57` (`W`), so other
users of the broadcast plane (Reticulum, HERMES) can ignore it:

```
frame := 0x57 type(u8) uv(table) call(from) fields...
```

`table` is the FNV-1a hash of the Roundtable's name, folded to 16 bits. It is
`0` for CQ info.

| Type | Name | Fields | Meaning |
|---|---|---|---|
| `0x10` | CALL | `u8 kind`, `str table_name`, `u8 bw_code`, `str grid` | Sent after a CQFRAME: what the CQ is for. `kind` 0 = chat (connect to me), 1 = Roundtable (join on this frequency). |
| `0x11` | TEXT | `uv id`, `u8 part`, `u8 total`, `bytes data` | One part of a Roundtable message. |
| `0x12` | JOIN | `str table_name`, `i8 snr_of_host` | A station joins, or re-reports how it hears the host. |
| `0x13` | SPEED | `u8 mode_index` | Host only: everyone moves to this mode after the frame. Sent twice, in the *old* mode. |
| `0x14` | AGAIN | `call of`, `uv id`, `uv missing` | Please resend the parts of `of`'s message `id` in the bitmap `missing` (bit n = part n+1). |
| `0x15` | FLOOR | `call to` | Pass the talking stick to `to`. Holding it is a courtesy, not a lock. |
| `0x16` | LEAVE | — | Leaving the table. |

**Speed ramp-up.** The host keeps the lowest `snr_of_host` reported in the
last 5 minutes. It moves the table to the fastest mode, within the chosen
bandwidth, whose required SNR plus a 3 dB margin is below that figure. It
moves down at once if a member reports worse. It moves up only once nobody
has reported worse for 2 minutes.

## CQ sequence (whole cycle)

1. **Whistle**: chirp train, see DESIGN.md.
2. **CQFRAME**: Mercury's own, carries the callsign (station ID).
3. **CALL**: table plane, says what the CQ is for.
4. Listen: a chat CQ is answered with a Mercury `CONNECT`. A Roundtable CQ is
   answered with JOIN.

## Versioning

HELLO carries `version`. A station that receives a higher version uses the
lower one. Unknown packet types are skipped: link packets by their length,
table frames whole.
