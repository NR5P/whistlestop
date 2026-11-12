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

The first time you open Whistlestop it starts in **Demo mode**: a simulated radio on a simulated 20 m band, and a demo station, **K7BOT**, who calls CQ every 45 seconds. Nothing is transmitted.

- **With Mercury installed** (the `mercury` engine in `~/.local/bin`, `/opt/homebrew/bin` or `/usr/local/bin`), Demo runs **two real Mercury modems**: one for you, one for K7BOT, joined by the simulated band. Everything in the modem strip, and everything you hear with **🔈 Listen**, is real Mercury: its CQ burst, its speed ladder, its acknowledgements.
- **Without it**, Demo simulates the modem too, and the CQ burst you hear is an imitation.
- **Settings → Mode → Demo on real Mercury** switches between the two; **Mercury engine** sets the path if Mercury lives somewhere else.

Try this:

1. Press **🔈 Listen** in the top bar to hear the band on your speakers. Look at the **Radio audio** waterfall at the bottom left: every so often you'll see (and hear) K7BOT's whistle as slanted strokes.
2. In **Search for whistles**, press **Current band**. Your radio steps through the 20 m digital segment. Within about one pass it stops, says "Whistle! measuring", retunes, and waits for the CQ.
3. When K7BOT's CQ is heard it appears under **Heard**. Press **Answer** (or tick **Answer automatically** before searching).
4. Once linked, type in **Chat** and press Enter. K7BOT answers. Ask it for a file: it offers one in the **Files** tab.
5. Drop a file on the window to send one back.
6. Try **Call CQ → Roundtable** to host a table. K7BOT joins.

When you're ready for the air, open **Settings**, enter your callsign, untick **Demo mode**, and follow "Setting up your station" below.

## Setting up your station

Whistlestop runs on **macOS, Windows and Linux**. Radios show up as `/dev/cu.…` ports on macOS, `COM3`-style ports on Windows and `/dev/ttyUSB0` / `/dev/ttyACM0` on Linux (on Linux, add yourself to the `dialout` group to open them). Demo on real Mercury needs Mercury's simulated-audio link, which Mercury offers on macOS and Linux only; on Windows, Demo uses the simulated modem.

You need four things: a radio with CAT control, an audio interface to it (most modern radios have USB audio built in), the **Mercury** modem, and a way for both Mercury and Whistlestop to control the radio.

### 1. Rig control

Only one program can open the radio's CAT (serial/USB) port. So one program owns it and shares it with the others over the network on this computer. In **Settings → Radio connection** choose one of three ways:

**Direct CAT (built in)** (nothing to install). Whistlestop speaks your radio's own control protocol itself and shares the radio with Mercury. See "Direct CAT (built in)".

**Let Whistlestop run rigctld**. Whistlestop starts Hamlib's `rigctld` for your radio and stops it when you quit.

- Install Hamlib once (macOS: `brew install hamlib`; Linux: your distribution's `hamlib` or `libhamlib-utils` package; Windows: the installer from hamlib.github.io, which Whistlestop finds in Program Files).
- Press **🔍 Detect radios**. Whistlestop looks at the USB devices your computer already knows about and recognises:
  - Icom radios by name (an IC-7300 names itself in its USB serial number), plus the IC-705 and IC-905 by Icom's own USB IDs;
  - Yaesu radios by their two-port USB chip (CAT is on the first port);
  - Kenwood, Elecraft, QRP Labs (QMX/QDX, which speak Kenwood TS-480), Xiegu and others by their USB names.
- Press **Use** beside your radio, or choose **Model**, **Port** and **Speed** yourself. Leaving them on **Automatic** detects them every time Whistlestop starts.
- **Detection never opens a port.** Opening a serial port raises its RTS and DTR lines, and on some radios that keys the transmitter. When Whistlestop starts rigctld, it holds RTS and DTR off.
- rigctld listens on `127.0.0.1:4532` (change **rigctld port** if something else uses it). Point Mercury there.

**Use a running rigctld / RigHelm**. If something already owns the radio:

- **RigHelm**: enable its built-in rigctld server (default `127.0.0.1:4532`).
- **Hamlib rigctld** you start yourself: `rigctld -m <model> -r <port>` (find the model number with `rigctl -l`).

Set **rigctld address** to it. Whichever way, the **Radio** pill in the top bar turns green when connected, and the log names the radio Hamlib reports.

### 2. Mercury

Install Mercury (macOS: drag `Mercury.app` to Applications). Then set it up so it shares the radio:

- **Radio / PTT**: Hamlib model **2 (NET rigctl)** pointing at `127.0.0.1:4532` (the default for all three ways: Whistlestop's direct-CAT sharing port, the rigctld it runs, or RigHelm's server). Mercury then keys the transmitter through the same server.
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

- **Top bar**: the Whistlestop mark; the **frequency display** (TX/RX lamp, dial frequency, band, radio mode); the **S-meter** and **SNR** meters; **🙋 Here / 🚪 Away** (see "Away"); **🛰 Remote** (see "Remote operation"); **DEMO** when in Demo mode; **🔈 Listen** (the radio's audio on your speakers); **📖 Guide** (this manual, in its own window); the **? Help** toggle; and **Settings**.
- **Modem strip**: what Mercury is doing right now (see "The modem strip"), and at its right end the **status pills** (Radio, Mercury, Audio, SDR: green means connected).
- **Left column** (top to bottom): **Call CQ**, **Search for whistles**, **Heard**, **Bandwidth**, **Radio**.
- **Centre**: tabs for **Chat**, **Roundtable**, **Files**, **Depot** (mail; see "The Depot") and **Log**.
- **Bottom**: the **Radio audio** waterfall (what your radio hears, 0–3 kHz) and, with an SDR, the **Wide receiver** waterfall. Drag the line above the waterfalls to resize them.

## Listening on your speakers

**🔈 Listen** in the top bar plays what the radio hears through the computer's speakers or headphones. The button turns green (🔊) while it's on. It is **off by default**.

- **The band, live**: whatever your receiver hears (noise, whistles, Mercury's data signals) plays with less than half a second of delay. If the computer falls behind, it skips ahead rather than drifting later and later.
- **In Demo mode** you also hear **your own CQ**: the 24-second whistle ("whee-ooo, whee-ooo"), then the CQ burst (an approximation of Mercury's 3-second digital ID). You also hear K7BOT's whistle and CQ, and the FT8-like signals on the simulated band. On the air you'd hear your own transmissions on the radio, not through Whistlestop.
- **Settings → Audio**: **Listen on** chooses the speaker (System default normally), and **Listen volume** sets the level; changes apply at once.

> Don't choose the radio's own USB audio for **Listen on**: that would play the band back into the radio's transmit audio.

## The whistle

### What it is

Every Whistlestop CQ starts with the **whistle**: a train of chirp pairs. An **UP** chirp sweeps from 1000 to 2000 Hz in 0.2 s, then a **DOWN** chirp sweeps back, again and again, two and a half times a second. On a speaker it sounds like a quick "wee-oo-wee-oo". Each chirp fades in and out over 10 ms, so there are no clicks or splatter.

This is **whistle v2**. Its shape was chosen by a study of ten candidate signals: fast and slow chirps, narrow warbles, steady tones, tone pulses and FT8-style Costas patterns, all on a simulated fading HF path in a busy band (`docs/WHISTLE-LAB.md`). The winning design repeats quickly enough to be caught whole in a half-second listen, and is half as wide as the first whistle.

### Why it exists

Mercury's digital CQ is very robust, but a receiver must be tuned within **±55 Hz** of the caller to sync to it. A station sweeping the band in 1–2 kHz steps would almost never land that close, so on its own Mercury can only scan a short list of exact channels.

The whistle fixes that:

- **It can be found far off frequency, fast.** A scanning station looks for the sweep's track in a short-frame spectrogram, at every timing and every offset up to about **±1100 Hz**, adding up the energy along it. This needs only half a second of audio, copes with fading, and gives a coarse offset (to a few hertz) that is never ambiguous: the UP and DOWN legs only line up at the true offset.
- **It measures its own offset precisely.** Once retuned close, matched filters on the UP and DOWN chirps finish the job. If the receiver is tuned off, the UP chirp's match comes slightly *early* and the DOWN chirp's slightly *late*, by the same amount, and the time between them gives the error to **about 1 Hz**. The searching station ends up well inside Mercury's ±55 Hz window. Radio drift is corrected automatically, because the measurement includes it.
- **It is distinctive.** Both detectors need UP and DOWN together. The scan detector levels away anything that sits in one frequency more than a quarter of the time (carriers, RTTY, FT8, modems). Steady tones and data bursts don't trigger it: on a simulated busy band, only 1 % of empty half-second listens score high enough to stop.

### Profiles

| Profile | Sweep | Finds callers up to | Use |
|---|---|---|---|
| Wide (default) | 1000–2000 Hz | ±1100 Hz away | normal 2.4 kHz filters |
| Narrow | 1250–1750 Hz | ±250 Hz away | 500 Hz filters; chosen automatically for bandwidths below 1.7 kHz |

Callers and searchers must use the same profile: both follow the **Bandwidth** setting.

### Hearing it

`docs/whistle-demo.wav` and the files in `demo-sounds/` let you hear the whistle and a whole CQ, clean and as heard on the air. Make your own with `whistlestop cq-demo demo-sounds --whistle 10`.

## Calling CQ

The **Call CQ** card:

- **Chat (1:1)** or **Roundtable**: what you're calling for. A Roundtable CQ invites anyone to join a group on your frequency; type the table's name.
- **Whistle**: how long the whistle lasts (4–60 s, default **10 s**). It should be at least as long as one full pass of the searching stations, about 6–8 s for the 20 m segment. Longer whistles reach slower searchers (or ones with longer dwells) but occupy the frequency longer.
- **Call CQ** starts the cycle:
  1. Whistlestop keys the radio and plays the whistle.
  2. Mercury sends its **CQFRAME**, a 3-second digital burst carrying your callsign. This is your station identification.
  3. Whistlestop sends a short **CALL** packet saying what the CQ is for (chat or Roundtable, your grid).
  4. It listens for answers (default **20 s**), then repeats, up to **5 cycles**.
- **Stop CQ** ends it at any point.

When someone answers, Mercury reports the incoming call and the CQ goes on hold at once ("Incoming call: CQ on hold" in the Log): a whistle in progress stops and nothing more is sent, because a whistle or CQFRAME on top of Mercury's answer would make it drop the call. When the link comes up the CQ stops for good and the Chat tab shows "Linked with …". If the call doesn't complete (Mercury gives up after about 45 s), the CQ carries on with its next cycle; the hold lasts at most 60 s.

> Courtesy: listen first. The whistle is a calling signal, not a beacon. Keep it as short as the searchers need, and don't call on a busy frequency.

### Listen before transmitting

Whistlestop also listens for you before it transmits on its own. It turns on Mercury's busy-channel detector while you're not in a link, and while that reads the frequency as busy (someone else transmitting):

- a **CQ** holds its whistle ("CQ waits: someone is transmitting on this frequency" in the Log, and "waiting for a clear channel" with the seconds left under the CQ), between cycles too;
- a **Station ID** waits;
- the Away **automatic deliver and collect** calls wait. The Station ID or CQ that sets one off has only just ended, so the detector usually still reads it for a moment; the call goes out once the channel clears.

A CQ, Station ID or automatic call waits at most **60 s**, then goes anyway and the Log says so. If an automatic call is still waiting after 90 s, or you come back, start something else or the mail is gone, it is dropped. Going anyway matters because a detector that reads busy all the time mustn't silence your station. That happens behind a receive filter narrower than about 1.8 kHz, so use a wide filter, or turn it off (**Settings → Search & CQ → Listen before transmitting**, on by default). What you send by hand (Connect, Send, a Station ID you press) always goes at once. During a link the detector is off, so the link behaves as it always has.

## Finding stations

There are three ways to find a calling station: searching with your one radio, the whistle watch while you're parked, and the wide search with an SDR. None is required: you can always tune by hand and press **Connect**.

### Search (one radio)

**Search for whistles** steps your radio through the digital segment of a band, listening on each channel:

- **Current band**: the segment(s) of the band you're on.
- **All bands**: every configured segment, band after band. A pass takes much longer, so callers would need very long whistles: use it for monitoring.
- **Search band** with the band list: one chosen band.

On each channel the radio settles (0.15 s), listens for **0.5 s**, and checks for a whistle. Channels are **2000 Hz** apart, so every caller in the segment is within ±1100 Hz of some channel. A 20 m pass takes about 6–8 s.

Listening is in two stages, the way military ALE scanners pause on a promising channel:

- If the half-second clearly holds a whistle, the search stops there.
- If it holds a hint of one (about 15 % of empty channels do, and almost every real whistle), the search **listens longer**, up to 2 s, and decides on that.
- Otherwise it moves on. Fast where nothing is there, patient where something might be.

When it finds a whistle it **retunes** by the coarse offset, **measures** the whistle precisely on the new frequency (three patterns, 1.2 s) and fine-tunes, then **waits up to 45 s for the CQ**. Right after a retune the whistle should be within about 10 Hz; if the measurement says **25 Hz or more**, the radio or sound card was probably still catching up when it listened. Audio from before the retune shows the whistle where it was before, so Whistlestop first listens again where it is ("Measured … Hz off after retuning; listening again here" in the Log). Only if the second look agrees does it retune there and measure once more before locking ("… off again; checking there"). Up to 3 retunes in all, counting the "further" ones. If the whistle ends meanwhile, it waits for the CQ where it is. When Mercury decodes the caller's CQFRAME, the call appears under **Heard**. With **Answer automatically** ticked, Whistlestop connects as soon as the caller has stopped transmitting: right after its CQFRAME the caller sends its CALL packet (about 3.7 s), so Whistlestop waits until that packet is heard, or **4.5 s** at most, before calling. A call sent sooner would land on top of the caller's own transmission and go unheard. If no link comes up within **60 s** of calling (say another station answered first), it gives up, so its call doesn't go on over the new link.

On a simulated busy band with fading, a half-second listen catches 82–84 % of whistles at −10 dB SNR (in 2500 Hz) on the first look, and 94–97 % at −5 dB; those it misses at first mostly get the 2 s listen, which catches 96–99 % at −10 dB. Weaker than about −10 dB, Mercury itself struggles to link.

### Whistle watch (parked)

Even when you're not searching, Whistlestop listens for whistles on the frequency you're on. If one is heard nearby, a banner under **Heard** says "Whistle +640 Hz from here" with **Tune to it**. A whistle is reported only when two checks in a row (about a second apart) agree on the offset within 15 Hz, so a single data burst isn't mistaken for one. The watch pauses while you're transmitting, while a call is being made or answered, during a link, for 8 s after a call starts or a link comes up, and for 30 s after a link ends, because Mercury's connect and disconnect bursts can look like a whistle. It also ignores audio from before your last retune. Turn it off in **Settings → Search & CQ**.

### Wide search (RTL-SDR)

With an RTL-SDR running, tick **Wide search (SDR)** in the Search card. The SDR's wide view is split into virtual receivers every 1.8 kHz (with the default whistle) across the band's digital segment, and the whistle detector runs on all of them every second. A whistle anywhere in the segment is found within about 1.5 s, without moving your radio.

- Hits appear under **Heard** with an **SDR** tag and a **Lock on** button, and as a 🚂 box on the wide waterfall.
- **auto-tune**: tune the radio to new hits by itself (when you're not linked, calling or searching).
- **With a search running, both methods work together**, auto-tune or not. While the stepping search is still going channel to channel (listening, or holding on something whistle-like), a fresh SDR hit pauses it: the radio goes straight to the hit, fine-locks, and waits up to 45 s for the CQ. If the hit isn't confirmed or no CQ comes, the search resumes on the channel it was on ("Back to the … search" in the Log). A search that has already found a whistle itself (measuring it, or waiting for its CQ) is never interrupted. Each SDR frequency is tried at most once every 2 minutes (within 200 Hz), so one caller can't keep pulling the search away.
- The stepping search still matters with an SDR: it covers any part of the segment the SDR can't see, and it hears through your radio's better front end.
- **Lock on** (or auto-tune) tunes your radio to the hit and **fine-locks using the radio's own audio**. That matters because RTL-SDR crystals are often 15–50 ppm off (a few hundred Hz on HF), too far for Mercury. In the demo the SDR is deliberately 240 Hz off and the fine-lock removes it.
- The SDR must be able to see the segment: with RX OUT it follows your radio (±800 kHz); with IF OUT it sees what your radio's IF passes.

### Heard

The **Heard** card lists recent CQs (from Mercury's CQFRAME decodes), Roundtable calls, and SDR whistle hits, newest first. Buttons: **Answer** (connect), **Join** (a Roundtable), **Lock on** (an SDR hit).

## Chat

Chat is a 1:1 conversation over Mercury's ARQ, which retransmits until everything arrives intact.

- **Connecting**: answer a CQ, or type a callsign in the box at the top of the Chat tab and press **Connect**. "Calling …" shows while Mercury places the call. A CQ you're calling stops first. Connect is refused while you're already linked or calling, while you're at a Roundtable (Leave first: a 1:1 link and a Roundtable don't mix), and until your own callsign is set (Settings → Station): without one Whistlestop sends nothing at all, and says so in the Log.
- **History**: the Chat tab keeps the newest **500** lines and the Files tab the newest **200** finished transfers (transfers still going are never dropped), so an unattended station running for days stays light. Depot mail is kept in full.
- **The link header** shows the other station's call, name and grid (from their HELLO), the agreed bandwidth, how they hear you ("hears you +7 dB", from their reports), the time since linking, and **Disconnect (73)**.
- **Messages** show **"… sending"** until the other station's app confirms they arrived, then **"✓ delivered"**. There are no read receipts and no typing indicators: they would cost airtime.
- **Time estimate**: under the input, "N bytes on air · ≈ T at R bps" tells you how long a message will take at the current link speed.
- **Chat never waits behind a file.** Files go in 1 KB pieces and at most two pieces wait inside Mercury, so a line you type during a big transfer goes out within a couple of pieces. (Whistlestop counts what it has handed to Mercury itself and takes off only what Mercury reports as sent, so a burst of Mercury's status reports can't make it hand over the rest of the file early.) The same limit is what lets **Cancel** stop a file going out.

**Disconnect (73)** sends a short goodbye, then Mercury delivers anything still queued before hanging up. When the other station says 73, Whistlestop waits up to 30 s for its disconnect; if that never arrives (say, it was lost after a QSY), Whistlestop ends the link at once without a goodbye exchange, so Mercury doesn't keep calling a station that has gone.

## Files

The **Files** tab sends and receives files over a 1:1 link.

### Sending

- **+ Files…** picks one or more files; **+ Folder…** picks a folder; or **drop** files and folders anywhere on the window.
- **Zip before sending** (on by default) packs everything into one zip with maximum compression. Folders are always zipped. Two files with the same name (from different folders) are both kept, the second as `name-1.ext`. Text and logs often shrink to a few percent; photos and already-compressed files don't shrink.
- Each transfer is **offered** first: the other station sees the name, size, zip contents and predicted time, and presses **Accept** or **Decline**.
- Progress shows bytes done, the measured speed, and the time left. **Cancel** stops a transfer on either side. (Each station numbers its own files from a random starting point, so a cancel from either end always names the right one.)
- If the link ends while an offer is still waiting for an answer, it shows **link ended before you answered**. The sender offers it again the next time the two of you link (with resume).
- **A file goes only to the station it was for.** A file sent during a link, or one cut off by a dropped link, is offered again (with resume) only on a link with that same station, never to whoever links next (while you're Away, that could be anyone). A file picked while not linked goes to the next station you link with.

### Receiving

An incoming offer shows the predicted time with **Accept** / **Decline**. Received files go to your **download folder** (Settings → Files; default `~/Downloads/Whistlestop`). **Show** reveals a received file in Finder. File names from the air are cleaned so they can't write anywhere else: only the name itself is kept (no folders), control characters are dropped, characters Windows forbids (`< > : " | ? *`) become `_`, a Windows device name such as `CON` or `NUL` gets a `_` in front, and a name longer than 200 bytes is shortened (keeping its extension). An existing file is never overwritten (`name-1.ext`, …).

### Integrity and resume

- Every file carries a **SHA-256** checksum. The receiver checks it and reports back: "✓ delivered, checksum good" on the sender, "✓ received" on the receiver.
- If the link drops, the partial file is kept (under `.partial` in the download folder, named by its checksum). When the same file is offered again, the receiver answers with how much it already has, and the transfer **resumes** from there.

### Estimates

**Estimate…** picks any file and shows a table of predicted times for every bandwidth at −5, 0, +5, +10 and +20 dB SNR, plus "now" for the current SNR. During a link, estimates use the speed Mercury reports and, after a few seconds of sending, the speed actually measured.

## Roundtables

A Roundtable is a group conversation on one frequency. There is no special broadcast channel: a table lives wherever its host called it.

### Hosting and joining

- **Host**: Call CQ → **Roundtable**, name the table, **Call CQ**. Your whistle and CQ invite others; Whistlestop sends a CALL packet with the table's name. The CQ keeps cycling to invite more stations, but never on top of the table: table messages wait while your whistle is on the air, and the next whistle waits until table traffic has gone out. Once the table speeds up beyond DATAC3 the CQ ends by itself (the Log says so), because stations still listening for calls at DATAC3 couldn't copy its CALL packets.
- **Join**: when a Roundtable CALL is heard, it appears under **Heard** with **Join**. Or type the name in the Roundtable tab and press **Join here** (you must be on the host's frequency). Both are off during a 1:1 link: a Roundtable switches Mercury to broadcast modes, which would cut the link. Disconnect first.
- After you **Leave**, Mercury goes back to listening in DATAC3, the calling mode, once your goodbye has gone out (a table that sped up would otherwise leave you deaf to calls).
- **Host**: Call CQ → **Roundtable**, name the table, **Call CQ**. Your whistle and CQ invite others; Whistlestop sends a CALL packet with the table's name.
- **Join**: when a Roundtable CALL is heard, it appears under **Heard** with **Join**. Or type the name in the Roundtable tab and press **Join here** (you must be on the host's frequency).
- **Joining right after a link**: Mercury can't switch to Roundtable listening while a 1:1 link is still closing (its disconnect takes several seconds, more if the other station is slow to answer). If you press **Join here** then, the tab shows "⏳ Joining … as soon as Mercury is free" and Whistlestop tries again every **2 s** for up to **60 s**; **Cancel** stops waiting, and so does calling a station or calling CQ (the Log says "Roundtable join … cancelled"). It doesn't retry while you're calling someone. While a link is up and you haven't hung up, joining is refused: end the link first.
- **Leave** says goodbye to the table.

### Talking

Type in the Roundtable tab and press Enter. Messages go out over Mercury's **broadcast** plane: one-way frames that everyone on the frequency decodes. A long message is split across several frames (up to 64) and put back together at each station. Under the input you see **exactly** how long it will take: broadcast frames have a fixed length, so "3 frames · exactly 9.6 s on air" is precise.

### Reliability and speed

Broadcast frames are not acknowledged, so Whistlestop adds:

- **Asking again**: if a station is missing parts of a message, it asks for just those parts every 20 s, and the sender repeats them. A message missed whole (a short one-frame line, say) shows up as a gap in the sender's message numbers, either at its next message or at the host's 90 s heartbeat, which carries the host's newest number. Missed messages are asked for whole: up to 16 per sender. A station sends at most 3 of these requests every 20 s, oldest first, so a long gap doesn't keep it on the air. A message already complete isn't shown twice when its parts are repeated for someone else, and a message still incomplete after 10 minutes is given up.
- **Speed ramp-up**: each member reports how well it hears the host (measured on the host's own frames) when it joins and again every **2 minutes**; a member silent for 7 minutes (three missed reports) no longer holds the table back. The host picks the fastest mode that the **weakest** member can copy, with **3 dB** of margin, within your bandwidth setting. It announces the change twice in the current mode, waits until both announcements are on the air, then everyone switches together. It slows down at once if someone reports worse, and speeds up only after 2 minutes without a downgrade. Turn this off in Settings ("Roundtable speed ramp-up").
- **Join windows**: a newcomer always calls and listens at DATAC3, so once a table has sped up it couldn't hear them. Every **3 minutes** a sped-up table drops to DATAC3 for **20 s**: the host announces it, everyone switches, the host sends the table's CALL (so stations nearby see it under **Heard** with **Join**), and anyone joining then is heard. Then the table goes back to its speed, or slower if a newcomer hears the host less well. You can keep talking during the window, just more slowly. The first window comes 3 minutes after the table first speeds up. A newcomer whose JOIN went out while the table was fast sends it again when it hears the window's CALL.
- **Staying in step**: broadcasts aren't acknowledged, so a member can miss a speed change. The member most likely to miss one is the weak member the host is slowing down for, because the announcement goes out at the faster speed. So:
  - A member whose own report says it can't copy the current speed, and which hears no slow-down within **40 s**, moves to DATAC3 by itself.
  - A host with nothing to say sends a short heartbeat (its current speed) every **90 s**.
  - A member that hears nothing of the table for **4 minutes** tries, in turn: the speed last announced, DATAC3 (where the join windows are), then the most robust mode. It reports in each time, so the host hears it again.
- **The floor**: click a member's name to pass them the floor (the "talking stick"). It's a courtesy, not a lock.

> Roundtables are always plain text, even with encryption turned on.

## The Depot

The **🏤 Depot** tab is Whistlestop's mailbox: messages that wait for a station instead of needing you both on the air at once. It has four boxes:

- **📬 Arrivals**: mail for you. Stations that connect while you're **Away** leave it here, and stations carrying mail for you hand it over when you link. New mail is marked **NEW** and counted on the tab; **Mark read**, **Reply** (fills in the To box) and **Delete** are on each message.
- **📮 Departures**: mail you've written that hasn't been handed over yet. **Connect to …** calls the station (or the relay) now.
- **🚚 In Transit**: mail you're carrying *for other people* (store and forward). This box only fills if **Carry mail for other stations** is on (see "Store and forward (relaying)").
- **✓ Delivered**: mail the next station confirmed, with the time (UTC) and who took it.

**Writing mail**: type a callsign in **To**, your message, and press **📮 Post**. It waits in Departures. The next time you're linked with that station, for any reason, Whistlestop hands it over at the start of the link, and it moves to Delivered when the other station's Whistlestop confirms it (a MAILACK). Nothing is "seen" or "read" on the other side: delivered means it's in their Arrivals.

**Via**: to reach a station through someone else, put the relay's callsign in **Via**. The mail is handed to the relay when you next link with them, and they carry it on (if they have relaying turned on). Leave Via empty to wait for the station itself.

The Depot is saved in `depot.json` beside the settings, so mail survives restarts. Repeat deliveries are ignored, so a link that drops mid-handover is harmless: unconfirmed mail is simply handed over again next time. If `depot.json` is ever damaged (cut short by a crash, or edited by hand) it is never overwritten: Whistlestop keeps it as `depot.json.bad`, starts a new Depot and says so in the Log, so the old mail can still be recovered from that file. The **To** and **Via** fields take callsigns only (1–12 letters, digits, `/` or `-`).

## Away

**🙋 Here / 🚪 Away** in the top bar says whether you're at the radio. When **Away**:

- Stations that connect get your **away message** (Settings → Automation), with your callsign in front.
- Anything they type in chat is saved to your **Arrivals**, so they can leave you a message just by connecting and typing.
- A link with nothing happening for the **Hang up quiet links after** time (default 180 s) is ended with a polite 73, so the next station can get through. Real traffic counts as activity, file pieces included, so a long transfer is never cut off (the signal reports both stations send every 30 s don't count).
- Automatic delivery and collection (if you turned them on) are allowed. They never happen while you're **Here**.

Away is a convenience, not a licence to leave a station running unattended where your rules don't allow it (see "Legal and good practice").

## Station ID beacon

The **Station ID beacon** (Settings → Automation, off by default) sends one short broadcast frame every N minutes (10 to 120, default 30) when the band is quiet: your callsign, grid, whether you're away, and the callsigns you hold mail for (up to what fits in one 120-byte frame). It has **no whistle**: it's an ID, not a call, so it doesn't stop scanning stations.

Stations on your frequency in the calling mode decode it. If your beacon lists their callsign, their Whistlestop shows **📬 … has mail for you** in the Heard card (and collects it, if they're away with automatic collection on). **Send one now** sends one immediately. A beacon (like a CQ and the automatic calls) waits while someone else is transmitting on the frequency (see "Listen before transmitting"), and is never sent during a link, a CQ, a search or a Roundtable, and (with the segment guard on) never outside the automatic-control segments: you'll see "Station ID skipped" in the Log instead.

## Automatic mail handling

Whistlestop notices stations it holds mail for whenever it hears them: a CQ, a Station ID, a Roundtable frame or a mail notice. The **Heard** card then shows **📮 N message(s) for CALL** with **Deliver now**; when someone says they hold mail for you it shows **📬 CALL has mail for you** with **Collect**. Those buttons are always there. Three switches in Settings → Automation (all **off** by default) let Whistlestop do more by itself:

| Switch | What happens | When |
|---|---|---|
| **Tell stations I hear that I have mail for them** | Sends a short NOTICE frame: "N messages waiting here, connect to me" | Any time the band is quiet, at most once per station every 15 minutes |
| **While away: connect and deliver** | Connects to the station, hands over the mail, waits for the confirmations and hangs up | Only while Away, the band is quiet and you're not calling or searching; once per station every 15 minutes |
| **While away: connect and collect** | Connects to a station that says it holds mail for you; it hands the mail over, and the link ends 10 s after the mail has come, or after 60 s of quiet if none does (mail can take 20–30 s to cross on HF) | Same conditions |

Every automatic transmission is written to the Log (🤖 and 📣 lines), so you can see what your station did while you were gone.

## Store and forward (relaying)

With **Carry mail for other stations** on (off by default), your Depot accepts mail addressed to someone else: a station that links with you can hand you mail for a third station, and you carry it in **In Transit** until you link with (or, while away, hear) that station. It's then delivered like your own mail and moves to Delivered with the callsign that took it. With relaying off, such mail is declined (logged) and stays with the sender.

Loop guards: each message carries a hop count, and at most **3** relay stations may carry a message on its way (sender → relay → relay → relay → the station it's for); a message you've already carried or delivered is never taken twice. Your Station ID and notices include the calls you're carrying mail for, so their owners can come and collect it.

## QSY (moving a link)

Once linked, either station can ask to **move the link** to another frequency, to get off a busy channel or to make room for a long transfer. On the link card, type a frequency in **↔ Move this link to** (kHz) and press **Ask**.

- The other station decides by **its own rules** (below) and answers.
- If it agrees, both stations retune a moment later. The answer has crossed by then, and Mercury's ARQ resends anything lost in the gap. The link carries on at the new frequency.
- The link card shows "on … with …, back to … when the link ends". When the link ends, for any reason, **both stations go back** to where they were.
- If it refuses, you see why ("I only QSY within 20m", "outside my digital segments"…).
- Whatever the rules, the **whole signal** must fit inside an amateur band: USB puts it 300–2700 Hz above the dial, so a dial within 2.7 kHz of a band's top edge is refused. While you're **Away** with **Automatic only in the automatic-control segments** on, a QSY must also stay inside those segments.

**Your rules** (Settings → QSY requests, **off by default**):

| Setting | What it does | Default |
|---|---|---|
| Accept QSY requests | Agree to requests that meet the rules below | off |
| Only within the band I'm on | Never leave the current band (where you know the antenna works) | on |
| Bands | With the above off: which bands a QSY may go to | 40, 30, 20 m |
| At most … kHz | How far from where you were (0 = anywhere allowed) | 50 kHz |
| Only inside the digital segments | Only where your configured segments say data is allowed (legal for an unattended station) | on |
| If SWR stays high there, end the link | A bad SWR sends both stations home | on |

**SWR and the tuner.** Whistlestop watches SWR on every transmission. After a QSY:
- if SWR is high and **auto-tune** is on (Radio Controls), the antenna tuner runs once at the new frequency;
- if SWR is still high (or there's no tuner and the rule is on), Whistlestop says why, ends the link, and both stations go home.

Auto-tune stays **off by default**: with it off, a high SWR after a QSY simply ends the link.

## File library

Share a folder of files that linked stations can browse and download, even while you're away.

**Sharing** (Settings → File library, **off by default**):

- **Share a file library**, and **Folder** (Choose…): the folder others may see. Put only files you're happy to share in it.
- Nothing outside the folder can be reached: no `..`, no absolute paths, no hidden files (names starting with `.`), and no links that lead outside it or to a hidden file or folder.
- **Only while I'm away**: open it only while you're Away.
- **Largest file** (default 5 MB): bigger files are listed but refused. At a few hundred bytes a second, a big file ties up the frequency for a long time.
- Each browse and download is noted in the Log.

**Downloading** from someone else's library: when linked, the **Files** tab shows **📚 Browse** for their library.
- Folders open with **Open**, **⬆ Up** goes back, and **Download** asks for a file.
- It comes as a normal file transfer, with a time estimate. It's accepted automatically because you asked, and saved in your **Download folder** (Settings → Files). A library file is sent only on the link it was asked for: if the link drops, ask for it again.

## Schedule

The schedule moves your radio to different frequencies at different times of day, and can send Station ID while there. Use it, for example, to sit on 40 m at night and 20 m by day while you're away.

Settings → Schedule (**off by default**):

- **Follow the schedule**, and **Only while I'm away** (on by default, so it never moves the radio under you).
- **+ Add a time slot**, then set:
  - **from–to** (HH:MM, **UTC**; a slot may cross midnight, e.g. 22:00–02:00; the same time for both, e.g. 08:00–08:00, means all day);
  - **days** (every day, Mon–Fri, Sat–Sun);
  - the **dial frequency** (kHz);
  - **Station ID**: send the Station ID beacon (every *N* minutes, as set under Automation) while the slot is on.
- The first slot in the list that is on wins. Between slots the radio stays where it is.
- Whistlestop checks every 20 seconds, and moves only when you're not linked, calling or searching. Each move is noted in the Log (🗓).
- Station ID follows the same segment rule as all automatic transmissions.

## Remote operation

Run Whistlestop on the computer wired to the radio (the **radio computer**), and operate it from another Whistlestop anywhere on your home network: the den, a laptop on the couch.

**On the radio computer:** press **🛰 Remote**, then under **Let other computers operate this one**:
- tick **Share this station on my LAN** (off by default);
- set a **Password** of at least 4 characters;
- the **Port** defaults to 7373.

The window then shows the address to connect to, for example `192.168.1.20:7373`, and who is connected. The button reads **🛰 Shared**, and turns amber while someone is operating.

**On the other computer:** press **🛰 Remote**, type the radio computer's address (its LAN address, or a name such as `shack.local`; add `:port` if it isn't 7373) and the password, and press **Connect to the radio computer**. The button turns teal (**🛰 Remote ●**), and from then on everything happens on the radio computer:
- search, CQ, chat, Roundtables, the Depot, QSY, the library, Radio Controls and the waterfalls;
- **Settings**: you edit how the radio computer operates (callsign, CQ, search, bandwidth, automation, QSY, the schedule, the SDR…), and it saves them. Changing what's settled on that computer itself is ignored, with a note saying so: its folders (Download, file library, Depot file), Mercury (program and address), the radio connection (rigctld, managed rig, CAT), Demo mode, its speakers and its remote-sharing settings. Sound devices can be picked only from those it lists. Only **Save & connect** restarts its station; other changes (Answer automatically, Call CQ) apply without dropping the radio or a link.
- **🔊 Listen**: plays the radio's received audio on this computer's speakers (Settings → Audio → Listen on and Listen volume are this computer's). It's 8 kHz audio, about 16 KB/s, nothing for a home network.
- **Files**: files you send are picked here and uploaded, up to **25 MB** each; only files uploaded this way can be sent from the radio computer. Files you receive are saved on the radio computer and also copied to this computer's Download folder (only files in its own Download folder are ever copied back).

**Disconnect** (in the Remote window) goes back to this computer's own station; the radio computer carries on as it was. If the connection drops, this computer goes back to its own station and says so. Turning sharing off or changing the password on the radio computer disconnects anyone operating at once. Run the same Whistlestop version on both computers. If they differ, **Connect** still works but says which versions differ: requests or settings one side doesn't know are skipped with a note.

**Security.** Only computers with private (home network) addresses can connect: 192.168.x.x, 10.x.x.x, 172.16–31.x.x, link-local and IPv6 unique-local. The password is never sent: the radio computer sends a fresh challenge and the other computer answers with an HMAC-SHA256 of it. A wrong password is refused, slowly, and after **10 wrong passwords within a minute** (from any computers together) every remote login is refused for **5 minutes**, the right password included; the radio computer's Log says so. At most **8** connections are accepted at once, and a login must finish within **15 s**. The stream itself isn't encrypted, so **don't forward the port to the internet**; use a VPN to reach home from outside. Anyone who knows the password can transmit with your station, under your callsign: choose it accordingly. On macOS, allow incoming connections for Whistlestop when the firewall asks.

## Phone apps (iPhone and Android)

Whistlestop also comes as native phone apps: SwiftUI on the iPhone and iPad (iOS 17 or newer), Jetpack Compose on Android (8.0 or newer). The screens are the phone's own, but underneath they run **the same Whistlestop engine** as the desktop, so a phone finds, calls and talks to a desktop station exactly as two desktops do: the same whistle, search, CQ, link, chat and Depot.

**Mercury is built into the apps.** The phone runs Rhizomatica's Mercury modem itself (version 1.9.23, unchanged, through Mercury's own embedding library), so there is nothing else to install. It starts with the app when the station isn't in Demo mode and its modem is this phone (Modem host `127.0.0.1`, the default), listening on the usual ports (8300/8301 and 8100). Point the Modem host at another computer to use a Mercury there instead. Mercury is free software under the GPL-3.0, and so are the phone apps.

**The screen.** The header always shows your callsign, the dial frequency and mode, the **Modem**, **Radio** and **Audio** lamps (green when ready), **TX** while transmitting, the **Here/Away** button and the Help button. Below it, five tabs:
- **Operate**: the waterfall (0–3 kHz, the whistle band shaded), the dial (type kHz and tap **Tune**), **Search** (current band, one band, or all bands; **Answer automatically**), the last **Whistle** heard, **Heard** (tap **Answer** to tune and connect, or **Join** on a Roundtable's call to tune and join it), **Call CQ** (choose **Chat (1:1)** or **Roundtable**; a Roundtable needs a name, and the button becomes **Whistle and host the table**) and **Connect** by callsign. While you're linked, a **Linked** card shows the station, mode, speed and time, the encryption state when encryption is on, with **Disconnect** and a **Move to kHz / Ask** row (QSY).
- **Chat**, in two halves chosen at the top:
  - **1:1 chat**: keyboard-to-keyboard over the link. The app switches here when a link comes up. A tick (✓) means the other station confirmed the line. With encryption on, a line under the header shows **Setting up encryption…**, then **🔒 Encrypted · code 264 097** (read the code to the other operator; see Encryption (optional)) or **🔓 Not encrypted** and why.
  - **Roundtable** (marked ● while you're at one): the same Roundtable as the desktop's (see Roundtables). Not at a table, it has **Join here** with a name (tune to the host's frequency first; off during a 1:1 link) and, while a closing link holds Mercury, "Joining … as soon as Mercury is free" with **Cancel** (retried every 2 s for up to 60 s). At a table: its name, whether you host, its mode, who holds the floor, and everyone at the table with how well they hear the host (tap one to pass them the floor), then the conversation, a box to say something to the table, and how much air time is left while your frames go out. **Leave** says goodbye. The app switches here when you join or host one.
- **Depot**: Arrivals, Departures, In Transit and Delivered. Tap a message to read it (that clears NEW), reply, connect now, or delete it. The pencil button writes new mail, with an optional **Via** relay.
- **Log**: what the engine did, newest first, in UTC.
- **Settings**: Help mode, your station (callsign, name, grid), Demo, the **Radio** (USB cable, serial bridge, rigctld or none on Android; the CAT bridge on the iPhone), the modem's host and port, and **Encryption** (Off, Prefer or Require; Off by default; leaving Off asks you to confirm the legal warning, and the choice applies from the next link). **Save** applies them. A change to the callsign, Demo, the modem's host or port, the radio connection or the audio restarts the station (which ends a link in progress); other changes (name, grid, Answer automatically) apply without a restart. The built-in Mercury starts with the settings in force; it can't be restarted inside the app, so after a change to its port or to how it keys the radio, the app asks you to close and reopen it. Everything else (search timing, QSY rules, the library, the schedule…) keeps the values in the phone's settings file, `whistlestop.toml` in the app's own storage, the same format as the desktop's.

**Help mode.** A phone has no hover, so Help works by tapping: turn it on with the **?** in the header (or in Settings) and every card shows a brass **?**; tap one for its explanation. Like the desktop, Help mode is off each time the app starts.

**Try it without a radio.** Turn on **Demo station** in Settings and tap Save: the phone gets the same simulated band as the desktop's Demo mode, with K7BOT whistling, calling CQ and chatting back.

**Connecting a radio.**
- **Audio**: the radio's sound goes through the phone's audio route, typically a USB audio interface (or the radio's own USB sound card) on a USB-C adapter. Both apps ask for the microphone permission, which covers that audio. On the iPhone, Whistlestop keeps listening with the screen off (background audio).
- **Rig control (CAT) on Android: plug the radio in.** Settings → **Radio** offers four ways to reach it:
  - **USB cable** (the choice when nothing else is set up): connect the radio's USB port to the phone (USB-C, or an OTG adapter). Whistlestop opens the radio's USB serial chip itself, through Android's USB host API: Silicon Labs CP210x and CP2105 (Icom, Kenwood, Yaesu), FTDI (Elecraft, Flex, many cables), WCH CH340/CH9102 (Xiegu, QRP radios), Prolific PL2303, and USB CDC radios (IC-705, IC-905, QRP Labs QMX/QDX, truSDX, X6100). It then works out the protocol and speed exactly as the desktop's direct CAT does: Icom CI-V (asking the radio its CI-V address), Kenwood, Elecraft and Yaesu (told apart by the radio's `ID;`), and the FT-817/818/857/897, at each family's usual speeds. On a Yaesu with two USB ports (CP2105), CAT is the first. RTS and DTR are held low, because on many radios they key the transmitter.
  - The first time, Android asks to **allow Whistlestop to use the USB device**; once allowed, the radio connects. Plugging the radio in offers to open Whistlestop (tick "always" and Android stops asking), and plugging it in while the app runs connects it about a second later.
  - **Protocol** stays on Automatic unless the radio isn't found; then choose its family.
  - **Serial bridge**: a serial-to-network adapter at `host:port` that passes the radio's own CAT through.
  - **rigctld**: a Hamlib `rigctld` on your network, at `host:port`.
  - **None (VOX)**: no CAT. Whistlestop can't read or tune the dial, so search and QSY need CAT; with VOX keying, calling and answering on the frequency you set by hand still work.
  - With USB or a serial bridge, Whistlestop shares the radio with the phone's Mercury on `127.0.0.1:4532` (the phone itself only, never the network), and Mercury keys the transmitter through it (Hamlib model 2, NET rigctl). Mercury starts once the radio has connected, waiting up to 30 s for it; if no radio connects, it starts without keying.
  - The line at the bottom of the card shows **Connected:** and the radio, or **Not connected** (the Log says why). Changing the radio connection and tapping **Save** restarts the station.
  - Mercury can't be restarted inside the app. If you change how the radio is reached after the modem has started, the card asks you to close Whistlestop (swipe it away) and open it again, so Mercury keys the radio the new way.
- **Rig control on the iPhone**: iPhones can't drive USB serial chips, so set **CAT bridge** to a serial-to-network adapter (`host:port`) that speaks the radio's own CAT.
- **Sound card**: when a USB audio device is plugged in (the radio's own USB audio, a SignaLink, a DigiRig), the Android app gives it to Mercury for both receive and transmit, preferring one device that does both; otherwise Mercury uses the phone's default audio route. The choice is made when the app starts, so plug the radio in before opening Whistlestop.
- The RTL-SDR wide search isn't available on phones yet.

**Not yet on the phones** (use the desktop for these for now): Files and the file library, the schedule, the Radio Controls window, the antenna tuner and SWR watch, and operating a desktop station remotely from the phone.

**Building the apps** (developers): see `mobile/README.md`. The engine is the `whistlestop-mobile` crate (`mobile/core`), built for the phones by `mobile/build-ios.sh` and `mobile/build-android.sh`, with Mercury from `mobile/build-mercury.sh`.

## Bandwidth and speed

### Your bandwidth limit

The **Bandwidth** card sets how wide Whistlestop may go, from narrow to wide: **250, 500, 1000, 1700, 2300, 2750 Hz**. The bar shows how much of a 2.75 kHz channel that is. It's locked during a 1:1 link (the link's bandwidth was agreed when it came up) and applies from the next one.

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
- **Require**: encrypt or hang up. Never falls back to plain text. If the other station doesn't take part within 120 s, Whistlestop disconnects and says why; a station with encryption off is told in the goodbye that this one requires it. Nothing (chat, files, mail) is ever sent in plain text first.

Turning it on asks you to confirm the legal warning. The phone apps have the same three choices in **Settings → Encryption** and show the check code on the link (Chat tab and the Linked card).

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
- **Frequency box**: type kHz (`14103.15`), MHz (`14.10315`) or Hz (`14103150`) and press **Tune** or Enter. A number under 100 is MHz, over 100 000 is Hz, anything between is kHz; a decimal comma works too.
- **Tuning is locked during a link** (and while calling a station): Mercury holds a link only within about ±55 Hz, so the band buttons, nudges, frequency box, waterfall clicks and searches are off until you disconnect. To move a link, use QSY.
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

## Direct CAT (built in)

With **Settings → Radio connection → Direct CAT (built in)**, Whistlestop controls the radio itself, in the radio's own language. No Hamlib, rigctld or other program is needed.

| Protocol | Radios |
|---|---|
| **Icom CI-V** | IC-7300, IC-7300MK2, IC-705, IC-7610, IC-9700, IC-7100, IC-905, IC-7851, IC-7600, IC-7200, IC-7410, IC-9100, older CI-V radios; **Xiegu** G90, X6100, X5105 (they speak CI-V) |
| **Kenwood** | TS-590S/SG, TS-890S, TS-990S, TS-480, TS-2000; **QRP Labs** QMX/QDX, **truSDX**, **FlexRadio** CAT ports (they speak Kenwood) |
| **Elecraft** | K3/K3S, K4, KX2, KX3 |
| **Yaesu** | FT-991/A, FT-891, FT-710, FTDX10, FTDX101D/MP, FT-950, FTDX3000, FTDX1200, FT-2000, FT-450 |
| **Yaesu (older)** | FT-817, FT-818, FT-857, FT-897 |

**Setting it up**

- **🔍 Detect radios** and **Use** fill in the port and protocol (detection reads the USB descriptions and never opens a port).
- **Protocol: Automatic** works the protocol out from the USB description; if that doesn't say, Whistlestop asks: a CI-V broadcast for Icom, then `ID;` (which tells Kenwood, Elecraft and Yaesu apart), then the FT-817 frequency read.
- **Speed: Automatic** tries the usual speeds for the protocol until the radio answers (Icom 19200 first, Yaesu 38400 and 4800, Elecraft 38400, Kenwood 115200). Choosing the radio's actual CAT speed makes connecting faster.
- **CI-V address**: **Ask the radio** finds it with a broadcast, even if you've changed it from the factory address. Turn it off to type an address in hex.
- **Full power**: the radio's maximum output, so the power slider is right (Kenwood, Elecraft and Yaesu set power in watts).
- **or network address**: `host:port` reaches a radio through a serial-over-network bridge (ser2net and similar) or a FlexRadio CAT port. A bridge that doesn't answer is given up on after about 3 s (per address the name resolves to), and a port that won't open is retried for 2 s, so a switched-off bridge never holds Whistlestop up for long.
- The port is opened with **RTS and DTR held low**, because on many radios those lines key the transmitter.

**Sharing with Mercury.** Only one program can open the serial port, so Whistlestop serves the rigctld protocol on **Share with Mercury on port** (default 4532). Set Mercury to **Hamlib model 2 (NET rigctl)** at `127.0.0.1:4532` and it keys the radio through Whistlestop. Loggers and other programs that speak rigctld can use it too. Turn sharing off only if Mercury keys the radio another way (VOX, or its own PTT port). If a program keys the radio through this port and then disconnects or crashes without unkeying, Whistlestop unkeys the radio for it. Quitting Whistlestop also waits (up to 3 s) until the radio is unkeyed.

**FT-817 family PTT.** Whistlestop checks that the radio did key or unkey, from its acknowledgement or, failing that, its TX status; if it can't confirm, it reports an error instead of assuming the radio is receiving. The PTT readout comes from the radio's own TX status.

**Kenwood TS-990.** Mode is read and set with the TS-990's own `OM` command (main band), including its data modes (USB-D, LSB-D, FM-D).

**What each protocol controls.** Frequency, mode (including the radio's data mode: Icom DATA, Kenwood DA, Elecraft DATA A, Yaesu DATA-U, FT-817 DIG), PTT and the S-meter work on every radio. The Radio Controls window offers what the radio supports:

| | Icom | Kenwood TS-590/890/990 | Elecraft | Yaesu | TS-480 / QMX / truSDX | FT-817 family |
|---|---|---|---|---|---|---|
| Power, AF, RF, squelch | ✓ | ✓ | ✓ | ✓ | ✓ (no mic gain) | — |
| SWR meter | ✓ | ✓ (approximate) | ✓ | ✓ | — | — |
| ALC, power meter | ✓ | ALC | — | ✓ | — | — |
| Preamp, attenuator | ✓ | ✓ | ✓ | ✓ | — | — |
| NB, NR, VOX | ✓ | ✓ | ✓ | ✓ | — | — |
| Auto notch, compressor | ✓ | ✓ | — | auto notch | — | — |
| Antenna tuner on/off and **Tune** | ✓ | ✓ | — | ✓ | — | — |
| Split, VFO A/B, RIT/XIT | ✓ | ✓ | ✓ | ✓ | — | — |

Meter readings come from each radio's own scale and are converted to S-units and SWR; on some radios (Kenwood SWR especially) the conversion is approximate.

**FT-817 family**: set the radio's DIG mode to USER-U in its menu. The FT-817 can't report PTT over CAT, so Whistlestop remembers what it last sent.

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

Settings are saved to `~/Library/Application Support/Whistlestop/whistlestop.toml` (macOS). **Save & connect** applies them and reconnects. Closing the Settings window without it throws your edits away. If the settings file has a value Whistlestop can't read (edited by hand, say), only that setting falls back to its default: the rest are kept, and the Log names each one ("⚠ Settings file: …"). A value that's out of range (a negative listen time, a whistle of hours, a port of 0…) is set to the nearest sensible value, and the Log says so ("⚠ Settings: …"); the same goes for settings sent by a remote operator. A callsign is trimmed and upper-cased, and characters other than letters, digits, `/` and `-` are removed. A file that can't be read at all is kept as `whistlestop.toml.bad` and the default settings are used.

| Setting | What it does | Default |
|---|---|---|
| Callsign | Your call, sent in Mercury's CQFRAME and CALL frames | N0CALL |
| Name, Grid | Sent to the other station at connect (HELLO) | empty |
| Demo mode | Simulated radio and band, with the K7BOT demo station | on |
| Demo on real Mercury | Demo runs two real Mercury modems when Mercury is installed | on |
| Mercury engine | Path to the Mercury engine (empty: look in the usual places) | automatic |
| Mercury host / TNC port / Broadcast port | Where Mercury listens | 127.0.0.1 / 8300 / 8100 |
| Radio connection | Use a running rigctld / RigHelm, let Whistlestop run rigctld, or Direct CAT (built in) | running rigctld |
| Protocol / Port / Speed / CI-V address | Direct CAT: the radio's control protocol and where it is; Automatic detects | Automatic |
| Full power | Direct CAT: the radio's maximum output | 100 W |
| Share with Mercury on port | Direct CAT: serve rigctld here for Mercury and other programs | 4532 (on) |
| rigctld address | The running server's address | 127.0.0.1:4532 |
| Model / Port / Speed | For the rigctld Whistlestop runs; Automatic detects at start | Automatic |
| rigctld port | Where Whistlestop's rigctld listens (point Mercury here) | 4532 |
| Audio in / out | The radio's sound device | system default |
| Wide receiver | Use an RTL-SDR; RX OUT or IF OUT; IF Hz and inversion; crystal ppm | off |
| Dwell per channel | How long search first listens on each channel (a promising channel gets up to 2 s) | 0.5 s |
| Channel step | Distance between search channels (at least 100 Hz, capped to the whistle's reach) | 2000 Hz |
| CQ cycles | How many whistle/CQ/listen cycles before giving up | 5 |
| Listen after CQ | How long to wait for answers each cycle | 20 s |
| Whistle watch | Report whistles while parked | on |
| Listen before transmitting | CQ, Station ID and automatic calls wait while the channel is busy (see "Listen before transmitting") | on |
| Roundtable speed ramp-up | Host adjusts the table's speed | on |
| Encryption | Off / Prefer / Require (see Encryption) | Off |
| Auto-tune on high SWR | Start the antenna tuner after a high-SWR transmission, when quiet (Radio Controls window) | off |
| SWR limit | Above this: red reading, log warning, auto-tune if on | 2.0 : 1 |
| Download folder | Where received files go | ~/Downloads/Whistlestop |
| Listen on / Listen volume | Speaker for the 🔈 Listen button, and its level | System default / 70% |
| Away message | What stations see when they connect while you're away | "Away from the radio…" |
| Hang up quiet links after | While away, end links idle this long | 180 s |
| Station ID beacon / every | Send a Station ID every N minutes (see "Station ID beacon") | off / 30 min |
| Tell stations about their mail | Send a NOTICE when you hear a station you hold mail for | off |
| Deliver automatically while away | Connect and deliver Depot mail | off |
| Collect automatically while away | Connect and collect mail waiting for you | off |
| Carry mail for other stations | Store and forward (In Transit) | off |
| Accept QSY requests (and its rules) | Let a linked station move the link (see "QSY (moving a link)") | off |
| Share a file library / Folder / Only while away / Largest file | See "File library" | off / — / off / 5 MB |
| Follow the schedule / Only while away / time slots | See "Schedule" | off / on / none |
| Share this station on my LAN / Password / Port (Remote window) | Let another Whistlestop on your LAN operate this one | off / — / 7373 |
| Automatic only in the segments | Automatic transmissions only inside the automatic-control segments | on |
| depot_file (settings file only) | Where Depot mail is kept | depot.json beside the settings |

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
- **`whistlestop whistle-wav`** and **`whistlestop cq-demo`** write sound files of the whistle and a whole CQ. Options: `--seconds` (whistle-wav, default 6) and `--whistle` (cq-demo, default 10) take 0.5 to 600 s; `--profile` is `wide` (default) or `narrow`.
- **The two-station bench** (`docs/BENCH.md`) runs two real Mercury modems and two Whistlestop stations on one computer through a simulated HF band: search, link, chat, files, Roundtables, narrow bandwidth, weak signals and encryption.

## How it works underneath

- **Mercury** does the modulation, error correction, acknowledgements and speed ladder. Whistlestop talks to it over its VARA-style TNC (control port 8300, data 8301) and its broadcast port (8100, KISS framing).
- **WSP, the Whistlestop Protocol**, rides on top: compact binary packets for chat, delivery confirmations, file offers, pieces and checks, signal reports, and Roundtable frames. Specification: `docs/PROTOCOL.md`.
- **The whistle** and the search: `docs/DESIGN.md`.

## Legal and good practice

- **Identification**: Mercury's CQFRAME carries your callsign in the clear on every CQ, and connections carry both calls. Identify by voice or CW as your rules require on long contacts.
- **The whistle** is a documented, swept-tone data signal under 2.1 kHz wide (this guide and the source code document it). Use it in the data segments, keep it as short as the searchers need, and listen before calling.
- **Encryption** is off by default and not legal on amateur bands in most countries (see Encryption).
- **Unattended operation**: Automatic answering and auto-tune are conveniences for an operator who is present. The Away features (Station ID beacon, mail notices, automatic delivery and collection) can transmit when you're not at the radio. In the US, a station under automatic control may send data unattended in the automatic-control segments of 47 CFR 97.221(b); outside them, only narrow (500 Hz or less) replies to a station that is under local control (97.221(c)). **Automatic only in the automatic-control segments** (on by default) keeps every automatic transmission inside the configured segments; turn it off only where your rules allow. You remain responsible for your station whatever it does on its own: check the Log.
- **QSY and the schedule** move an unattended station's frequency. Keep **Only inside the digital segments** on, and allow only bands and distances your antenna handles: a high SWR ends a QSY.
- **Beacons**: the Station ID is a short identification and mail list sent at most every 10 minutes, never a continuous beacon. Some rules restrict beacons on HF: keep the interval long and turn it off where it isn't allowed.

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
