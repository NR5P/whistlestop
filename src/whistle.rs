//! The Whistlestop "whistle": a chirp calling signal that scanners can find
//! far off frequency, and that tells them exactly how far off they are.
//!
//! The signal is a train of linear chirp pairs, UP (f_lo -> f_hi) then DOWN
//! (f_hi -> f_lo), each `chirp_s` long, so the pattern repeats every
//! P = 2 * chirp_s.
//!
//! When the receiver is mistuned so the audio arrives `df` Hz higher than
//! sent, the matched-filter peak of an up-chirp moves *earlier* by df/k and
//! that of a down-chirp *later* by df/k (k = sweep rate, Hz/s). So the gap
//! from an UP peak to the following DOWN peak is
//!
//! ```text
//!     gap = T + 2*df/k        ->       df = (gap - T) * k / 2
//! ```
//!
//! The train is periodic, so the gap is known modulo P; that bounds the
//! unambiguous range to |df| < k*P/4 (+/-1000 Hz with the defaults).
//! See docs/DESIGN.md.

use num_complex::Complex64;
use rustfft::{Fft, FftPlanner};
use std::f64::consts::PI;
use std::sync::Arc;

/// Mercury runs its modem at 8 kHz; the detector does too.
pub const FS: u32 = 8000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WhistleParams {
    pub fs: u32,
    pub f_lo: f64,
    pub f_hi: f64,
    pub chirp_s: f64,
    /// Raised-cosine edge on each chirp, limits key clicks / splatter.
    pub taper_s: f64,
    pub amplitude: f64,
}

impl Default for WhistleParams {
    fn default() -> Self {
        Self { fs: FS, f_lo: 500.0, f_hi: 2500.0, chirp_s: 0.5, taper_s: 0.01, amplitude: 0.5 }
    }
}

impl WhistleParams {
    /// Sweep rate in Hz/s.
    pub fn k(&self) -> f64 {
        (self.f_hi - self.f_lo) / self.chirp_s
    }
    pub fn period_s(&self) -> f64 {
        2.0 * self.chirp_s
    }
    pub fn n_chirp(&self) -> usize {
        (self.chirp_s * self.fs as f64).round() as usize
    }
    /// Largest |offset| that can be measured without ambiguity.
    pub fn max_offset_hz(&self) -> f64 {
        self.k() * self.period_s() / 4.0
    }
    /// The other offset this measurement could mean: a caller beyond the
    /// unambiguous range shows up one ambiguity period (2 x max offset,
    /// 2000 Hz with the defaults) away, on the other side.
    pub fn alias_of(&self, offset_hz: f64) -> f64 {
        offset_hz - offset_hz.signum() * 2.0 * self.max_offset_hz()
    }

    pub fn with_fs(mut self, fs: u32) -> Self {
        self.fs = fs;
        self
    }
}

/// Named whistle profiles. Callers and searchers must use the same one.
///
/// * `wide`   — 500 -> 2500 Hz, finds callers up to +/-1000 Hz off (default)
/// * `narrow` — 1250 -> 1750 Hz, fits a 500 Hz filter, +/-250 Hz capture
pub fn profile(name: &str) -> WhistleParams {
    match name {
        "narrow" => WhistleParams { f_lo: 1250.0, f_hi: 1750.0, ..Default::default() },
        _ => WhistleParams::default(),
    }
}

fn taper(n: usize, n_edge: usize) -> Vec<f64> {
    let mut w = vec![1.0; n];
    for i in 0..n_edge.min(n / 2) {
        let r = 0.5 - 0.5 * (PI * i as f64 / n_edge as f64).cos();
        w[i] = r;
        w[n - 1 - i] = r;
    }
    w
}

fn chirp_phase(p: &WhistleParams, up: bool) -> impl Iterator<Item = f64> + '_ {
    let fs = p.fs as f64;
    (0..p.n_chirp()).map(move |i| {
        let t = i as f64 / fs;
        if up {
            2.0 * PI * (p.f_lo * t + 0.5 * p.k() * t * t)
        } else {
            2.0 * PI * (p.f_hi * t - 0.5 * p.k() * t * t)
        }
    })
}

/// One chirp of real audio, ready for the radio.
pub fn chirp(p: &WhistleParams, up: bool) -> Vec<f32> {
    let w = taper(p.n_chirp(), (p.taper_s * p.fs as f64) as usize);
    chirp_phase(p, up).zip(w).map(|(ph, w)| (p.amplitude * w * ph.cos()) as f32).collect()
}

/// The complex (analytic) template the detector correlates against.
pub fn chirp_template(p: &WhistleParams, up: bool) -> Vec<Complex64> {
    let w = taper(p.n_chirp(), (p.taper_s * p.fs as f64) as usize);
    chirp_phase(p, up).zip(w).map(|(ph, w)| Complex64::from_polar(w, ph)).collect()
}

/// Audio for a whistle of at least `duration_s`: whole UP/DOWN pairs.
pub fn whistle(duration_s: f64, p: &WhistleParams) -> Vec<f32> {
    let mut pair = chirp(p, true);
    pair.extend(chirp(p, false));
    let n_pairs = ((duration_s / p.period_s()).ceil() as usize).max(1);
    pair.repeat(n_pairs)
}

// ---------------------------------------------------------------- detection

#[derive(Clone, Debug)]
pub struct Detection {
    /// Audio arrived this much higher than sent. On USB: raise the dial by this.
    pub offset_hz: f64,
    /// Mean normalised correlation of the pairs used (0..1).
    pub score: f64,
    /// How many distinct UP chirps supported the estimate.
    pub n_pairs: usize,
    /// Disagreement between the pairs (lower is better).
    pub spread_hz: f64,
}

fn next_pow2(n: usize) -> usize {
    n.next_power_of_two()
}

/// Spectrum of the analytic signal of `x`, zero-padded to `nfft`.
fn analytic_spectrum(x: &[f64], fft: &Arc<dyn Fft<f64>>, nfft: usize) -> Vec<Complex64> {
    let mut buf: Vec<Complex64> = x.iter().map(|&v| Complex64::new(v, 0.0)).collect();
    buf.resize(nfft, Complex64::new(0.0, 0.0));
    fft.process(&mut buf);
    for (i, c) in buf.iter_mut().enumerate() {
        if i == 0 || i == nfft / 2 {
            continue;
        } else if i < nfft / 2 {
            *c *= 2.0;
        } else {
            *c = Complex64::new(0.0, 0.0);
        }
    }
    buf
}

/// Local maxima above `threshold`, strongest first, at least `min_sep` apart;
/// returned in time order.
fn peaks(r: &[f64], threshold: f64, min_sep: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..r.len()).filter(|&i| r[i] > threshold).collect();
    idx.sort_by(|&a, &b| r[b].partial_cmp(&r[a]).unwrap());
    let mut picked: Vec<usize> = Vec::new();
    for i in idx {
        if picked.iter().all(|&j| i.abs_diff(j) >= min_sep) {
            picked.push(i);
        }
    }
    picked.sort_unstable();
    picked
}

/// Parabolic sub-sample peak position.
fn refine(r: &[f64], i: usize) -> f64 {
    if i > 0 && i + 1 < r.len() {
        let (a, b, c) = (r[i - 1], r[i], r[i + 1]);
        let d = a - 2.0 * b + c;
        if d < 0.0 {
            return i as f64 + 0.5 * (a - c) / d;
        }
    }
    i as f64
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

pub struct Detector {
    pub params: WhistleParams,
    pub threshold: f64,
    up: Vec<Complex64>,
    down: Vec<Complex64>,
    planner: FftPlanner<f64>,
}

impl Detector {
    pub fn new(params: WhistleParams) -> Self {
        Self {
            params,
            threshold: 0.12,
            up: chirp_template(&params, true),
            down: chirp_template(&params, false),
            planner: FftPlanner::new(),
        }
    }

    /// Normalised correlation of `x` against the UP and DOWN templates, for
    /// every start index where the template fits. Values lie in 0..1 and do
    /// not depend on input gain.
    pub fn correlate(&mut self, x: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let l = self.params.n_chirp();
        if x.len() < l {
            return (vec![], vec![]);
        }
        let nfft = next_pow2(x.len() + l);
        let fwd = self.planner.plan_fft_forward(nfft);
        let inv = self.planner.plan_fft_inverse(nfft);
        let xa = analytic_spectrum(x, &fwd, nfft);

        // Sliding window energy (analytic energy = 2x real energy).
        let mut cum = vec![0.0f64; x.len() + 1];
        for (i, v) in x.iter().enumerate() {
            cum[i + 1] = cum[i] + v * v;
        }
        let n_out = x.len() - l + 1;

        let run = |tmpl: &[Complex64]| -> Vec<f64> {
            let mut t: Vec<Complex64> = tmpl.to_vec();
            t.resize(nfft, Complex64::new(0.0, 0.0));
            fwd.process(&mut t);
            let tnorm: f64 = tmpl.iter().map(|c| c.norm_sqr()).sum::<f64>().sqrt();
            let mut c: Vec<Complex64> = xa.iter().zip(&t).map(|(a, b)| a * b.conj()).collect();
            inv.process(&mut c);
            (0..n_out)
                .map(|n| {
                    let e = (2.0 * (cum[n + l] - cum[n])).max(1e-20).sqrt();
                    // rustfft's inverse is unnormalised: divide by nfft.
                    c[n].norm() / nfft as f64 / (e * tnorm)
                })
                .collect()
        };
        let ru = run(&self.up.clone());
        let rd = run(&self.down.clone());
        (ru, rd)
    }

    /// Strongest single-chirp correlation in the block. A scanner uses this to
    /// *hold* on a promising channel before a full pair has arrived.
    pub fn peak_level(&mut self, x: &[f64]) -> f64 {
        let (ru, rd) = self.correlate(x);
        ru.iter().chain(&rd).cloned().fold(0.0, f64::max)
    }

    pub fn detect(&mut self, x: &[f64]) -> Option<Detection> {
        let p = self.params;
        let (ru, rd) = self.correlate(x);
        if ru.is_empty() {
            return None;
        }
        let max_u = ru.iter().cloned().fold(0.0, f64::max);
        let max_d = rd.iter().cloned().fold(0.0, f64::max);
        let min_sep = (0.5 * p.chirp_s * p.fs as f64) as usize;
        // Absolute threshold rejects noise; the relative one rejects the
        // sidelobes of a strong whistle.
        let ups = peaks(&ru, self.threshold.max(0.5 * max_u), min_sep);
        let downs = peaks(&rd, self.threshold.max(0.5 * max_d), min_sep);
        if ups.is_empty() || downs.is_empty() {
            return None;
        }

        let fs = p.fs as f64;
        let period = p.period_s() * fs;
        let t = p.chirp_s * fs;
        let lo = t - period / 2.0; // gap folded into (lo, lo + P]
        // The train is periodic, so every UP/DOWN combination gives the same
        // offset once the gap is folded modulo P. Use them all; the median
        // filter below discards any combination involving a noise peak.
        let mut cand: Vec<(f64, f64, usize)> = Vec::new(); // (offset, score, up index)
        for &iu in &ups {
            let tu = refine(&ru, iu);
            for &id in &downs {
                let td = refine(&rd, id);
                let gap = lo + (td - tu - lo).rem_euclid(period);
                cand.push(((gap - t) / fs * p.k() / 2.0, 0.5 * (ru[iu] + rd[id]), iu));
            }
        }
        let med = median(&mut cand.iter().map(|c| c.0).collect::<Vec<_>>());
        let kept: Vec<_> = cand.into_iter().filter(|c| (c.0 - med).abs() < 25.0).collect();
        if kept.is_empty() {
            return None;
        }
        let wsum: f64 = kept.iter().map(|c| c.1).sum();
        let est = kept.iter().map(|c| c.0 * c.1).sum::<f64>() / wsum;
        let mut ups_used: Vec<usize> = kept.iter().map(|c| c.2).collect();
        ups_used.dedup();
        ups_used.sort_unstable();
        ups_used.dedup();
        let lo_o = kept.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        let hi_o = kept.iter().map(|c| c.0).fold(f64::NEG_INFINITY, f64::max);
        Some(Detection {
            offset_hz: est,
            score: wsum / kept.len() as f64,
            n_pairs: ups_used.len(),
            spread_hz: hi_o - lo_o,
        })
    }
}

// ----------------------------------------------------- simple channel model

/// Shift audio by `offset_hz` (a mistuned SSB receiver) and apply the
/// receiver's audio passband. Used by the simulator and tests.
pub fn ssb_channel(x: &[f64], offset_hz: f64, fs: u32, passband: (f64, f64)) -> Vec<f64> {
    let n = x.len();
    if n == 0 {
        return vec![];
    }
    let nfft = next_pow2(n);
    let mut planner = FftPlanner::new();
    let fwd = planner.plan_fft_forward(nfft);
    let inv = planner.plan_fft_inverse(nfft);
    let mut xa = analytic_spectrum(x, &fwd, nfft);
    inv.process(&mut xa);
    let fsf = fs as f64;
    // Shift, take the real part, then band-limit.
    let mut y: Vec<Complex64> = xa
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let v = if i < n { c / nfft as f64 * Complex64::from_polar(1.0, 2.0 * PI * offset_hz * i as f64 / fsf) } else { Complex64::new(0.0, 0.0) };
            Complex64::new(v.re, 0.0)
        })
        .collect();
    fwd.process(&mut y);
    for (i, c) in y.iter_mut().enumerate() {
        let f = if i <= nfft / 2 { i as f64 } else { (nfft - i) as f64 } * fsf / nfft as f64;
        if f < passband.0 || f > passband.1 {
            *c = Complex64::new(0.0, 0.0);
        }
    }
    inv.process(&mut y);
    y.iter().take(n).map(|c| c.re / nfft as f64).collect()
}

pub const PASSBAND: (f64, f64) = (300.0, 2700.0);

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};
    use rand_distr::{Distribution, Normal};

    /// Whistle through a mistuned receiver with noise at `snr_db` (2500 Hz BW).
    fn received(off: f64, snr_db: f64, start: usize, len: usize, rng: &mut StdRng) -> Vec<f64> {
        let p = WhistleParams::default();
        let w: Vec<f64> = whistle(6.0, &p).iter().map(|&v| v as f64).collect();
        let sig_p = p.amplitude * p.amplitude / 2.0;
        let noise_total = sig_p / 10f64.powf(snr_db / 10.0) * (FS as f64 / 2.0) / 2500.0;
        let nd = Normal::new(0.0, noise_total.sqrt()).unwrap();
        let shifted = ssb_channel(&w, off, FS, (0.0, 4000.0));
        let noisy: Vec<f64> = shifted.iter().map(|v| v + nd.sample(rng)).collect();
        let y = ssb_channel(&noisy, 0.0, FS, PASSBAND);
        y[start..start + len].to_vec()
    }

    #[test]
    fn measures_offset_accurately() {
        let mut rng = StdRng::seed_from_u64(1);
        let mut d = Detector::new(WhistleParams::default());
        for &off in &[0.0, 37.3, -250.0, 480.0, -730.0, 900.0] {
            for &snr in &[10.0, -5.0, -10.0] {
                for trial in 0..3 {
                    let x = received(off, snr, 1000 + trial * 2500, 17600, &mut rng);
                    let det = d.detect(&x).unwrap_or_else(|| panic!("missed off={off} snr={snr}"));
                    assert!((det.offset_hz - off).abs() < 2.0, "off={off} snr={snr} got {}", det.offset_hz);
                }
            }
        }
    }

    #[test]
    fn no_false_alarms_on_noise() {
        let mut rng = StdRng::seed_from_u64(2);
        let nd = Normal::new(0.0, 0.1).unwrap();
        let mut d = Detector::new(WhistleParams::default());
        for _ in 0..100 {
            let n: Vec<f64> = (0..17600).map(|_| nd.sample(&mut rng)).collect();
            let n = ssb_channel(&n, 0.0, FS, PASSBAND);
            assert!(d.detect(&n).is_none());
        }
    }

    #[test]
    fn ignores_a_steady_tone_and_noise() {
        // A carrier (e.g. someone tuning up) must not look like a whistle.
        let mut d = Detector::new(WhistleParams::default());
        let x: Vec<f64> = (0..17600).map(|i| (2.0 * PI * 1500.0 * i as f64 / 8000.0).sin()).collect();
        assert!(d.detect(&x).is_none());
    }

    #[test]
    fn far_caller_aliases_and_alias_of_recovers_it() {
        // Seen from 1450 Hz away (outside +/-1000), the whistle measures
        // about -550 Hz; the alias candidate is the true +1450.
        let p = WhistleParams::default();
        let mut rng = StdRng::seed_from_u64(9);
        let x = received(1450.0, 5.0, 1000, 17600, &mut rng);
        let d = Detector::new(p).detect(&x).expect("partial whistle still detected");
        assert!((d.offset_hz + 550.0).abs() < 3.0, "{}", d.offset_hz);
        assert!((p.alias_of(d.offset_hz) - 1450.0).abs() < 3.0);
    }

    #[test]
    fn whistle_is_whole_pairs() {
        let p = WhistleParams::default();
        assert_eq!(whistle(2.5, &p).len(), 3 * 2 * p.n_chirp());
        assert!((p.max_offset_hz() - 1000.0).abs() < 1e-9);
    }
}
