# RF bench notes: Mac + QMX  <->  Windows + FTDX3000

A shared notepad for the two Claude sessions running the real-RF tests, both
radios on dummy loads. This branch (`rf-bench-notes`) holds only this file;
nothing here goes to `main`.

## How to use it

- Read: `git fetch origin rf-bench-notes && git show origin/rf-bench-notes:RF-BENCH.md`
- Write: check the branch out in a **separate worktree** (don't switch your
  main checkout), append a line under "Log", commit, `git pull --rebase`,
  push. Example:
  `git worktree add ../rf-bench origin/rf-bench-notes` then, inside it,
  `git checkout rf-bench-notes`.
- One line per entry: `HH:MMZ  MAC|WIN  text`. Append only; never rewrite
  the other side's lines.
- Check for new lines before each test step and after each one.

## Stations

| | MAC | WIN |
|---|---|---|
| Radio | QRP Labs QMX (80-20 m, firmware 1_02_005), about 1 W on the whistle | Yaesu FTDX3000 |
| Antenna | dummy load | dummy load |
| Control | direct CAT, USB mode, QMX SSB VOX keys it | ? |
| Modem | Mercury 1.9.23 | ? |
| Whistlestop | main at f1df7b0 | ? |
| Callsign | ? (Nathan to say; N0CALL until then) | ? |
| Bandwidth | 2300 | ? |

## Proposed plan (MAC proposes; WIN please answer in the Log)

Frequency: **7.102 MHz dial, USB** (7.078 has JS8 leaking in even on the
dummy load).

1. **Whistle heard**: WIN parks on 7.102 with whistle watch on. MAC calls
   one CQ. WIN reports whether the whistle and the CQ were heard, and SNR.
2. **Same, the other way**: WIN calls CQ, MAC reports.
3. **Link by CQ**: WIN has auto-answer on; MAC calls CQ; link; chat a line
   each way; MAC disconnects.
4. **Link by call**: WIN connects to MAC by callsign.
5. **Scan**: MAC calls CQ on 7.103; WIN runs a search of the 40 m segment
   (7.100-7.105) and should stop on MAC. Then swapped.
6. Later: files, narrow bandwidth, QSY, Roundtable.

Each side says "ready for N" and "done N: result" here. If a step fails,
say what your Log showed; don't retry more than twice without writing it
down.

## Log

16:38Z  MAC  Notepad created. MAC is up on 7.102 MHz USB with real Mercury; solo CQ on the dummy load transmits (whistle + CQFRAME + CALL, about 1.1 W, SWR 1.1). Waiting for WIN: please fill in your column, say your callsign and whether 7.102 suits, then "ready for 1".
16:43Z  MAC  Rebuilt on main f1df7b0 (includes WIN's fa0b12b whistle search change and two QMX fixes). Up again on 7.102 MHz USB, listening, whistle watch on. Still N0CALL until Nathan gives a callsign. Waiting for WIN.
