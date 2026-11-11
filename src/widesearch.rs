//! Wide whistle search: find CQ whistles anywhere in a band segment at once,
//! from the RTL-SDR's wide IQ stream, without stepping the main radio.
//!
//! 1. **Decimate**: mix the segment's centre to 0 Hz and low-pass/decimate
//!    the dongle's 2.048 MS/s down to `fs` (64 kHz for most segments).
//! 2. **Channelize**: once a second, FFT the last 2.2 s and cut out a virtual
//!    USB receiver (dial + 300..2700 Hz, 8 kHz audio) every `step` Hz across
//!    the segment. Run the normal whistle detector on each.
//! 3. **Verify for free**: re-cut a virtual receiver centred exactly on each
//!    candidate. A real caller reads ~0 Hz there; an alias (a caller beyond
//!    +/-1000 Hz of the channel that saw it) doesn't. No retune needed.
//!
//! The dongle's crystal error (often 20-50 ppm) shifts every hit by the same
//! few hundred Hz, so the engine hands a hit to the main radio, whose own
//! audio fine-tunes the lock before Mercury needs it (+/-55 Hz).

use crate::bandplan::{self, Segment};
use crate::whistle::{Detector, FS, WhistleParams};
use num_complex::{Complex32, Complex64};
use rustfft::FftPlanner;
use std::collections::VecDeque;
use std::f64::consts::PI;

/// Mix by `-shift_hz`, low-pass, keep every `decim`-th sample.
pub struct Decimator {
    decim: usize,
    taps: Vec<f32>,
    /// History written twice so the newest `taps.len()` samples are always
    /// one contiguous slice: hist[idx .. idx + L].
    hist: Vec<Complex32>,
    idx: usize,
    count: usize,
    phasor: Complex64,
    step: Complex64,
    since_norm: u32,
}

impl Decimator {
    pub fn new(fs_in: f64, decim: usize, shift_hz: f64) -> Self {
        let fs_out = fs_in / decim as f64;
        let n = 8 * decim + 1;
        let fc = 0.42 * fs_out / fs_in; // cycles/sample
        let m = (n - 1) as f64 / 2.0;
        let mut taps: Vec<f64> = (0..n)
            .map(|i| {
                let x = i as f64 - m;
                let s = if x == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * x).sin() / (PI * x) };
                let w = 0.42 - 0.5 * (2.0 * PI * i as f64 / (n - 1) as f64).cos() + 0.08 * (4.0 * PI * i as f64 / (n - 1) as f64).cos();
                s * w
            })
            .collect();
        let g: f64 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= g);
        Self {
            decim,
            taps: taps.into_iter().map(|t| t as f32).collect(),
            hist: vec![Complex32::new(0.0, 0.0); 2 * n],
            idx: 0,
            count: 0,
            phasor: Complex64::new(1.0, 0.0),
            step: Complex64::from_polar(1.0, -2.0 * PI * shift_hz / fs_in),
            since_norm: 0,
        }
    }

    pub fn process(&mut self, x: &[Complex32], out: &mut Vec<Complex32>) {
        let l = self.taps.len();
        for &s in x {
            let m = s * Complex32::new(self.phasor.re as f32, self.phasor.im as f32);
            self.phasor *= self.step;
            self.since_norm += 1;
            if self.since_norm >= 4096 {
                self.phasor /= self.phasor.norm();
                self.since_norm = 0;
            }
            self.hist[self.idx] = m;
            self.hist[self.idx + l] = m;
            self.idx = (self.idx + 1) % l;
            self.count += 1;
            if self.count == self.decim {
                self.count = 0;
                let w = &self.hist[self.idx..self.idx + l];
                let mut acc = Complex32::new(0.0, 0.0);
                for (a, &t) in w.iter().zip(&self.taps) {
                    acc += a * t;
                }
                out.push(acc);
            }
        }
    }
}

/// Output rate for a segment: the smallest of 64/128/256 kHz whose usable
/// 80 % holds the segment plus a whistle's width on each side.
pub fn rate_for(seg: &Segment) -> u32 {
    let need = seg.hi_hz - seg.lo_hz + 6000.0;
    [64_000u32, 128_000, 256_000].into_iter().find(|&r| need <= 0.8 * r as f64).unwrap_or(256_000)
}

#[derive(Clone, Debug, PartialEq)]
pub struct WideHit {
    /// The caller's USB dial frequency, as the SDR sees it (before the main
    /// radio fine-tunes it).
    pub dial_hz: f64,
    pub score: f64,
    pub band: String,
}

/// Watches one segment: holds the last few seconds of its decimated stream
/// and scans it for whistles.
pub struct WideWatch {
    pub seg: Segment,
    pub center_rf: f64,
    pub fs: u32,
    ring: VecDeque<Complex32>,
    fresh: usize,
    det: Detector,
    params: WhistleParams,
    step_hz: f64,
    planner: FftPlanner<f32>,
}

const WINDOW_S: f64 = 2.2;

impl WideWatch {
    pub fn new(seg: Segment, params: WhistleParams, step_hz: f64) -> Self {
        let center_rf = 0.5 * (seg.lo_hz + seg.hi_hz) + 1500.0;
        Self {
            fs: rate_for(&seg),
            seg,
            center_rf,
            ring: VecDeque::new(),
            fresh: 0,
            det: Detector::new(params),
            params,
            step_hz,
            planner: FftPlanner::new(),
        }
    }

    pub fn push(&mut self, y: &[Complex32]) {
        self.ring.extend(y.iter().copied());
        self.fresh += y.len();
        let keep = (3.0 * self.fs as f64) as usize;
        if self.ring.len() > keep {
            let extra = self.ring.len() - keep;
            self.ring.drain(..extra);
        }
    }

    pub fn reset(&mut self) {
        self.ring.clear();
        self.fresh = 0;
    }

    /// A scan is due once a second, when a full window is held.
    pub fn due(&self) -> bool {
        self.fresh >= self.fs as usize && self.ring.len() >= (WINDOW_S * self.fs as f64) as usize
    }

    /// Dial frequencies of the virtual receivers covering the segment.
    pub fn channels(&self) -> Vec<f64> {
        bandplan::channels(&self.seg, self.step_hz, self.params.f_lo - 200.0, self.params.f_hi + 200.0)
    }

    /// Cut 8 kHz USB receiver audio for `dial` out of the window spectrum.
    fn channel_audio(&mut self, spec: &[Complex32], dial: f64) -> Vec<f64> {
        let n = spec.len();
        let m = n * FS as usize / self.fs as usize;
        let bin = self.fs as f64 / n as f64;
        let k0 = ((dial - self.center_rf) / bin).round() as i64;
        let mut y = vec![Complex32::new(0.0, 0.0); m];
        for (i, v) in y.iter_mut().enumerate().take(m / 2) {
            let f = i as f64 * bin;
            if (crate::whistle::PASSBAND.0..=crate::whistle::PASSBAND.1).contains(&f) {
                *v = spec[(k0 + i as i64).rem_euclid(n as i64) as usize];
            }
        }
        self.planner.plan_fft_inverse(m).process(&mut y);
        let n8 = (WINDOW_S * FS as f64) as usize;
        y.iter().take(n8).map(|c| c.re as f64).collect()
    }

    pub fn scan(&mut self) -> Vec<WideHit> {
        self.fresh = 0;
        let n_win = (WINDOW_S * self.fs as f64) as usize;
        if self.ring.len() < n_win {
            return vec![];
        }
        let n = n_win.next_power_of_two();
        let mut spec: Vec<Complex32> = self.ring.range(self.ring.len() - n_win..).copied().collect();
        spec.resize(n, Complex32::new(0.0, 0.0));
        self.planner.plan_fft_forward(n).process(&mut spec);

        // 1. Every channel.
        let mut candidates: Vec<f64> = vec![];
        for d in self.channels() {
            let a = self.channel_audio(&spec, d);
            if let Some(r) = self.det.detect(&a) {
                let c = d + r.offset_hz;
                if !candidates.iter().any(|x| (x - c).abs() < 20.0) {
                    candidates.push(c);
                }
            }
        }
        // 2. Verify each candidate with a receiver centred on it.
        let mut hits: Vec<WideHit> = vec![];
        for c in candidates {
            let a = self.channel_audio(&spec, c);
            if let Some(v) = self.det.detect(&a)
                && v.offset_hz.abs() < 30.0 {
                    let dial = c + v.offset_hz;
                    match hits.iter_mut().find(|h| (h.dial_hz - dial).abs() < 60.0) {
                        Some(h) if h.score >= v.score => {}
                        Some(h) => {
                            h.dial_hz = dial;
                            h.score = v.score;
                        }
                        None => hits.push(WideHit { dial_hz: dial, score: v.score, band: self.seg.band.clone() }),
                    }
                }
        }
        hits
    }
}

/// A caller's whistle as complex baseband: `dial_off` is its dial minus the
/// stream's centre frequency. `t0` is the time into the whistle at sample 0.
pub fn complex_whistle(fs: f64, n: usize, dial_off: f64, t0: f64, p: &WhistleParams, phase: &mut f64) -> Vec<Complex32> {
    (0..n)
        .map(|i| {
            let t = t0 + i as f64 / fs;
            let q = t.rem_euclid(p.period_s());
            let fa = if q < p.chirp_s { p.f_lo + p.k() * q } else { p.f_hi - p.k() * (q - p.chirp_s) };
            *phase += 2.0 * PI * (dial_off + fa) / fs;
            Complex32::new(phase.cos() as f32, phase.sin() as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use rand_distr::{Distribution, Normal};

    fn seg20() -> Segment {
        Segment { band: "20m".into(), lo_hz: 14_100_500.0, hi_hz: 14_112_000.0 }
    }

    fn noise(n: usize, sigma: f32, rng: &mut StdRng) -> Vec<Complex32> {
        let d = Normal::new(0.0f32, sigma).unwrap();
        (0..n).map(|_| Complex32::new(d.sample(rng), d.sample(rng))).collect()
    }

    #[test]
    fn decimator_passes_in_band_and_rejects_far_signals() {
        let fs = 2_048_000.0;
        let tone = |f: f64| -> Vec<Complex32> {
            (0..fs as usize / 4).map(|i| Complex32::from_polar(1.0, (2.0 * PI * f * i as f64 / fs) as f32)).collect()
        };
        let power = |f: f64| -> f32 {
            let mut d = Decimator::new(fs, 32, 100_000.0);
            let mut out = vec![];
            d.process(&tone(f), &mut out);
            out[200..].iter().map(|c| c.norm_sqr()).sum::<f32>() / (out.len() - 200) as f32
        };
        assert!((power(110_000.0) - 1.0).abs() < 0.05, "in band {}", power(110_000.0));
        assert!(power(400_000.0) < 1e-4, "far {}", power(400_000.0)); // > 40 dB down
        assert!(power(100_000.0 + 48_000.0) < 1e-3); // just outside: 30 dB down
    }

    #[test]
    fn finds_callers_anywhere_in_the_segment_without_aliases() {
        let p = WhistleParams::default();
        let mut w = WideWatch::new(seg20(), p, 900.0);
        let fs = w.fs as f64;
        let mut rng = StdRng::seed_from_u64(3);
        // Two callers: one between channels, at an offset that aliases from
        // its neighbours, and one weak.
        let callers = [(14_103_150.0, 0.3f32), (14_108_420.0, 0.08f32)];
        let n = (3.0 * fs) as usize;
        let mut x = noise(n, 0.05, &mut rng);
        for (dial, amp) in callers {
            let mut ph = 0.0;
            for (s, c) in x.iter_mut().zip(complex_whistle(fs, n, dial - w.center_rf, 0.13, &p, &mut ph)) {
                *s += c * amp;
            }
        }
        // A carrier, to be ignored.
        for (i, s) in x.iter_mut().enumerate() {
            *s += Complex32::from_polar(0.3, (2.0 * PI * (14_105_000.0 - w.center_rf) * i as f64 / fs) as f32);
        }
        w.push(&x);
        let mut hits = w.scan();
        hits.sort_by(|a, b| a.dial_hz.partial_cmp(&b.dial_hz).unwrap());
        assert_eq!(hits.len(), 2, "{hits:?}");
        for (h, (dial, _)) in hits.iter().zip(callers) {
            assert!((h.dial_hz - dial).abs() < 3.0, "{} vs {dial}", h.dial_hz);
        }
    }

    #[test]
    fn quiet_band_has_no_hits() {
        let mut w = WideWatch::new(seg20(), WhistleParams::default(), 900.0);
        let mut rng = StdRng::seed_from_u64(4);
        for _ in 0..4 {
            w.push(&noise((2.2 * w.fs as f64) as usize, 0.05, &mut rng));
            assert!(w.scan().is_empty());
        }
    }

    #[test]
    fn full_pipeline_from_dongle_rate_iq() {
        // 2.048 MS/s from a dongle tuned to 14.2 MHz; caller at 14.1037 MHz.
        let p = WhistleParams::default();
        let seg = seg20();
        let mut w = WideWatch::new(seg, p, 900.0);
        let fs_in = 2_048_000.0;
        let dongle = 14_200_000.0;
        let mut dec = Decimator::new(fs_in, (fs_in / w.fs as f64) as usize, w.center_rf - dongle);
        let mut rng = StdRng::seed_from_u64(5);
        let n = (2.5 * fs_in) as usize;
        let mut ph = 0.0;
        let mut x = complex_whistle(fs_in, n, 14_103_700.0 - dongle, 0.4, &p, &mut ph);
        for (s, z) in x.iter_mut().zip(noise(n, 0.3, &mut rng)) {
            *s = *s * 0.2 + z;
        }
        let mut out = vec![];
        dec.process(&x, &mut out);
        w.push(&out);
        let hits = w.scan();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!((hits[0].dial_hz - 14_103_700.0).abs() < 3.0, "{}", hits[0].dial_hz);
    }
}
