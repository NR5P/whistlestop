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
| `call` | `str`, upper case, at most 12 bytes. A receiver accepts only 1–12 of `A–Z a–z 0–9 / -`; a packet or frame with any other callsign is dropped whole. |
| `hash32` | 32 bytes, SHA-256 |

## Link plane

Mercury's ARQ delivers a byte stream, and WSP splits it into packets:

```
packet := uv(length of body) body
body   := type(u8) fields...
```

| Type | Name | Fields | Meaning |
|---|---|---|---|
| `0x01` | HELLO | `u8 version`, `u8 caps`, `str name`, `str grid`, `str app` | First thing each side sends after `CONNECTED`. `caps` bit 0 = resume, bit 1 = zip, bit 2 = wants encryption (see below). |
| `0x02` | CHAT | `uv id`, `str text` | A chat line. `id` counts up per sender. |
| `0x03` | RECEIVED | `uv id` | The CHAT `id` arrived. Sent automatically on receipt, so the sender sees "✓ delivered". (No read receipts and no typing indicators: they cost airtime.) |
| `0x04` | OFFER | `uv fid`, `str name`, `uv size`, `hash32 sha`, `u8 flags`, `uv files` | A file is offered. `flags` bit 0 = zip made by Whistlestop. `files` = entries inside the zip. |
| `0x05` | ANSWER | `uv fid`, `u8 accept`, `uv resume_from` | Accept (`1`) or decline (`0`). `resume_from` = bytes already held. |
| `0x06` | CHUNK | `uv fid`, `uv offset`, `bytes data` | Piece of an accepted file, at most 1024 bytes. Bytes beyond the offered size are ignored. |
| `0x07` | DONE | `uv fid`, `u8 ok` | The receiver checked the SHA-256: `1` good, `0` bad (sender may offer again). |
| `0x08` | CANCEL | `uv fid` | Either side stops a transfer. The fid is the offering station's, so a CANCEL can name a file in either direction; Whistlestop starts its own fids at a random number below 2^20 so the two stations' fids don't meet. |
| `0x09` | REPORT | `i8 snr_db`, `u8 mode_index` | How this station hears the other, sent at most every 30 s while linked. |
| `0x0A` | BYE | `str text` | Leaving (e.g. `73`). Then the sender disconnects. |
| `0x0B` | KEYC | `u8 suite`, `hash32 commit` | Encryption only: commitment to this link's public key. |
| `0x0C` | KEYX | `u8 suite`, `32 bytes pubkey` | Encryption only: the public key, sent after the peer's KEYC arrives. |
| `0x0D` | SEALED | `ciphertext + 16-byte tag` (rest of body) | Encryption only: one encrypted inner packet. |
| `0x0E` | MAIL | `uv id`, `call from`, `call to`, `uv created`, `u8 hops`, `str text` | Depot mail, handed over when linked. `to` = this station: it goes to Arrivals; otherwise (relaying on) In Transit. `hops` counts the relays that have carried it: a relay stores `hops + 1` and refuses mail once that exceeds 3. `from`/`to` are callsigns (see `call`); posting to anything else is refused. |
| `0x0F` | MAILACK | `uv id` | Mail `id` is safely stored; the sender moves it to Delivered. |
| `0x20` | QSY | `uv id`, `uv dial_hz`, `str why` | "Let's both move to `dial_hz`." The other station decides by its own rules. |
| `0x21` | QSYANS | `uv id`, `u8 accept`, `str reason` | The answer. When accepted, the asker moves 3 s after it arrives and the answerer once Mercury has sent everything (plus 2 s). Both return to where they were when the link ends. |
| `0x22` | LIST | `uv id`, `str path` | "What's in your library at `path`?" (`""` = the top). |
| `0x23` | LISTANS | `uv id`, `str path`, `str note`, `uv n`, n × (`str name`, `uv size`, `u8 dir`) | The listing (at most 200 entries), or none and why in `note`. |
| `0x24` | FETCH | `uv id`, `str path` | "Please send me this library file." |
| `0x25` | FETCHANS | `uv id`, `u8 ok`, `str reason` | Refused (with `reason`), or OK: an OFFER with `fid = id` follows, which the asker accepts automatically. An `id` equal to a file the sender is already offering is refused ("busy: ask again"). The offer is for this link only: it isn't repeated on a later link. |

**Scheduling.** The sender keeps at most `2 × 1024` bytes of CHUNKs inside
Mercury's TX buffer (it watches Mercury's `BUFFER` reports), and nothing else
is queued behind them. A CHAT therefore waits for at most two chunks, even
while a multi-megabyte file is going.

**Resume.** The receiver keeps a partial file under its SHA-256. When the same
`sha` is offered again, ANSWER carries the length it already has, and the
sender starts there.

## Encryption (optional)

> ⚠ **Not legal on amateur bands.** In the US, 47 CFR 97.113(a)(4) forbids
> "messages encoded for the purpose of obscuring their meaning", and most
> other countries have the same rule. Encryption is off by default, and the
> app shows this warning and asks for confirmation before turning it on.
> A **signing-only** mode (authentication, no secrecy: every packet carries a
> tag but stays readable) would be legal on ham bands and could be added
> later with the same handshake; it is not built yet.

Scope: the **link plane** only (chat, files, reports). The table plane
(Roundtables, CQ info) is always plain text, since it is broadcast to
everyone.

**Setting.** `e2e` in the config: `off` (default), `prefer` (encrypt if the
peer also wants to, otherwise plain text), `require` (never plain text).
A station with `prefer` or `require` sets HELLO `caps` bit 2 (`CAP_E2E = 4`);
the bit means "wants encryption on this link", not merely "could".

**Suite 1.** X25519 key agreement, HKDF-SHA256, ChaCha20-Poly1305 with a
16-byte tag.

**Handshake.** Each station makes a new X25519 key pair for every link.

```
both: HELLO (caps | CAP_E2E), KEYC(suite, commit = SHA-256("WSP1 E2E commit" ‖ suite ‖ pubkey))
both, on the peer's KEYC:  KEYX(suite, pubkey)
both, on the peer's KEYX:  check SHA-256(...) = the peer's commit, then derive keys
```

Neither side reveals its public key until it holds the other's commitment.
Without this, a man in the middle could try many keys of its own until the
short authentication string came out the same on both sides. With it, the
codes of two separately relayed links match by chance only (1 in 10⁶).

The handshake is symmetric. The stations are ordered by public key (the
lower key is "lo"), not by who called, so it does not depend on how Mercury
words `CONNECTED`. It costs 70 bytes each way (KEYC 35 + KEYX 35), and the
first sealed packet can go after two one-way trips.

**Keys.**

```
shared = X25519(my secret, peer pubkey)          (rejected if all-zero)
salt   = SHA-256("WSP1 E2E transcript" ‖ suite ‖ u32be(len) ‖ HELLO_lo ‖ u32be(len) ‖ HELLO_hi ‖ pub_lo ‖ pub_hi)
K_lo→hi = HKDF-SHA256(salt, shared, "WSP1 E2E lo->hi", 32)
K_hi→lo = HKDF-SHA256(salt, shared, "WSP1 E2E hi->lo", 32)
SAS     = u32be(HKDF-SHA256(salt, shared, "WSP1 E2E SAS", 4)) mod 10⁶, shown as "ddd ddd"
```

HELLO bodies are hashed exactly as sent (type byte onward). A change to
either HELLO by anyone in between gives different keys, so the first SEALED
packet fails.

**SEALED.** `body = 0x0D ‖ ChaCha20-Poly1305(K_dir, nonce, inner body, aad = 0x0D)`.
The inner body is a normal link packet body (type byte onward, no length
prefix; the outer length covers it). The nonce is `4 zero bytes ‖ u64le(n)`,
where `n` counts the packets sent in that direction from 0. It is never sent:
Mercury's ARQ delivers in order, so the receiver uses the next count it
expects. A replayed, dropped, reordered or altered packet fails the tag
check, and the receiver hangs up. HELLO, KEYC, KEYX and SEALED are never
valid inside SEALED.

**Overhead.** 17 bytes per packet (type byte + tag), plus 1 more when the
length prefix crosses 128 bytes. A "73" chat line goes from 6 to 23 bytes.
A full 1024-byte file chunk (about 1030 bytes on the air) grows by 17 bytes, 1.6 %.

**Ordering and fallback.**

* Until keys are agreed (or the link is known to be plain), only HELLO,
  KEYC and KEYX are sent. Chat, files and reports wait in the queue.
* `prefer`: if the peer's HELLO lacks `CAP_E2E` (or the peer sends plain
  packets before any HELLO), the link runs in plain text and the operator is
  told so. An attacker on the path can force this, which is what `require`
  is for.
* `require`: a peer HELLO without `CAP_E2E`, or no keys within 120 s, makes
  the station send `BYE "encryption problem, disconnecting"` and disconnect,
  with the reason in the chat. It never falls back.
* Once encrypting, any plain application packet is a protocol error: the
  station hangs up.
* A station with `off` ignores KEYC and KEYX. Older versions skip them as
  unknown types. Either way the encrypting side sees a HELLO without
  `CAP_E2E`.

**Forgetting.** Key pairs, the shared secret and session keys live in
memory only, in types that wipe themselves on drop. The engine drops the
link's security state at `DISCONNECTED`. Nothing is written to disk.

**Showing it.** The link header shows "🔒 encrypted" and the code. The
operators read the code to each other once (by voice, or any channel no one
else controls). If it matches, no one is in the middle.

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
| `0x11` | TEXT | `uv id`, `u8 part`, `u8 total`, `bytes data` | One part of a Roundtable message. `1 ≤ part ≤ total ≤ 64` (AGAIN's bitmap is 64 bits); other values are dropped. |
| `0x12` | JOIN | `str table_name`, `i8 snr_of_host`, optional `u8 bw_code` | A station joins, or re-reports how it hears the host: a member sends it again every 2 min, with the SNR Mercury measured on the host's own frames (on joining, on the host's CALL). `snr_of_host` = −128 means not measured yet: the host leaves it out of its speed choice (an older host reads it as a very weak member and slows down). `bw_code` is the member's bandwidth setting, coded as in CALL; the host never picks a mode wider than the narrowest member's (absent: an older station; older stations ignore the byte). The host counts a member's report for 7 min. `table_name` is empty when the frame wouldn't fit the mode's broadcast frame with it (DATAC15: 27 bytes); the host goes by `table`. |
| `0x13` | SPEED | `u8 mode_index`, optional `uv last_id` | Host only: `last_id` is the host's newest message id (absent or 0: none; older stations ignore it). A member that sees a gap in a sender's ids (from TEXT ids or `last_id`) sends AGAIN for each skipped message with `missing` = all ones (at most 16 per sender). Everyone moves to this mode after the frame. Sent twice, in the *old* mode. **Join windows**: newcomers call and listen in DATAC3, so the host of a table running faster drops it to DATAC3 every 3 min (a SPEED to DATAC3, then its CALL), listens 20 s for JOINs, then sends SPEED back up. A host with nothing to send repeats SPEED for the current mode every 90 s as a heartbeat. A member that hears nothing of its table for 4 min tries the last announced mode, DATAC3, then the most robust mode, re-sending its JOIN in each. A member whose own JOIN report is below the current mode's minimum + margin, and which hears no SPEED within 40 s, moves to DATAC3 by itself. No new frame type: any station that follows SPEED takes part. |
| `0x14` | AGAIN | `call of`, `uv id`, `uv missing` | Please resend the parts of `of`'s message `id` in the bitmap `missing` (bit n = part n+1). When the frame wouldn't fit the mode's broadcast frame, the highest bits are dropped (asked for in a later round). |
| `0x15` | FLOOR | `call to` | Pass the talking stick to `to`. Holding it is a courtesy, not a lock. |
| `0x16` | LEAVE | — | Leaving the table. The host's LEAVE closes it: members leave too. |
| `0x17` | BEACON | `str grid`, `u8 away`, `uv n`, n × `call` | Station ID: grid, whether the station is away, and the calls it holds mail for (as many as fit). No whistle. |
| `0x18` | NOTICE | `call to`, `u8 count` | "I have `count` messages for `to`: connect to me." |

**Fitting small frames.** Mercury cuts a broadcast longer than the mode's
frame (its size less 3 bytes: 27 in DATAC15, 51 in DATAC4, 123 in DATAC3),
so every table frame must fit the mode it goes out in. A sender whose TEXT
frames were made for a faster mode (queued, or kept for AGAIN, before the
table slowed down) splits that message again, same `id`, into a different
number of parts, and sends all of them; a receiver holding parts with
another `total` starts the message over (receivers before 2026-11-13
kept the parts they had and dropped the new ones). If the new split needs
more parts than AGAIN can name at the new speed, the rest is sent as
further messages with new ids. Where AGAIN can't name every part
of a message in one frame (its bitmap grows with the part number), a long
message is sent as several, each of at most as many parts as AGAIN can
name. The host leaves out of the ramp any mode in which some station at
the table couldn't send TEXT, or an AGAIN for 8 parts to any other (calls
of 8+ characters in DATAC15).

**Speed ramp-up.** The host keeps the lowest `snr_of_host` reported in the
last 5 minutes. It moves the table to the fastest mode, within the chosen
bandwidth, whose required SNR plus a 3 dB margin is below that figure. It
moves down at once if a member reports worse. It moves up only once nobody
has reported worse for 2 minutes (doubled, up to 32, each time a move up
is undone within 5 minutes: Mercury's SNR reads higher in the slow modes),
and nothing has been heard from the table for two frames (a member sending
can't hear the SPEED). A station hands
Mercury its next broadcast frame only after Mercury's PTT OFF for the last:
Mercury keeps queued broadcast frames as one byte stream and, after a MODE
change, reads what's left at the new frame size (garbled from then on).

## CQ sequence (whole cycle)

1. **Whistle**: chirp train, see DESIGN.md.
2. **CQFRAME**: Mercury's own, carries the callsign (station ID).
3. **CALL**: table plane, says what the CQ is for.
4. Listen: a chat CQ is answered with a Mercury `CONNECT`. A Roundtable CQ is
   answered with JOIN.

## Versioning

HELLO carries `version`. A station that receives a higher version uses the
lower one. Unknown packet types are skipped: link packets by their length,
table frames whole. A known packet with a field that doesn't decode (say, a
bad callsign) is skipped too, on a plain link and inside an authenticated
SEALED; only during the encryption handshake does it end the link. The length
prefix is at most 4 bytes and 1 MiB: anything else means the stream is
corrupt, and since it has no resync point the receiver ends the link.
