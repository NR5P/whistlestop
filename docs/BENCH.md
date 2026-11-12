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

Slower scenarios run only when named (not part of a run with no names):

| Name | What it checks |
|---|---|
| `cancel` | 12 KB files both ways at once; one cancelled mid-transfer, first by its receiver, then by its sender. The other finishes byte-exact, the cancelled one reads "cancelled…" at both ends, no other transfer row changes. |
| `chat_during` | A chat typed during a 16 KB transfer waits for at most ~2 file pieces (3 KB of file may go first). |
| `resume` | 12 KB A→B, BRAVO hangs up mid-transfer, they link again: ALPHA re-offers, "already has … resuming", the rest arrives byte-exact. |
| `e2e_require` | ALPHA on encryption Require, BRAVO Off. ALPHA queues a chat and a file as soon as the link is up; ALPHA must end the link with a reason, and neither may reach BRAVO. |
| `away_long` | BRAVO away, `away_idle_s` 60 (Settings' minimum), 500 Hz: a 4 KB file takes several minutes. No hang-up mid-transfer; hang-up about 60 s after it ends (the 30 s signal reports don't count). |
| `away_weak` | The same with 20 dB more noise and a 3 KB file. |

For example `... bench-run cancel chat_during resume e2e_require away_long away_weak`
(about 30 min).

## Cancel, resume, Require, away timeout (2026-11-11)

**After the fixes: 18/18** (and the standard set 34/34 on the same build):

| Test | Result |
|---|---|
| A→B cancelled by its receiver / by its sender | both PASS: "cancelled…" at both ends, B→A unaffected |
| Chat during a 16 KB transfer | PASS: arrived in 13 s, 2 KB of file went first |
| Link dropped mid-transfer, then relinked | PASS: same fid, "already has 2.0 KB — resuming", the rest byte-exact |
| Encryption Require vs Off | PASS: ended 34 s after linking with the reason; nothing reached BRAVO in plain text |
| Away (idle 60 s), 4 KB and 3 KB (weak) at 500 Hz | PASS: no hang-up mid-transfer (215 s, 165 s); hung up at idle 63 s / 60 s afterwards |

Fixed: Whistlestop now counts what it writes to Mercury and takes off only
the decreases Mercury reports (bytes sent), so at most two pieces wait in
Mercury; the transfer speed is measured from those decreases. Bytes of a file
coming in count as link activity. A late DONE no longer overwrites a
cancelled row.

First runs, before the fixes:

| Test | Result |
|---|---|
| Files both ways, A→B cancelled by its receiver | PASS ×2: both ends "cancelled…", B→A byte-exact, no other row touched; the cancel took 52 s to show at both ends |
| Files both ways, A→B cancelled by its sender | **FAIL ×2**: the file was already all in Mercury's buffer; it arrived complete and both rows ended Done |
| Chat during a 16 KB transfer | **FAIL**: 8 KB of file went ahead of the chat (34 s), not ≤ 2 pieces |
| Link dropped mid-transfer, then relinked | PASS: 3 KB kept on B's disk, "BRAVO already has 3.0 KB — resuming", the rest byte-exact in 45 s |
| Encryption Require vs Off | PASS: ALPHA hung up 35 s after linking ("the other station does not have encryption on, and this station requires it"); the chat and file ALPHA queued never reached BRAVO |
| Away (idle 60 s), 4 KB at 500 Hz | PASS: no hang-up during the 215 s transfer; hung up at idle 64 s afterwards |
| Away (idle 60 s), 3 KB at 500 Hz, channel SNR ≈ +7 dB | PASS: 169 s transfer, hung up at idle 60 s afterwards (Mercury stayed on DATAC3, ~33 s per 1 KB piece) |
| Away (idle 40 s), 3 KB at 500 Hz (first run) | **FAIL**: BRAVO hung up "idle 43 s" with Mercury handing it 120 bytes every 3.5 s of the first piece |

What they found:

- **Flow control to Mercury doesn't hold.** `pump()` means to keep at most
  2 × CHUNK in Mercury's buffer, but each `BUFFER n` line *replaces*
  `mercury_buffer`. Mercury reads the data socket 128 bytes at a time and
  reports `BUFFER` after every read, so the count it reports trails what
  Whistlestop has written, and pump() writes more. Mercury's log shows the
  whole 12 KB file written within one second (and a 4 KB file at 500 Hz).
  So a sender's cancel can't stop it, a chat waits behind the file, the
  sender's progress bar runs ahead (9.7 KB "sent" while B had 2 KB), and the
  transfer estimate on the same link drops to "about 5 s" for 12 KB.
- **The away timeout counts whole packets only.** Bytes arriving from
  Mercury don't count as activity until a whole link packet (a 1 KB file
  piece) is in; when a piece takes longer than `away_idle_s` (slow modes,
  retries) an away station hangs up mid-transfer.

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

That fallback hang-up is an `ABORT` (2026-11-11). As a `DISCONNECT`, Mercury
deferred it "until delivered" and kept polling the departed station on the
home frequency for 120 s; on the bench those transmissions muted BRAVO's
receiver during the first scan trials, which then missed the whistle.

## Results (Mercury 1.9.23 ×2, 2026-11-11, Ubuntu on WSL 2): 34/34

All 34 tests passed: the full run passed everything except scan speed, and
`qsylib scan` then passed 10/10 once the scan waited for quiet air (below).
Before these fixes the same bench gave 24/34.

| Test | Result |
|---|---|
| Search → whistle → CQFRAME → link | linked after 56 s (incl. search); B tuned +2 Hz from A |
| Chat both ways + delivered | B→A 9.2 s, A→B 13.7 s |
| 16 KB binary B→A | 1618 bps goodput on QAM16C2 |
| No false whistles during link traffic | 0 / 0 |
| Roundtable | join seen by host, 339-char message in 43 s, ramped to DATAC1 |
| Depot: away station collects by itself / hangs up when done | 22 s / 29 s |
| Scan speed (normal, weak, very weak) | 4/4 each; medians 7.3, 4.8, 3.6 s |

Bugs this run found and fixed:

- **A CQ cancelled the call answering it.** Mercury reports an incoming call
  with PENDING; the CQ carried on and its next CQFRAME, sent during Mercury's
  accept handshake, made Mercury drop the call. A CQ is now held while a call
  is arriving. Mercury also sends PENDING/CANCELPENDING around our *own*
  CQFRAME (VARA-style), so those are told apart by when we sent one.
- **Automatic links never went idle.** The signal reports both stations send
  every 30 s no longer count as activity; a collect link ends 10 s after its
  mail arrives.
- **A lost disconnect after a QSY** left Mercury polling the departed station
  for 120 s on the home frequency; the fallback hang-up is now an `ABORT`.
- **Bench:** the scan scenario waits until neither modem has keyed for 25 s,
  so the previous link's teardown can't mute the scanner's first trial.

## Earlier results (Mercury 1.9.23 ×2, 2026-11-10): 18/18 passed, plus Depot 7/7

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
