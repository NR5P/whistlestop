# Whistlestop — design

Whistlestop is an HF keyboard-to-keyboard chat and file-transfer program that
runs on top of the open-source **Mercury** modem. It is built around one idea:

> When you call CQ, you blow a **whistle**: a distinctive chirp signal that
> scanning stations can hear even when they are tuned well off your
> frequency. A scanning station that hears the whistle **stops**, works out
> exactly how far off it is, **retunes onto you**, hears your digital CQ, and
> links up.

There are no time slots, and no fixed calling frequency or broadcast channel.
You call CQ wherever you want inside the band segment, and the scanners find
you.

---

## 1. Why a whistle is needed at all

Mercury's digital CQ (`CQFRAME`, sent in the DATAC16 mode, 3.1 s) is very
robust, down to about −7 dB SNR, but the receiver has to be tuned within
**±55 Hz** of the transmitter or it never syncs (Mercury
`docs/OFFSET-ACQUISITION-FINDINGS.md`). A scanner that steps across a band
segment 1–2 kHz at a time almost never lands within 55 Hz of a caller, so
Mercury on its own cannot "find" a CQ by scanning. It can only scan a short
list of known, exact channels, the way BPQ32 does.

The whistle fixes this. A linear chirp has two properties that suit the job:

1. **Huge capture range.** A chirp sweeping 500 → 2500 Hz still produces a
   strong matched-filter peak when the receiver is hundreds of Hz off. The
   signal just moves along the sweep and part of it falls outside the
   passband. A detector catches it from about ±1 kHz away, so scan steps can
   be about 2 kHz wide.
2. **It measures its own frequency error.** On a chirp, a frequency offset
   shows up as a **time shift** of the correlation peak. An up-chirp's peak
   moves *earlier* by Δf/k and a down-chirp's moves *later* by Δf/k (k is the
   sweep rate in Hz/s). Pair an up-chirp with a down-chirp and the gap
   between their peaks is

   ```
   gap = T + 2·Δf / k      →      Δf = (gap − T) · k / 2
   ```

   With k = 4000 Hz/s and 8 kHz sampling, one sample of timing error is
   about 0.25 Hz of frequency error. The scanner can retune to **within a
   few Hz**, far inside Mercury's ±55 Hz window. (LoRa radios use the same
   trick.)

On a radio you hear the whistle as a "whee-ooo, whee-ooo" sound, like a
train whistle going past. That is the signature sound you asked for: anyone
who hears it knows a Whistlestop station is calling.

## 2. The CQ cycle (caller)

```
 ┌──────────── whistle ────────────┐┌─ CQFRAME ─┐┌──── listen ────┐
 │ UP DN UP DN UP DN … (W seconds) ││ DATAC16   ││ for CONNECT    │  repeat
 └─────────────────────────────────┘└───────────┘└────────────────┘
```

* **Whistle**: repeated UP/DOWN chirp pairs. The default is 0.5 s per chirp
  (500 → 2500 Hz), so one pair lasts 1 s. The whistle runs for `W` seconds,
  and `W` must be at least the listeners' full scan cycle, as with ALE
  "scanning calls". Whistlestop generates this audio itself and keys the
  radio through `rigctld`.
* **CQFRAME**: Mercury's own compact digital CQ, which carries your callsign.
  This is the legal digital ID and the "CQ in digital" from your idea.
* **Listen**: Mercury sits in `LISTEN ON`. A station that heard you sends
  `CONNECT`, and Mercury reports `CONNECTED`. The cycle repeats until
  someone answers or you stop it.

## 3. Scanning (listener)

```
 for each channel in the scan plan:
     tune radio (rigctld "F")
     dwell D seconds, feeding audio to the whistle detector
       ├─ no whistle            → next channel
       ├─ partial whistle (one strong chirp, no pair yet) → HOLD (extend dwell)
       └─ whistle pair found    → Δf measured
             retune dial by +Δf   (USB: a higher audio tone means dial up)
             STOP scanning, wait for Mercury's "CQFRAME <call>"
             show "CQ from <call>". Operator (or auto-answer) → CONNECT
```

* Mercury stays in `LISTEN ON` the whole time. Whistlestop never sends
  `LISTEN OFF` to hop channels (that flushes Mercury's decoders; see TNC.md
  "Scanning hosts").
* **Dwell**: the detector needs one complete UP+DOWN pair inside its window.
  The pair lasts 2T = 1 s, and the pattern repeats with period 2T, so a
  window of 4T = 2 s always contains a complete pair. The default dwell is
  2.2 s, which allows for retune settling.
* **Step**: 2 kHz by default. A 20 kHz slice of band is 10 channels, a
  22 s scan cycle, so the caller's whistle should run about 25 s. This
  trade-off is your "not so fast that you miss it, fast enough to find them
  quickly". Both numbers are in the config.

## 3b. Wide search with a second receiver (optional)

One radio is enough: Search (section 3) steps it through the segment. An
RTL-SDR, as a second receiver on RX OUT or IF OUT, makes the search near
instant and leaves the main radio free:

1. **Decimate**: the dongle's 2.048 MS/s is mixed to each digital segment's
   centre and low-passed/decimated to 64 kHz (128/256 kHz for wide segments
   like 10 m).
2. **Channelize**: once a second, one FFT of the last 2.2 s is cut into
   virtual USB receivers every 1.8 kHz across the segment. The normal whistle
   detector runs on each (20 m: 10 virtual receivers).
3. **Verify for free**: each candidate gets a virtual receiver centred exactly
   on it. A real caller reads about 0 Hz there; an alias does not. No retune.
4. **Hand off**: the main radio tunes to the hit and measures the whistle on
   its own audio. That removes the dongle's crystal error (15–50 ppm is a few
   hundred Hz on HF) before Mercury needs ±55 Hz.

In the demo (simulated 17 ppm dongle error, +240 Hz) the SDR spots the
caller 1.5 s into its whistle. The radio fine-locks −240.0 Hz and links in
about 14 s.

## 4. Talking once linked

The application layer is **WSP, the Whistlestop Protocol**: see
[PROTOCOL.md](PROTOCOL.md). In short, 1:1 chat and files go over Mercury's
ARQ, and Roundtables go over its broadcast plane. There is no fixed broadcast
frequency: a Roundtable lives wherever its host called it.

**Speed.** On a 1:1 link Mercury climbs its own 7-rung ladder
(MFSK → QAM16C2) as SNR allows, inside the bandwidth you picked. A Roundtable
has no ARQ to do that, so the host picks the fastest mode that every member's
reported SNR supports, with a 3 dB margin.

**Bandwidth, narrow to wide:** 250 · 500 · 1000 · 1700 · 2300 · 2750 Hz. Each
step caps the Roundtable modes. 1:1 links use Mercury's BW500 ladder below
2300 Hz and BW2300/BW2750 above. The whistle has a narrow profile
(1250–1750 Hz) for narrow filters.

## 5. Station wiring (macOS)

```
                ┌──────────────┐
  Whistlestop ──┤ rigctld :4532├── CAT/PTT ── radio
      │  │      └──────────────┘                 ▲
      │  │   audio (whistle TX + detector RX)    │
      │  └───────── USB sound card ──────────────┤
      │                                          │
      └── TCP 8300/8301/8100 ── Mercury ─────────┘ (audio, PTT via rigctld)
```

Only one program can open the radio's serial port, so **`rigctld` owns
it**. Mercury uses Hamlib model `2` (NET rigctl) pointed at
`localhost:4532`, and so does Whistlestop. Both share the USB sound card
through CoreAudio.

## 6. Regulatory notes (US)

* The whistle is a documented, public, swept-tone data signal under 2.8 kHz
  wide, so it falls under Part 97.309(b) (publicly documented codes). This
  document and the source code are that documentation.
* The station is identified by the CQFRAME, which carries the callsign in
  the clear, and by the ARQ CALL frames.
* Stay inside the band's data segments. Keep whistles short and only as
  long as the scan plan needs: a whistle is a calling signal, not a beacon.

## 7. Does anything else do this?

* **ALE (MIL-STD-188-141, PC-ALE, amateur ALE nets)** is the closest in
  purpose. Scanning radios stop for a "scanning call" longer than one scan
  cycle. But ALE is channelized: scanners sit exactly on preset channels, and
  the call is 8-FSK, not a chirp.
* **Selcall (Codan/Barrett, CCIR 493)** and **maritime DSC**: also
  channelized calling.
* **LoRa** uses the same up/down-chirp trick to find a signal and measure its
  frequency offset, but on UHF and not for scanning.
* **Chirp sounders/ionosondes** use HF chirps to measure propagation, not to
  call anyone.

As far as we know, nothing else combines a per-application signature chirp,
detection far off frequency in a non-channelized segment, and self-measured
offset so a scanner retunes exactly onto the caller.

## 8. Known limits (to test on the air)

* Simulated so far: AWGN plus an SSB passband. Real fading (Watterson) may
  blank single chirps; repeated pairs should carry it. Test with Mercury's
  channel model and on air.
* A caller more than ±1000 Hz from a scan channel aliases by 2000 Hz. The
  search then checks the alias candidate (`WhistleParams::alias_of`).
* The whistle must last at least the searchers' scan pass (about 24 s for
  20 m). Searching "all bands" needs much longer whistles, or the RTL-SDR
  wide search below.

## 9. Roadmap

1. ✅ Whistle generator, detector and offset estimator (tested to −14 dB, under 1 Hz error)
2. ✅ Mercury TNC client, rigctld rig control (Hamlib or RigHelm), search, whistle watch, CQ cycle
3. ✅ WSP: chat, files (offer/accept/resume, built-in zip), Roundtable (speed ramp, AGAIN, floor)
4. ✅ egui interface: waterfalls, RTL-SDR wide view (RX OUT / IF OUT), time predictions
5. ✅ Demo mode: simulated band, radio, Mercury and a demo station
6. ⬜ On-air tests with two stations and real Mercury
7. ✅ **Wide whistle search on the RTL-SDR** (section 3b): optional; one radio
   still searches by itself
8. ⬜ Whistle "network IDs" (sweep patterns per group), packaging (.app/.dmg)
