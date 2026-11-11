# Whistlestop User Guide

Everything about Whistlestop: what each part of the screen does, how calling, searching, chatting, files and Roundtables work, how to set up a station, and what to do when something goes wrong. This same guide opens in its own window from the **📖 Guide** button in the top bar, and turning on **? Help** mode explains any control you hover over.

## Welcome

Whistlestop is an HF keyboard chat, file transfer and group conversation program for amateur and other radio services. It runs on top of **Mercury**, the open-source HF data modem from Rhizomatica, which does the modulation, error correction and acknowledgements. Whistlestop adds what an operator works with:

- **The whistle**: a signature chirp sound that starts every CQ. Stations searching the band hear it, measure exactly how far off frequency they are, retune onto you, and hear your digital CQ. There are no time slots and no fixed calling channel.
- **Chat**: 1:1 keyboard conversation, with a "✓ delivered" confirmation for every message.
- **Files**: send files and whole folders, zipped automatically, offered first so the other station sees the size and time before accepting, checked end to end, and resumed if the link drops.
- **Roundtables**: a group conversation on one frequency, run by a host who speeds the group up when everyone hears well and slows it down when someone fades.
- **Search**: sweep a band's digital segment with your one radio, or watch the whole segment at once with an optional RTL-SDR second receiver.
- **Honest numbers**: the modem strip shows exactly what the modem is doing (mode, modulation, carriers, bandwidth, speed, error correction, SNR), and every message and file shows how long it will take.

> Whistlestop is new. Its whistle and protocol are tested in simulation and with two real Mercury modems on one computer, but not yet on the air. Treat on-air results as experiments, and report what you find.

## Quick start: Demo mode

The first time you open Whistlestop it starts in **Demo mode**: a simulated radio, a simulated 20 m band, a simulated Mercury modem and a demo station, **K7BOT**, who calls CQ every 45 seconds. Nothing is transmitted. Try this:

1. Look at the **Radio audio** waterfall at the bottom left. Every so often you'll see K7BOT's whistle as slanted strokes.
2. In **Search for whistles**, press **Current band**. Your radio steps through the 20 m digital segment. Within about one pass it stops, says "Whistle! measuring", retunes, and waits for the CQ.
3. When K7BOT's CQ is heard it appears under **Heard**. Press **Answer** (or tick **Answer automatically** before searching).
4. Once linked, type in **Chat** and press Enter. K7BOT answers. Ask it for a file: it offers one in the **Files** tab.
5. Drop a file on the window to send one back.
6. Try **Call CQ → Roundtable** to host a table. K7BOT joins.

When you're ready for the air, open **Settings**, enter your callsign, untick **Demo mode**, and follow "Setting up your station" below.

## Setting up your station

You need four things: a radio with CAT control, an audio interface to it (most modern radios have USB audio built in), the **Mercury** modem, and a **rig control server** that both Mercury and Whistlestop share.

### 1. Rig control

Only one program can open the radio's CAT (serial/USB) port. So a rig control *server* owns it, and everything else (Whistlestop and Mercury) talks to that server over the network on this computer. In **Settings → Radio connection** choose one of two ways:

**Let Whistlestop run rigctld** (easiest). Whistlestop starts Hamlib's `rigctld` for your radio and stops it when you quit.

- Install Hamlib once (macOS: `brew install hamlib`).
- Press **🔍 Detect radios**. Whistlestop looks at the USB devices macOS already knows about and recognises:
  - Icom radios by name (an IC-7300 names itself in its USB serial number), plus the IC-705 and IC-905 by Icom's own USB IDs;
  - Yaesu radios by their two-port USB chip (CAT is on the first port);
  - Kenwood, Elecraft, QRP Labs (QMX/QDX, which speak Kenwood TS-480), Xiegu and others by their USB names.
- Press **Use** beside your radio, or choose **Model**, **Port** and **Speed** yourself. Leaving them on **Automatic** detects them every time Whistlestop starts.
- **Detection never opens a port.** Opening a serial port raises its RTS and DTR lines, and on some radios that keys the transmitter. When Whistlestop starts rigctld, it holds RTS and DTR off.
- rigctld listens on `127.0.0.1:4532` (change **rigctld port** if something else uses it). Point Mercury there.

**Use a running rigctld / RigHelm**. If something already owns the radio:

- **RigHelm**: enable its built-in rigctld server (default `127.0.0.1:4532`).
- **Hamlib rigctld** you start yourself: `rigctld -m <model> -r <port>` (find the model number with `rigctl -l`).

Set **rigctld address** to it. Either way, the **Radio** pill in the top bar turns green when connected, and the log names the radio Hamlib reports.

### 2. Mercury

Install Mercury (macOS: drag `Mercury.app` to Applications). Then set it up so it shares the radio:

- **Radio / PTT**: Hamlib model **2 (NET rigctl)** pointing at the same rigctld address (`127.0.0.1:4532` by default, whether Whistlestop runs rigctld or something else does). Mercury then keys the transmitter through the same server.
- **Sound card**: the radio's USB audio codec, both directions.
- **Ports**: leave the TNC port at **8300** (data on 8301) and the broadcast port at **8100**, or set the same numbers in Whistlestop **Settings → Mercury modem**.

The **Mercury** pill turns green when Whistlestop is connected. Mercury accepts one program at a time on its TNC port, so close VarAC or any other program that uses it.

### 3. Audio

Whistlestop listens to the radio audio itself (for the whistle detector and the waterfall) and plays the whistle when you call CQ. In **Settings → Radio & audio**, choose the radio's USB audio device for **Audio in** and **Audio out**. The **Audio** pill shows the input is running.

Set levels so Mercury's own level meters sit in their recommended range. The whistle is sent at a moderate level (half of full scale), close to Mercury's own transmit level. Watch your ALC: it should barely move.

### 4. Radio settings

- Mode **USB** or **PKTUSB** (data mode), filter at least **2.4 kHz** wide for wide bandwidths (narrow filters work with the 500 Hz settings).
- Turn off speech processing, noise reduction and noise blanker on receive: they distort data signals and confuse the whistle detector.
- Set power so the radio runs cool on long transmissions: data and the whistle are 100% duty cycle while transmitting.

### 5. Optional: RTL-SDR second receiver

An RTL-SDR dongle connected to your radio's **RX OUT** (receive antenna output) or **IF OUT** jack gives a wide band waterfall and the instant **wide search**. See "Wide receiver (RTL-SDR)". Nothing to install: Whistlestop talks to the dongle directly over USB.

## Help mode and the Guide

- **📖 Guide** in the top bar opens this manual in **its own window**, so you can keep it beside the main one while you work. The button is lit while the window is open; click it again, or close the window, to put it away. **Contents** on the left lists every section; **Search the guide** filters sections and highlights the words you typed; **← / →** at the bottom of a section move to the previous or next one.
- **? Help** in the top bar turns on **Help mode**. A brass banner confirms it's on. Hover over any control (a button, slider, readout, waterfall or setting) and a card explains what it does, with the Guide section to read for more. Turn it off with **? Help** again or **Turn off** on the banner. It is **off every time Whistlestop starts**, so the cards never get in the way once you know your way around.
- **Open the Guide** on the Help banner opens the Guide window at "A tour of the screen".

## A tour of the screen

From top to bottom:

- **Top bar**: the Whistlestop mark; the **frequency display** (TX/RX lamp, dial frequency, band, radio mode); the **S-meter** and **SNR** meters; the **status pills** (Radio, Mercury, Audio, SDR: green means connected); **DEMO** when in Demo mode; **📖 Guide** (this manual, in its own window); the **? Help** toggle; and **Settings**.
- **Modem strip**: what Mercury is doing right now (see "The modem strip").
- **Left column** (top to bottom): **Call CQ**, **Search for whistles**, **Heard**, **Bandwidth**, **Radio**.
- **Centre**: tabs for **Chat**, **Roundtable**, **Files** and **Log**.
- **Bottom**: the **Radio audio** waterfall (what your radio hears, 0–3 kHz) and, with an SDR, the **Wide receiver** waterfall. Drag the line above the waterfalls to resize them.

## The whistle

### What it is

Every Whistlestop CQ starts with the **whistle**: a train of chirp pairs. An **UP** chirp sweeps from 500 to 2500 Hz in half a second, then a **DOWN** chirp sweeps back, again and again. On a speaker it sounds like "whee-ooo, whee-ooo". Each chirp fades in and out over 10 ms, so there are no clicks or splatter.

### Why it exists

Mercury's digital CQ is very robust, but a receiver must be tuned within **±55 Hz** of the caller to sync to it. A station sweeping the band in 1–2 kHz steps would almost never land that close, so on its own Mercury can only scan a short list of exact channels.

The whistle fixes that:

- **It can be found far off frequency.** A receiver matching against a perfect copy of the chirps still finds the whistle when it is tuned up to about **±1000 Hz** away. The sweep simply arrives at a different pitch, and part of it stays inside the radio's passband.
- **It measures its own offset.** If the receiver is tuned off, the UP chirp's match comes slightly *early* and the DOWN chirp's slightly *late*, by the same amount. The time between them gives the frequency error to **under 1 Hz**, so the searching station retunes straight onto the caller, well inside Mercury's ±55 Hz window. Radio drift is corrected automatically, because the measurement includes it.
- **It is distinctive.** The detector requires UP and DOWN matches that agree with each other, so carriers, FT8, noise and data bursts don't trigger it.

### Profiles

| Profile | Sweep | Finds callers up to | Use |
|---|---|---|---|
| Wide (default) | 500–2500 Hz | ±1000 Hz away | normal 2.4 kHz filters |
| Narrow | 1250–1750 Hz | ±250 Hz away | 500 Hz filters; chosen automatically for bandwidths below 1.7 kHz |

Callers and searchers must use the same profile: both follow the **Bandwidth** setting.

### Hearing it

`docs/whistle-demo.wav` and the files in `demo-sounds/` let you hear the whistle and a whole CQ, clean and as heard on the air. Make your own with `whistlestop cq-demo demo-sounds --whistle 24`.

## Calling CQ

The **Call CQ** card:

- **Chat (1:1)** or **Roundtable**: what you're calling for. A Roundtable CQ invites anyone to join a group on your frequency; type the table's name.
- **Whistle**: how long the whistle lasts (6–60 s, default **24 s**). It should be at least as long as one full pass of the searching stations, about 24 s for the 20 m segment. Longer whistles reach more searchers but occupy the frequency longer.
- **Call CQ** starts the cycle:
  1. Whistlestop keys the radio and plays the whistle.
  2. Mercury sends its **CQFRAME**, a 3-second digital burst carrying your callsign. This is your station identification.
  3. Whistlestop sends a short **CALL** packet saying what the CQ is for (chat or Roundtable, your grid).
  4. It listens for answers (default **20 s**), then repeats, up to **5 cycles**.
- **Stop CQ** ends it at any point.

When someone answers, Mercury reports the connection, the CQ stops by itself, and the Chat tab shows "Linked with …".

> Courtesy: listen first. The whistle is a calling signal, not a beacon. Keep it as short as the searchers need, and don't call on a busy frequency.

## Finding stations

There are three ways to find a calling station: searching with your one radio, the whistle watch while you're parked, and the wide search with an SDR. None is required: you can always tune by hand and press **Connect**.

### Search (one radio)

**Search for whistles** steps your radio through the digital segment of a band, listening on each channel:

- **Current band**: the segment(s) of the band you're on.
- **All bands**: every configured segment, band after band. A pass takes much longer, so callers would need very long whistles: use it for monitoring.
- **Search band** with the band list: one chosen band.

On each channel the radio settles (0.15 s), listens for **2.2 s**, and checks for a whistle. Channels are **1800 Hz** apart, so every caller in the segment is within the ±1000 Hz capture of some channel. A 20 m pass covers 10 channels in about 24 s. If something whistle-like is heard but not a full UP/DOWN pair yet, the search **holds** for 1.5 s more.

When it finds a whistle it **retunes** by the measured offset, **verifies** on the new frequency (and fine-tunes), then **waits up to 45 s for the CQ**. If the first measurement could be an alias (a caller more than 1000 Hz away shows up 2000 Hz off on the other side), the search checks the alternative frequency too. When Mercury decodes the caller's CQFRAME, the call appears under **Heard**. With **Answer automatically** ticked, Whistlestop connects at once.

### Whistle watch (parked)

Even when you're not searching, Whistlestop listens for whistles on the frequency you're on. If one is heard nearby, a banner under **Heard** says "Whistle +640 Hz from here" with **Tune to it**. The watch pauses while you're transmitting and during a link, and ignores audio from before your last retune. Turn it off in **Settings → Search & CQ**.

### Wide search (RTL-SDR)

With an RTL-SDR running, tick **Wide search (SDR)** in the Search card. The SDR's wide view is split into virtual receivers every 1.8 kHz across the band's digital segment (10 on 20 m), and the whistle detector runs on all of them every second. A whistle anywhere in the segment is found within about 1.5 s, without moving your radio.

- Hits appear under **Heard** with an **SDR** tag and a **Lock on** button, and as a 🚂 box on the wide waterfall.
- **auto-tune**: tune the radio to new hits by itself (when you're not linked, calling or searching).
- **Lock on** (or auto-tune) tunes your radio to the hit and **fine-locks using the radio's own audio**. That matters because RTL-SDR crystals are often 15–50 ppm off (a few hundred Hz on HF), too far for Mercury. In the demo the SDR is deliberately 240 Hz off and the fine-lock removes it.
- The SDR must be able to see the segment: with RX OUT it follows your radio (±800 kHz); with IF OUT it sees what your radio's IF passes.

### Heard

The **Heard** card lists recent CQs (from Mercury's CQFRAME decodes), Roundtable calls, and SDR whistle hits, newest first. Buttons: **Answer** (connect), **Join** (a Roundtable), **Lock on** (an SDR hit).

## Chat

Chat is a 1:1 conversation over Mercury's ARQ, which retransmits until everything arrives intact.

- **Connecting**: answer a CQ, or type a callsign in the box at the top of the Chat tab and press **Connect**. "Calling …" shows while Mercury places the call.
- **The link header** shows the other station's call, name and grid (from their HELLO), the agreed bandwidth, how they hear you ("hears you +7 dB", from their reports), the time since linking, and **Disconnect (73)**.
- **Messages** show **"… sending"** until the other station's app confirms they arrived, then **"✓ delivered"**. There are no read receipts and no typing indicators: they would cost airtime.
- **Time estimate**: under the input, "N bytes on air · ≈ T at R bps" tells you how long a message will take at the current link speed.
- **Chat never waits behind a file.** Files go in 1 KB pieces and at most two pieces wait inside Mercury, so a line you type during a big transfer goes out within a couple of pieces.

**Disconnect (73)** sends a short goodbye, then Mercury delivers anything still queued before hanging up.

## Files

The **Files** tab sends and receives files over a 1:1 link.

### Sending

- **+ Files…** picks one or more files; **+ Folder…** picks a folder; or **drop** files and folders anywhere on the window.
- **Zip before sending** (on by default) packs everything into one zip with maximum compression. Folders are always zipped. Text and logs often shrink to a few percent; photos and already-compressed files don't shrink.
- Each transfer is **offered** first: the other station sees the name, size, zip contents and predicted time, and presses **Accept** or **Decline**.
- Progress shows bytes done, the measured speed, and the time left. **Cancel** stops a transfer on either side.

### Receiving

An incoming offer shows the predicted time with **Accept** / **Decline**. Received files go to your **download folder** (Settings → Files; default `~/Downloads/Whistlestop`). **Show** reveals a received file in Finder. File names from the air are cleaned so they can't write anywhere else, and an existing file is never overwritten (`name-1.ext`, …).

### Integrity and resume

- Every file carries a **SHA-256** checksum. The receiver checks it and reports back: "✓ delivered, checksum good" on the sender, "✓ received" on the receiver.
- If the link drops, the partial file is kept (under `.partial` in the download folder, named by its checksum). When the same file is offered again, the receiver answers with how much it already has, and the transfer **resumes** from there.

### Estimates

**Estimate…** picks any file and shows a table of predicted times for every bandwidth at −5, 0, +5, +10 and +20 dB SNR, plus "now" for the current SNR. During a link, estimates use the speed Mercury reports and, after a few seconds of sending, the speed actually measured.

## Roundtables

A Roundtable is a group conversation on one frequency. There is no special broadcast channel: a table lives wherever its host called it.

### Hosting and joining

- **Host**: Call CQ → **Roundtable**, name the table, **Call CQ**. Your whistle and CQ invite others; Whistlestop sends a CALL packet with the table's name.
- **Join**: when a Roundtable CALL is heard, it appears under **Heard** with **Join**. Or type the name in the Roundtable tab and press **Join here** (you must be on the host's frequency).
- **Leave** says goodbye to the table.

### Talking

Type in the Roundtable tab and press Enter. Messages go out over Mercury's **broadcast** plane: one-way frames that everyone on the frequency decodes. A long message is split across several frames (up to 64) and put back together at each station. Under the input you see **exactly** how long it will take: broadcast frames have a fixed length, so "3 frames · exactly 9.6 s on air" is precise.

### Reliability and speed

Broadcast frames are not acknowledged, so Whistlestop adds:

- **Asking again**: if a station is missing parts of a message, it asks for just those parts every 20 s, and the sender repeats them.
- **Speed ramp-up**: each member reports how well it hears the host. The host picks the fastest mode that the **weakest** member can copy, with **3 dB** of margin, within your bandwidth setting. It announces the change twice in the current mode, waits until both announcements are on the air, then everyone switches together. It slows down at once if someone reports worse, and speeds up only after 2 minutes without a downgrade. Turn this off in Settings ("Roundtable speed ramp-up").
- **The floor**: click a member's name to pass them the floor (the "talking stick"). It's a courtesy, not a lock.

> Roundtables are always plain text, even with encryption turned on.

## Bandwidth and speed

### Your bandwidth limit

The **Bandwidth** card sets how wide Whistlestop may go, from narrow to wide: **250, 500, 1000, 1700, 2300, 2750 Hz**. The bar shows how much of a 2.75 kHz channel that is.

- **1:1 links** use Mercury's two ladders: below 2300 Hz, the narrow **BW500** ladder; at 2300 and 2750, the full ladder. At connect, both stations advertise their limit and the narrower one wins.
- **Roundtables** use any mode that fits inside your limit.
- **The whistle** switches to the narrow profile below 1700 Hz.

### How Mercury picks the speed

On a 1:1 link Mercury starts each direction on the step that the SNR measured at connect supports, then climbs or descends its ladder by itself from what actually gets through, separately in each direction. You don't control the step; you control the limit.

| Mode | Modulation | Carriers | Spacing | Width | Raw rate | Needs about | Error correction |
|---|---|---|---|---|---|---|---|
| MFSK500 | 32-MFSK | 32 tones | 15.6 Hz | 500 Hz | 31 bps | −14 dB | LDPC 1/2 |
| MFSK | 32-MFSK | 32 tones | 31.25 Hz | 1000 Hz | 60 bps | −11 dB | LDPC 1/2 |
| DATAC15 | OFDM QPSK | 3 | 62.5 Hz | 200 Hz | 68 bps | −7 dB | LDPC 1/3 |
| DATAC4 | OFDM QPSK | 4 | 62.5 Hz | 250 Hz | 87 bps | −4 dB | LDPC 0.30 |
| DATAC3 | OFDM QPSK | 9 | 62.5 Hz | 500 Hz | 321 bps | 0 dB | LDPC 1/2 |
| DATAC1 | OFDM QPSK | 27 | 62.5 Hz | 1700 Hz | 980 bps | +5 dB | LDPC 1/2 |
| DATAC17 | OFDM QPSK | 33 | 62.5 Hz | 2100 Hz | 1410 bps | +10 dB | LDPC 0.59 |
| QAM16C2 | OFDM 16-QAM | 33 | 62.5 Hz | 2100 Hz | 3100 bps | +16 dB | LDPC 0.60 |

The wide ladder is MFSK → DATAC15 → DATAC4 → DATAC3 → DATAC1 → DATAC17 → QAM16C2; the narrow ladder is MFSK500 → DATAC15 → DATAC4 → DATAC3. Real throughput (goodput) is lower than the raw rate because of turnarounds and acknowledgements: on a clean channel at QAM16C2, about **1600 bps**.

## The modem strip

The strip under the top bar spells out what Mercury is doing:

- **MODEM**: **TRANSMITTING** (red) or **RECEIVING** (green), and what for: a 1:1 link, a Roundtable, or idle and listening.
- **MODE**: the mode on the air now, and its modulation (OFDM QPSK, OFDM 16-QAM, or 32-MFSK). During a link this is the step Mercury reports; otherwise the mode it listens on.
- **CARRIERS**: how many OFDM carriers (or MFSK tones) and how far apart.
- **BANDWIDTH**: how wide the signal is now, **your limit**, and what the two stations **agreed** at connect.
- **SPEED**: the mode's raw rate (from Mercury during a link), and the speed **measured** from what actually drained.
- **ERROR CORRECTION**: the LDPC code, and the frame size and duration.
- **SNR**: Mercury's signal-to-noise measurement.
- **SPEED LADDER**: the steps Mercury can use on this link, with the current one lit. Hover it for every step's details.
- On the right: Mercury's version.

## Encryption (optional)

> **Encryption is not legal on amateur radio bands** in the US (47 CFR 97.113(a)(4)) and most other countries: amateur transmissions may not be encoded to obscure their meaning. Use it only where it is lawful, for example on frequencies of a service that allows it, or over a wired or test link. You are responsible for where you transmit.

Encryption protects **1:1 chat and files**. It is **off by default**. In **Settings → Encryption** choose:

- **Off**: never encrypt.
- **Prefer**: encrypt when the other station also wants to; otherwise plain text, clearly marked "🔓 Not encrypted".
- **Require**: encrypt or hang up. Never falls back to plain text. If the other station doesn't take part within 120 s, Whistlestop disconnects and says why.

Turning it on asks you to confirm the legal warning.

### How it works

- **Fresh keys for every link**: each station makes a new **X25519** key pair when the link comes up. Both stations derive the same secret without it ever going over the air.
- **Commit, then reveal**: each station first sends a hash of its public key and reveals the key only after the other's hash has arrived. That stops a man in the middle from trying keys until the check code matches.
- **Keys**: the shared secret goes through **HKDF-SHA256** into one **ChaCha20-Poly1305** key per direction (256-bit), the same cipher WireGuard and TLS 1.3 use.
- **Every packet is sealed and checked**: anything altered, replayed, dropped or reordered fails, and the link is closed.
- **Forgotten afterwards**: keys live only in memory, wipe themselves, and are dropped when the link ends. Nothing is ever written to disk, so a recording of the link can't be decrypted later (forward secrecy).

### The check code

When encryption is up, the link header shows **🔒 encrypted · code 264 097**. Read your code to the other operator once, by voice or another channel. **If both screens show the same code, no one is in the middle.** If the codes differ, disconnect: someone is relaying the link.

### Cost

Each packet grows by **17 bytes** (a 16-byte tag plus a type byte). A "73" chat line goes from 6 to 23 bytes; a 1 KB file piece grows about 1.6 %. The key exchange costs about 70 bytes each way, once per link.

## Wide receiver (RTL-SDR)

An RTL-SDR dongle adds a second, wide receiver. It is optional.

- **Connecting**: **RX OUT** gives the dongle the antenna signal; it retunes to keep your VFO in view (direct sampling on V3 dongles below 28.8 MHz, the upconverter on a Blog V4). **IF OUT** gives it your radio's first IF: set the **IF frequency** and whether it is **inverted** (higher RF comes out lower).
- **Crystal ppm**: corrects the dongle's frequency error for the picture. The wide search doesn't depend on it, because hits are fine-locked with the main radio.
- **The waterfall** shows the band around your dial: the teal bars on top are the digital segments, the brass line is your dial and the shaded band your receive passband. **Hover** to read a frequency; **click** to tune (the clicked signal lands in the middle of your passband). The **span** buttons zoom (25 kHz to 1.2 MHz).
- **Wide search**: see "Finding stations".

## Rig controls

### On the main screen

The **Radio** card keeps only the essentials:

- **Band buttons** (80–10 m): jump to that band's digital segment.
- **−1k / −100 / +100 / +1k**: nudge the dial.
- **Frequency box**: type kHz (`14103.15`) or Hz and press **Tune** or Enter.
- **Mode**: PKTUSB, USB or LSB (Whistlestop's signals are USB).
- **Power**: the radio's RF power, if it supports remote setting.
- **🎛 Radio controls**: opens the full controls window (below). The button is lit while it's open.
- **SWR 1.3**: the SWR of the current or last transmission, red when above your limit.
- **Tune carrier** / **Stop**: Mercury sends a steady 1 kHz tone for an external antenna tuner. It always stops by itself after 60 s.

The top bar's S-meter and frequency display follow the radio, including changes you make on the radio's own knob.

### The Radio Controls window

**🎛 Radio controls** opens a separate, movable window with everything your radio offers through Hamlib. Close it when you're done, so the main screen stays uncluttered. Whistlestop asks the radio for its capabilities when it connects and **shows only the controls your radio reports**, so there are no buttons that do nothing. Values are read from the radio about every 1.5 s while the window is open.

- **Transmit meters**: **SWR**, **ALC** and **Power out**, read while transmitting, plus the SWR of the last transmission. See "Antenna tuner and SWR".
- **Antenna tuner**: **ATU** (tuner in or out), **⟳ Tune now**, the Mercury **Tune carrier**, and **Auto-tune when SWR is above …** with its limit.
- **Receive**: **Volume (AF)**, **RF gain**, **Squelch**, **AGC** (Off, Fast, Medium, Slow, Auto), **Preamp** and **Attenuator** (the steps your radio offers), **NB**, **NR** (with level), **Notch**, and **Filter width** (300–4000 Hz).
- **Transmit**: **Power**, **Mic gain**, **Compression**, and the **Comp**, **VOX** and **Monitor** switches.
- **VFO, split and offsets**: **A / B**, **A = B**, **A ⇄ B**, **Split**, **RIT** and **XIT** (type an offset, **Set**, **Clear**).

> For data, keep NB, NR, Notch, compression, VOX, split, RIT and XIT **off** and squelch **open**. They distort data signals or move you off the other station. The window reminds you.

## Antenna tuner and SWR

### The SWR watch

Whenever the radio transmits (a CQ whistle, Mercury data, a tune carrier), Whistlestop reads **SWR**, **ALC** and **power out** from the radio, if it reports them. It keeps the highest SWR of each transmission:

- The **Radio** card and the Radio Controls window show it (**SWR 1.3**).
- If it's above your **SWR limit** (default **2.0 : 1**), the reading turns red and the log warns: "High SWR: 2.8 on 40m (limit 2.0)".

Watching never transmits anything.

### Tuning

- **ATU** switches the radio's built-in tuner in or out.
- **⟳ Tune now** switches the tuner in and starts a tune cycle. The radio sends its own short carrier while it finds a match, so **listen first**.
- **Tune carrier** is for an *external* tuner: Mercury sends a steady 1 kHz tone (at most 60 s) while you tune by hand.

### Auto-tune (optional, off by default)

Tick **Auto-tune when SWR is above** in the Radio Controls window to let Whistlestop start the tuner by itself after a transmission whose SWR went above the limit. Because tuning puts a carrier on the air, it is careful:

- It waits until the band is quiet: **never** during a 1:1 link, while calling CQ or searching, or with Roundtable messages waiting. "High SWR: will tune as soon as the band is quiet" shows meanwhile.
- At most once every **2 minutes** per band, so a bad antenna doesn't keep it tuning.
- It needs a radio whose tuner Hamlib can control (the **ATU** and **Tune now** buttons appear when it can).

After a tune, the window shows "Tuned on 40m at 14:02". The setting and the limit are saved.

## Band plan

Searches cover these segments by default (USB, the US automatically-controlled digital segments of 47 CFR 97.221(b), where HF data stations already gather, away from FT8/FT4 and the NCDXF beacons):

| Band | Segment |
|---|---|
| 80 m | 3585–3600 kHz |
| 40 m | 7100–7105 kHz |
| 30 m | 10140–10150 kHz |
| 20 m | 14095–14099.5 and 14100.5–14112 kHz |
| 17 m | 18105–18110 kHz |
| 15 m | 21090–21100 kHz |
| 12 m | 24925–24930 kHz |
| 10 m | 28120–28189 kHz |

They're stored in the settings file (`segments`), so you can change or add them. The whistle occupies dial +500 to +2500 Hz, so channels are chosen to keep it inside the segment.

## Settings reference

Settings are saved to `~/Library/Application Support/Whistlestop/whistlestop.toml` (macOS). **Save & connect** applies them and reconnects.

| Setting | What it does | Default |
|---|---|---|
| Callsign | Your call, sent in Mercury's CQFRAME and CALL frames | N0CALL |
| Name, Grid | Sent to the other station at connect (HELLO) | empty |
| Demo mode | Simulated radio, band, Mercury and demo station | on |
| Mercury host / TNC port / Broadcast port | Where Mercury listens | 127.0.0.1 / 8300 / 8100 |
| Radio connection | Use a running rigctld / RigHelm, or let Whistlestop run rigctld | running rigctld |
| rigctld address | The running server's address | 127.0.0.1:4532 |
| Model / Port / Speed | For the rigctld Whistlestop runs; Automatic detects at start | Automatic |
| rigctld port | Where Whistlestop's rigctld listens (point Mercury here) | 4532 |
| Audio in / out | The radio's sound device | system default |
| Wide receiver | Use an RTL-SDR; RX OUT or IF OUT; IF Hz and inversion; crystal ppm | off |
| Dwell per channel | How long search listens on each channel | 2.2 s |
| Channel step | Distance between search channels (capped to the whistle's capture) | 1800 Hz |
| CQ cycles | How many whistle/CQ/listen cycles before giving up | 5 |
| Listen after CQ | How long to wait for answers each cycle | 20 s |
| Whistle watch | Report whistles while parked | on |
| Roundtable speed ramp-up | Host adjusts the table's speed | on |
| Encryption | Off / Prefer / Require (see Encryption) | Off |
| Auto-tune on high SWR | Start the antenna tuner after a high-SWR transmission, when quiet (Radio Controls window) | off |
| SWR limit | Above this: red reading, log warning, auto-tune if on | 2.0 : 1 |
| Download folder | Where received files go | ~/Downloads/Whistlestop |

The **Whistle** length and **Answer automatically** are set on the main screen and saved too.

## Troubleshooting

**A pill in the top bar is red.**

- **Radio**: rigctld (or RigHelm's server) isn't running or the address is wrong. Check **Settings → rigctld**.
- **Mercury**: Mercury isn't running, the ports differ, or another program (VarAC, vARIM) already holds Mercury's TNC port. Only one program at a time.
- **Audio**: the audio device in Settings isn't available. Pick it again; on macOS allow microphone access for Whistlestop in System Settings → Privacy.
- **SDR**: no dongle found, or the wide receiver is off in Settings.

**The radio isn't detected.**

- Is it switched on and connected by USB (not just an audio cable)? Some radios need their USB driver (Silicon Labs CP210x) on older macOS.
- Detection recognises radios by their USB descriptions. If yours shows as "USB serial (choose the model)", or not at all, choose **Model** and **Port** by hand.
- The radio's CAT speed must match **Speed** (or leave it Automatic and set the radio to Hamlib's default for the model).

**High SWR warnings.**

Check the antenna and feed line first. With a built-in tuner, press **Tune now** or turn on auto-tune. If SWR stays high after tuning, the antenna is outside what the tuner can match on that band.

**Search never stops on a whistle.**

- The caller's whistle may be shorter than one search pass. Ask callers to use a longer whistle, or search a single band.
- Your filter may be too narrow for the wide whistle: use 2.4 kHz, or both use a narrow bandwidth.
- Noise reduction or a noise blanker on receive smears chirps. Turn them off.

**It stops on a whistle but never connects.**

- "No CQ followed the whistle": the whistle was heard but Mercury didn't decode the CQFRAME. The caller may be weak; try again on their next cycle.
- Both stations need matching Mercury generations (1.9.20 and later don't talk to 1.9.19 or older).

**Messages stay at "… sending".**

The other station's app hasn't confirmed them yet. On a weak link Mercury may be retrying on its slowest step; watch the modem strip. If the link drops, unconfirmed messages are not resent automatically: send them again.

**Files are slow.**

Check the modem strip: the step and SNR tell you why. Zip text-heavy files. Use a wider bandwidth if both stations and the band allow.

**A Roundtable member hears nothing after a speed change.**

They may have missed the announcement. Leaving and rejoining puts them back on the calling mode; the host's next speed decision brings everyone together again.

**The whistle sounds distorted on the air.**

Reduce audio drive until the ALC barely moves, and turn off speech processing.

## Testing without a radio

- **Demo mode** runs everything simulated, with K7BOT to talk to.
- **`whistlestop whistle-wav`** and **`whistlestop cq-demo`** write sound files of the whistle and a whole CQ.
- **The two-station bench** (`docs/BENCH.md`) runs two real Mercury modems and two Whistlestop stations on one computer through a simulated HF band: search, link, chat, files, Roundtables, narrow bandwidth, weak signals and encryption.

## How it works underneath

- **Mercury** does the modulation, error correction, acknowledgements and speed ladder. Whistlestop talks to it over its VARA-style TNC (control port 8300, data 8301) and its broadcast port (8100, KISS framing).
- **WSP, the Whistlestop Protocol**, rides on top: compact binary packets for chat, delivery confirmations, file offers, pieces and checks, signal reports, and Roundtable frames. Specification: `docs/PROTOCOL.md`.
- **The whistle** and the search: `docs/DESIGN.md`.

## Legal and good practice

- **Identification**: Mercury's CQFRAME carries your callsign in the clear on every CQ, and connections carry both calls. Identify by voice or CW as your rules require on long contacts.
- **The whistle** is a documented, swept-tone data signal under 2.1 kHz wide (this guide and the source code document it). Use it in the data segments, keep it as short as the searchers need, and listen before calling.
- **Encryption** is off by default and not legal on amateur bands in most countries (see Encryption).
- **Unattended operation**: Whistlestop is meant to be attended. Automatic answering and auto-tune are conveniences for an operator who is present.

## Glossary

- **ARQ**: automatic repeat request: the receiver acknowledges, and missing data is sent again.
- **Broadcast plane**: Mercury's one-way, unacknowledged frames, used for Roundtables.
- **Chirp**: a tone whose pitch sweeps steadily.
- **CQFRAME**: Mercury's compact digital CQ (3 s, carries your callsign).
- **Dwell**: how long a search listens on each channel.
- **FEC / LDPC**: forward error correction; LDPC is the code family Mercury uses.
- **Goodput**: useful data delivered per second, after all overheads.
- **Matched filter**: comparing a signal with a perfect copy of what you expect; the basis of the whistle detector.
- **OFDM**: many closely spaced carriers sent at once, each slowly modulated.
- **QPSK / 16-QAM**: 2 or 4 bits per symbol on each carrier.
- **SAS / check code**: the 6-digit code operators compare to rule out a man in the middle.
- **SNR**: signal-to-noise ratio, in dB.
- **TNC port**: Mercury's control and data interface for programs like Whistlestop.
- **WSP**: the Whistlestop Protocol.
