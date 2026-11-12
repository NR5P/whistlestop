# Two- and three-station test bench (real Mercury)

`src/bin/twotest.rs` runs **two real Mercury modems and two Whistlestop
engines on one computer** (three, with CHARLIE, for the three-station
scenarios), joined by the **ether** (`src/ether.rs`), a simulated HF band:

- a virtual radio per station: a rigctld server tracking dial and PTT;
- Mercury's own `-x sock` lockstep audio protocol (`audioio/sock_wire.h`);
- an audio link for Whistlestop (its whistle goes on the air);
- the channel between them: shift by the difference of dial frequencies
  (USB), band noise, optional fading, a 300–2700 Hz passband, and the
  receiver muted while its station transmits; with three stations, extra
  loss on single paths (`Ether::set_pair_loss`), so the paths can differ.

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

## The real apps as stations

`--a=KIND` and `--b=KIND` put a real app on ALPHA or BRAVO instead of an
engine in the bench's own process:

| KIND | Station | Its Mercury |
|---|---|---|
| `engine` (default) | Whistlestop's engine inside `twotest` | `mercury -x sock` on this computer |
| `desktop` | the desktop app (`target/release/whistlestop`, or `WHISTLESTOP_BIN`), its own settings file | `mercury -x sock` on this computer |
| `ios` | the iPhone app in the booted Simulator | built into the app, ports 8320/8330 |
| `android` | the Android app (debug build) in the emulator (`ANDROID_SERIAL`) | built into the app (arm64) |

```bash
ANDROID_SERIAL=emulator-5554 target/release/twotest ~/Desktop/mercury/source/mercury bench-run/apps --a=ios --b=android
```

The bench operates each app by remote operation, the way one desktop
operates another. A phone app does this only when its folder holds
`bench.json` (the bench writes it, with the settings, before launching
the app, and removes it after): its built-in Mercury then uses the ether
(`-x sock`) instead of the phone's sound, and the app serves remote
operation on 7381 (ALPHA) or 7382 (BRAVO). The emulator reaches the
computer at 10.0.2.2, and its Mercury's audio comes over TCP (7401/7402),
bridged to the ether's socket; remote operation goes through `adb forward`.
Build and install the apps first (`mobile/README.md`).

`TWOTEST_PORT_OFFSET=2000` moves every port the bench opens on this
computer (ether, Mercury, remote operation, bridges), so two benches can
run at once in different work folders.

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

## A real phone on USB

`ANDROID_SERIAL` set to a phone (anything not `emulator-…`) runs the
`android` station on that phone: the bench reaches it with `adb reverse`
(its rig, audio and Mercury-audio ports) and `127.0.0.1` instead of the
emulator's `10.0.2.2`, and removes the reverses afterwards. Keep the
phone unlocked (Developer options → Stay awake): a locked phone can't
start the app. The bench saves the phone's own `whistlestop.toml` and puts
it back at the end (its settings turn on remote operation with the bench
password).

## Over the air: `airtest` (computer ↔ Android phone, real sound)

`src/bin/airtest.rs` checks the phone's own audio path: no radio and no
simulated band. ALPHA (`N0MAC`) is an engine on this computer with Mercury
on its default speakers and microphone (`-x coreaudio`, ports 8350/8150);
BRAVO (`N0PHN`) is the Android app on the phone named by `ANDROID_SERIAL`,
its built-in Mercury on the phone's speaker and microphone (`bench.json`
with an empty `mercury_audio`), operated remotely through `adb forward`.
Both use Radio → none (VOX: nothing to key).

```bash
ANDROID_SERIAL=<phone> target/release/airtest ~/Desktop/mercury/source/mercury air-run [whistle] [link] [e2e] [table]
```

`whistle` (the phone's audio output starts, the computer hears nothing
in 15 s of quiet, then hears the phone's CQ whistle), `link` (the phone
calls the computer, then chat both ways with delivery
confirmed), `e2e` (Prefer on both: same check code, sealed chat), `table`
(the computer hosts a Roundtable by CQ, the phone hears the CALL and
joins, both talk). Results: `air-run/AIR-REPORT.md`, with both Mercury
logs beside it. Like the bench, it puts the phone's own settings back
afterwards, and it refuses to start while port 8350 is taken.

### First results (2026-11-12, MacBook Air ↔ Galaxy S23, same room)

2/6. Both Mercurys came up and the phone linked to the computer in 36 s
(MFSK, 38 bps; the phone measured the computer at 12 dB). Phone →
computer worked: all 62 bytes it sent arrived. Computer → phone did not:
the phone decoded the computer's ACCEPT (on the MFSK floor) but none of
the 83 bytes of data in 13 frames, so the HELLO never completed, no chat
could pass, and the phone declared the link lost after 240 s. The same
direction failed for the Roundtable CALL (DATAC3) and the whistle.
Mercury's phone input already uses Android's UNPROCESSED preset, so the
phone's voice processing isn't the cause; the MacBook's small speakers
into the phone's microphone are the weak direction.

A second try with the computer's volume at 80 % (the first was 60 %):
0 links. The computer heard the phone's CALL and sent its ACCEPT twice;
the phone heard neither and fell back to calling on the MFSK floor. So
louder doesn't help; the computer → phone path is the problem.

Found on the way: Radio → none was treated as a rigctld host called
"none" (fixed: "Radio: none (no CAT)", test `radio_none_is_no_cat`), and
on the phone Whistlestop's own audio output (the whistle) never started:
cpal's Android backend asks the app's JavaVM and Context for AudioManager's
mixer bursts after opening an output stream, and panicked because the app
had never handed them over (`ndk_context`), killing the audio thread. Fixed
(`Core.initAndroid`, called before the engine starts); a panic while
building any audio stream now reports its message instead of "did not
start". Checked with `airtest whistle`: 4/4, no false alarm in 15 s of
quiet, then the computer heard the phone's CQ whistle 3 s in (score 0.9).

## The apps against each other, and a bug hunt (2026-11-12)

`--a=ios --b=android` (Simulator ↔ emulator, each app with its built-in
Mercury): link, chat, Roundtable, encryption (same check code, sealed chat,
6 KB file byte-exact) and the Depot all passed, 21/21 over two runs.

Fixed after a review of the engine and the apps:

- **The phones restarted the whole station to reach a late Mercury.** The
  engine starts before the app's Mercury opens its ports, so
  `connect_when_ready` sent `Start` again, which dropped the radio's shared
  rigctld port: Mercury's one connection for keying a USB radio. Now it
  sends `ReconnectModem`, which only connects to Mercury (test
  `reconnect_modem_reaches_a_late_mercury_without_a_restart`).
- **Auto-answer could leave "Calling X" up for good** when its CONNECT
  failed (Mercury busy, or no modem), keeping the whistle watch, beacons
  and schedule off; and a Connect by hand during its 4.5 s wait sent a
  second CONNECT.
- **A waiting Roundtable join kept retrying** through a call or a CQ of
  your own; calling now cancels it, and it never retries mid-call.
- **The bench's remote operation on a phone listened on every interface**
  with the well-known password: now loopback only (`remote::serve_on`,
  test `serve_on_loopback_is_not_on_the_lan`).
- **Android: rotating the phone jumped to Chat** while linked (or to the
  Roundtable while at one): the activity is recreated and the "new link"
  check saw the old link as new. Checked on the emulator before and after.
- **The bench crashed** in any scenario that sends a file (narrow, e2e,
  resume…) when run without `files`, which made the `to-send` folder.

Not changed: after a "measure again" hop in search, a silent next look
still waits for the CQ at the re-measured frequency, which may be the
stale one. Worth a look with the `scan` scenario.

## Three-station scenarios

Naming any of these adds a third station, **CHARLIE**, always an engine
station (its own Mercury on control port 8340 and broadcast 8140, rigctld
4652, app audio 7320, all moved by `TWOTEST_PORT_OFFSET`; logs
`mercury-CHARLIE.log`, `whistlestop-CHARLIE.log`). It waits on 14.098 MHz, out of
earshot, until a scenario tunes it to ALPHA's 14.10315 MHz. Without one of
these names the bench is the two-station bench, unchanged.

| Name | What it checks |
|---|---|
| `table3` | ALPHA hosts a Roundtable; CHARLIE is far away (it and ALPHA hear each other 25 dB weaker, channel SNR ≈ +8 dB; BRAVO hears both at ≈ +33 dB). The host's CALL heard by both; CHARLIE joins first, then BRAVO; the host sees both joins; a ~300-character message from each station reaches the other two; the host runs at the speed the weakest member can copy (not faster) and CHARLIE copies it; CHARLIE then only listens for 5+ minutes while BRAVO talks: the host must stay at CHARLIE's speed; CHARLIE leaves (LEAVE seen by host and BRAVO); the host may speed up for BRAVO; CHARLIE joins again after the speed-up (heard? the table slows for it?); everyone leaves: all three listen in DATAC3 again and hear ALPHA's next CQ CALL. About 20 min. |
| `relay3` | ALPHA posts Depot mail for CHARLIE via BRAVO (relay on). ALPHA links BRAVO: the mail is In Transit at BRAVO (hops 1, handed by ALPHA) and ALPHA shows it Delivered, taken by BRAVO (MAILACK). BRAVO links CHARLIE by hand: it arrives in CHARLIE's Arrivals exactly once, hops 1 on the wire, and BRAVO's In Transit empties. Then twice more by automation: CHARLIE away with automatic collection hears BRAVO's Station ID listing it and collects by itself; BRAVO away with automatic delivery hears CHARLIE's Station ID and delivers by itself. About 8 min. |
| `cq_pileup` | ALPHA calls CQ (20 s whistle, 2 cycles); BRAVO and CHARLIE both search with automatic answer, find it and call it at the same moment. ALPHA links with exactly one; the winner's link carries a chat; the other gives up cleanly (no link, no call or incoming call left showing, search stopped, its Mercury back to LISTENING); no whistle reported anywhere once ALPHA is linked; the other links ALPHA once the first link ends; no Mercury left out of LISTENING. About 8 min. |
| `busy3` | BRAVO and CHARLIE linked on ALPHA's frequency, an 8 KB file BRAVO→CHARLIE, ALPHA parked there: ALPHA calls BRAVO (busy) and must give up cleanly while their link and file carry on; ALPHA's whistle watch must report no whistles from their traffic; ALPHA then calls CQ on top of them (how long it transmits over them is measured) and their link must survive; once they're done ALPHA links BRAVO; no Mercury stuck. About 8 min. |

`... bench-run table3 relay3 cq_pileup busy3` takes about 45 min.

## Three stations (2026-11-11): 28/32

Mercury 1.9.23 ×3 on the ether, Ubuntu on WSL 2; two full runs, 28/32
each, the same four failing (all product behaviour, listed below). The
standard two-station set on the same build: 34/34.

| Test | Result |
|---|---|
| Roundtable: host's CALL heard | **FAIL** (every three-station run): the CALL frame went out in DATAC16, 14 bytes, and every receiver discarded it |
| Roundtable: joins, a 300-char message from each member to the other two | PASS: both joins in 4 s; messages across in 11–36 s |
| Roundtable: speed for the weakest member | PASS: host stayed DATAC3 for CHARLIE (+7 dB report) though BRAVO could do more; CHARLIE copied the host |
| Roundtable: weakest member only listens for 5 min | **FAIL**: host sped up to DATAC1 once CHARLIE's JOIN was 5 minutes old (CHARLIE, at ≈ +5 dB, still copied it this time, under the 3 dB margin) |
| Roundtable: LEAVE; speed-up when the weak member left | PASS / PASS (already DATAC1, see above) |
| Roundtable: a station joining after the speed-up | **FAIL**: its JOIN goes out in DATAC3, the host listens in DATAC1 and never hears it |
| Roundtable: everyone left → DATAC3, next CQ CALL heard | PASS |
| Relay: hand-over to BRAVO, MAILACK | PASS: 23 s; In Transit at BRAVO with hops 1, ALPHA's Delivered says BRAVO |
| Relay: BRAVO links CHARLIE | PASS: arrived once, hops 1 on the wire, BRAVO's transit empty |
| Relay: CHARLIE (away) collects after BRAVO's Station ID | PASS: alert in 4 s, collected in 22–30 s, hung up |
| Relay: BRAVO (away) delivers after CHARLIE's Station ID | PASS: delivered in 39 s, hung up |
| CQ pileup: both answer, ALPHA links exactly one | PASS: both CONNECTs within the same second; CHARLIE won after 113 s |
| CQ pileup: winner's link, loser clean, loser links later, no modem stuck | PASS ×4: but neither call got through at first (no PENDING at ALPHA, so its second CQ whistle went out on top of both callers); CHARLIE linked 70 s after both called, and BRAVO's Mercury kept calling for 139 s, 50 s into the ALPHA–CHARLIE link (a chat took 55 s) |
| CQ pileup: no false whistles once linked | **FAIL** (both runs): BRAVO, its call over, reported whistles (−10 Hz; −16 Hz) from the ALPHA–CHARLIE traffic |
| Busy channel: ALPHA calls BRAVO (in a link with CHARLIE) | PASS: ALPHA gave up after 111–118 s; their link and 8 KB file fine |
| Busy channel: no false whistles at ALPHA; CQ on top; ALPHA links later; no modem stuck | PASS ×4: ALPHA's CQ keyed 17.5 s, 5–7 s of it over their transmissions; nothing held it back |

What they found (product code, for the product's own fixes):

- **The CQ's CALL packet can go out in the wrong mode.** `step_cq` sends
  `CQFRAME` and queues the CALL broadcast frame at once. When the frame
  reaches Mercury while it is still sending the CQFRAME (DATAC16), Mercury
  sends it in DATAC16, cut to 14 bytes ("Discarding broadcast frame: size 14
  != expected 126" at every receiver). With three Mercuries on one computer
  it happened every time for the first CQ of the run. A Roundtable that
  isn't heard doesn't show under Heard. (Mercury: the switch back to the
  listen mode for broadcast TX is skipped while `trx == TX`.)
- **A Roundtable that has sped up can't be joined.** A JOIN is sent in
  DATAC3, and the host listens only in the table's mode. The first member
  that is strong enough makes the host speed up at once, so a second station
  joining a few seconds later is never heard.
- **Members report once, and the host forgets them.** A member's "hears the
  host at" figure is sent only in its JOIN, and the host counts only members
  heard in the last 5 minutes. A member that only listens drops out of the
  ramp after 5 minutes and the host speeds up past it.
- **A JOIN reports the last frame heard from anyone.** It takes Mercury's
  latest `SN`, which is of whatever frame came last: BRAVO reported +10 dB
  "hears the host", the SNR of CHARLIE's JOIN, while it heard ALPHA's frames
  at +20–25.
- **A station that answers a CQ automatically doesn't show it is calling**
  (`calling` stays empty, the whistle watch keeps running), and its Mercury
  kept calling for 139 s after losing a pileup, on top of the winner's link.
- **The whistle watch still mistakes other stations' traffic for a whistle
  now and then** (once in the pileup's link, once for BRAVO's CALL bursts at
  CHARLIE during `relay3`).
- **No listen-before-talk.** CQ (whistle and CQFRAME) and a call to a busy
  station go out on top of a link already on the frequency (Mercury's
  `BUSYDETECT` is not used). Their link survived on this bench.
- **Two calls at once hide each other.** Neither got through at first, so
  ALPHA's Mercury never said PENDING and the CQ went on to its next whistle
  and CQFRAME on top of both callers.
- **The relay hand-over is logged as delivered to the final station**: ALPHA's
  Log said "✓ Mail delivered to CHARLIE" when BRAVO took it.
- **Mercury's SNR depends on the mode measured.** At +33 dB on the ether,
  DATAC3 frames read +9…+12 dB and DATAC16 ones +20…+25 dB, so a table whose
  members report from DATAC3 frames never ramps beyond DATAC1.

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
