# Whistle lab: what lets a scanner find a caller fastest?

`cargo run --release --bin whistlelab` (about 3 minutes; `-- quick` for a rough
pass). Raw numbers go to stdout as CSV, the summary to stderr.

## The question

A scanning station hops through a digital segment, dwelling a short time on
each channel. The caller's attention signal ("whistle") must be caught in
**one dwell**: missing it means passing over the caller. The faster a scanner
can hop (shorter dwell, wider channel step), the shorter the whistle needs to
be. Does a faster, narrower, back-and-forth whistle help, or no sweep at all,
or a special pulse pattern?

## Method

Ten candidates, all constant-envelope at the same power:

| | design | width | repeat period |
|---|---|---|---|
| A | **today**: chirps 500→2500→500 Hz, 0.5 s each way | 2000 Hz | 1.0 s |
| B | fast wide chirps, 0.1 s each way | 2000 Hz | 0.2 s |
| C | fast narrow warble, 0.1 s each way | 400 Hz | 0.2 s |
| D | slow narrow warble, 0.5 s each way | 400 Hz | 1.0 s |
| E | steady tone | — | — |
| F | Costas 7-tone pulse (FT8's pattern), 40 ms, 25 Hz | 175 Hz | 0.28 s |
| G | fast up-sweeps only (no back-and-forth) | 2000 Hz | 0.1 s |
| H | tone pulses 0.1 s on / 0.1 s off | — | 0.2 s |
| I | **medium chirps, 0.2 s each way** | 1000 Hz | 0.4 s |
| J | Costas 7-tone pulse, 20 ms, 50 Hz | 350 Hz | 0.14 s |

Each was sent over:
- **an ITU-R F.1487 "moderate" HF path**: two Rayleigh-fading paths, 1 ms apart, about 1 Hz Doppler spread. AWGN was also run.
- **into a busy band**: 1–3 FT8 signals, and sometimes a carrier, RTTY, PSK31 or a 16–33-carrier OFDM modem, each at −15 to +10 dB.

The scanner is an SSB receiver (300–2700 Hz). It doesn't know the caller's frequency offset (up to ±1200 Hz) or where in the pattern the caller is.

**Detectors:**
- Every design gets a **non-coherent time–frequency track detector**: a short-frame spectrogram, whitened, with energy summed along the design's expected track for every timing and offset. Being non-coherent, it survives fading.
- **A\*** is today's Whistlestop detector: the coherent matched filter in `whistle.rs`.
- Every detector's threshold is set from busy-band dwells *without* a whistle, so that **every design false-alarms on 1 % of dwells**. A false alarm means a needless stop.

Results use 80 trials per point and 500 for each threshold. SNR is in 2500 Hz, as for FT8.

## Results

Probability of catching the whistle in **one dwell**, caller on the channel's centre, moderate fading:

| design | 0.15 s | 0.25 s | 0.5 s | 1 s | 2 s |
|---|---|---|---|---|---|
| **−10 dB** | | | | | |
| A today's whistle, track detector | 25 % | 59 % | 75 % | 94 % | 99 % |
| A\* today's whistle, **today's detector** | 0 % | 0 % | 3 % | 95 % | 100 % |
| B fast wide | 30 % | 41 % | 54 % | 81 % | 94 % |
| C fast narrow warble | 15 % | 53 % | 54 % | 74 % | 93 % |
| F Costas 40 ms | 1 % | 3 % | 44 % | 76 % | 93 % |
| G up-sweeps only | 36 % | 39 % | 60 % | 81 % | 98 % |
| **I medium chirps** | 28 % | 53 % | 69 % | **93 %** | 99 % |
| E steady tone | 33 % | 40 % | 25 % | 29 % | 23 % |
| H tone pulses | 39 % | 40 % | 36 % | 40 % | 39 % |
| **−5 dB** | | | | | |
| A today's whistle, track detector | 63 % | 89 % | 95 % | 99 % | 100 % |
| A\* today's whistle, **today's detector** | 0 % | 0 % | 3 % | 99 % | 100 % |
| B fast wide | 71 % | 79 % | 89 % | 99 % | 100 % |
| C fast narrow warble | 45 % | 74 % | 85 % | 96 % | 99 % |
| F Costas 40 ms | 6 % | 34 % | 84 % | 95 % | 100 % |
| G up-sweeps only | 74 % | 81 % | 96 % | 99 % | 100 % |
| **I medium chirps** | 73 % | **90 %** | 96 % | 99 % | 100 % |
| E steady tone | 73 % | 85 % | 76 % | 76 % | 91 % |

Fastest scan that still catches ≥ 90 % in one dwell (moderate fading, 1 % false stops). The channel step is the widest whose every offset still meets 90 %, up to 2400 Hz:

| SNR | best designs | scan speed | whistle long enough for a scanner to cover 12 kHz |
|---|---|---|---|
| −15 dB | only A\* (today's detector), 2 s dwell, 400 Hz step | 0.2 kHz/s | ~60 s |
| −10 dB | **I medium chirps**, 1 s dwell, 2000 Hz step | 2.0 kHz/s | ~7 s |
| −5 dB | **I medium chirps**, 0.5 s dwell, 2400 Hz step | 4.8 kHz/s | ~3 s |
| 0 dB | **I**, B, G (fast chirps), 0.25 s dwell | 9.6 kHz/s | ~2 s |

For comparison, today's app scans with a 2.2 s dwell and an 1800 Hz step (0.8 kHz/s), and its CQ whistle lasts 24 s.

## What it shows

1. **The detector, not the waveform, is what slows today's scanner.**
   - Today's coherent matched filter must hold a whole 0.5 s chirp at the right moment. It **can't decide in a dwell of 0.5 s or less, even at 0 dB** (0 % in the table), so the scanner sits 2.2 s on every channel.
   - A non-coherent time–frequency detector catches the *same* whistle 89 % of the time in a quarter-second dwell at −5 dB.
   - The coherent filter is better only at the very weakest signals (−15 dB), and only with long dwells.
2. **Steady tones and on/off pulses are the worst choice.** The band is full of carriers and FT8 tones. To avoid stopping on them, the threshold must be so high that a real whistle is caught only 23–40 % of the time at −10 dB, however long the dwell. Military ALE, DSC and commercial selcall all avoid bare tones for the same reason.
3. **Back-and-forth (up then down) beats one direction.**
   - Up-sweeps only (G) is just as easy to *detect*.
   - But a one-way chirp can't tell "a bit later" from "a bit higher in frequency". The up+down pair separates the two, which is how the caller's exact offset is read and the scanner retunes precisely. LoRa uses down-chirps for the same reason.
4. **Faster and narrower doesn't help on its own.**
   - The narrow warbles (C, D) and Costas patterns (F, J) are no better than chirps. A partly captured wide chirp still detects at a wide channel step, so narrowness buys little extra step. Meanwhile, energy spread over short narrow tones is easier to lose in fading and FT8 clutter.
   - Costas patterns need long symbols to be sensitive, which is why FT8's are 160 ms, and long symbols make them slow.
5. **The repeat period should be well inside the dwell.**
   - Fast-repeating chirps (0.1–0.2 s legs) work in 0.25–0.5 s dwells. The 1 s pattern of today's whistle can't fit.
   - This matches every scanning system in the field. ALE repeats a 131 ms word against 200–500 ms dwells. DSC sends a 2 s dot pattern. Barrett's preamble is 0.5 s × the number of channels + 1 s.
6. **Physics still rules weak signals.** Detection needs a certain amount of whistle energy in the dwell. At −15 dB no shape can be caught in under about a second, so a scanner that wants weak callers must dwell longer.

## Built: whistle v2 (2026-11-11)

The recommendation below is now Whistlestop's whistle (`whistle.rs`):
1000→2000 Hz chirps of 0.2 s each way, a non-coherent scan detector
(`TrackDetector`) with thresholds from `whistlelab calibrate`, a 0.5 s
dwell that stretches to 2 s on a hint, a 2000 Hz step, and a 10 s CQ whistle.

On the two-station real-Mercury bench (`docs/BENCH.md`), it found the caller
12 times out of 12, at medians of 5–8.5 s. The first whistle took 14.5 s,
with a whistle 2.4× as long.

Things the simulation didn't show and the bench did:
- strong signals crossing wrong tracks (fixed with a cell-count rule);
- data bursts lighting every bin (fixed with per-frame whitening);
- callers just past the passband edge (fixed by walking to them).

## Recommendation (as made before building)

- **Whistle v2**: 1000 Hz wide (1000→2000→1000 Hz audio), 0.2 s each way, repeated. It is the best or tied-best design at every signal level here. It's half as wide as today's whistle, so it takes less of the channel from others, and it still resolves the offset by the up/down pair (±500 Hz unambiguous; the alias check covers the rest).
- **A time–frequency track detector** in place of the coherent matched filter for scanning. Keep the matched filter for the fine lock once the scanner has stopped.
- **Two-stage dwell**, as ALE scanners "pause" on a promising channel:
  - dwell 0.5 s;
  - if the score shows a hint of a whistle, stay up to 2 s to confirm.

  Fast where nothing is there, patient where something might be. This is untested here; the lab can measure it next.
- **Whistle length = one scan cycle plus margin**, as ALE and selcall do: about 6–8 s at a 0.5 s dwell, instead of 24 s.

## Sources

- MIL-STD-188-141 scanning call rules and 2 / 5 channels/s scan rates: NTIA/ITS FTSC test plan, https://its.ntia.gov/publications/download/96-327.pdf; ALE signal, https://www.sigidwiki.com/wiki/ALE
- ITU-R M.493 DSC dot pattern for scanning receivers: https://law.resource.org/pub/us/cfr/ibr/004/itu-r.M-493-11.2004.pdf
- Barrett selcall preamble = 0.5 s × channels + 1 s: https://www.manualslib.com/manual/1625112/Barrett-2030.html?page=131
- FT8/FT4 Costas sync and fading thresholds (Franke, Somerville, Taylor, QEX 2020): https://downloads.hamradiodeluxe.com/FT4_FT8_QEX.pdf
- Costas arrays' "thumbtack" ambiguity: https://encyclopediaofmath.org/wiki/Costas_array
- Chirp time–frequency coupling, LoRa up/down chirps: https://en.wikipedia.org/wiki/Ambiguity_function, https://arxiv.org/pdf/2412.13930
- HF channel conditions, ITU-R F.1487: https://www.itu.int/dms_pubrec/itu-r/rec/f/R-REC-F.1487-0-200005-I!!PDF-E.pdf

Caveats: this is a simulation, with 80 trials per point (±4–5 % on each probability). The band clutter is a model, not a recording, and the non-coherent detector is one reasonable design, not the best possible.
