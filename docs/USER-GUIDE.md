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

> Whistlestop is new, and in beta. Its whistle and protocol are tested in simulation, with two real Mercury modems on one computer, and between two real radios transmitting into dummy loads, but not yet between stations on real antennas. Treat on-air results as experiments, and report what you find (see "Reporting a problem").

## Quick start: Demo mode

The first time you open Whistlestop it starts in **Demo mode**: a simulated radio on a simulated 20 m band, and a demo station, **K7BOT**, who calls CQ every 45 seconds. Nothing is transmitted.

- **With Mercury installed** (the `mercury` engine in `~/.local/bin`, `/opt/homebrew/bin` or `/usr/local/bin`), Demo runs **two real Mercury modems**: one for you, one for K7BOT, joined by the simulated band. Everything in the modem strip, and everything you hear with **🔈 Listen**, is real Mercury: its CQ burst, its speed ladder, its acknowledgements. Quitting stops both. If Whistlestop ever ends without quitting (a crash, or it was force-quit), they'd keep running in the background, so the next time Demo starts it stops any left by a Whistlestop that's no longer running.
- **Without it**, Demo simulates the modem too, and the CQ burst you hear is an imitation.
- **Settings → Mode → Demo on real Mercury** switches between the two; **Mercury engine** sets the path if Mercury lives somewhere else.

Try this:

1. Press **🔈 Listen** in the top bar to hear the band on your speakers. Look at the **Radio audio** waterfall at the bottom left: every so often you'll see (and hear) K7BOT's whistle as slanted strokes.
2. In **Search for whistles**, press **Current band**. Your radio steps through the 20 m digital segment. Within about one pass it stops, says "Whistle! measuring", retunes, and waits for the CQ.
3. When K7BOT's CQ is heard it appears under **Heard**. Press **Answer** (or tick **Answer automatically** before searching).
4. Once linked, type in **Chat** and press Enter. K7BOT answers. Ask it for a file: it offers one in the **Files** tab.
5. Drop a file on the window to send one back.
6. Try **Call CQ → Roundtable** to host a table. K7BOT joins.

When you're ready for the air, follow "Getting started on the air" below.

## Getting started on the air

The short path from a new install to a first contact, for beta testers. Each step points to the section with the details. Do it on a **dummy load** first: everything up to step 6 transmits only into it.

### What you need

- A radio with CAT control over USB, and its USB sound card (or an interface such as a SignaLink or DigiRig). See "Tested radios" for the two that have been through the RF bench.
- A computer running **Windows, macOS or Linux**, with the Whistlestop build for it.
- **Mercury 1.9.23**, the modem Whistlestop drives (below).
- Your callsign, and a 20 m or 40 m antenna for step 7.

### 1. Get Mercury

Mercury is Rhizomatica's open-source HF modem. Download **version 1.9.23** from its releases page, `github.com/Rhizomatica/mercury/releases/tag/v1.9.23`:

- **Windows**: `mercury-1.9.23-w64-09f258f8.zip`. Unzip it somewhere permanent; the command-line engine is `mercury.exe`. (The `Setup.exe` installer installs Mercury's own window, `mercury-ui.exe`, which runs the modem inside itself; Whistlestop is tested with `mercury.exe`.)
- **macOS**: `Mercury-1.9.23-universal.dmg`. The image holds the command-line engine `mercury` beside `Mercury.app`. Copy `mercury` to `/usr/local/bin` (or `~/.local/bin`), where Whistlestop also finds it for Demo mode.
- **Linux**: build it from the source (Mercury's README, "Compile and Install from Git repository on Linux"), then `sudo make install`.

`mercury -V` prints the version. Both stations of a contact need Mercury **1.9.20 or later** (older ones can't connect to them), but use 1.9.23: the bench, the RF tests and the phone apps all run 1.9.23. **1.9.24** (13 November 2026) changes how Mercury reports a link's end and its calls to programs like Whistlestop, and hasn't been tested with Whistlestop yet.

Run only one program on Mercury at a time: close Mercury's own window (`mercury-ui`), VarAC and anything else that uses its TNC port (8300).

### 2. Set up Whistlestop

1. Open **Settings**. Under **Station**, type your **Callsign** (and name and grid if you like); under **Mode**, untick **Demo mode**.
2. **Radio connection**: choose **Direct CAT (built in)**, press **🔍 Detect radios**, then **Use** beside your radio. Leave **Protocol** and **Speed** on Automatic, and **Share with Mercury on port** on, at 4532. (Or use Hamlib's rigctld: see "Rig control".)
3. **Audio in** and **Audio out**: the radio's USB sound card (a QMX shows as **QMX Transceiver**, a Yaesu or Icom as a **USB Audio CODEC**). Leave **Listen on** on the computer's speakers, never the radio.
4. Press **Save & connect**. The **Radio** pill turns green and the frequency display follows the radio's dial; the **Audio** pill turns green and the **Radio audio** waterfall shows the band's noise.

On the radio: **USB** or its data mode (PKTUSB, DATA-U), a filter of at least **2.4 kHz**, and noise reduction, noise blanker and speech processing **off** (see "Radio settings", and "QRP Labs QMX" for a QMX).

### 3. Start Mercury

With direct CAT, start Mercury **after** Whistlestop: it keys the radio through Whistlestop's sharing port, which is there only once Whistlestop has connected to the radio.

First, find the sound card's id: `mercury -z` lists every device with the id to give Mercury. Then, from the folder Mercury is in (on Windows, `mercury.exe` in a Command Prompt there):

```
mercury -R 2 -A 127.0.0.1:4532 -i "<capture id>" -o "<playback id>" -G -L mercury.log
```

- `-R 2 -A 127.0.0.1:4532`: Mercury keys the radio through Hamlib model 2 (NET rigctl) at Whistlestop's sharing port (the same address for the rigctld Whistlestop runs, or RigHelm's server).
- `-i` and `-o`: the radio's sound card, as `mercury -z` printed it. Leave Mercury's sound system (`-x`) on its default.
- `-G`: opens Mercury's UI port (10000), so Whistlestop can set its transmit level (see "Transmit level").
- `-L mercury.log`: Mercury writes its own log there; send it with a problem report.
- The TNC port (8300, data 8301) and broadcast port (8100) are Mercury's defaults and Whistlestop's: leave them.

Then **click the red Mercury pill** in Whistlestop. It turns green, and the modem strip shows Mercury's version.

**How Mercury keys the radio: `-P`.** `-R` chooses Hamlib keying by itself; `-P` chooses another way:

| `-P` | Keys the radio by | Use it when |
|---|---|---|
| `hamlib` (or just `-R 2`) | a CAT command, through Whistlestop's sharing port | normally: every tested setup |
| `serial` | the RTS line of a serial port, `-A COM7` or `-A /dev/ttyUSB0` (DTR or both: `line` under `[ptt]` in mercury.ini) | an interface keyed by RTS (DigiRig, AIOC). Never the radio's CAT port: Whistlestop has it open |
| `cm108` | a GPIO pin of a CM108-type USB sound chip (`cm108_gpio` under `[ptt]`, pin 3 by default) | an interface keyed that way (AIOC) |
| `none` | nothing: Mercury doesn't key | only a radio keyed by its own VOX |
| `hermes_shm` | HERMES station hardware (Linux only) | not for Whistlestop stations |

Whatever Mercury uses, Whistlestop keys the radio itself for the whistle, through its own radio control. With anything other than `hamlib`, keep **Share with Mercury on port** on anyway if other programs use it, or turn it off.

**Check the keying, into the dummy load:** `mercury -R 2 -A 127.0.0.1:4532 -Q` keys the radio for one second, unkeys it and exits, without starting the modem. If the radio doesn't key, the problem is between Mercury and the sharing port, not in Whistlestop.

### 4. Leave these settings alone at first

Every default has been through the bench. Change these only once the basics work, and say in a report what you changed:

| Leave | At | Why |
|---|---|---|
| Bandwidth | 2300 Hz | the full Mercury ladder; narrower settings use the narrow ladder and narrow whistle |
| Whistle | 10 s | about one search pass of 20 m; agree longer ones with the other station for All bands |
| Dwell per channel, Channel step | 0.5 s, 2000 Hz | the search's timing is tuned to them |
| CQ cycles, Listen after CQ | 5, 20 s | |
| Listen before transmitting, Whistle watch | on | |
| Roundtable speed ramp-up | on | |
| Encryption | **Off** | illegal on US amateur bands (see "Encryption (optional)") |
| Transmit level | Mercury's own | until you've watched the ALC: then try **Auto (from ALC)** (see "Transmit level") |
| Auto-tune on high SWR | off | |
| Everything under Automation, QSY requests, File library, Schedule, Remote | off | they transmit or open your station without you |
| Automatic only in the segments | on | |
| Search segments (`segments` in the settings file) | the US automatic-control segments | |

### 5. First check, on a dummy load

1. **Radio control**: press a **band button** (20 m). The radio's dial moves; turn the radio's knob and the frequency display follows.
2. **Keying and level**: press **Tune carrier** in the Radio card. The radio transmits Mercury's 1 kHz tone into the dummy load (it stops by itself after 60 s); the TX lamp turns red; press **Stop**. Watch the radio's power and ALC.
3. **A CQ**: set the **Whistle** to 4 s and press **Call CQ**. The radio keys for the whistle, then for Mercury's CQFRAME (about 3 s), then the short CALL packet. The ALC should barely move: if it climbs, turn the audio down (see "Transmit level"). Press **Stop CQ**.
4. **The Log tab**: a **⚠** line here means something to fix first. "⚠ Mercury is transmitting but the radio isn't keyed" means Mercury's keying (step 3) isn't reaching the radio. A CQ that says "CQ waits: someone is transmitting on this frequency" is Mercury's busy detector reading noise: it goes after **60 s** at most.
5. **Two stations** (two testers in one place, or two radios of your own): with both on dummy loads a few metres apart, each hears the other weakly. One calls CQ; the other presses **Current band** in **Search for whistles**, with **Answer automatically** ticked. The search stops on the whistle, the CQ appears under **Heard**, the stations link, and you can chat and send a small file. A radio with poor image rejection may lock on the wrong side of the dial at such close range: see **Mirror images** under "Search".

### 6. First contact on the air

1. Arrange a schedule with another tester (by e-mail, phone or voice): a time, a band, and who calls. Pick a frequency inside the band's digital segment ("Band plan"), for example 14100.5–14112 kHz on 20 m.
2. **Listen first**, and check the frequency is clear, on the radio and on the waterfall.
3. The caller sets the **Whistle** to 20–30 s for the first tries and presses **Call CQ**. The other station presses **Current band** with **Answer automatically** ticked, or tunes to the agreed frequency and waits for the CQ under **Heard**, then **Answer**.
4. Once linked: chat a few lines, send a small file (a few kB), then **Disconnect (73)**.
5. Then try it without the schedule's frequency: the caller anywhere in the segment, the other station searching.
6. Whatever happens, press **💾 Save log…** on the Log tab afterwards and send it (see "Reporting a problem"), from **both** stations if the link misbehaved.

Identify as your licence requires: Mercury's CQFRAME carries your callsign on every CQ, and a link carries both calls (see "Legal and good practice").

### Known limitations in this beta

- **File-offer time estimates can be off**: the time shown with an offer is a prediction, and Mercury's real speed can differ a lot (it climbs its speed ladder only while there's data to send).
- **A CQ can wait up to 60 s** when Mercury's busy detector reads local noise as a signal (see "Listen before transmitting").
- **A radio with poor image rejection can fool Search** at close range (see **Mirror images** under "Search").
- **Roundtables of three or more stations** are only partly tested.
- **Not yet tested between stations on real antennas**: you're the first.

## Tested radios

These radios have been through the RF bench: two stations transmitting into dummy loads, on 16 November 2026. There, **CQ and answer, chat, 3 kB file transfers, the Station ID, and a search answering a CQ** all worked. Roundtables, the Depot, Away, QSY, the file library and the SDR wide search passed in earlier test sessions.

| Radio | Computer | Radio connection | Mercury keys it by | Notes |
|---|---|---|---|---|
| **Yaesu FTDX3000** | Windows | **Direct CAT (built in)**, shared as Hamlib NET rigctl at `127.0.0.1:4532` | `-R 2 -A 127.0.0.1:4532` | CAT is the first of the two USB ports ("Enhanced"); Whistlestop opens it with RTS high, for the radio's CAT RTS menu (see "Direct CAT (built in)"). The ALC reads high even at modest levels (Mercury at 0 dB read 54 % at 5 W, the whistle 62 %): use **Auto (from ALC)** (see "Transmit level"). A keyed FTDX3000 ignores a new frequency, which QSY waits out. |
| **QRP Labs QMX** | macOS | **Direct CAT (built in)**: a QMX speaks Kenwood TS-480, and the Log names it ("QRP Labs QMX (firmware 1_02_005)") | `-R 2 -A 127.0.0.1:4532`, with the QMX's **SSB VOX** doing the keying | Needs the SSB firmware (1_02_000 or newer), **USB** mode (not DIGI), SSB Input USB and SSB VOX on (see "QRP Labs QMX"). Its opposite-sideband image was only about **8 dB** down on the bench, enough to fool a search at close range (see **Mirror images**). A short 1500 Hz "ding" follows Mercury's transmissions: harmless. |

Any other radio in the "Direct CAT (built in)" table, or any radio Hamlib's rigctld supports, should work the same way, but hasn't been tested with Whistlestop on the air yet. If you use one, a report saying what worked is just as useful as one about a problem. Describe it with:

- **The radio**: make, model and firmware version, and how it's connected (its own USB cable, or an interface: which one).
- **How Whistlestop reaches it**: Direct CAT (the protocol and speed, and the line the Log shows when it connects, which names the radio), or rigctld (the Hamlib model number).
- **How Mercury keys it**: the whole Mercury command line, or its `[ptt]` section.
- **The radio's own settings** for data: mode (USB, DATA-U, PKTUSB…), filter width, CAT speed, any CAT RTS / DTR or "USB send" menu, and its USB audio levels (in and out).
- **What worked**: tuning from Whistlestop, keying (the TX lamp), the S-meter, SWR, ALC and power readings, the Radio Controls window, and the transmit level that kept the ALC to a bar or two.
- The saved Log from your first session with it.

## Reporting a problem

A good report lets the problem be found without you there. Send:

1. **The Log**, saved with **💾 Save log…** on the **Log** tab (below), from your station, and from the other station too if it was a link that misbehaved.
2. **Mercury's log**, if you started Mercury with `-L mercury.log` (see "Start Mercury"): the file of that name in the folder you started it from.
3. **What you did, what you expected, and what happened**, with the time in **UTC** (the Log's times are UTC), the band and frequency, and the other station's callsign.
4. **Your radio**, described as in "Tested radios" if it isn't one of the tested ones.

Send it to the address your beta invitation gave you, or open an issue at `github.com/NR5P/whistlestop/issues`.

### The Log tab

The **Log** tab lists what your station did: every transmission, connection, search step, warning (⚠, in red) and automatic action, each with its UTC time. It shows the newest **500** lines. Above them:

- **💾 Save log…** saves a text file (named like `whistlestop-log-20261116-1403Z.txt`; the dialog opens in your download folder, Settings → Files, once that exists) with Whistlestop's **version and build**, your computer's system, when it was saved, your callsign, the radio and Mercury's version, **your settings** (the remote-operation password is left out), and then **every Log line since Whistlestop started**, each with its date and time. Read it before sending it: the settings include your name, grid, away message and folder names.
- **📋 Copy log** copies the same text, to paste into an e-mail or an issue.
- **📂 Log files** shows the Log's own file in Finder or Explorer.
- At the right: the **version**, and in brackets the build (git commit) it was made from, for example `Whistlestop 0.1.0 (v0.1.0-beta.2)` for a beta release; a build made between releases shows the last beta, how many changes since and the commit (`v0.1.0-beta.2-3-g9e7faad1c`). Put it in every report.

**The Log's file.** Whistlestop writes every Log line to `whistlestop.log` beside the settings file as it happens (see "Settings reference" for the folder), so nothing is lost when the tab's 500 lines move on, or if Whistlestop crashes. Each time Whistlestop starts, the last session's file is kept as `whistlestop-previous.log`: **after a crash, send that one** (use **📂 Log files** to find it). A session that writes more than **20 MB** of Log starts the file afresh, keeping the full one as `whistlestop-previous.log`. While you operate another computer (see "Remote operation"), its Log is the one on the tab, and Save log saves its newest 500 lines; this computer's own Log still goes to this computer's file.

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
  - Yaesu radios by their two-port USB chip (CAT is on the first, "Enhanced" port; the other is never offered: it keys the radio). Where the system says which USB interface a port is (Linux, Windows, recent macOS), that decides it; otherwise the two ports' shared USB serial number does;
  - Kenwood, Elecraft, QRP Labs (QMX/QDX, which speak Kenwood TS-480), Xiegu and others by their USB names.
- Press **Use** beside your radio, or choose **Model**, **Port** and **Speed** yourself. Leaving them on **Automatic** detects them every time Whistlestop starts. Model and port always come from the same radio: with a model chosen and the port on Automatic, it's the port of the radio of that model (or of the only radio there is); with a port chosen and the model on Automatic, it's the model of the radio on that port. If that can't be told, Whistlestop asks you to choose instead of guessing.
- On Linux, ports are kept by their stable `/dev/serial/by-id/…` names, so two USB devices swapping `ttyUSB0`/`ttyUSB1` after a reboot can't send CAT to the wrong one.
- **Detection never opens a port.** Opening a serial port raises its RTS and DTR lines, and on some radios that keys the transmitter. When Whistlestop starts rigctld, it holds RTS and DTR off.
- rigctld listens on `127.0.0.1:4532` (change **rigctld port** if something else uses it). Point Mercury there. If something already answers on that port (another rigctld, RigHelm's server), Whistlestop says so and doesn't start a second one beside it. If rigctld stops straight away (a wrong or busy port), the Log says why, from its own messages (kept in `rigctld.log` beside the settings).
- A rigctld left running by a Whistlestop that ended without quitting (a crash, a force-quit) is stopped the next time Whistlestop starts one.

**Use a running rigctld / RigHelm**. If something already owns the radio:

- **RigHelm**: enable its built-in rigctld server (default `127.0.0.1:4532`).
- **Hamlib rigctld** you start yourself: `rigctld -m <model> -r <port>` (find the model number with `rigctl -l`).

Set **rigctld address** to it. Whichever way, the **Radio** pill in the top bar turns green when connected, and the log names the radio Hamlib reports.

### 2. Mercury

Install Mercury: which version and file to download, and the command line to start it with, are in "Getting started on the air" (steps 1 and 3). Set it up so it shares the radio:

- **Radio / PTT**: Hamlib model **2 (NET rigctl)** pointing at `127.0.0.1:4532` (the default for all three ways: Whistlestop's direct-CAT sharing port, the rigctld it runs, or RigHelm's server). Mercury then keys the transmitter through the same server.
- **Sound card**: the radio's USB audio codec, both directions.
- **Ports**: leave the TNC port at **8300** (data on 8301) and the broadcast port at **8100**, or set the same numbers in Whistlestop **Settings → Mercury modem**.
- **Transmit level** (optional): start Mercury with **-G** (its UI port, 10000) and Whistlestop can set Mercury's transmit level for you, by hand or automatically from the radio's ALC. See "Transmit level".

The **Mercury** pill turns green when Whistlestop is connected. Mercury accepts one program at a time on its TNC port, so close VarAC or any other program that uses it. Whistlestop tries Mercury when it starts and doesn't keep retrying (each attempt would take Mercury from a program using it). If you start Mercury afterwards (with direct CAT it has to come second, since it keys the radio through Whistlestop's sharing port), **click the red Mercury pill** to connect; the radio connection stays up.

### 3. Audio

Whistlestop listens to the radio audio itself (for the whistle detector and the waterfall) and plays the whistle when you call CQ. In **Settings → Radio & audio**, choose the radio's USB audio device for **Audio in** and **Audio out**. The **Audio** pill shows the input is running. The output to the radio is opened only while Whistlestop has something to send (the whistle, the tune carrier) and closed 0.5 s after, so the radio's sound card isn't kept busy with silence; a device that's missing or won't play is still reported when the station starts.

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

The window opens at 1440 × 920 points; on a screen too small for that (a 1080p laptop at 125 % scaling, say) it opens maximized instead, so nothing is cut off at the bottom.

From top to bottom:

- **Top bar**: the Whistlestop mark; the **frequency display** (TX/RX lamp, dial frequency, band, radio mode); the **S-meter** and **SNR** meters; **🙋 Here / 🚪 Away** (see "Away"); **🛰 Remote** (see "Remote operation"); **DEMO** when in Demo mode; **🔈 Listen** (the radio's audio on your speakers); **📖 Guide** (this manual, in its own window); the **? Help** toggle; and **Settings**.
- **Modem strip**: what Mercury is doing right now (see "The modem strip"), and at its right end the **status pills** (Radio, Mercury, Audio, SDR: green means connected).
- **Left column** (top to bottom): **Call CQ**, **Search for whistles**, **Heard**, **Bandwidth**, **Radio**.
- **Centre**: tabs for **Chat**, **Roundtable**, **Files**, **Depot** (mail; see "The Depot") and **Log** (with **Save log…**, **Copy log** and the version; see "The Log tab" under "Reporting a problem").
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
- **It is distinctive.** Both detectors need UP and DOWN together. The scan detector levels away anything that sits in one frequency most of the time (carriers, RTTY, FT8, modems), and it ranks each frequency against itself, so clicks, static crashes and bursts count for no more than the loudest noise there, however loud they are. Steady tones and data bursts don't trigger it: on recordings of a real receiver across all nine search segments, evening and day, about 1 % of empty half-second listens score high enough to stop (up to 7 % on the busiest, 40 m in the evening), and almost none on a quiet band full of clicks, while 92 % of whistles at −10 dB are caught in that half second.
- **Mercury's MFSK is told apart by its shape.** Mercury's slowest mode (MFSK, and MFSK500), used by links on weak signals and by Roundtables at their slowest speed, sends one tone at a time hopping over 1–2 kHz, which is where the whistle sweeps. To the scan detector that looks like a whistle, at any strength. The scan detector lets MFSK through only now and then (in 7–60 % of its looks, depending on its speed and the whistle's width), and never with a high score: 12 at most, while a whistle from −5 dB up scores 14 to 40. The matched filters tell them apart too: a whistle's chirp pairs all give the same offset (three or four of them within 2 Hz in 1.6 s, from −10 dB up), while MFSK gives one chance pair, or a few that disagree by 10–45 Hz. A **strong** signal (from about 0 dB) whose pairs don't line up and that scores under 14 is taken for **data**. Mercury's own signals in every mode, at −5 to +30 dB, give no whistle reports. Mercury's OFDM modes (DATAC1 to QAM16C2) never pass the scan at all.

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
- **Stop CQ** ends it at any point: the whistle stops at once; a CQFRAME already on the air is left to finish (Mercury unkeys the radio at its end, a few seconds), and the short CALL frame that would follow it isn't sent.

When someone answers, Mercury reports the incoming call and the CQ goes on hold at once ("Incoming call: CQ on hold" in the Log): a whistle in progress stops and nothing more is sent, because a whistle or CQFRAME on top of Mercury's answer would make it drop the call. When the link comes up the CQ stops for good and the Chat tab shows "Linked with …". If the call doesn't complete (Mercury gives up after about 45 s), the CQ carries on with its next cycle; the hold lasts at most 60 s.

> Courtesy: listen first. The whistle is a calling signal, not a beacon. Keep it as short as the searchers need, and don't call on a busy frequency.

### Listen before transmitting

Whistlestop also listens for you before it transmits on its own. It turns on Mercury's busy-channel detector while you're not in a link, and while that reads the frequency as busy (someone else transmitting):

- a **CQ** holds its whistle ("CQ waits: someone is transmitting on this frequency" in the Log, and "waiting for a clear channel" with the seconds left under the CQ), between cycles too;
- a **Station ID** waits;
- the Away **automatic deliver and collect** calls wait. The Station ID or CQ that sets one off has only just ended, so the detector usually still reads it for a moment; the call goes out once the channel clears.
- at a **Roundtable**, everything your station sends to the table (your messages, its reports, joins and goodbyes, and as host its announcements) waits too ("Roundtable message waits: …"). The host's CQ whistle for its table is the usual case: a message sent on top of it is lost whole.

**Your own transmission's tail doesn't count.** Some radios make a short sound in their own receiver as they switch back from transmit: a QMX, for one, gives a 1500 Hz "ding" that dies away in under a second and that the detector reads as busy. So a busy reading that starts within **1.5 s** of your own transmission ending is set aside until **3 s** after it ended; if the frequency still reads busy then, someone really is there and the wait begins as usual.

**Silent audio doesn't count either.** A receiver that sends almost no audio (under −100 dBFS: a radio on a dummy load, or its data output turned down) hears no one, yet the detector can read the sound card's own faint tones as busy. So once the radio's audio has been that quiet for **2 s**, a busy reading is set aside (the Log already warns that the audio is almost silent). As soon as a station comes on, the audio isn't silent any more and busy counts again.

A CQ, Station ID, automatic call or Roundtable frame waits at most **60 s**, then goes anyway and the Log says so. If an automatic call is still waiting after 90 s, or you come back, start something else or the mail is gone, it is dropped. Going anyway matters because a detector that reads busy all the time mustn't silence your station. That happens behind a receive filter narrower than about 1.8 kHz, so use a wide filter, or turn it off (**Settings → Search & CQ → Listen before transmitting**, on by default). What you send by hand (Connect, Send, a Station ID you press) always goes at once. During a link the detector is off, so the link behaves as it always has.

## Finding stations

There are three ways to find a calling station: searching with your one radio, the whistle watch while you're parked, and the wide search with an SDR. None is required: you can always tune by hand and press **Connect**.

### Search (one radio)

**Search for whistles** steps your radio through the digital segment of a band, listening on each channel:

- **Current band**: the segment(s) of the band you're on.
- **All bands**: every configured segment, band after band. A pass takes much longer (the default segments, 80 to 10 m, are 75 channels and about **50 s** a pass on the air), so a caller is found only with a whistle about that long: on the RF bench a 60 s whistle on 30 m was found in the first cycle, and 6 s ones were missed for five cycles. Use it for monitoring, or agree long whistles with the caller.
- **Search band** with the band list: one chosen band.

On each channel the radio settles (0.15 s), listens for **0.5 s**, and checks for a whistle. Channels are **2000 Hz** apart, so every caller in the segment is within ±1100 Hz of some channel. A 20 m pass takes about 6–8 s.

Listening is in two stages, the way military ALE scanners pause on a promising channel:

- If the half-second clearly holds a whistle, the search stops there.
- If it holds a hint of one (about 15 % of empty channels do, and almost every real whistle), the search **listens longer**, up to 2 s, and decides on that.
- Otherwise it moves on. Fast where nothing is there, patient where something might be.

When it finds a whistle it **retunes** by the coarse offset, **measures** the whistle precisely on the new frequency (three patterns, 1.2 s) and fine-tunes, then **waits up to 45 s for the CQ**. It locks on only when that measurement **confirms** a whistle, the same test the whistle watch uses: its chirp pairs line up, or the scan scores it 14 or more, which only a whistle does. Otherwise it listens once more and then moves on ("Whistle not confirmed after retuning"), so a busy band's signals don't hold it for 45 s. On recordings of real bands, none of the signals that passed the first look was confirmed, while 95 % of whistles at −10 dB were. If the measurement shows the signal is data (MFSK: see "It is distinctive" above), the search moves on at once instead ("Not a whistle on … kHz after all: data" in the Log), so someone's MFSK link or table doesn't hold it for 45 s every pass. Right after a retune the whistle should be within about 10 Hz; if the measurement says **25 Hz or more**, the radio or sound card was probably still catching up when it listened. Audio from before the retune shows the whistle where it was before, so Whistlestop first listens again where it is ("Measured … Hz off after retuning; listening again here" in the Log). Only if the second look agrees does it retune there and measure once more before locking ("… off again; checking there"). Up to 3 retunes in all, counting the "further" ones. If the whistle ends meanwhile, it waits for the CQ where it is ("Whistle ended; staying on … for the CQ"). It does the same when the first find was unmistakable (a scan score of 14 or more, which only a whistle reaches) and the whistle is gone by the time the radio has retuned: the search reached the caller in the whistle's last second or two, and the CQ comes next. A weaker find that can't be heard again after the retune is dropped ("Whistle not confirmed after retuning"). When Mercury decodes the caller's CQFRAME, the call appears under **Heard**. With **Answer automatically** ticked, Whistlestop connects as soon as the caller has stopped transmitting: right after its CQFRAME the caller sends its CALL packet (about 3.7 s), so Whistlestop waits until that packet is heard, or **4.5 s** at most, before calling. A call sent sooner would land on top of the caller's own transmission and go unheard. If no link comes up within **60 s** of calling (say another station answered first), it gives up, so its call doesn't go on over the new link. If the CALL packet says the CQ is for a **Roundtable**, answering means joining it: Whistlestop joins the table instead of linking 1:1 with its host ("Auto-answer: …'s CQ is for the Roundtable …; joining it" in the Log).

On a simulated busy band with fading, a half-second listen catches 82–84 % of whistles at −10 dB SNR (in 2500 Hz) on the first look, and 94–97 % at −5 dB; those it misses at first mostly get the 2 s listen, which catches 96–99 % at −10 dB. Weaker than about −10 dB, Mercury itself struggles to link.

**Mirror images.** A receiver also hears, weakly, what's on the other side of its dial: the **opposite sideband**, mirrored. Good radios keep it 40 dB or more down, but a strong whistle (a nearby station, or two stations on dummy loads in one room) can still show there, and the whistle's up-and-down sweep looks the same mirrored, so the scan can't tell it from the real thing. On the RF bench a QRP Labs QMX heard a whistle on 7100.5 kHz from its 7102.8 kHz channel, only about **8 dB** below the real one, as if it were 690 Hz below that channel; the search locked on 7102.15 kHz and missed the CQ (Mercury can't decode a mirrored CQFRAME). The image of a caller on dial *D* reads, on channel *C*, *C* − *D* − **3000 Hz** (the whistle sweeps 1000–2000 Hz, centred on 1500), and it moves **the opposite way** when the radio retunes. So, after a find, Whistlestop also notes where the caller would be if the find was an image: **channel − offset − 3000 Hz**. Retuning by the find's offset brings a real whistle to about 0 Hz; an image doubles its offset instead, or goes out of view. When the look after the retune shows that, Whistlestop tries the image's dial once (not for a find within 100 Hz of the edge of the scan's ±1100 Hz reach: that's a rough estimate of a whistle further off, which the retune corrects as described above) ("That looks like the whistle's mirror image (the radio's other sideband); trying … kHz" in the Log). If it finds the whistle there, it steps **120 Hz** aside before locking ("stepping 120 Hz aside to be sure it isn't a mirror image"): a real whistle then reads −120 Hz, an image +120 Hz. A real one is locked on ("checked a step aside"); an image sends it 3 kHz down to the real caller ("It moved the wrong way: a mirror image after all"). If nothing is heard at the image's dial, it goes back to the first estimate ("Not there either; back to … kHz"). Each place is tried once. A real whistle whose first look was rough, or that ended just as the search reached it, costs about **1.4 s** more. If your searches often report mirror images, check the radio's receive I/Q (image) calibration, if it has one.

### Whistle watch (parked)

Even when you're not searching, Whistlestop listens for whistles on the frequency you're on. If one is heard nearby, a banner under **Heard** says "Whistle +640 Hz from here" with **Tune to it**. A whistle is reported only when two checks in a row (about a second apart) agree on the offset within 15 Hz, so a single data burst isn't mistaken for one, and each check needs the whistle's chirp pairs to line up (or a scan score of 14 or more, which only a whistle reaches): the two checks share 0.6 s of audio, and someone's MFSK link or table on your frequency would otherwise be reported as whistles. The watch pauses while you're transmitting, while a call is being made or answered, during a link, for 8 s after a call starts or a link comes up, and for 30 s after a link ends, because Mercury's connect and disconnect bursts can look like a whistle. It also ignores audio from before your last retune. Turn it off in **Settings → Search & CQ**.

### Wide search (RTL-SDR)

With an RTL-SDR running, tick **Wide search (SDR)** in the Search card. The SDR's wide view is split into virtual receivers every 1.8 kHz (with the default whistle) across the band's digital segment, and the whistle detector runs on all of them every second. A whistle anywhere in the segment is found within about 1.5 s, without moving your radio. Each hit is checked with a receiver centred on it: three or more of its chirp pairs must agree (a whistle gives about five in the 2.2 s look, three from −12 dB up), so MFSK traffic in the segment isn't shown as callers.

- Hits appear under **Heard** with an **SDR** tag and a **Lock on** button, and as a 🚂 box on the wide waterfall.
- **auto-tune**: tune the radio to new hits by itself (when you're not linked, calling, being called or searching, and not at a Roundtable or joining one). **Lock on** is refused at a Roundtable too ("leave it first"): it would take the radio off the table's frequency.
- **With a search running, both methods work together**, auto-tune or not. While the stepping search is still going channel to channel (listening, or holding on something whistle-like), a fresh SDR hit pauses it: the radio goes straight to the hit, fine-locks, and waits up to 45 s for the CQ. If the hit isn't confirmed or no CQ comes, the search resumes on the channel it was on ("Back to the … search" in the Log). A search that has already found a whistle itself (measuring it, or waiting for its CQ) is never interrupted. Each SDR frequency is tried at most once every 2 minutes (within 200 Hz), so one caller can't keep pulling the search away.
- The stepping search still matters with an SDR: it covers any part of the segment the SDR can't see, and it hears through your radio's better front end.
- **Lock on** (or auto-tune) tunes your radio to the hit and **fine-locks using the radio's own audio**. That matters because RTL-SDR crystals are often 15–50 ppm off (a few hundred Hz on HF), too far for Mercury. In the demo the SDR is deliberately 240 Hz off and the fine-lock removes it.
- The SDR must be able to see the segment: with RX OUT it follows your radio (±800 kHz); with IF OUT it sees what your radio's IF passes. A segment wider than about 200 kHz (only a custom one: the default segments all fit) is watched only in its middle 200 kHz; channels past that would wrap round in the SDR's spectrum and show whistles that aren't there.
- If the dongle stops sending samples for 10 s, the wide receiver stops and releases it (turn it off and on in Settings to open it again).

### Heard

The **Heard** card lists recent CQs (from Mercury's CQFRAME decodes), Roundtable calls, and SDR whistle hits, newest first. Buttons: **Answer** (connect), **Join** (a Roundtable), **Lock on** (an SDR hit). Answer and Join first tune the radio to where the station was heard (a search may have moved on since). Mercury reports a frame once it has ended and been decoded, so if the radio moved by **300 Hz or more** in the last **2 s** before that (a search stepping on, a retune by hand or from the app), the entry and the Log keep the frequency the radio was on before the move, where the station actually is. A smaller move, such as a search fine-tuning onto the caller, keeps the new frequency, which is the more exact one, also when a bigger move follows it within those 2 s. None of them is offered during a link or a call, or at a Roundtable.

## Chat

Chat is a 1:1 conversation over Mercury's ARQ, which retransmits until everything arrives intact.

- **Connecting**: answer a CQ, or type a callsign in the box at the top of the Chat tab and press **Connect**. "Calling …" shows while Mercury places the call, with **Cancel the call**; meanwhile Connect, Call CQ, Search, the dial, the radio's mode and the tune carrier wait (Whistlestop refuses them until the call links or is cancelled). A CQ you're calling stops first. Connect is refused while you're already linked or calling, while you're at a Roundtable (Leave first: a 1:1 link and a Roundtable don't mix), and until your own callsign is set (Settings → Station): without one Whistlestop sends nothing at all, and says so in the Log.
- **History**: the Chat tab keeps the newest **500** lines and the Files tab the newest **200** finished transfers (transfers still going are never dropped), so an unattended station running for days stays light. Depot mail is kept in full.
- **The link header** shows the other station's call, name and grid (from their HELLO), the agreed bandwidth, how they hear you ("hears you +7 dB", from their reports), the time since linking, and **Disconnect (73)**.
- **Messages** show **"… sending"** until the other station's app confirms they arrived, then **"✓ delivered"**. If the link ends before that confirmation comes, the message shows **"✗ not confirmed"** (in red): it may or may not have reached the other station, so say it again on the next link if it matters. (The confirmation itself can be what was lost: a weak link sometimes drops the last packets.) There are no read receipts and no typing indicators: they would cost airtime.
- **Time estimate**: under the input, "N bytes on air · ≈ T at R bps" tells you how long a message will take at the current link speed.
- **Length**: one message can be at most **64 KB** (about 20 minutes at 500 bps). Over that, the line under the input says so and **Send** is greyed out; send it as a file instead (Files tab), which can resume if the link drops. (The other station would take a single message over 1 MB for a corrupt stream and hang up.)
- **Chat never waits behind a file.** Files go in 1 KB pieces and at most two pieces wait inside Mercury, so a line you type during a big transfer goes out within a couple of pieces. (Whistlestop counts what it has handed to Mercury itself and takes off only what Mercury reports as sent, so a burst of Mercury's status reports can't make it hand over the rest of the file early.) The same limit is what lets **Cancel** stop a file going out.

**Disconnect (73)** sends a short goodbye, then Mercury delivers anything still queued before hanging up. When the other station says 73, Whistlestop waits up to 30 s for its disconnect (longer on a slow link, where Mercury's disconnect takes longer to cross: the wait follows the slowest speed the link has run at, about 75 s at 38 bps and 150 s at 19 bps, never more than 180 s); if that never arrives (say, it was lost after a QSY), Whistlestop ends the link at once without a goodbye exchange, so Mercury doesn't keep calling a station that has gone.

## Files

The **Files** tab sends and receives files over a 1:1 link.

### Sending

- **+ Files…** picks one or more files; **+ Folder…** picks a folder; or **drop** files and folders anywhere on the window.
- **Zip before sending** (on by default) packs everything into one zip with maximum compression. Folders are always zipped. Two files with the same name (from different folders) are both kept, the second as `name-1.ext`. Text and logs often shrink to a few percent; photos and already-compressed files don't shrink. A single file that zipping wouldn't make smaller is sent as it is (the Log says "Not zipped: … doesn't get smaller"), so it never goes out bigger than it is.
- Each transfer is **offered** first: the other station sees the name, size, zip contents and predicted time, and presses **Accept** or **Decline**; the sender's row says "offered, waiting for an answer" until then, and its Log says when it's accepted ("… accepted name: sending") or declined. Nothing is kept before you accept: pieces a station sends without waiting for your answer are ignored. An empty file (0 bytes) arrives the moment it's accepted.
- Progress shows bytes done, the measured speed, and the time left. **Cancel** stops a transfer on either side. (Each station numbers its own files from a random starting point, so a cancel from either end always names the right one.)
- If the link ends while an offer is still waiting for an answer, it shows **link ended before you answered**. The sender offers it again the next time the two of you link (with resume).
- **A file goes only to the station it was for.** A file sent during a link, or one cut off by a dropped link, is offered again (with resume) only on a link with that same station, never to whoever links next (while you're Away, that could be anyone). A file picked while not linked goes to the next station you link with.

### Receiving

An incoming offer shows the predicted time with **Accept** / **Decline** on the Files tab (its count on the tab), and is also said in the chat and as a pop-up ("📁 … offers …: Accept or Decline it on the Files tab"), since nothing moves on the link until you answer. Received files go to your **download folder** (Settings → Files; default `~/Downloads/Whistlestop`). **Show** reveals a received file in Finder. File names from the air are cleaned so they can't write anywhere else: only the name itself is kept (no folders), control characters are dropped, characters Windows forbids (`< > : " | ? *`) become `_`, a Windows device name such as `CON` or `NUL` gets a `_` in front, so does a name starting with `.` (it would be hidden, and `.partial` is where unfinished downloads are kept), and a name longer than 200 bytes is shortened (keeping its extension). An existing file is never overwritten (`name-1.ext`, …).

### Integrity and resume

- Every file carries a **SHA-256** checksum. The receiver checks it and reports back: "✓ delivered, checksum good" on the sender, "✓ received" on the receiver.
- If the link drops, the partial file is kept (under `.partial` in the download folder, named by its checksum). When the same file is offered again, the receiver answers with how much it already has, and the transfer **resumes** from there.

### Estimates

**Estimate…** picks any file and shows a table of predicted times for every bandwidth at −5, 0, +5, +10 and +20 dB SNR, plus "now" for the current SNR. During a link, estimates use the speed Mercury reports and, after a few seconds of sending, the speed actually measured.

## Roundtables

A Roundtable is a group conversation on one frequency. There is no special broadcast channel: a table lives wherever its host called it.

### Hosting and joining

- **Host**: Call CQ → **Roundtable**, name the table, **Call CQ**. Your whistle and CQ invite others; Whistlestop sends a CALL packet with the table's name. The CQ keeps cycling to invite more stations, but never on top of the table: table messages wait while your whistle is on the air, and the next whistle waits until table traffic has gone out. Once the table speeds up beyond DATAC3 the CQ ends by itself (the Log says so), because stations still listening for calls at DATAC3 couldn't copy its CALL packets.
- **Join**: when a Roundtable CALL is heard, it appears under **Heard** with **Join**. The host's CQ row shows **Join** too (not Answer): answering a Roundtable's CQ means joining the table, as auto-answer does, not a 1:1 call that would take the host away from it. While the pointer is over the **Heard** list its rows stay where they are, so a station heard again can't move a row under your click; new ones are added at the bottom, and the newest-first order returns when the pointer leaves. Or type the name in the Roundtable tab and press **Join here** (you must be on the host's frequency). Both are off during a 1:1 link: a Roundtable switches Mercury to broadcast modes, which would cut the link. Disconnect first.
- After you **Leave**, Mercury goes back to listening in DATAC3, the calling mode, once your goodbye has gone out (a table that sped up would otherwise leave you deaf to calls). If you call someone straight after leaving, the switch waits until Mercury is free again (after that link, or the call is given up), and is tried every 2 s until Mercury takes it.
- **Calling CQ again for your own table** (after its CQ ended) keeps the table as it is: who's there and what was said.
- **While you're at a table**, Whistlestop won't take you away from it: **Connect**, **Answer**, **Search** and **Call CQ** for a 1:1 chat (or for another table) are greyed out (on the desktop, hovering says why), and so is joining another table; the station refuses them with "you're at the Roundtable …: leave it first" if asked another way. The automation waits too: the schedule doesn't retune the radio, no Station ID goes out, and the Depot doesn't connect by itself to deliver or collect mail. If another station connects to you anyway (Mercury answers calls by itself), your table messages, resend requests and the host's speed changes wait until that link ends.
- **Joining right after a link**: Mercury can't switch to Roundtable listening while a 1:1 link is still closing (its disconnect takes several seconds, more if the other station is slow to answer). If you press **Join here** then, the tab shows "⏳ Joining … as soon as Mercury is free" and Whistlestop tries again every **2 s** for up to **60 s**; **Cancel** stops waiting, and so does calling a station or calling CQ (the Log says "Roundtable join … cancelled"). It doesn't retry while you're calling someone. While a link is up and you haven't hung up, joining is refused: end the link first.
- **Leave** says goodbye to the table.

### Talking

Type in the Roundtable tab and press Enter. Messages go out over Mercury's **broadcast** plane: one-way frames that everyone on the frequency decodes. A long message is split across several frames (up to 64) and put back together at each station. Under the input you see **exactly** how long it will take: broadcast frames have a fixed length, so "3 frames · exactly 9.6 s on air" is precise.

### Reliability and speed

Broadcast frames are not acknowledged, so Whistlestop adds:

- **Listening first**: with **Listen before transmitting** on (Settings, on by default), your table messages, reports and other table frames wait while someone else is transmitting on the frequency, as a CQ does ("Roundtable message waits: someone is transmitting on this frequency" in the Log), and go as soon as it's clear, or after **60 s** of waiting whatever the detector says. A message sent on top of someone (the host's CQ whistle for the table, say) is lost whole, and if it was your only one nobody knows to ask for it again.
- **Asking again**: if a station is missing parts of a message, it asks for just those parts every 20 s, and the sender repeats them. It waits until nothing more of that message has come for two frames' time (the sender may still be sending the rest, and the request would go out on top of it), and asks only into a quiet table: nothing heard for two frames' time, and nothing of its own waiting to go out. A part asked for again while it's still waiting to go out is sent once. A message missed whole (a short one-frame line, say) shows up as a gap in the sender's message numbers, either at its next message or at the host's 90 s heartbeat, which carries the host's newest number. Missed messages are asked for whole: up to 16 per sender. A station sends one of these requests at a time, at most every 20 s: the sender answers at once, and a second request straight after would go out on top of the answer. They take turns (the one asked longest ago goes next), so one that can't be answered doesn't hold up the others; and a station that leaves the table isn't asked for anything more. A message already complete isn't shown twice when its parts are repeated for someone else, and a message still incomplete after 10 minutes is given up.
- **Speed ramp-up**: each member reports how well it hears the host (measured on the host's own frames, or on its CALL when joining; "hasn't measured the host yet" until then, which doesn't count; a join made before the CALL's SNR came in sends a second report as soon as it does) and its bandwidth setting, when it joins and again every **2 minutes**; a member silent for 7 minutes (three missed reports) no longer holds the table back, and one not heard at all for **15 minutes** is taken off the table's list ("… not heard for 15 minutes; taken off the table"). The host picks the fastest mode that the **weakest** member can copy, with **3 dB** of margin, within the **narrowest bandwidth at the table** (the host's own included). It announces the change twice in the current mode and switches once Mercury reports both announcements off the air (and any table frame that was ahead of them), or at the latest three frames' time later; then everyone switches together. A member whose host runs an older version that moves the table wider than the member's bandwidth gets a warning in the chat ("… wider than your bandwidth setting …: you may not copy it"). It slows down at once if someone reports worse, and speeds up only after 2 minutes without a downgrade (doubled, up to 32 minutes, each time a speed-up is undone within 5 minutes: Mercury reads the same signal a few dB better in the slow modes, so reports swing with the speed; back to 2 minutes once a speed-up has held for 10), and not while a member is sending (a station transmitting can't hear the announcement; the host looks again every 30 s). No speed change, and no join window, starts while the host's own CQ whistle or CQ frame is on the air: the announcements wait a few seconds for it. A speed change waiting to be made when a 1:1 link comes in waits for the link to end; leaving the table drops it. Each frame is handed to Mercury only once it has finished sending the last one, so none are left waiting in Mercury when the speed changes. Turn this off in Settings ("Roundtable speed ramp-up").
- **The slowest speeds carry little per frame**: one broadcast frame holds 27 bytes in DATAC15 and 51 in DATAC4 (123 in DATAC3), and Mercury cuts anything longer, so Whistlestop makes everything fit:
  - Your messages are split into as many frames as they need, at any speed. In DATAC15 a long message (over about 150 to 450 characters, depending on the callsigns) goes out as **several messages**, so that any part of each can still be asked for again; they show as consecutive lines, cut between words. A station's messages are always shown in the order it sent them: one that's complete waits (for up to 2 minutes) while an earlier one from the same station is still being filled in.
  - A member's 2-minute report leaves the table's name out when it wouldn't fit (the host knows its table by number), and a request to repeat parts asks for the first few missing parts, then the rest 20 s later.
  - A message made for a faster speed (still waiting to go out, or repeated for someone after the table slowed down) is split again for the new speed and sent again whole; stations holding parts of it start over (stations on versions before 2026-11-13 don't, and miss that message). If it would need more parts than a message may have at the new speed, the rest goes as further messages, as when you type a long one there.
  - A callsign of **8 or more characters** (KD2ABC/P) leaves no room for text in a DATAC15 frame, so while such a station is at the table (heard in the last 7 minutes) the host skips DATAC15 and uses the next speed down (MFSK), which copies weaker signals anyway. DATAC4 and faster fit any callsign.
- **Join windows**: a newcomer always calls and listens at DATAC3, so once a table has sped up it couldn't hear them. Every **3 minutes** a sped-up table drops to DATAC3 for **20 s**: the host announces it, everyone switches, the host sends the table's CALL (so stations nearby see it under **Heard** with **Join**), and anyone joining then is heard. Then the table goes back to its speed, or slower if a newcomer hears the host less well, and never wider than the narrowest bandwidth at the table, a newcomer's included. You can keep talking during the window, just more slowly. The first window comes 3 minutes after the table first speeds up. A newcomer whose JOIN went out while the table was fast sends it again when it hears the window's CALL.
- **Staying in step**: broadcasts aren't acknowledged, so a member can miss a speed change. The member most likely to miss one is the weak member the host is slowing down for, because the announcement goes out at the faster speed. So:
  - A member whose own report says it can't copy the current speed, and which hears no slow-down within **40 s**, moves to DATAC3 by itself.
  - A host with nothing to say sends a short heartbeat (its current speed) every **90 s**. The heartbeat, and each member's 2-minute report, wait for a gap in the table's traffic (nothing heard for two frames' time) so as not to go out on top of someone's message, but by at most a minute.
  - A member that hears nothing of the table for **4 minutes** (counted from the end of its own talking, since while it sends a long message it can't hear the table; but its talking counts for at most 4 minutes past the last frame it actually heard, so typing into a silent table doesn't put this off for ever) tries, in turn: the speed last announced, DATAC3 (where the join windows are), then the most robust mode. It reports in each time, so the host hears it again.
  - A 1:1 link that comes in while you're at a table puts the table on hold: these clocks start again when the link ends, so the silence during it isn't taken for a lost table.
- **When the host leaves**, the table is closed: its members hear the host's goodbye, leave too (quietly: nobody is left to say goodbye to, and all of them keying up at once would only collide), and the Chat says "The host … closed the Roundtable". A member that joined by name doesn't know who hosts until the host's first speed announcement or heartbeat (up to about 2½ minutes); if a station leaves before then and no host is heard for the next **3 minutes**, the member takes the table as closed and leaves too. A member that hears nothing at all of its table for **15 minutes** (all the speeds above tried meanwhile) takes it as closed the same way and leaves, so the schedule, Station ID and Depot automation, which wait while you're at a table, carry on.
- **At the table** lists you and every station heard at the table, with how well each hears the host. A member also sees the host (marked **(host)**) as soon as it hears the host's speed announcements, before the host has said anything.
- **The floor**: click a member's name to pass them the floor (the "talking stick"). It's a courtesy, not a lock. For up to **1 minute** after the floor goes to someone else, or until they're heard talking, the other stations hold their routine 2-minute reports, so a report doesn't go out on top of the message everyone is waiting for (a station that can't hear the one given the floor wouldn't otherwise know it's on the air).

> Roundtables are always plain text, even with encryption turned on.

## The Depot

The **🏤 Depot** tab is Whistlestop's mailbox: messages that wait for a station instead of needing you both on the air at once. It has four boxes:

- **📬 Arrivals**: mail for you. Stations that connect while you're **Away** leave it here, and stations carrying mail for you hand it over when you link. New mail is marked **NEW** and counted on the tab; **Mark read**, **Reply** (fills in the To box) and **Delete** are on each message.
- **📮 Departures**: mail you've written that hasn't been handed over yet. **Connect to …** calls the station (or the relay) now.
- **🚚 In Transit**: mail you're carrying *for other people* (store and forward). This box only fills if **Carry mail for other stations** is on (see "Store and forward (relaying)").
- **✓ Delivered**: mail the next station confirmed, with the time (UTC) and who took it.

**Writing mail**: type a callsign in **To**, your message, and press **📮 Post**. It waits in Departures. The next time you're linked with that station, for any reason, Whistlestop hands it over at the start of the link, and it moves to Delivered when the other station's Whistlestop confirms it (a MAILACK). Nothing is "seen" or "read" on the other side: delivered means it's in their Arrivals. A message can be at most **64 KB**, like a chat message; a longer one isn't posted (it says so under the box, and your text is kept).

**Via**: to reach a station through someone else, put the relay's callsign in **Via**. The mail is handed to the relay when you next link with them, and they carry it on (if they have relaying turned on). Leave Via empty to wait for the station itself.

The Depot is saved in `depot.json` beside the settings, so mail survives restarts. Repeat deliveries are ignored, so a link that drops mid-handover is harmless: unconfirmed mail is simply handed over again next time. If `depot.json` is ever damaged (cut short by a crash, or edited by hand) it is never overwritten: Whistlestop keeps it as `depot.json.bad` (`depot.json.bad-2` and so on if that name is taken), starts a new Depot and says so in the Log, so the old mail can still be recovered from that file. If it can't be renamed (a sync or virus checker holding it, say), it is left as it is and this session's mail is kept in memory only, as below. If it is there but can't be opened (its permissions changed, say), Whistlestop leaves it alone, says so in the Log, and keeps this session's mail in memory only until you fix it and restart. The **To** and **Via** fields take callsigns only (1–12 letters, digits, `/` or `-`).

## Away

**🙋 Here / 🚪 Away** in the top bar says whether you're at the radio. When **Away**:

- Stations that connect get your **away message** (Settings → Automation), with your callsign in front.
- Anything they type in chat is saved to your **Arrivals**, so they can leave you a message just by connecting and typing. Another away station's automatic away message is not: it's a reply, not mail.
- **Accept files by myself, each up to … MB** (Settings → Automation → Away; on the phones, Settings → While away; **off** by default): a file a station offers while you're away is accepted as if you had pressed Accept, when it's no bigger than the limit (default **5 MB**, 0.1–200 MB in the desktop Settings). A bigger one waits for you; the Log says which ("🤖 Away: accepting … by itself", or "… is over the … limit … it waits for you"). Files are saved in your download folder like any other, with the same name cleaning, and never opened. With the switch off, offers wait for you while you're away, and those still unanswered when the link ends are offered again on the next one.
- A link with nothing happening for the **Hang up quiet links after** time (default 180 s) is ended with a polite 73, so the next station can get through. Real traffic counts as activity, file pieces included, so a long transfer is never cut off (the signal reports both stations send every 30 s don't count).
- Automatic delivery and collection (if you turned them on) are allowed. They never happen while you're **Here**.

Away is a convenience, not a licence to leave a station running unattended where your rules don't allow it (see "Legal and good practice").

## Station ID beacon

The **Station ID beacon** (Settings → Automation, off by default) sends one short broadcast frame every N minutes (10 to 120, default 30) when the band is quiet and you're not at a Roundtable (it would go out at the table's speed, in the middle of its traffic): your callsign, grid, whether you're away, and the callsigns you hold mail for (up to what fits in one 120-byte frame). It has **no whistle**: it's an ID, not a call, so it doesn't stop scanning stations.

Stations on your frequency in the calling mode decode it. If your beacon lists their callsign, their Whistlestop shows **📬 … has mail for you** in the Heard card (and collects it, if they're away with automatic collection on). **Send one now** sends one immediately. A beacon (like a CQ and the automatic calls) waits while someone else is transmitting on the frequency (see "Listen before transmitting"), and is never sent during a link, a CQ, a search or a Roundtable, and (with the segment guard on) never outside the automatic-control segments: you'll see "Station ID skipped" in the Log instead.

## Automatic mail handling

Whistlestop notices stations it holds mail for whenever it hears them: a CQ, a Station ID, a Roundtable frame or a mail notice. The **Heard** card then shows **📮 N message(s) for CALL** with **Deliver now**; when someone says they hold mail for you it shows **📬 CALL has mail for you** with **Collect**. Those buttons are always there. Three switches in Settings → Automation (all **off** by default) let Whistlestop do more by itself:

| Switch | What happens | When |
|---|---|---|
| **Tell stations I hear that I have mail for them** | Sends a short NOTICE frame: "N messages waiting here, connect to me" | Any time the band is quiet, at most once per station every 15 minutes |
| **While away: connect and deliver** | Connects to the station, hands over the mail, waits for the confirmations and hangs up (after **60 s** with nothing happening if the other station never confirms, say because it doesn't carry mail for others) | Only while Away, the band is quiet and you're not calling or searching; once per station every 15 minutes |
| **While away: connect and collect** | Connects to a station that says it holds mail for you; it hands the mail over, and the link ends after 10 s of quiet once the mail has come and Mercury has sent your confirmation (on a slow link that can take half a minute more), or after 60 s of quiet if none does (mail can take 20–30 s to cross on HF) | Same conditions |

Every automatic transmission is written to the Log (🤖 and 📣 lines), so you can see what your station did while you were gone. With **Automatic only in the segments** on, an automatic delivery or collection needs your whole signal (300–2700 Hz above the dial) inside an automatic-control segment, and a known dial (with no radio control, type the frequency in); when it isn't, nothing is sent and the Log says so once per station every 15 minutes ("🤖 Away: not collecting your mail from …: a signal from 7103.000 kHz reaches 7105.700 kHz, outside the automatic-control segments"). Tune lower in the segment, or connect by hand.

## Store and forward (relaying)

With **Carry mail for other stations** on (off by default), your Depot accepts mail addressed to someone else: a station that links with you can hand you mail for a third station, and you carry it in **In Transit** until you link with (or, while away, hear) that station. It's then delivered like your own mail and moves to Delivered with the callsign that took it. With relaying off, such mail is declined (logged) and stays with the sender.

Loop guards: each message carries a hop count, and at most **3** relay stations may carry a message on its way (sender → relay → relay → relay → the station it's for); a message you've already carried or delivered is never taken twice. Your Station ID and notices include the calls you're carrying mail for, so their owners can come and collect it.

## QSY (moving a link)

Once linked, either station can ask to **move the link** to another frequency, to get off a busy channel or to make room for a long transfer. On the link card, type a frequency in **↔ Move this link to** (kHz) and press **Ask**.

- The other station decides by **its own rules** (below) and answers.
- If it agrees, both stations retune a moment later. The answer has crossed by then, and Mercury's ARQ resends anything lost in the gap. The link carries on at the new frequency. A station doesn't retune while its own radio is still transmitting (some radios, an FTDX3000 for one, ignore a new frequency while keyed), for up to **15 s**. Each station then reads its dial back: many radios don't answer a frequency command, so one lost on the CAT line would leave the radio where it was. If the dial isn't on the new frequency, Whistlestop tunes again, up to three times ("⚠ The radio is on …, not …: tuning again"), and if it still won't take, the Log says "⚠ QSY to … failed" instead of claiming the move. The same check is made when the link ends and the station goes back to its home frequency.
- The link card shows "on … with …, back to … when the link ends". When the link ends, for any reason, **both stations go back** to where they were.
- If it refuses, you see why ("I only QSY within 20m", "outside my digital segments"…). If it doesn't answer within **90 s**, the request is dropped ("No answer to the QSY request…") and you can ask again.
- The box takes kHz (14106.5) or MHz (14.1065), like the Tune box.
- Whatever the rules, the **whole signal** must fit inside an amateur band: USB puts it 300–2700 Hz above the dial, so a dial within 2.7 kHz of a band's top edge is refused. While you're **Away** with **Automatic only in the automatic-control segments** on, a QSY must also stay inside those segments.

**Your rules** (Settings → QSY requests, **off by default**):

| Setting | What it does | Default |
|---|---|---|
| Accept QSY requests | Agree to requests that meet the rules below | off |
| Only within the band I'm on | Never leave the current band (where you know the antenna works) | on |
| Bands | With the above off: which bands a QSY may go to | 40, 30, 20 m |
| At most … kHz | How far from where you were a move within the same band may go (0 = anywhere in the band). A move to another band you allow above isn't limited by it | 50 kHz |
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
- **Largest file** (default 5 MB, 0.1–200 MB): bigger files are listed but refused. At a few hundred bytes a second, a big file ties up the frequency for a long time.
- At most **3** library files go to one station at a time; asking for more gets "busy: … ask again when one has arrived".
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
- Whistlestop checks every 20 seconds, and moves only when you're not linked, calling, searching, or at (or joining) a Roundtable. Each move is noted in the Log (🗓).
- Station ID follows the same segment rule as all automatic transmissions.

## Remote operation

Run Whistlestop on the computer wired to the radio (the **radio computer**), and operate it from another Whistlestop anywhere on your home network: the den, a laptop on the couch.

**On the radio computer:** press **🛰 Remote**, then under **Let other computers operate this one**:
- tick **Share this station on my LAN** (off by default);
- set a **Password** of at least 4 characters;
- the **Port** defaults to 7373.

The window then shows the address to connect to, for example `192.168.1.20:7373`, and who is connected. The **🛰 Remote** button's text turns green while this station is shared, and the button turns amber while someone is operating it (teal on the other computer, while it operates this one); hovering says who. Its label stays the same, so nothing else in the top bar moves.

**On the other computer:** press **🛰 Remote**, type the radio computer's address (its LAN address, or a name such as `shack.local`; add `:port` if it isn't 7373) and the password, and press **Connect to the radio computer**. The button turns teal (**🛰 Remote ●**), and from then on everything happens on the radio computer:
- search, CQ, chat, Roundtables, the Depot, QSY, the library, Radio Controls and the waterfalls;
- **Settings**: you edit how the radio computer operates (callsign, CQ, search, bandwidth, automation, QSY, the schedule, the SDR…), and it saves them. Changing what's settled on that computer itself is ignored, with a note saying so: its folders (Download, file library, Depot file), Mercury (program and address), the radio connection (rigctld, managed rig, CAT), Demo mode, its speakers and its remote-sharing settings. Sound devices can be picked only from those it lists. Only **Save & connect** restarts its station; other changes (Answer automatically, Call CQ) apply without dropping the radio or a link.
- **🔊 Listen**: plays the radio's received audio on this computer's speakers (Settings → Audio → Listen on and Listen volume are this computer's). It's 8 kHz audio, about 16 KB/s, nothing for a home network.
- **Files**: files you send are picked here and uploaded, up to **25 MB** each; only files uploaded this way can be sent from the radio computer. Files you receive are saved on the radio computer and also copied to this computer's Download folder (only files in its own Download folder are ever copied back). Each is copied once: a list of what's been copied (`.whistlestop-copied` in this computer's Download folder) stops a reconnect from copying everything again.

**Disconnect** (in the Remote window) goes back to this computer's own station; the radio computer carries on as it was. If the connection drops, this computer goes back to its own station and says so. Turning sharing off or changing the password on the radio computer disconnects anyone operating at once. Run the same Whistlestop version on both computers. If they differ, **Connect** still works but says which versions differ: requests or settings one side doesn't know are skipped with a note.

**Security.** Only computers with private (home network) addresses can connect: 192.168.x.x, 10.x.x.x, 172.16–31.x.x, link-local and IPv6 unique-local. The password is never sent: the radio computer sends a fresh challenge and the other computer answers with an HMAC-SHA256 of it. A wrong password is refused, slowly, and after **10 wrong passwords within a minute** from one computer, that computer's logins are refused for **5 minutes**, the right password included; the radio computer's Log says so and names it. Other computers (yours) can still log in, so a stranger guessing can't lock you out. A backstop covers someone changing address to get round that: after **30 wrong passwords within 10 minutes** from all computers together, logins from computers that haven't logged in since Whistlestop started are refused for **5 minutes**; one that has logged in (yours) still can. At most **8** connections are accepted at once, at most **2** of them logins still in progress from one computer, and a login must finish within **15 s**. An operator whose computer stops answering (an app that hung, Wi-Fi gone) is dropped once the radio computer can't send to it for **30 s**. While **Connect** is reaching the radio computer the window stays usable and shows **Connecting…** (a wrong address gives up within about 15 s). The stream itself isn't encrypted, so **don't forward the port to the internet**; use a VPN to reach home from outside. Anyone who knows the password can transmit with your station, under your callsign: choose it accordingly. On macOS, allow incoming connections for Whistlestop when the firewall asks.

### With no window (headless)

A radio computer doesn't need a screen. On a Raspberry Pi, a small server or any computer beside the radio, run:

```
whistlestop serve
```

It runs the same station the app does, with the same settings file (see "Settings reference"), but with no window, and shares it on the LAN as above. You operate it from Whistlestop on another computer with **🛰 Remote**, exactly as above. Everything else in this section applies, the security rules included.

- **Sharing is always on** in this mode, since it's the only way in, whatever **Share this station on my LAN** says. It needs a password of at least 4 characters. If the settings have none, it stops and says how to set one: `whistlestop serve --set-password` asks for one twice (it shows as you type), saves it in the settings, then starts. Or give it in the environment as `WHISTLESTOP_REMOTE_PASSWORD`, which is used for that run only and never saved.
- **Port**: the settings' remote port (7373 by default), or `--port N`.
- **When it starts**, it prints the address to connect to (for example `192.168.1.20:7373`), where the settings and the Log file are, and how many settings couldn't be read, if any.
- **The Log** is printed as it comes, each line with its date and time (UTC), for a terminal or the system log (journald). It's also written to `whistlestop.log` beside the settings, as in the app (see "The Log tab"). `--quiet` stops the printing but not the file.
- **Where the program is**: on Linux, `whistlestop` from the download (or your own build); on Windows, `whistlestop.exe serve` in a Command Prompt in its folder; on macOS, `/Applications/Whistlestop.app/Contents/MacOS/whistlestop serve` in Terminal.
- **Stopping**: Ctrl-C, SIGTERM (what `systemctl stop` sends) or SIGHUP (the terminal closing) stops the station the way quitting the app does: remote operators are disconnected, then it waits up to **3 s** for the radio to be unkeyed.
- **Settings** a remote operator changes are saved on this computer, as with the app. `serve` never writes the settings file itself, except a password you set with `--set-password`.

**Setting up the radio connection.** A remote operator can't change what belongs to the radio computer: the radio connection, Mercury, Demo mode, folders, and sound devices other than those it lists. So set those up on that computer before you rely on `serve`. You can run the app there once (on a Pi, its desktop or a screen plugged in), or edit `whistlestop.toml` by hand. `whistlestop devices` lists the sound devices by the names the settings use. With no settings file, the defaults are used, and those are **Demo mode**: the station runs, but on no radio.

**Running it at boot (Linux).** A systemd service keeps it running and restarts it if it stops. For example, `/etc/systemd/system/whistlestop.service`:

```
[Unit]
Description=Whistlestop (no window)
After=network-online.target sound.target
Wants=network-online.target

[Service]
User=pi
ExecStart=/usr/local/bin/whistlestop serve
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Then run `sudo systemctl enable --now whistlestop`, and use `journalctl -u whistlestop -f` to watch its Log. Run it as the user whose settings it should use (here `pi`), and add that user to the `dialout` and `audio` groups so it can reach the radio's USB serial port and sound card.

Mercury is its own program here too, started with the same command as with the app (see "3. Start Mercury"). With direct CAT, it must start **after** `whistlestop serve`. A second service does that: the same file named `mercury.service`, with `ExecStart=` set to your Mercury command (full paths) and `After=whistlestop.service` added under `[Unit]`.

**Raspberry Pi.** Use a Pi 4 or Pi 5 with 64-bit Raspberry Pi OS (Lite, with no desktop, is enough). There are no ready-made ARM downloads yet, so build it on the Pi itself:

1. The libraries the Linux build needs, even with no screen (it's the same program as the app): `sudo apt install git pkg-config libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev`
2. Rust, from rustup.rs: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
3. The source and the build: `git clone https://github.com/NR5P/whistlestop && cd whistlestop && cargo build --release`. The first build takes a while on a Pi.
4. Install it where the service above expects it: `sudo install target/release/whistlestop /usr/local/bin/`
5. Set the password (`whistlestop serve --set-password`), set up the radio connection (above), then enable the service.

To update later, run `git pull && cargo build --release` in that folder, install it again (step 4), then `sudo systemctl restart whistlestop`. Run the same Whistlestop version on the Pi and on the computers that operate it. Mercury also has to be built for the Pi from Rhizomatica's source (its releases have no ARM download either). Check Mercury's CPU use at the speeds you want, on a dummy load, before you rely on the Pi.

## Phone apps (iPhone and Android)

Whistlestop also comes as native phone apps: SwiftUI on the iPhone and iPad (iOS 17 or newer), Jetpack Compose on Android (8.0 or newer). The screens are the phone's own, but underneath they run **the same Whistlestop engine** as the desktop, so a phone finds, calls and talks to a desktop station exactly as two desktops do: the same whistle, search, CQ, link, chat and Depot.

**Mercury is built into the apps.** The phone runs Rhizomatica's Mercury modem itself (version 1.9.23, unchanged, through Mercury's own embedding library), so there is nothing else to install. It starts with the app when the station isn't in Demo mode and its modem is this phone (Modem host `127.0.0.1`, the default), listening on the usual ports (8300/8301 and 8100). Point the Modem host at another computer to use a Mercury there instead. Mercury is free software under the GPL-3.0, and so are the phone apps.

**The screen.** The header always shows your callsign, the dial frequency and mode, the **Modem**, **Radio** and **Audio** lamps (green when ready), **TX** while transmitting, the **Here/Away** button and the Help button. Below it, five tabs:
- **Operate**: the waterfall (0–3 kHz, the whistle band shaded), the dial (type kHz and tap **Tune**), **Search** (current band, one band, or all bands; **Answer automatically**), the last **Whistle** heard, **Heard** (tap **Answer** to tune and connect, or **Join** on a Roundtable's call to tune and join it), **Call CQ** (choose **Chat (1:1)** or **Roundtable**; a Roundtable needs a name, and the button becomes **Whistle and host the table**) and **Connect** by callsign. While a call is going out, the Connect card shows **Calling …** with **Cancel**, and Connect, Answer, Join, Tune and Search wait for it (as they do during a link). The band menu lists the bands you have digital segments for. kHz fields take a comma or a point as the decimal. **Answer automatically** is saved the moment you switch it, on its own: edits waiting in Settings still wait for Save. While you're linked, a **Linked** card shows the station, mode, speed and time, the encryption state when encryption is on, with **Disconnect** and a **Move to kHz / Ask** row (QSY).
- **Chat**, in two halves chosen at the top:
  - **1:1 chat**: keyboard-to-keyboard over the link. The app switches here when a link comes up. A tick (✓) means the other station confirmed the line. With encryption on, a line under the header shows **Setting up encryption…**, then **🔒 Encrypted · code 264 097** (read the code to the other operator; see Encryption (optional)) or **🔓 Not encrypted** and why.
  - **Roundtable** (marked ● while you're at one): the same Roundtable as the desktop's (see Roundtables). Not at a table, it has **Join here** with a name (tune to the host's frequency first; off during a 1:1 link) and, while a closing link holds Mercury, "Joining … as soon as Mercury is free" with **Cancel** (retried every 2 s for up to 60 s). At a table: its name, whether you host, its mode, who holds the floor, and everyone at the table with how well they hear the host (tap one to pass them the floor), then the conversation, a box to say something to the table, and how much air time is left while your frames go out. **Leave** says goodbye. The app switches here when you join or host one.
- **Depot**: Arrivals, Departures, In Transit and Delivered. Tap a message to read it (that clears NEW), reply, connect now, or delete it. The pencil button writes new mail, with an optional **Via** relay.
- **Log**: what the engine did, newest first, in UTC.
- **Settings**: Help mode, your station (callsign, name, grid), Demo, the **Radio** (USB cable, serial bridge, rigctld or none on Android; the CAT bridge on the iPhone), the modem's host and port, **While away** (accept files by yourself while away, each up to a size you set; off by default), and **Encryption** (Off, Prefer or Require; Off by default; leaving Off asks you to confirm the legal warning, and the choice applies from the next link). **Save** applies them. A change to the callsign, Demo, the modem's host or port, the radio connection or the audio restarts the station (which ends a link in progress); other changes (name, grid, Answer automatically) apply without a restart. The built-in Mercury starts with the settings in force; it can't be restarted inside the app, so after a change to its port or to how it keys the radio, the app asks you to close and reopen it. Everything else (search timing, QSY rules, the library, the schedule…) keeps the values in the phone's settings file, `whistlestop.toml` in the app's own storage, the same format as the desktop's.

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
  - **None (VOX)**: no CAT. Whistlestop can't read or tune the dial, so search and QSY need CAT; with VOX keying, calling and answering on the frequency you set by hand still work. Until you type the radio's frequency into the dial, Whistlestop doesn't know it: the Log says "on the radio's frequency (not known here: no radio control)" instead of a number, and with **Automatic only in the segments** on, nothing automatic (Station ID, mail notices, automatic delivery and collection) is sent, since it can't be checked against the segments; the Log says so. Type the frequency in once (and again whenever you retune by hand).
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

On a 1:1 link Mercury starts each direction on the step that the SNR measured at connect supports, then climbs or descends its ladder by itself from what actually gets through, separately in each direction. You don't control the step; you control the limit. It climbs on data: a direction that only carries a short chat line now and then can stay on its slowest step however good the signal (on the air, 2026-11-15, a +8 dB link sent its chat lines in MFSK at 63 bps), and the same link climbed to DATAC17 within 30 s once a file was going.

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

**Slow links.** Setting up encryption takes a few hundred bytes each way, so over a weak link in Mercury's slowest modes it can take a couple of minutes (on a −4 dB test link, over two). Whistlestop waits as long as the set-up keeps moving: it gives up only when nothing of it has arrived from the other station for **120 s**.

If encryption fails during a link (a check fails, or the set-up never finishes), the link's header changes at once to **🔓 Not encrypted: encryption failed (…); disconnecting**, and Whistlestop hangs up. The other station is told why in a plain goodbye; its Log shows that reason as "the other station ended the link (unencrypted message, unverified): …", since a plain message can't be checked.

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

**🎛 Radio controls** opens a separate, movable window with everything your radio offers through Hamlib. Close it when you're done, so the main screen stays uncluttered. Whistlestop asks the radio for its capabilities when it connects and **shows only the controls your radio reports**, so there are no buttons that do nothing. Values are read from the radio about every 1.5 s while the window is open. During a link or a call, the controls that move what the radio hears or sends on (VFO A/B, swap, split, RIT/XIT) are refused ("not while linked…"): the link would be lost.

- **Transmit meters**: **SWR**, **ALC** and **Power out**, read while transmitting, plus the SWR of the last transmission. See "Antenna tuner and SWR".
- **Transmit level**: who sets Mercury's transmit level (**Mercury's own**, **Set here**, **Auto (from ALC)**), the level, and the level Mercury confirmed. See "Transmit level".
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
| **Kenwood** | TS-590S/SG, TS-890S, TS-480, TS-2000 (the TS-990S only through Hamlib: see below); **QRP Labs** QMX/QDX, **truSDX**, **FlexRadio** CAT ports (they speak Kenwood) |
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
- The port is opened with **RTS and DTR held low**, because on many radios those lines key the transmitter. One exception: a **Yaesu**'s own CAT port (the first of the two ports its USB cable shows, recognised by its USB interface number) is opened with **RTS high** and 2 stop bits, because with the radio's **CAT RTS** menu on (often the factory setting) it doesn't answer otherwise. The other port of the pair, which keys the transmitter on RTS, is never used for CAT; a port Whistlestop can't place (chosen by hand on a platform that doesn't report USB interfaces) keeps RTS low, so with it set the radio's CAT RTS to DISABLE.

**Sharing with Mercury.** Only one program can open the serial port, so Whistlestop serves the rigctld protocol on **Share with Mercury on port** (default 4532). Set Mercury to **Hamlib model 2 (NET rigctl)** at `127.0.0.1:4532` and it keys the radio through Whistlestop. Loggers and other programs that speak rigctld can use it too. Turn sharing off only if Mercury keys the radio another way (VOX, or its own PTT port). **The port stays open when you save Settings**: Hamlib's NET rigctl connects once, when Mercury starts, and never again, so a port that closed and reopened left Mercury unable to key the radio until it was restarted. Now Mercury stays connected while Whistlestop restarts the radio behind it; for the moment in between, its commands get an error (Mercury can't key the radio then) and the same connection works again as soon as the radio is back. The Log says "Radio shared at 127.0.0.1:4532 again". The port closes if you turn sharing off, change its number, or choose another way to reach the radio. If a program keys the radio through this port and then disconnects or crashes without unkeying, Whistlestop unkeys the radio for it. Quitting Whistlestop also waits (up to 3 s) until the radio is unkeyed.

**Never left transmitting.** Whistlestop keeps the radio from staying keyed when something goes wrong:
- If the radio's sound output stops taking the whistle (unplugged, or its stream failed), the whistle gives up 3 s after its own length: the radio is unkeyed, the CQ stops, and the Log says "⚠ The whistle didn't finish playing …".
- Over direct CAT, an unkey is checked: Kenwood, Elecraft and Yaesu radios don't acknowledge commands, so Whistlestop reads the TX state back, sends the unkey again (up to 3 times) if the radio is still transmitting, and reports a failure if it won't stop.
- If an unkey fails (CAT upset by RF, rigctld not answering), Whistlestop keeps trying on every radio poll (0.7 s) until it goes through ("⚠ Unkeying the radio failed …; trying again", then "Radio unkeyed").
- The TX lamp shows what the radio reports, or what Whistlestop or Mercury keyed, so a transmission made another way (your mic, the antenna tuner) clears when it ends.
- On the shared port, a PTT command needs its state as a number (`T 1`, `T 0`, or `T VFOA 0` in rigctld's VFO mode); anything else is refused, never taken as key up.
- When Whistlestop stops (or restarts the station for a Settings change), it takes the radio off the shared port first (so Mercury can't key the radio after that), stops Mercury listening, then unkeys the radio, trying up to 3 times.
- With rigctld, a reply that comes too late, or a dropped connection, makes Whistlestop open a fresh connection for the next command, so a late answer is never taken for the next command's; it also reconnects by itself to a rigctld that restarted. The same radio error is logged once (again if it changes, or after a minute), and "Radio: back" when it clears.

**FT-817 family PTT.** Whistlestop checks that the radio did key or unkey, from its acknowledgement or, failing that, its TX status; if it can't confirm, it reports an error instead of assuming the radio is receiving. The PTT readout comes from the radio's own TX status.

**Kenwood TS-890S.** Mode is read and set with its own `OM` command (main band), including its data modes (USB-D, LSB-D, FM-D). **TS-990S**: it has no `IF` command and keys differently, so direct CAT refuses it with a note; use Hamlib (Settings → Radio → Managed rigctld).

**QRP Labs QMX.** A QMX says it is a TS-480; Whistlestop asks for its firmware version (`VN;`) and, when that names a QMX, treats it as one (the Log shows "QRP Labs QMX (firmware 1_02_005)"). Checked against a real QMX on firmware 1_02_005:

- **Use USB mode, not DIGI.** The QMX's DIGI mode sends one tone at a time (it measures the audio's pitch and makes that frequency), so it can send the whistle but not Mercury's signal, which is many tones at once. That needs the SSB firmware (1_02_000 or newer) and **Mode: USB** or **PKTUSB** (both put the QMX in USB). On the radio, set **SSB → Input** to **USB**, with CESSB off and compression 0.
- **Keying: turn on the QMX's SSB VOX.** In USB the QMX transmits only while audio is arriving on its USB sound card. On the radio set **SSB → VOX → VOX** to **ON** (threshold 20; the hold time can stay at 10), with **SSB → Input** on **USB**. It then goes to transmit within about 0.2 s of the audio starting and back to receive under 0.1 s after it stops. Whistlestop and Mercury still send their CAT key command, but one sent ahead of the audio is ignored by the QMX (with SSB VOX off, only a key command sent while audio is already playing takes), so the VOX does the keying. The QMX manual says to disable **System config → CAT timeout** when using VOX.
- **Nothing transmitted?** Check that Whistlestop's **Audio out** and Mercury's sound card are the **QMX Transceiver** (audio sent to the computer's own speakers keys nothing), that SSB VOX is ON, and that the QMX shows USB, not DIGI.
- **VFO A or B.** The frequency is read from, and set on, whichever VFO the QMX is using. (The same goes for a TS-480, TS-2000 and truSDX.) On a QMX, Whistlestop sets **both VFOs** to the frequency you tune: while it transmits, the QMX reports its other VFO's frequency, so keeping the two together means the dial readout never jumps during a transmission and there is no doubt which frequency goes out. Whatever was in the other VFO is overwritten.
- **Meters.** The S-meter is the QMX's own reading in dB, taken as dB above S0. While transmitting, **SWR** and **Power out** are the QMX's own measurements; power is shown as a share of **Full power**, so set that to 5 W (or what your QMX makes).
- **No power, RF gain, squelch or volume control**: the QMX has no CAT commands that fit them, so the Radio Controls window doesn't offer them. Set power with the supply voltage, as usual for a QMX.
- One USB cable carries CAT and audio: choose **QMX Transceiver** for **Audio in** and **Audio out**, and the same device in Mercury.
- **The "ding" after transmitting.** A QMX that still has a USB audio stream coming in from the computer when it goes back to receive rings at 1500 Hz for about half a second (you hear it in its speaker, and it shows in its receive audio). Whistlestop opens its audio output to the radio only while it has something to send (the CQ whistle) and closes it **0.5 s** after, so its own transmissions, and transmissions keyed by anything else, don't ring. Mercury keeps its stream open past the end of its transmissions, so a short ding still follows each of Mercury's; it's harmless, and it isn't taken for another station (see "Your own transmission's tail doesn't count").

**Per-model details.** The S-meter is read on each model's own scale (TS-2000 0–30, TS-480 0–20, TS-590 0–30, TS-890 0–70 dots; Elecraft K3/KX 0–15, or 0–21 in its extended K31 mode). Yaesu squelch, mic gain and RF gain use each model's range (e.g. squelch 0–255 on the FTDX3000, FT-950, FTDX1200, FTDX5000, FT-2000, FTDX10 and FT-710; RF gain 0–30 on the FT-891). Split on Yaesu radios uses `ST` on the FTDX101 and FT-891 and the transmit-VFO command `FT` on the others, and is read back after setting; the FT-991/991A and FT-891 have no VFO select. The Elecraft K3/KX always receives on VFO A: choosing VFO B is refused (use split to transmit on B).

**What each protocol controls.** Frequency, mode (including the radio's data mode: Icom DATA, Kenwood DA, Elecraft DATA A, Yaesu DATA-U, FT-817 DIG), PTT and the S-meter work on every radio. The Radio Controls window offers what the radio supports:

| | Icom | Kenwood TS-590/890/990 | Elecraft | Yaesu | TS-480 / truSDX | FT-817 family |
|---|---|---|---|---|---|---|
| Power, AF, RF, squelch | ✓ | ✓ | ✓ | ✓ | ✓ (no mic gain) | — |
| SWR meter | ✓ | ✓ (approximate) | ✓ | ✓ | — | — |
| ALC, power meter | ✓ | ALC | — | ✓ | — | — |
| Preamp, attenuator | ✓ | ✓ | ✓ | ✓ | — | — |
| NB, NR, VOX | ✓ | ✓ | ✓ | ✓ | — | — |
| Auto notch, compressor | ✓ | ✓ | — | auto notch | — | — |
| Antenna tuner on/off and **Tune** | ✓ | ✓ | — | ✓ | — | — |
| Split, VFO A/B, RIT/XIT | ✓ | ✓ | ✓ | ✓ | — | — |

The QMX has its own list: see "QRP Labs QMX" above. Meter readings come from each radio's own scale and are converted to S-units and SWR; on some radios (Kenwood SWR especially) the conversion is approximate.

**FT-817 family**: set the radio's DIG mode to USER-U in its menu. The FT-817 can't report PTT over CAT, so Whistlestop remembers what it last sent.

## Antenna tuner and SWR

### The SWR watch

Whenever the radio transmits (a CQ whistle, Mercury data, a tune carrier), Whistlestop reads **SWR**, **ALC** and **power out** from the radio, if it reports them. It keeps each transmission's SWR: the second-highest reading of it, so one odd reading taken as the power ramps up or down doesn't count (a real mismatch reads high all through). A transmission with a single reading keeps that one.

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

## Transmit level

Mercury's transmit audio level decides how hard it drives the radio. Too much and the radio's **ALC** (automatic level control) squeezes the signal, which distorts data; too little and the signal is weak. For data, the ALC should **barely tick: a bar or two**, never pushed hard. Mercury keeps its level as **tx_gain_db** in its mercury.ini (0 dB by default, from -20 to +20 dB), read when it starts.

**Radio controls → Transmit level** lets Whistlestop set it while Mercury runs:

- **Mercury's own** (the default): Whistlestop leaves Mercury's level alone.
- **Set here**: Mercury's level is the one on the slider (**-20 to +10 dB**, steps of 0.5 dB). It's set when Whistlestop connects to Mercury and whenever you move the slider.
- **Auto (from ALC)**: the slider is the **most** Auto will use. Auto starts there; after each of Mercury's transmissions it looks at the radio's ALC over that transmission (read every 0.7 s, by its median, so one reading doesn't count) and:
  - above **30 %**: turns the level down **2 dB** (no lower than -20 dB);
  - above **12 %** (more than a bar or two): turns it down **1 dB**;
  - **1 %** or less (not ticking at all) on **3 transmissions in a row**: turns it back up **1 dB**, never past the slider, and never back to a level that already read above 12 % (it would only come down again). Mercury's frames differ: on the FTDX3000 its CQFRAME read 31 to 38 % where the frame after it read nothing, at the same level, so a quiet frame alone isn't a reason to turn up;
  - in between (barely ticking): leaves it.

  It needs two ALC readings or more from the transmission, not counting its edges: when the ALC moved at all, a reading of none was taken as the radio keyed before Mercury's audio started, or after it ended, and is left out (a short frame, like a 1 s acknowledgement, has too few and changes nothing), and a radio that reports ALC (Hamlib's `l ALC`, or direct CAT). The whistle and the tuning carrier don't count: Mercury's level doesn't change them. Auto starts again from the top each time Whistlestop connects to Mercury (when it starts, after **Save & connect**) and when you change the slider or the choice, and finds its level again within a few transmissions.

**The whistle** (the CQ's call-up tone) is Whistlestop's own audio, so Mercury's level doesn't change it; it follows the same choice instead, shown on the **Whistle** line in dB from its usual level: **Mercury's own** leaves it at its usual level; **Set here** plays it at the same dB as the slider (at most +6 dB, full scale); **Auto** gives it its own Auto, starting at the slider, from the radio's ALC while it plays, by the same rule (each whistle that plays to the end is one step; the Log says "Whistle level: -2.0 dB (…)"). On the FTDX3000 at 5 W the whistle at its usual level read ALC 62 %.

**Now** shows the level Mercury confirmed last; the line under it says why it was set ("Auto: the radio's ALC read 54%, more than a bar or two (12%)") or why it couldn't be. Each change is also in the Log ("Transmit level: Mercury -2.0 dB (…)"). Mercury saves every change in its mercury.ini, so it starts at that level next time too.

**Mercury's UI port.** Set here and Auto talk to Mercury through its WebSocket, which it opens only when started with **-G** (on port 10000; **-U** chooses another, and **Settings → Mercury modem → UI port (-G)** must match). Without it the Log says once "⚠ Transmit level: couldn't set … can't reach Mercury's UI port … start Mercury with -G", and Mercury keeps its own level. The UI port also serves Mercury's web page; it's on the Mercury computer's network like its other ports.

What the numbers mean depends on the radio, and on the day. On an FTDX3000 at 5 W (2026-11-14), Mercury at 0 dB read ALC 54 %, -5 dB 45 %, -8 dB 36 % and -10 dB 22 %; the next day -8 dB read 48 % and -10 dB 33 to 41 %: all far more than a bar or two, so expect Auto to end well below -10 dB on a radio like that. Start the slider where you'd like it to stop at most (0 dB is fine: Auto comes down 2 dB a transmission while the ALC reads high).

**Try it in Demo.** Demo mode's Mercury has a stand-in UI port, and the simulated radio's ALC follows Mercury's level along those FTDX3000 numbers. Choose **Auto (from ALC)** at 0 dB and call CQ a few times: Auto comes down 2 dB a time to -10 dB (ALC 22 %), then 1 dB a time to **-12 dB**, where the simulated ALC barely ticks (7 %), and stays there; the whistle comes down the same way.

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

Settings are saved to `whistlestop.toml` in `~/Library/Application Support/Whistlestop` (macOS), `%APPDATA%\Whistlestop` (Windows) or `~/.config/Whistlestop` (Linux); the Log's file `whistlestop.log` (see "The Log tab") and the Depot's `depot.json` are kept beside it. **Save & connect** applies them and reconnects. Closing the Settings window without it throws your edits away. If the settings file has a value Whistlestop can't read (edited by hand, say), only that setting falls back to its default: the rest are kept, and the Log names each one ("⚠ Settings file: …"). A value that's out of range (a negative listen time, a whistle of hours, a port of 0…) is set to the nearest sensible value, and the Log says so ("⚠ Settings: …"); the same goes for settings sent by a remote operator. A callsign is trimmed and upper-cased, and characters other than letters, digits, `/` and `-` are removed. A file that can't be read at all is copied to `whistlestop.toml.bad` and the default settings are used. An earlier, different copy is never replaced: the next one is `whistlestop.toml.bad-2`, and so on (starting again with the same unreadable file makes no new copy). If no copy can be made, nothing is saved over the file this session either. A file that's there but can't be opened (its permissions, or another program holding it) is left alone: the default settings are used for this session and nothing is saved over it until it can be opened again. Settings are written to a temporary file first and then moved into place, so a crash mid-save never leaves a cut-short file.

| Setting | What it does | Default |
|---|---|---|
| Callsign | Your call, sent in Mercury's CQFRAME and CALL frames | N0CALL |
| Name, Grid | Sent to the other station at connect (HELLO); the grid also goes in CQ CALLs and the Station ID, as its first 8 letters and digits (a Maidenhead locator: EM12ab) | empty |
| Demo mode | Simulated radio and band, with the K7BOT demo station | on |
| Demo on real Mercury | Demo runs two real Mercury modems when Mercury is installed | on |
| Mercury engine | Path to the Mercury engine (empty: look in the usual places) | automatic |
| Mercury host / TNC port / Broadcast port | Where Mercury listens | 127.0.0.1 / 8300 / 8100 |
| Mercury UI port (-G) | Mercury's WebSocket, for the transmit level | 10000 |
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
| Transmit level (Radio controls) | Mercury's own, Set here, or Auto (from ALC); the level, -20 to +10 dB | Mercury's own / 0 dB |
| Download folder | Where received files go | ~/Downloads/Whistlestop |
| Listen on / Listen volume | Speaker for the 🔈 Listen button, and its level | System default / 70% |
| Away message | What stations see when they connect while you're away | "Away from the radio…" |
| Hang up quiet links after | While away, end links idle this long | 180 s |
| Accept files by myself, each up to | While away, accept offered files up to this size | off / 5 MB |
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

**Mercury transmits but nothing goes out.**

- **"⚠ Mercury is transmitting but the radio isn't keyed"**: Mercury said it was transmitting (its PTT ON) for over 1.5 s while the radio read unkeyed on two polls in a row, so nothing went out. Mercury's PTT isn't reaching the radio: check its **-P / -R / -A** options or **[ptt]** in mercury.ini (with direct CAT: Hamlib model 2 at the sharing port), then restart Mercury. It's said once, and only for a radio whose PTT reading works (it has read keyed before, for a whistle say); "Radio: Mercury is keying it again" when it's fixed.

**The radio stays in transmit.**

- **"⚠ The radio has been transmitting for 30 s and nothing here keyed it"**: the radio reads keyed while neither Whistlestop nor Mercury is keying it, for **30 s** (long enough for a hand-keyed transmission or a tuner). Said once; "Radio: back in receive" follows when it ends. Check the radio: a stuck key or PTT line, its VOX, or the radio itself (on the RF bench a QMX locked in transmit, at 0 W, until it was switched off and on, and ignored every unkey command).
- While you're **Away**, Whistlestop also sends the radio one unkey command and says so; if it's still transmitting **30 s** later, the Log says it needs someone at the radio. It doesn't keep trying.

**The search locks on a whistle but no CQ follows.**

- **"That looks like the whistle's mirror image"** or **"a mirror image after all"** in the Log: your radio heard the caller on its other sideband as well, and Whistlestop corrected for it (see **Mirror images** under Search). Seen often, it means the radio's opposite-sideband rejection is poor (on the RF bench a QMX's image was only about 8 dB down; 40 dB or more is normal): check its receive I/Q (image) calibration, and keep strong nearby stations further away.

**A pill in the top bar is red.**

- **Radio**: rigctld (or RigHelm's server) isn't running or the address is wrong. Check **Settings → rigctld**.
- **Mercury**: Mercury isn't running, the ports differ, or another program (VarAC, vARIM) already holds Mercury's TNC port. Only one program at a time. Mercury started after Whistlestop: click the red pill to connect.
- **Audio**: the audio device in Settings isn't available. Pick it again; on macOS allow microphone access for Whistlestop in System Settings → Privacy. It also turns red if the device stops working while running (unplugged, a driver error): the Log says "⚠ Sound: …". A momentary glitch that leaves the sound running (a buffer underrun or overrun, as can happen on Windows when Mercury opens the same sound card) doesn't count. If the radio's audio arrives but is almost silent (below −100 dBFS for 10 s, outside transmissions), the Log says "⚠ The radio's audio is almost silent …": the radio isn't sending receive audio, usually because its USB/data output level (a menu setting on many radios) is turned down; check the recording level in your sound settings too. When the audio comes back and stays back for **5 s**, the Log says "Radio audio: back". (A radio on a dummy load is near-silent between another station's frames, so the warning isn't repeated for each frame: once the audio has come back, a later silence is reported only after **5 minutes** of it.) Until it's fixed Mercury can't decode, and the whistle search and watch ignore it (a radio's converter left on its last bits can hold faint tones that look like a whistle). "Radio audio: back" when it returns. Plug it back in, then **Save & connect** to open it again.

**"⚠ Search stopped: …" in the Log.** A search step needed the radio (a retune) and the radio didn't answer: the search ends and says why, instead of looking as if it were still running. Check the Radio pill, then search again.
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
