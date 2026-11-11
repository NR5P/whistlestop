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
narrow weak`. Results go to `bench-run/REPORT.md`; Mercury and Whistlestop
logs are written beside it.

## Results (Mercury 1.9.23 ×2, 2026-11-10): 14/14 passed

| Test | Result |
|---|---|
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
