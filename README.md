# Whistlestop

HF keyboard chat, file transfer and group **Roundtables** over the open-source
[Mercury](https://github.com/Rhizomatica/mercury) modem, with a new way to
call CQ.

**The whistle.** A CQ starts with a signature chirp signal ("whee-ooo,
whee-ooo"). Scanning stations hear it from up to ±1 kHz away and measure
exactly how far off they are, to under 1 Hz. They retune onto the caller,
inside Mercury's ±55 Hz capture window, then hear the digital CQ with the
callsign, and link up. There are no time slots and no fixed calling channel.
Scanning is optional: a station parked on one frequency still sees whistles
nearby and can tune to them with one click.

## Features

- **Chat** 1:1 over Mercury ARQ, with "✓ delivered" confirmation and live air-time
  estimates as you type
- **Roundtable**: group conversation wherever the host calls it. The host ramps
  the speed up to the fastest mode every member can copy. Missed pieces are
  re-requested, and the floor can be passed.
- **CQ**: whistle, then Mercury CQFRAME (your ID), then a CALL packet saying
  chat or Roundtable
- **Search**: sweep the current band's digital segment, one band, or all bands;
  stop on a whistle, lock on, answer
- **Files**: offer/accept with predicted time, built-in **zip** for files and
  folders, SHA-256 check, **resume** after a dropped link, drag and drop
- **Time predictions**: messages, files, and a bandwidth × SNR estimate table
- **Bandwidth, narrow → wide**: 250, 500, 1000, 1700, 2300, 2750 Hz
- **Rig control** via rigctld (Hamlib, or RigHelm's built-in server):
  frequency, bands, mode, power, S-meter, tune carrier
- **Wide receiver (optional)**: an RTL-SDR (RX OUT or IF OUT) adds a band
  waterfall (click to tune) and a **wide whistle search**. It watches the
  whole digital segment at once, finds whistles within about 1.5 s, and hands
  them to the main radio to fine-lock. Without a dongle, the one radio does
  the searching.
- **Encryption (optional, off by default)**: 1:1 chat and files can be sealed
  end to end (X25519 + ChaCha20-Poly1305, fresh keys per link, forgotten when
  it ends), with a 6-digit code the two operators compare to rule out a man
  in the middle. **Not legal on amateur bands** in the US (47 CFR
  97.113(a)(4)) or most other countries. Roundtables are never encrypted.
- **Demo mode**: the whole app with a simulated radio, band, Mercury and a
  demo station (K7BOT). No radio needed.

## Run

```bash
cargo run --release
```

Whistlestop builds on **macOS, Windows and Linux** (Rust stable; CI builds
and tests all three, see `.github/workflows/build.yml`):

- **macOS**: nothing extra.
- **Windows**: nothing extra (the MSVC toolchain from rustup). Radios appear as
  `COM` ports; Hamlib's Windows installer is found in Program Files.
- **Linux**: `sudo apt install pkg-config libasound2-dev libudev-dev
  libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev
  libxi-dev libgl1-mesa-dev` (Debian/Ubuntu names). Add yourself to the
  `dialout` group for serial ports.

Demo on real Mercury needs Mercury's simulated-audio link, a Unix socket,
which Mercury only offers on macOS and Linux; on Windows, Demo uses the
simulated modem.

It opens in Demo mode. Click **Search → Current band** and watch it find
K7BOT, or **Call CQ**. Then open **Settings** to enter your callsign and turn
Demo off.

```bash
cargo run --release -- whistle-wav whistle.wav
```

writes the whistle to a WAV file so you can hear it (`docs/whistle-demo.wav`
is a 6 s sample).

## On the air

1. Start **rigctld** (`rigctld -m <model> -r <port>`) or enable RigHelm's
   rigctld server (127.0.0.1:4532).
2. Start **Mercury**. Set its radio to Hamlib model 2 (NET rigctl) at the same
   address, and its sound card to the radio's.
3. In Whistlestop **Settings**: callsign, untick Demo, choose the radio's audio
   in/out, then **Save & connect**.

## Encryption

Settings → **Encryption** has three choices: **Off** (default), **Prefer**
(encrypt when the other station does too) and **Require** (encrypt or hang
up). Turning it on shows the legal warning below and asks you to confirm.

> ⚠ Encryption is **not legal on amateur radio bands** in the US
> (47 CFR 97.113(a)(4)) and in most other countries: amateur transmissions may
> not be encoded to obscure their meaning. Use it only where it is lawful.
> Whistlestop does not check which band you are on.

When a link is encrypted the link header shows **🔒 encrypted** and a code
such as `482 913`. Read it to the other operator by voice or another channel
no one else controls. If both screens show the same code, no one is in the
middle; if not, disconnect. Each link makes new keys, and they are wiped
from memory when it ends. Nothing is written to disk. Sealing costs 17 bytes
per packet. Details: [docs/PROTOCOL.md](docs/PROTOCOL.md#encryption-optional).

## Docs

- [docs/DESIGN.md](docs/DESIGN.md): the whistle, search, CQ cycle, station
  wiring, prior art, limits, roadmap
- [docs/PROTOCOL.md](docs/PROTOCOL.md): WSP, the application protocol

## Layout

| Path | What |
|---|---|
| `src/whistle.rs` | chirp generator, detector, offset estimator |
| `src/engine.rs` | search, whistle watch, CQ, link, files, Roundtable |
| `src/protocol.rs` | WSP encoding and file/Roundtable state |
| `src/e2e.rs` | optional link encryption: key exchange, sealing, SAS |
| `src/tnc.rs` | Mercury TNC client |
| `src/rig.rs` | rigctld client |
| `src/audio.rs` | sound card, simulated band, audio spectrum |
| `src/sdr.rs` | RTL-SDR wide receiver |
| `src/widesearch.rs` | SDR channelizer + wide whistle search |
| `src/modes.rs` | Mercury modes, bandwidths, speed ramp, time predictions |
| `src/fake_mercury.rs` | Mercury stand-in + demo station |
| `src/gui/` | the egui interface |
| `prototype-python/` | first Python prototype of the whistle maths (reference only) |

Tests: `cargo test --release` (82, including click tests that drive the real
window through its accessibility tree, a full headless demo: search →
lock → CQ → connect → chat → zipped file, and the same over an encrypted
link).
