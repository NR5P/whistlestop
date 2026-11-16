**Beta for testers.** HF keyboard chat, file transfer and group Roundtables over the [Mercury](https://github.com/Rhizomatica/mercury) modem, with the whistle CQ.

### Download

- **Windows 10/11 (64-bit):** `whistlestop-…-windows-x64.zip`. Unzip it and run `whistlestop.exe`. `READ-ME-FIRST.txt` inside has the steps.
- The `.sha256` file holds the zip's checksum.
- You also need Mercury: [Mercury releases](https://github.com/Rhizomatica/mercury/releases).

The program isn't signed yet, so SmartScreen will ask you to confirm the first time: click **More info**, then **Run anyway**.

### Start here

1. Whistlestop opens in **Demo mode**: a simulated radio and band and a demo station, K7BOT. Nothing is transmitted.
2. When you're ready for the air, follow "Setting up your station" in the built-in **Guide**.
3. Make your first transmissions into a **dummy load** at low power.

### Tested on the air so far

On two stations, into dummy loads:
- Yaesu **FTDX3000** (direct CAT)
- QRP Labs **QMX**

All of these worked:
- CQ and answer
- chat
- file transfers with checksums
- Roundtables
- the Depot
- Away mode
- Station IDs
- moving a link to another frequency
- search, including the RTL-SDR wide search

### Known limitations

- The estimate shown when a file is offered can be well off at the start of a link.
- Mercury's busy detector can hold a CQ for up to 60 s when there's local noise on the frequency.
- Radios that hear signals on their opposite sideband (poor image rejection, as on the QMX) can still confuse Search, although it now checks for that.
- Roundtables with three or more stations are only partly tested.
- Encryption is off by default. It is **not legal on amateur bands** in the US (47 CFR 97.113(a)(4)) and most other countries.

### Reporting problems

Please [open an issue](https://github.com/NR5P/whistlestop/issues). Include:
- your radio and how it's connected;
- the Whistlestop and Mercury versions;
- what happened;
- the lines from the Log tab around that time.
