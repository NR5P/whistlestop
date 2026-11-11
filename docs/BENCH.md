# Two-station test bench (real Mercury)

`src/bin/twotest.rs` runs **two real Mercury modems and two Whistlestop
engines on one computer**, joined by the **ether** (`src/ether.rs`), a
simulated HF band:

- a virtual radio per station: a rigctld server tracking dial and PTT;
- Mercury's own `-x sock` lockstep audio protocol (`audioio/sock_wire.h`);
- an audio link for Whistlestop (its whistle goes on the air);
- the channel between them: shift by the difference of dial frequencies
  (USB), band noise, optional fading, a 300–2700 Hz passband, and the
  receiver muted while its station transmits.

## Build Mercury from source

```bash
cd ~/Desktop/mercury/source
make -j8 internal_deps EXTRA_CFLAGS="-I$PWD/common/iniparser"
make -j8 mercury EXTRA_CFLAGS="-I$PWD/common/iniparser"
```

## Run

```bash
cargo run --release --bin twotest -- ~/Desktop/mercury/source/mercury bench-run
```

Add scenario names to run only some of them: `watch link chat files table
narrow weak e2e depot scan qsylib`. Results go to `bench-run/REPORT.md`; Mercury and Whistlestop
logs are written beside it.

## Whistle v2 against the first whistle (2026-11-11)

Scenario `scan`: BRAVO scans the 20 m segment (10 channels), starting from a
different channel and moment each trial; ALPHA calls CQ once. Time from the
start of the whistle until BRAVO is tuned within Mercury's ±55 Hz. Both
versions ran the same scenario on the same simulated band (one-sided USB
passband, image rejection > 65 dB), each with its own defaults.

| Signal | First whistle: 0.5 s chirps, 2.2 s dwell, 24 s whistle | Whistle v2: 0.2 s chirps, 0.5 s dwell (2 s when promising), 10 s whistle |
|---|---|---|
| strong | 4/4, median 14.5 s | 4/4, median 8.5 s |
| weak (+14 dB) | 4/4, median 14.5 s | 4/4, median 6.2 s |
| very weak (+8 dB) | 4/4, median 14.6 s | 4/4, median 5.1 s |

v2 finds the caller in a third to a half of the time, inside a whistle less
than half as long. The bench also shaped the scanner. It needed:

- **Two whitenings in the scan detector.** Per frequency bin, so carriers,
  RTTY and FT8 vanish; per frame, so a data burst (Mercury's OFDM) can't
  pose as a whistle.
- **A cell-count rule.** A loud signal crossing a wrong track at a few points
  can't pass.
- **Walking to a whistle.** A strong caller just below the passband edge
  shows only the bottom of its sweep, so the first estimate can be hundreds
  of hertz out. The scanner retunes again (up to two hops), and if the
  whistle ends meanwhile, it waits for the CQ there.
- **A one-sided simulated passband.** The ether used to let the opposite
  sideband and fold-over through.

## QSY and the file library (2026-11-11)

BRAVO away, accepting QSYs (20 m only) and sharing a library:

| Test | Result |
|---|---|
| Browse BRAVO's library over real Mercury | listing in 9 s |
| Download a 3 KB file from it | 40 s, byte-exact |
| QSY to 40 m | refused by BRAVO's rules ("I only QSY within 20m") |
| QSY to 14.106 MHz | both moved in 12 s; chat crossed on the new frequency in 26 s |
| End the link | both back on 14.10315 MHz |

Making it reliable took a polite hang-up. A station now sends BYE and waits
until Mercury has sent it before disconnecting. A station that receives BYE
waits up to 30 s for the other's disconnect (Mercury's takes about 20 s)
before hanging up itself; doing it sooner collides with theirs. A station
back from a QSY stays 8 s so the disconnect finishes there.

## Results (Mercury 1.9.23 ×2, 2026-11-10): 18/18 passed, plus Depot 7/7

| Test | Result |
|---|---|
| Depot: mail handed over when linked | confirmed 32 s after linking |
| Depot: away greeting, chat saved to Arrivals | greeting at once; note saved in 4 s |
| Depot: Station ID → away station collects by itself | beacon heard in 4 s; mail collected in 33 s; link hung up when done |
| Whistle watch, B parked 600 Hz off | measured +600.0 Hz |
| Search → whistle → CQFRAME → link | linked in ~90 s (incl. search); B tuned **+0.0 Hz** from A; Mercury decoded the CQFRAME |
| Chat both ways + delivered | 9–14 s per message |
| Zipped folder A→B | 56 KB of logs → 1.5 KB zip, SHA-256 good both ends |
| 16 KB binary B→A | ~1600 bps goodput on QAM16C2 (predicted 68 s, took 79–84 s) |
| No false whistles during link traffic | 0 |
| Roundtable | CALL heard, join, 339-char multi-frame message, reply; host ramped both to DATAC1 |
| Narrow 500 Hz | negotiated 500, file delivered on DATAC3 at ~144 bps |
| Weak signal + fading | Mercury SNR ≈ 0–3 dB, chat delivered in 9 s on DATAC3 |

Bugs this bench found and fixed: Roundtable speed switch timing, false
whistles from data bursts, transfer-time estimates, a sideband leak in the
ether's frequency shifter, and link data arriving before `CONNECTED`.
