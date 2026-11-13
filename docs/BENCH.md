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
| `table_slow` | ALPHA hosts "Sunday Morning Net"; once BRAVO has joined, 12 dB of noise is added and ALPHA's ramp margin set from BRAVO's report, so the table drops from DATAC1 to DATAC15 (27-byte frames). A message ALPHA queues while the change is announced (cut for DATAC1) is split again and reaches BRAVO; BRAVO's reports (31 bytes with the name) still reach the host; a ~560-character message from BRAVO goes as several and all of it arrives. About 15 min. |
| `scanhard` | Scan speed on a hard band, 4 trials each, as `scan` does: fading (depth 0.5) at about +11 dB, about 0 dB and about −5 dB SNR (signals weakened by path loss, so the noise stays well under full scale); then +11 dB with an FT8-like signal (8 tones, 0.16 s symbols, 12.64 s of every 15 s) and a carrier inside the caller's passband and another FT8 signal in the next channel. Passes with at least 3/4 found (2/4 at −5 dB) and no lock on anything but ALPHA. The band's interference is `ether::Interferer`. About 20 min. |

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

## Fourth bug hunt: desktop, radio drivers, Roundtables, a hard scan (2026-11-13)

- **Desktop window** (`15da623`): mode, tune and the rig's own controls
  wait while linked or calling (the engine refuses them too, test
  `radio_controls_wait_for_the_link`); CQ waits while a call is going out,
  which has its own **Cancel the call**; Search, Lock on and the mail
  buttons wait at a table, a link, a call or a CQ; chat Send only while
  linked; Answer and Join on a Heard entry tune there first. Log toasts
  count lines by a sequence number (the 500-line log no longer hides new
  ones). Settings typed but unsaved are dropped on connect and disconnect.
  Sliders take typed values beyond their range.
- **Radio drivers**: per-model S-meter tables and IDs for Kenwood (TS-890S
  uses OM; TS-990S goes through Hamlib), the K3's VFO B refused, a refused
  unkey reported as an error; Yaesu ranges, split and levels by model, and
  the CAT port opened with two stop bits and RTS high, as RigHelm does
  (Yaesu's CP2105 radios: the Enhanced port, interface 0, is CAT). The
  managed rigctld stops a leftover copy, says when its port is taken, keeps
  a log, and reads `rigctl -l` by its columns.
- **Roundtables** (`1c6595f`): requests to resend take turns (never asked
  first, then the oldest ask: test `resend_requests_take_turns`); a member
  who leaves is forgotten by the assembler; a message too long for the
  slowest speed is split into new pieces; the ramp's backoff resets after
  10 min; leaving or joining clears the last table's queues.
- **Hard scan** (`scanhard`, opt-in): `scan` had no fading or interference
  and its "very weak" was +8 dB. The band can now carry interference
  (`ether::Interferer`: a carrier, or an FT8-like signal, 8 tones 6.25 Hz
  apart, 0.16 s symbols, 12.64 s of every 15 s), and `scanhard` runs 4 trials
  each with fading (depth 0.5) at +11, 0 and −5 dB SNR in 2500 Hz, then +11 dB
  with FT8 and a carrier inside the caller's passband and FT8 next door.
  The SNR line now says the noise bandwidth and remembers each station's
  last TX level (it printed −99 dBFS for a station that hadn't sent yet).
- **Heard frequency**: the hard scan caught BRAVO filing ALPHA's CQ under
  14098.300 kHz: the bench had retuned BRAVO for the next trial while ALPHA's
  CQFRAME was still on the air, and Mercury reported it after the move. Heard
  entries (and the Log line) now take the dial from 2 s before the report when
  the radio moved 300 Hz or more since (test
  `a_frame_heard_after_a_retune_keeps_its_frequency`); Answer and Join tune
  there. Both scan scenarios check every CQ heard from ALPHA is filed under
  ALPHA's frequency.

Why trial 0 of a scan is the slowest (7–11 s against 3–6 s): the search
always starts at the bottom of the segment (the bench's different starting
frequency per trial changes nothing; only the delay before the CQ does), so
in trial 0 it reaches ALPHA's channel just as the whistle starts, catches
part of it, and has to come round again (10 channels, about 6.5 s a pass).
On top of that, 1 % of 0.5 s dwells on noise score as a whistle by design
(`TrackDetector::found`), and checking one costs about 2 s: 3 in about 160
dwells in one `scanhard` run. Together they make the occasional 3/4.

**Results**: `cargo test` all pass (177 in the library); standard bench on
real Mercury **36/36** (all three `scan` conditions 4/4), `scanhard` **6/6**
(and 8/8 and 5/5 in two runs before it: every trial found, also at −5 dB with
fading, never a lock on the FT8 or the carrier), `table3` **10/10**; phone UI
tests 7/7 on the iPhone Simulator and the Android emulator. A CQFRAME
arriving after a retune is rare on the bench (once in the three `scanhard`
runs, before the fix); `a_frame_heard_after_a_retune_keeps_its_frequency`
covers it.

## Mercury's MFSK taken for whistles (2026-11-13)

Mercury 1.9.23 on the ether, Ubuntu on WSL 2, on top of `b2de518`:

| Set | Result |
|---|---|
| Standard two-station | **35/35** |
| Three-station + `watch_mfsk` (new) | **34/34**, `cq_pileup` included |
| `watch_mfsk` on the code before this fix | **1/3**: CHARLIE reported a whistle at +51 Hz from the table's frames, and its search locked onto the table to wait for a CQ |

`cq_pileup`'s "no false whistles once linked" failed twice on 11-13: the
station that lost the pileup reported whistles from the winner's link. Both
times that link had come up in **MFSK** (Mercury's control mode after a slow,
contested connect; the runs that passed linked in OFDM). MFSK sends one of 32
tones 31.25 Hz apart every 40 ms, hopped 13 tones a symbol, over 1016–1984 Hz:
the whistle's own band. Each bin is lit about as often as a whistle lights it,
so the scan detector, which levels away anything steady and any burst that
lights the whole band, takes it for a whistle at any strength (score 20–90 in
1.6 s, against 2.3 to stop). The matched filter then often found one chance
chirp pair, and the watch's "two looks in a row agree" didn't help: the looks
overlap by 0.6 s, so the same pair counts twice.

Mercury's own signals (`mercury -x fifo -t -m <mode>`, 20 s of each, noise
added for −5 to +30 dB): only MFSK and MFSK500 pass the scan; DATAC1, DATAC3,
DATAC4, DATAC15 and DATAC17 never do. With the old test the watch reported
whistles from MFSK500 at every strength (up to 5 in 20 s) and from MFSK at
+10 and +30 dB.

What tells them apart:

- **The chirp pairs line up.** A whistle's pairs all give the same offset:
  three or four within 2 Hz from −10 dB up (−12 dB: 3 of 4 looks). MFSK gives
  one pair, or two or three that disagree by 10–45 Hz.
- **A whistle owns its track.** Of the frames along the best track, the share
  whose strongest (levelled) cell is on it: whistles 0.6–1 from −5 dB up,
  MFSK 0.2–0.4 at any strength. This is what keeps a strong whistle with a
  strong carrier inside its sweep: the matched filter's clipping stands down
  when the whistle fills the band, and it measures nothing.

The watch now needs the pairs to line up, or a signal that owns its track.
Search, after retuning onto a stop, takes a strong signal (score 15 and up)
whose pairs don't line up and that doesn't own its track for data and moves
on ("Not a whistle on … after all") instead of waiting 45 s for a CQ: an MFSK
QSO used to hold it on every pass. The SDR wide search needs three agreeing
pairs in its 2.2 s check (whistles give five; 40/40 looks down to −12 dB);
before, 9 of 12 looks at MFSK traffic showed callers.

## Roundtables at the slowest speed (2026-11-13)

Mercury 1.9.23 on the ether, Ubuntu on WSL 2, on top of `3bc4b05`:

| Set | Result |
|---|---|
| Standard two-station | **35/35** |
| Slower (`cancel chat_during resume e2e_require away_long away_weak`) | **18/18** |
| `table_slow` (new) | **4/4** after the fixes below (its setup now adds noise until BRAVO's report lets a margin of 20 dB or less set DATAC15) |
| Three-station (`table3 relay3 cq_pileup busy3`) | **31/32**: `cq_pileup` "no false whistles once linked" failed in both of today's runs (CHARLIE, having lost the pileup, reported whistles at −4/+194 and −49/−87 Hz from the ALPHA–BRAVO traffic). Not touched by these changes; it passed on 11-12 before `341a375` made shown TX follow the radio, which may have been holding the whistle watch off after the losing call |

`table_slow` hosts "Sunday Morning Net" and pushes the table down to
DATAC15 (27-byte broadcast frames). Found and fixed:

- **Frames too big for the mode were cut by Mercury and lost.** A member's
  report with the table name (31 bytes), a request to repeat a whole
  message (30+), and a message made for a faster speed (queued, or kept to
  repeat, before the table slowed down: 120–500 bytes). Reports leave the
  name out, requests ask for fewer parts, and such a message is split again
  (same id, a different total; a receiver starts it over). At DATAC15 a
  long message goes as several, each with no more parts than a request can
  name; the host skips DATAC15 while a call of 8+ characters is at the table.
- **Frames piled up in Mercury and came out garbled after a speed change.**
  Mercury's frames take 0.2–0.6 s longer than `frame_s` (DATAC3 3.81 s,
  DATAC15 4.35 s, DATAC1 4.79 s, measured from its PTT), so a long message
  left frames queued in Mercury; it keeps them as one byte stream and after
  a `MODE` reads them at the new frame size. ALPHA's Mercury decoded every
  one of BRAVO's later frames (+10 dB) as `type=0` and dropped them. Table
  frames now go to Mercury one at a time, after its PTT OFF.
- **A member sending a long message left the table.** Four minutes of its
  own transmitting counted as "nothing heard"; it moved to DATAC3 and the
  rest went where the host wasn't listening.
- **Requests to repeat went out on top of what they asked for**, every 20 s
  while the message was still coming in, three at a time (13 s of air in
  20 at DATAC15), and each request queued the parts again (BRAVO sent 13
  frames a minute, mostly repeats). Now: only once the message has stopped
  coming and the table is quiet for two frames; a part already queued isn't
  queued again; heartbeats and reports also wait for a gap (a minute at
  most). The normal Roundtable message went from 40–65 s to 16 s.
- **The speed see-sawed.** On the same channel Mercury reads DATAC15 frames
  +16.6 dB and DATAC3 ones +10.1 dB, so the host sped up from DATAC15, the
  reports dropped, it slowed down (up, down, up within 6 min, catching a
  member mid-message). A speed-up undone within 5 minutes now doubles the
  wait before the next (2 → 32 min), and none is made while a member is
  sending.
- **A station's messages could show out of order** (the second of a long
  message complete before a repeated part of the first): later ones now wait
  up to 2 minutes for an earlier one from the same station.
- **Back-to-back requests to repeat hid the answers.** Missing parts of two
  messages, ALPHA sent two requests in a row every round; BRAVO answered the
  first at once, on top of the second, and ALPHA (transmitting) never heard
  it. One request at a time now.

Rebased onto upstream `4a3e03c` (its speed changes wait for Mercury's PTT
OFFs, honest reports, members' bandwidth): cargo test all pass (165 lib);
standard 35/35, `table3` 10/10; with the one-request fix, `table
table_slow` 6/6 and `table3` 10/10 again.

Also: Mercury closing while it had the radio keyed left TX shown for ever;
a grid setting over ~100 characters hung the station at its next Station ID.
## The rest of the third hunt's list (2026-11-13)

- **Roundtable speed changes** now switch when Mercury reports both
  announcements off the air (its PTT OFF lines, counting a table frame still
  on the air from before), with a later fallback when it doesn't say; before,
  a fixed timer could switch while the second announcement was still waiting.
- **Members' reports**: only the host's SNR actually measured (on its frames,
  or on the CALL a member joined on), else "unknown" (−128), which the host
  leaves out; before, a member reported the last 1:1 link's SNR or a made-up
  0 dB. The JOIN/report carries the member's bandwidth (a trailing byte older
  versions skip): the host never picks a mode wider than the narrowest
  member's (test `the_narrowest_member_caps_the_table`: without the cap the
  host went to QAM16C2, 2100 Hz, with a 500 Hz member). A member whose older
  host moves the table wider is warned.
- **Whistle offsets**: `fine_offset` picks among all the offsets a
  measurement could mean, so a caller at +1050 Hz isn't put at −950 (test
  `far_callers_alias_the_right_way`).
- **Simulated band**: the frequency shift's phase carries on across 50 ms
  blocks (a +333 Hz whistle measured 340 before; test
  `a_blockwise_shift_keeps_its_offset`).
- **Test band**: stopping it closes its connections and listeners (test
  `stop_closes_its_connections`).
- **Desktop Remote button**: one label in every state, the state in its
  colour (`084446b`). **iPhone**: a number sent as null reads as 0 instead of
  freezing the app (`456d91d`).
- **Flaky tests**: the six engine tests that time a search across a band
  simulated in real time now take turns (several at once starved each other:
  `sdr_hit_interrupts_stepping_search` heard "whistles" in the noise or timed
  out, `real_mercury_demo_search_link_chat` waited too long for Mercury). The
  library suite takes about 4.5 min instead of 1.5, and passed twice running;
  `sdr_hit_interrupts_stepping_search` also allows 150 s to link.

**Results**: `cargo test` all pass (156 in the library); standard bench on
real Mercury **35/35**, `table3` **10/10**, and `link table table3` 15/15 with
the PTT-timed speed changes; phone UI tests 7/7 on the iPhone Simulator and
the Android emulator.

Not reproduced: "scan speed, weak signal" missing 1 of 4. Three `scan`
runs after these changes found all 36 trials (normal, weak and very weak);
the earlier misses were at +14 dB, not at the weakest level, so it isn't
simply sensitivity. If it shows again, `bench-run/whistlestop-BRAVO.log` of
that run is the place to look.

## Third bug hunt: transmit safety, phones, search and audio (2026-11-12, night)

Four more read-throughs (rig/CAT/PTT, search and SDR, audio and modes, the
UI logic of all three apps), each finding checked against the code:

- **Never left transmitting** (`341a375`): a whistle whose sound output
  stopped held PTT for ever (playback now gives up 3 s after the sound's
  length, and the CQ unkeys); a failed unkey was never retried (now owed and
  retried every poll); a late rigctld reply was read as the next command's,
  giving wrong PTT results both ways (a fresh connection after any timeout,
  which also follows a restarted rigctld; test `rig::a_late_reply_is_not_the_next_commands`
  fails without it); TX shown stayed on after a hand-mic or tuner
  transmission; shutdown unkeyed before closing the port Mercury keys
  through; `T VFOA 0` or a bare `T` on the shared port keyed the radio.
- **Phones** (`2d874c7`): they couldn't see an outgoing call (Join even went
  through mid-CONNECT; the engine now refuses it too); Answer automatically
  saved every unsaved Settings edit; numbers typed in Settings were lost on
  iOS or fought on Android, and an out-of-range port failed Save silently;
  chat stopped scrolling after 500 lines; comma decimals; bands offered that
  Search refuses.
- **Search, SDR, audio** (`270ad77`): an SDR hit or Lock on retuned the radio
  off a Roundtable; a failed retune left a dead search shown as running; a
  failed sound device left audio green with a frozen waterfall; one NaN
  sample panicked the detectors; a custom wide segment over ~200 kHz showed
  phantom whistles 256 kHz away; a stalled RTL-SDR held its thread and the
  device; MFSK500's BITRATE code (103, not 100) and Mercury's "0 BPS"
  placeholder (ETAs of 22 h).
- **Bench**: the iPhone Simulator's and Android emulator's own app settings
  were left as the bench's stations (`0aee326`).

Not changed:
- (Fixed afterwards, below: the Roundtable speed-change timing, members'
  reports and bandwidth; the test band's threads; the simulated band's
  phase; `fine_offset`'s aliases; the Remote button; the iPhone's null
  numbers.)
- "scan speed, weak signal" misses 1 of 4 now and then (twice in five runs
  today, before and after these changes; 4/4 on the reruns).

## Closed tables, links at a table, remote backstop (2026-11-12, night)

Mercury 1.9.23 on the ether, Ubuntu on WSL 2, on top of `1c40080`:

| Set | Result |
|---|---|
| Standard two-station | **35/35** (34, plus the new "the host leaving closes it for the member") |
| `watch table` | 4/4 |
| `table3` | **10/10** |

Found and fixed:

- **A member stayed at a closed table for good.** When the host left (or
  went off the air), its members stayed, trying the table's speeds every
  4 minutes, with the schedule, Station ID and Depot automation waiting
  for them to leave. The host's LEAVE now closes the table for its members,
  and a member that hears nothing of its table for 15 minutes leaves. A
  member now knows the host from the CALL it joined on, so the goodbye is
  recognised even before the host's next CALL or SPEED. Bench: BRAVO left
  4–5 s after ALPHA did, and said why.
- **A 1:1 link at a table could be cut by a mode change.** A member that
  took an incoming link heard nothing of the table during it, so after
  4 minutes it took the table for lost and sent Mercury a `MODE` in the
  middle of the link. The table's clocks now wait while a link is up.
- **Remote logins:** the per-computer lockout table never forgot an address
  (someone changing address grew it without end); and as a backstop, 30
  wrong passwords within 10 minutes from all computers together pause
  logins from computers that haven't logged in this session for 5 minutes.
- Bench: the two-station Roundtable scenario turns listen-before-transmit
  off. Mercury's busy detector was still latched from the previous
  scenario's noise step, and the CQ waited out its 60 s, as long as the
  bench waits for its CALL (CALL heard false on the first run).

`soak_growth::transfer_list_is_bounded` failed once in a full `cargo test`
(201+ rows kept) and passed in three loaded reruns; its message now lists
the rows still open, should it come back.

## Second bug hunt (2026-11-12, evening)

A read-through of the protocol, encryption, Depot, settings, remote and
phone-bridge code (four reviewers, every finding checked against the code),
then fixes with a test each where one can be written. Made alongside the
"Busy-channel listening…", "Fix hand calls vs auto-answer…" and "Schedule
and Station ID leave a Roundtable alone…" commits, and merged onto them:
where both made the same fix (settings checks, Connect at a table, the
schedule and Station ID at a table, stale packets before CONNECTED), theirs
was kept.

- **File pieces before the answer were kept**: a station that sent a
  file's pieces without waiting for Accept got the file saved in your
  Downloads unasked. Pieces now count only once the offer is accepted
  (`file_pieces_before_the_answer_are_not_kept`, with a fake station that
  does exactly that; fails without the fix).
- **An empty file never finished** (nor one already fully on disk from an
  earlier try): no piece ever came to complete it. Accepting finishes it.
- **Roundtable messages vanished after a restart**: message numbers
  restarted at 1, and the other members ignore numbers they finished in the
  last 10 minutes. They now start at a random point, like file numbers.
- **Nothing kept a 1:1 link off a Roundtable**: Connect, Answer, Search, a
  chat CQ, the schedule (it retuned a host off its own table), Station ID
  and the Depot's automatic connects all went ahead at a table, and an
  incoming link then had the table's broadcasts and speed changes (MODE
  commands) cut into it. All are refused or greyed out at a table, and table
  traffic waits while a link is up. Calling CQ again for your own table no
  longer wipes its members and lines. Resend requests from the last table
  aren't sent to the next.
- **Answering a Roundtable CQ automatically linked 1:1 with its host**
  instead of joining the table (found by the phone UI test: the simulated
  K7BOT did the same, so hosting a table in the demo got a 1:1 link).
  Auto-answer now joins (`auto_answer_joins_a_roundtable_cq`, fails without
  the fix), and the simulated bot no longer connects to a table's CQ.
- **Mercury closing mid-link left the link half alive**: keys, queues and
  the peer stayed, so a reconnect started with a dead encrypted session.
  It's now ended as a disconnect. Packets left over from a previous link
  (or half of one) are no longer fed into the next link's security layer.
- **Encryption failures**: the header kept saying "Encrypted" while the
  link was being dropped; a station's plain goodbye explaining the failure
  was itself reported as "unencrypted packet"; and "the other station has
  encryption off" stated as fact what may have been tampering ("didn't
  offer encryption").
- **Settings out of range crashed the station**: a negative CQ listen time
  or an infinite beacon interval (by hand in the file, or from a remote
  operator) panicked when used. (Fixed upstream too; its `sanitize` is
  kept.) Added here: the settings file is also checked when it's loaded,
  and it's written to a temporary file and moved into place.
- **An unreadable `depot.json` was overwritten with an empty Depot** on the
  next mail. It's now left alone and reported.
- **Depot automation**: a delivery link to a station that never confirms
  stayed up indefinitely (now 60 s idle); another away station's automatic
  away message was filed as mail.
- **Library**: a peer could make the station hold any number of copies of
  a library file in memory by asking repeatedly; at most 3 at a time now.
  A received file named `.partial` broke resume for good; names starting
  with a dot now get a `_`.
- **Remote operation**: ten wrong passwords from anyone locked the owner out
  too, and connections that never logged in could fill every slot. The
  lockout is now per computer, with at most 2 logins in progress from one.
  Every reconnect copied all received files again (`qsl.txt`, `qsl-2.txt`
  …); a record in the Download folder stops that. A client that stopped
  answering was never dropped. Connecting from the desktop froze the window
  for up to ~15 s on a wrong address; it now connects in the background.
- **Demo left Mercury running**: a Whistlestop that ended without quitting
  (a crash, a force-quit, a test run) left its two demo Mercurys running
  for good; 20 had piled up here, using 32 % CPU together. The next Demo
  start now stops those of any Whistlestop that's gone (checked by killing
  a run mid-way: 2 orphans, then 0 after the next start).

Not changed: the remote login proof (HMAC of a challenge) can be cracked
offline by someone who records a login on the same network; the guide
already says the stream isn't encrypted and the port must stay on the LAN.


Fixed right after: a `Tnc::command` that timed out could have its late
reply taken by the next command (a CONNECT's late OK made the next MODE look
accepted, and MODE's own BUSY then answered the command after). Replies a
timed-out command still owes are now skipped, for up to 30 s
(`a_late_reply_is_not_the_next_commands`, fails without the fix). And on a
phone, a Mercury start still waiting for the radio when the station was
stopped started Mercury anyway, and blocked the next start: a stop now
cancels it.

**Results after the fixes** (before the merge): the standard bench on
two real Mercurys 33/34 (the miss, "scan speed, weak signal" 3/4 while this
computer was compiling, passed 4/4 alone right after); after the auto-answer
fix `watch link chat table e2e` 11/11 and `link chat table` 6/6; phone UI
tests 7/7 on the iPhone Simulator and 7/7 on the Android emulator.

**After merging onto the upstream commits**: the full standard bench on two
real Mercurys **34/34**; phone apps rebuilt, UI tests **7/7** on the iPhone
Simulator and **7/7** on the Android emulator; `cargo test` all pass
(including the soak suites and the desktop window tests) except
`sdr_hit_interrupts_stepping_search`, which fails now and then when the whole
library suite runs at once, upstream's code without these changes included,
and passes on its own (3/3).

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

Fixed since: a "measure again" hop could chase stale audio. A look right
after a retune that read 25 Hz or more off moved by that much, but audio
from before the retune shows the whistle where it was, so the move retuned
twice, and a silent next look then waited for the CQ there. Search now
listens again in place first and moves only if two looks agree.

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
| `watch_mfsk` | ALPHA hosts "MFSK Net", BRAVO joins, noise is added until BRAVO's report lets a 20 dB ramp margin take the table down to MFSK (Mercury's slowest mode: one tone at a time hopping over the whistle's own 1–2 kHz). CHARLIE, parked on the frequency and not at the table, must report no whistles while ALPHA and BRAVO each send a message; then CHARLIE searches the band while ALPHA sends another, and must not lock onto the table to wait for a CQ (it may stop to look, and should know it for data). About 10 min. |

`... bench-run table3 relay3 cq_pileup busy3` takes about 45 min, `watch_mfsk` 10 more.

## Busy channel, Roundtable sync, merged with the phone work (2026-11-12)

Mercury 1.9.23 on the ether, Ubuntu on WSL 2, after merging `cde492a`
(phones, deferred auto-answer, pending joins, bench by station kind):

| Set | Result |
|---|---|
| Standard two-station | 31/34. The three scan-speed tests failed because of a bench bug: `update` starts from each station's own settings every time, so every scan trial turned listen-before-transmit back on, and ALPHA's CQ waited out Mercury's busy reading. Fixed. Rerun of `watch link scan`: 7/8. Scan speed normal 4/4 (median 4.9 s), weak 4/4 (4.8 s), very weak 3/4 (7.6 s). The miss came after a false alarm on an empty channel at +5 dB. |
| `cancel chat_during resume e2e_require away_long away_weak` | 18/18 |
| Three-station (`table3 relay3 cq_pileup busy3`) | **32/32** (from 26/32 earlier in the day, and 28/32 on 11-11) |

Found and fixed on the way (real Mercury, then the fake one):

- **Automatic deliver/collect never happened with listen-before-transmit
  on.** The Station ID that sets one off has only just ended, so Mercury's
  busy detector still reads it; the call was skipped and not retried. It
  now waits for the channel to clear (90 s at most).
- **Mercury's busy detector sticks on a sudden noise step.** The bench steps
  the noise up 22–28 dB at once, and the detector then read BUSY for
  minutes (a 60 s wait for every CQ). Whistlestop's 60 s cap lets the CQ go
  anyway. The scan test turns listen-before-transmit off. Turning the
  setting off now also drops a BUSY already latched, and sends
  `BUSYDETECT OFF`.
- **A weak Roundtable member lost the table.** The host slowed down for it,
  but announced that at the faster speed, which it couldn't copy: host and
  member on different modes for 7 minutes, LEAVE unheard, the next join
  unheard. Now:
  - a member whose report says it can't copy the speed moves to DATAC3
    itself when no announcement comes within 40 s;
  - the host sends a heartbeat (SPEED, with its newest message id) when
    quiet for 90 s;
  - a member hearing nothing for 4 minutes tries the other likely modes;
  - a member the host may not know re-sends its JOIN at the join window's
    CALL.
- **A one-frame Roundtable message lost whole was never asked for.** Gaps in
  a sender's message ids (or the heartbeat's) are now asked for with AGAIN,
  at most 3 requests every 20 s.
- **The first join window came 40 s after the table first sped up**; now a
  full 3 minutes.
- **Connect by hand was refused while an auto-answer waited** (it should
  replace it), and an automatic call's 60 s give-up stayed armed after
  Disconnect, so it could ABORT the next call made by hand.
- The GUI click test `click_library_and_qsy_row` checked for the QSY row
  before a frame had drawn the new link (a race in the test).

Native Windows: Smart App Control blocks the build scripts of a fresh
build, so the Windows target was checked from WSL with
`cargo check --target x86_64-pc-windows-msvc`.

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
