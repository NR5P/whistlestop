//! The Whistlestop "whistle": a chirp calling signal that scanners can find
//! far off frequency, and that tells them exactly how far off they are.
//!
//! The signal (whistle v2) is a train of linear chirp pairs, UP
//! (1000 -> 2000 Hz) then DOWN (2000 -> 1000 Hz), each 0.2 s, so the pattern
//! repeats every P = 0.4 s. A short repeat lets a scanner catch a whole
//! pattern in a half-second dwell; docs/WHISTLE-LAB.md has the study that
//! chose it.
//!
//! Two detectors:
//!
//! * `TrackDetector`, for scanning: non-coherent, it adds up energy along
//!   the whistle's sweep in a short-frame spectrogram for every timing and
//!   offset (±1200 Hz), so it decides in one short dwell, survives fading,
//!   and gives a coarse offset that is never ambiguous.
//! * `Detector`, for the fine lock: coherent matched filters on the UP and
//!   DOWN chirps, accurate to about a hertz (below).
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
        Self {
            fs: FS,
            f_lo: 1000.0,
            f_hi: 2000.0,
            chirp_s: 0.2,
            taper_s: 0.01,
            amplitude: 0.5,
        }
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
    /// unambiguous range shows up one ambiguity period (2 × max offset)
    /// away, on the other side. (Callers further out alias by more periods:
    /// the engine's `fine_offset` checks those too.)
    pub fn alias_of(&self, offset_hz: f64) -> f64 {
        offset_hz - offset_hz.signum() * 2.0 * self.max_offset_hz()
    }

    /// How far off a scanner can be and still catch the whistle in a
    /// dwell: up to most of the receiver's passband for the wide whistle,
    /// the matched filter's unambiguous range for the narrow one (which is
    /// used behind a 500 Hz filter).
    pub fn scan_reach_hz(&self) -> f64 {
        if self.f_hi - self.f_lo >= 900.0 {
            1100.0
        } else {
            self.max_offset_hz()
        }
    }

    pub fn with_fs(mut self, fs: u32) -> Self {
        self.fs = fs;
        self
    }
}

/// Named whistle profiles. Callers and searchers must use the same one.
///
/// * `wide`   — 1000 -> 2000 Hz, scanners find callers up to +/-1100 Hz off
///   (default)
/// * `narrow` — 1250 -> 1750 Hz, fits a 500 Hz filter, +/-250 Hz capture
pub fn profile(name: &str) -> WhistleParams {
    match name {
        "narrow" => WhistleParams {
            f_lo: 1250.0,
            f_hi: 1750.0,
            ..Default::default()
        },
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
    chirp_phase(p, up)
        .zip(w)
        .map(|(ph, w)| (p.amplitude * w * ph.cos()) as f32)
        .collect()
}

/// The complex (analytic) template the detector correlates against.
pub fn chirp_template(p: &WhistleParams, up: bool) -> Vec<Complex64> {
    let w = taper(p.n_chirp(), (p.taper_s * p.fs as f64) as usize);
    chirp_phase(p, up)
        .zip(w)
        .map(|(ph, w)| Complex64::from_polar(w, ph))
        .collect()
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

impl Detection {
    /// Several of the whistle's chirp pairs, all giving the same offset. A
    /// whistle from −10 dB up does this (three or four pairs within 2 Hz in
    /// 1.6 s); data doesn't. Mercury's MFSK, one tone at a time hopping
    /// over the whistle's own 1–2 kHz, gives one chance pair, or a few
    /// that disagree by 10–45 Hz.
    pub fn lined_up(&self) -> bool {
        self.n_pairs >= 2 && self.spread_hz <= LINED_UP_HZ
    }
}

/// Most the pairs of a `lined_up` detection may disagree by.
pub const LINED_UP_HZ: f64 = 5.0;

/// From this scan score up, a whistle's chirps line up in the matched
/// filter (a whistle at −10 dB scores about 15–20 in 1.2–1.6 s).
pub const LINE_UP_Z: f64 = 15.0;

/// A `TrackHit::fit` this good is a whistle's own track: Mercury's MFSK
/// and MFSK500 reach 0.39 at most (its own signals, any strength); a strong
/// whistle that the matched filter can't measure, 0.58 at least.
pub const FIT_MIN: f64 = 0.5;

/// A scan hit `h`, with the matched filter's `d` on the same audio: a
/// signal this strong whose chirps don't line up and that doesn't own its
/// track is data, not a whistle. Mercury's MFSK lights the whistle's band
/// just as a whistle does and passes the scan detector at any strength.
/// (A strong carrier inside the sweep can blind the matched filter to a
/// strong whistle: its clipping stands down when the whistle fills the
/// band. The fit still knows it.)
pub fn looks_like_data(h: &TrackHit, d: Option<&Detection>) -> bool {
    h.z >= LINE_UP_Z && !d.is_some_and(Detection::lined_up) && h.fit < FIT_MIN
}

/// The whistle watch's test of one look (it needs the matched filter's
/// offset anyway, so it can be stricter than `looks_like_data`): the
/// chirps line up, or the scan's track is the signal's own.
pub fn watch_confirms(h: &TrackHit, d: &Detection) -> bool {
    d.lined_up() || h.fit >= FIT_MIN
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
    idx.sort_by(|&a, &b| r[b].total_cmp(&r[a]));
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
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
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
            // 0.12 for a 0.5 s chirp; noise correlation grows as the
            // template shortens.
            threshold: 0.12
                * (4000.0 / (params.n_chirp() as f64 * 8000.0 / params.fs as f64)).sqrt(),
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
        let mut xa = analytic_spectrum(x, &fwd, nfft);
        // Clip strong narrow peaks (carriers, RTTY, FT8) to 3x the band's
        // median level, so they don't swamp the normalisation. Noise and the
        // whistle itself (spread over 1000 Hz) are almost never clipped.
        let fsf = self.params.fs as f64;
        let (b0, b1) = (
            (PASSBAND.0 / fsf * nfft as f64) as usize,
            (PASSBAND.1 / fsf * nfft as f64) as usize,
        );
        let mut mags: Vec<f64> = xa[b0..b1.min(nfft / 2)].iter().map(|c| c.norm()).collect();
        if !mags.is_empty() {
            let cap = 3.0 * median(&mut mags);
            // Only when a few bins stand out (a carrier in noise); if many
            // do, there's no noise floor to judge by, so leave it alone.
            let over = mags.iter().filter(|m| **m > cap).count();
            // (A carrier in noise lifts ~5 % of bins; a lone tone with no
            // noise at all, ~15 %.)
            if cap > 0.0 && over * 10 < mags.len() {
                for c in xa.iter_mut().take(nfft / 2) {
                    let m = c.norm();
                    if m > cap {
                        *c *= cap / m;
                    }
                }
            }
        }
        // The clipped signal in time, for the window energies.
        let mut xw = xa.clone();
        inv.process(&mut xw);
        let mut cum = vec![0.0f64; x.len() + 1];
        for i in 0..x.len() {
            cum[i + 1] = cum[i] + (xw[i] / nfft as f64).norm_sqr() / 2.0;
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

    /// Measure a whistle already found and tuned to (by the scan): only
    /// offsets within ±`within` Hz count, so a lower threshold is safe.
    pub fn detect_near(&mut self, x: &[f64], within: f64) -> Option<Detection> {
        let saved = self.threshold;
        self.threshold = saved * 0.75;
        let d = self.detect(x).filter(|d| d.offset_hz.abs() <= within);
        self.threshold = saved;
        d
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
                let edge = 0.04 * period;
                if gap - lo < edge || lo + period - gap < edge {
                    continue; // UP and DOWN coincide: an impulse, not a whistle
                }
                cand.push(((gap - t) / fs * p.k() / 2.0, 0.5 * (ru[iu] + rd[id]), iu));
            }
        }
        if cand.is_empty() {
            return None;
        }
        let med = median(&mut cand.iter().map(|c| c.0).collect::<Vec<_>>());
        let kept: Vec<_> = cand
            .into_iter()
            .filter(|c| (c.0 - med).abs() < 25.0)
            .collect();
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

// ------------------------------------------------------- scan detection

/// What a scan dwell found: how sure, and roughly where.
#[derive(Clone, Copy, Debug)]
pub struct TrackHit {
    /// Strength of the best match, as a z-score over the busy-band noise
    /// floor (see `TrackDetector::found`).
    pub z: f64,
    /// Coarse offset (Hz, audio higher than sent), good to about ±30 Hz and
    /// never ambiguous: the UP and DOWN legs only line up at the true one.
    pub offset_hz: f64,
    /// Of the frames along the best track, the share whose strongest cell
    /// (after levelling) is on it. A whistle owns its track: 0.75–1 from
    /// −5 dB up, a carrier in the sweep or not (the levelling removes it).
    /// Mercury's MFSK, one tone hopping over the same band, is on any
    /// track only now and then: 0.2–0.4.
    pub fit: f64,
}

/// The scanning detector: a short-frame spectrogram, each frequency bin
/// whitened by its own median (so steady carriers vanish), then for every
/// pattern timing and every offset within reach, the cells along the
/// whistle's UP/DOWN track are added up. The best sum, as a z-score, says
/// whether a whistle is there; its offset says where.
///
/// Non-coherent, so it doesn't care about fading phase, and it needs only
/// as much signal as the dwell holds, not a whole chirp at the right moment.
pub struct TrackDetector {
    pub params: WhistleParams,
    /// Offsets searched, ± Hz.
    pub reach_hz: f64,
    frame: usize,
    win: Vec<f64>,
    planner: FftPlanner<f64>,
}

impl TrackDetector {
    /// "Worth listening longer" after a 0.5 s dwell: 15 % of busy-band
    /// dwells with no whistle score this high; 82–84 % of whistles at
    /// −10 dB do (calibrated with `whistlelab calibrate`).
    pub const MAYBE: f64 = 2.48;

    /// "Whistle found" for a block of `secs` seconds: 1 % of busy-band
    /// dwells with no whistle score above it (3.98 at 0.5 s, 1.99 at 2 s;
    /// longer blocks give steadier scores). Calibrated with
    /// `whistlelab calibrate`.
    pub fn found(secs: f64) -> f64 {
        let r = (secs.clamp(0.5, 2.0) / 0.5).log2() / 2.0; // 0 at 0.5 s, 1 at 2 s
        3.98 + (1.99 - 3.98) * r
    }
    /// Largest value one whitened cell may add.
    const CAP: f64 = 16.0;

    pub fn new(params: WhistleParams) -> Self {
        // ~16 ms frames: the chirp moves about one bin per frame.
        let frame = ((0.016 * params.fs as f64) as usize).next_power_of_two();
        let win = (0..frame)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / frame as f64).cos())
            .collect();
        Self {
            params,
            reach_hz: params.scan_reach_hz(),
            frame,
            win,
            planner: FftPlanner::new(),
        }
    }

    /// Track frequency (Hz) at time t into the pattern.
    fn track(&self, t: f64) -> f64 {
        let p = &self.params;
        let t = t.rem_euclid(p.period_s());
        if t < p.chirp_s {
            p.f_lo + p.k() * t
        } else {
            p.f_hi - p.k() * (t - p.chirp_s)
        }
    }

    /// Look for the whistle in a block of audio (real, at `params.fs`).
    pub fn scan(&mut self, x: &[f64]) -> Option<TrackHit> {
        let (l, fs) = (self.frame, self.params.fs as f64);
        let hop = l / 2;
        let bw = fs / l as f64;
        let fft = self.planner.plan_fft_forward(l);
        let (k0, k1) = (
            (PASSBAND.0 / bw).ceil() as usize,
            ((PASSBAND.1 / bw).floor() as usize).min(l / 2 - 1),
        );
        let nb = k1 - k0 + 1;
        let mut rows: Vec<Vec<f64>> = vec![];
        let mut buf = vec![Complex64::new(0.0, 0.0); l];
        let mut start = 0;
        while start + l <= x.len() {
            for i in 0..l {
                buf[i] = Complex64::new(x[start + i] * self.win[i], 0.0);
            }
            fft.process(&mut buf);
            rows.push((k0..=k1).map(|k| buf[k].norm_sqr()).collect());
            start += hop;
        }
        let nt = rows.len();
        if nt < 5 {
            return None;
        }
        // Whiten each bin by its 75th percentile (noise: ln 4 for a mean-1
        // exponential). Anything in a bin more than a quarter of the time
        // (carriers, RTTY, FT8, a modem) is levelled away; the whistle
        // visits each bin only a few percent of the time. Then cap single
        // cells, so a click or a burst can't fake a whole track.
        for b in 0..nb {
            let mut col: Vec<f64> = rows.iter().map(|r| r[b]).collect();
            col.sort_by(|a, c| a.total_cmp(c));
            let q = (col[(col.len() * 3) / 4] / 4f64.ln()).max(1e-30);
            rows.iter_mut().for_each(|r| r[b] /= q);
        }
        // Then each frame by its own median across the band: a data burst
        // (an OFDM modem, a CQFRAME) lights every bin at once and is levelled
        // back to noise, while a whistle lights only one or two bins.
        for r in rows.iter_mut() {
            let mut v = r.clone();
            let m = median(&mut v) / std::f64::consts::LN_2;
            if m > 1.0 {
                r.iter_mut().for_each(|c| *c /= m);
            }
            r.iter_mut().for_each(|c| *c = c.min(Self::CAP));
        }
        let pre: Vec<Vec<f64>> = rows
            .iter()
            .map(|r| {
                let mut p = vec![0.0; nb + 1];
                for (i, c) in r.iter().enumerate() {
                    p[i + 1] = p[i] + c;
                }
                p
            })
            .collect();
        let hop_s = hop as f64 / fs;
        let frame_s = l as f64 / fs;
        let period_frames = (self.params.period_s() / hop_s).round().max(1.0) as usize;
        let nd = (self.reach_hz / bw).ceil() as i64;
        let e = (-Self::CAP).exp();
        let (mu, var) = (1.0 - e, 1.0 - 2.0 * Self::CAP * e - e * e);
        // Cells along a real track are mostly a bit above the noise; a loud
        // signal crossing a wrong track makes only a few huge ones. So a
        // track also needs enough cells over 3× the noise (a noise cell
        // does that with probability e^-3).
        let p0 = (-3.0f64).exp();
        let z_at = |dt: usize, df: i64| -> Option<f64> {
            let (mut s, mut n, mut over) = (0.0, 0.0, 0.0);
            for (t, p) in pre.iter().enumerate() {
                let tc = (t + dt) as f64 * hop_s;
                let (fa, fb) = (self.track(tc), self.track(tc + frame_s));
                let (lo, hi) = (fa.min(fb), fa.max(fb));
                let blo = (lo / bw).round() as i64 + df - k0 as i64;
                let bhi = (hi / bw).round() as i64 + df - k0 as i64;
                let (blo, bhi) = (blo.max(0), bhi.min(nb as i64 - 1));
                if blo > bhi {
                    continue;
                }
                s += p[bhi as usize + 1] - p[blo as usize];
                n += (bhi - blo + 1) as f64;
                over += (blo..=bhi).filter(|&b| rows[t][b as usize] > 3.0).count() as f64;
            }
            // At least half the track inside the passband: a sliver at
            // the edge is too easy to fake.
            if n < 0.5 * nt as f64 {
                return None;
            }
            let z = (s - n * mu) / (n * var).sqrt();
            let z_over = (over - n * p0) / (n * p0 * (1.0 - p0)).sqrt();
            Some(if z_over >= 3.0 { z } else { z.min(z_over) })
        };
        let mut best: Option<(TrackHit, usize, i64)> = None;
        for dt in 0..period_frames {
            for df in -nd..=nd {
                if let Some(z) = z_at(dt, df)
                    && best.is_none_or(|b| z > b.0.z)
                {
                    best = Some((
                        TrackHit {
                            z,
                            offset_hz: df as f64 * bw,
                            fit: 0.0,
                        },
                        dt,
                        df,
                    ));
                }
            }
        }
        let fit_at = |dt: usize, df: i64| -> f64 {
            let (mut n, mut on) = (0usize, 0usize);
            for (t, r) in rows.iter().enumerate() {
                let tc = (t + dt) as f64 * hop_s;
                let (fa, fb) = (self.track(tc), self.track(tc + frame_s));
                let blo = (fa.min(fb) / bw).round() as i64 + df - k0 as i64;
                let bhi = (fa.max(fb) / bw).round() as i64 + df - k0 as i64;
                if blo < 0 || bhi >= nb as i64 {
                    continue;
                }
                n += 1;
                let top = (0..nb).max_by(|&a, &b| r[a].total_cmp(&r[b])).unwrap_or(0) as i64;
                on += usize::from((blo - 1..=bhi + 1).contains(&top));
            }
            on as f64 / n.max(1) as f64
        };
        // Sub-bin offset from the neighbouring hypotheses (parabola).
        best.map(|(mut h, dt, df)| {
            h.fit = fit_at(dt, df);
            if let (Some(a), Some(c)) = (z_at(dt, df - 1), z_at(dt, df + 1)) {
                let d = a - 2.0 * h.z + c;
                if d < 0.0 {
                    h.offset_hz += (0.5 * (a - c) / d).clamp(-0.5, 0.5) * bw;
                }
            }
            h
        })
    }
}

// ----------------------------------------------------- simple channel model

/// Shift audio by `offset_hz` (a mistuned SSB receiver) and apply the
/// receiver's audio passband. Used by the simulator and tests.
pub fn ssb_channel(x: &[f64], offset_hz: f64, fs: u32, passband: (f64, f64)) -> Vec<f64> {
    ssb_channel_at(x, offset_hz, fs, passband, 0.0)
}

/// `ssb_channel` for one block of a longer stream that starts `t0` seconds
/// in: the shift's phase carries on from the block before, instead of
/// restarting at zero (which jolts every offset that isn't a whole number
/// of cycles per block: the simulated band's 50 ms blocks moved a +333 Hz
/// whistle to 339 Hz).
pub fn ssb_channel_at(x: &[f64], offset_hz: f64, fs: u32, passband: (f64, f64), t0: f64) -> Vec<f64> {
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
            let v = if i < n {
                c / nfft as f64
                    * Complex64::from_polar(1.0, 2.0 * PI * offset_hz * (t0 + i as f64 / fsf))
            } else {
                Complex64::new(0.0, 0.0)
            };
            Complex64::new(v.re, 0.0)
        })
        .collect();
    fwd.process(&mut y);
    for (i, c) in y.iter_mut().enumerate() {
        let f = if i <= nfft / 2 {
            i as f64
        } else {
            (nfft - i) as f64
        } * fsf
            / nfft as f64;
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

    /// A whistle shifted block by block (as the simulated band does, 50 ms
    /// at a time) is measured where it is: the shift's phase carries on
    /// across blocks. (Restarted each block, +333 Hz measured 339.)
    #[test]
    fn a_blockwise_shift_keeps_its_offset() {
        let p = WhistleParams::default();
        let x = whistle(3.0, &p);
        let x: Vec<f64> = x.iter().map(|v| *v as f64).collect();
        let block = 400;
        let mut y = vec![];
        for (k, b) in x.chunks(block).enumerate() {
            let t0 = (k * block) as f64 / FS as f64;
            y.extend(ssb_channel_at(b, 333.0, FS, (0.0, 4000.0), t0));
        }
        let d = Detector::new(p).detect(&y).expect("detected");
        assert!((d.offset_hz - 333.0).abs() < 3.0, "measured {:.1}", d.offset_hz);
    }

    /// One sample that isn't a number (off a socket, say) doesn't panic the
    /// detectors: the whole minute of audio held would otherwise panic them
    /// on every look.
    #[test]
    fn a_nan_sample_does_not_panic() {
        let p = WhistleParams::default();
        let mut x = vec![0.01; 3 * FS as usize];
        x[1000] = f64::NAN;
        x[2000] = f64::INFINITY;
        let _ = TrackDetector::new(p).scan(&x);
        let _ = Detector::new(p).detect(&x);
    }

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
        for &off in &[0.0, 37.3, -250.0, 400.0, -380.0] {
            for &snr in &[10.0, -5.0] {
                for trial in 0..3 {
                    let x = received(off, snr, 1000 + trial * 2500, 17600, &mut rng);
                    let det = d
                        .detect(&x)
                        .unwrap_or_else(|| panic!("missed off={off} snr={snr}"));
                    assert!(
                        (det.offset_hz - off).abs() < 2.0,
                        "off={off} snr={snr} got {}",
                        det.offset_hz
                    );
                }
            }
        }
    }

    #[test]
    fn confirms_weak_whistles_near_centre() {
        // After the scan has retuned onto a weak whistle: three patterns
        // (1.2 s) at −10 dB, a few tens of hertz off.
        let mut rng = StdRng::seed_from_u64(31);
        let mut d = Detector::new(WhistleParams::default());
        for &off in &[0.0, 23.0, -41.0, 60.0] {
            for trial in 0..5 {
                let x = received(off, -10.0, 700 + trial * 900, 9600, &mut rng);
                let det = d
                    .detect_near(&x, 150.0)
                    .unwrap_or_else(|| panic!("missed off={off} trial {trial}"));
                assert!(
                    (det.offset_hz - off).abs() < 3.0,
                    "off={off} got {}",
                    det.offset_hz
                );
            }
        }
    }

    #[test]
    fn confirmation_is_quiet_on_noise() {
        let mut rng = StdRng::seed_from_u64(32);
        let nd = Normal::new(0.0, 0.1).unwrap();
        let mut d = Detector::new(WhistleParams::default());
        let hits = (0..200)
            .filter(|_| {
                let n: Vec<f64> = (0..9600).map(|_| nd.sample(&mut rng)).collect();
                d.detect_near(&ssb_channel(&n, 0.0, FS, PASSBAND), 150.0)
                    .is_some()
            })
            .count();
        assert!(hits <= 2, "{hits} of 200 noise windows confirmed");
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
        let x: Vec<f64> = (0..17600)
            .map(|i| (2.0 * PI * 1500.0 * i as f64 / 8000.0).sin())
            .collect();
        assert!(d.detect(&x).is_none());
    }

    #[test]
    fn far_caller_aliases_and_alias_of_recovers_it() {
        // Seen from 700 Hz away (outside +/-500), the matched filter
        // measures about -300 Hz; the alias candidate is the true +700.
        let p = WhistleParams::default();
        let mut rng = StdRng::seed_from_u64(9);
        let x = received(700.0, 5.0, 1000, 17600, &mut rng);
        let d = Detector::new(p).detect(&x).expect("whistle detected");
        assert!((d.offset_hz + 300.0).abs() < 3.0, "{}", d.offset_hz);
        assert!((p.alias_of(d.offset_hz) - 700.0).abs() < 3.0);
    }

    #[test]
    fn track_detector_finds_far_callers_in_half_a_second() {
        // One 0.5 s dwell, callers anywhere within ±1100 Hz: found, with
        // an unambiguous coarse offset.
        let mut rng = StdRng::seed_from_u64(21);
        let mut t = TrackDetector::new(WhistleParams::default());
        for &off in &[0.0, 180.0, -420.0, 650.0, -900.0, 1050.0] {
            for trial in 0..3 {
                let x = received(off, -5.0, 700 + trial * 1100, 4000, &mut rng);
                let h = t.scan(&x).unwrap();
                assert!(h.z > TrackDetector::found(0.5), "off={off}: z={:.1}", h.z);
                assert!(
                    (h.offset_hz - off).abs() < 80.0,
                    "off={off}: got {}",
                    h.offset_hz
                );
            }
        }
    }

    #[test]
    fn data_bursts_dont_look_like_whistles_to_the_scan() {
        // A strong OFDM burst (33 carriers, 62.5 Hz apart, new phases every
        // 16 ms) for part of the window: every bin lights at once.
        let mut rng = StdRng::seed_from_u64(23);
        let nd = Normal::new(0.0, 0.05).unwrap();
        let mut t = TrackDetector::new(WhistleParams::default());
        let mut over = 0;
        for trial in 0..60 {
            let n = 12_800; // 1.6 s, the whistle watch's window
            let mut x: Vec<f64> = (0..n).map(|_| nd.sample(&mut rng)).collect();
            let (start, len) = (800 + trial * 97, 4000 + trial * 61);
            let mut ph: Vec<f64> = (0..33).map(|k| k as f64).collect();
            #[allow(clippy::needless_range_loop)]
            for i in start..(start + len).min(n) {
                if i % 128 == 0 {
                    for p in ph.iter_mut() {
                        *p = (nd.sample(&mut rng).abs() * 80.0).floor() % 4.0 * PI / 2.0;
                    }
                }
                let tt = i as f64 / 8000.0;
                x[i] += ph
                    .iter()
                    .enumerate()
                    .map(|(k, p)| 0.15 * (2.0 * PI * (500.0 + 62.5 * k as f64) * tt + p).cos())
                    .sum::<f64>();
            }
            let x = ssb_channel(&x, 0.0, FS, PASSBAND);
            if t.scan(&x).is_some_and(|h| h.z > TrackDetector::found(1.6)) {
                over += 1;
            }
        }
        assert!(over <= 1, "{over} of 60 bursts looked like whistles");
    }

    /// Mercury's MFSK: one of 32 tones `spacing` Hz apart, centred on
    /// 1500 Hz, for each symbol (`sym` samples), hopped 13 tones a symbol.
    /// MFSK: 31.25 Hz and 40 ms (1015.6–1984.4 Hz); MFSK500: 15.625 Hz and
    /// 80 ms. `n` samples at `snr_db` (2500 Hz noise bandwidth).
    fn mfsk(n: usize, spacing: f64, sym: usize, snr_db: f64, rng: &mut StdRng) -> Vec<f64> {
        use rand::Rng;
        let noise = 0.5 / 10f64.powf(snr_db / 10.0) * (FS as f64 / 2.0) / 2500.0;
        let nd = Normal::new(0.0, noise.sqrt()).unwrap();
        let (mut ph, mut f) = (0.0f64, 0.0);
        let start = (rng.next_u32() as usize) % sym;
        let x: Vec<f64> = (0..n)
            .map(|i| {
                if (i + start) % sym == 0 || i == 0 {
                    let s = (i + start) / sym;
                    let k = ((rng.next_u32() % 32) as usize + s * 13) % 32;
                    f = 1500.0 + spacing * (k as f64 - 15.5);
                }
                ph += 2.0 * PI * f / FS as f64;
                ph.cos() + nd.sample(rng)
            })
            .collect();
        ssb_channel(&x, 0.0, FS, PASSBAND)
    }

    /// A whistle `off` Hz off at `snr_db`, with a strong carrier inside the
    /// sweep if `carrier`: `n` samples from `start`.
    fn whistle_in(p: &WhistleParams, off: f64, snr_db: f64, carrier: bool, start: usize, n: usize, rng: &mut StdRng) -> Vec<f64> {
        let w: Vec<f64> = whistle(6.0, p).iter().map(|&v| v as f64).collect();
        let noise = p.amplitude * p.amplitude / 2.0 / 10f64.powf(snr_db / 10.0) * (FS as f64 / 2.0) / 2500.0;
        let nd = Normal::new(0.0, noise.sqrt()).unwrap();
        let mut x: Vec<f64> = ssb_channel(&w, off, FS, (0.0, 4000.0)).iter().map(|v| v + nd.sample(rng)).collect();
        if carrier {
            for (j, v) in x.iter_mut().enumerate() {
                *v += 3.0 * (2.0 * PI * 1420.0 * j as f64 / 8000.0).sin();
            }
        }
        ssb_channel(&x, 0.0, FS, PASSBAND)[start..start + n].to_vec()
    }

    #[test]
    fn mfsk_is_not_a_whistle() {
        // CHARLIE, parked on ALPHA's frequency, reported ALPHA and BRAVO's
        // MFSK link as whistles (bench, cq_pileup): MFSK passes the scan
        // detector at any strength. The watch's look (1.6 s) and search's
        // (1.2 s), both whistle shapes, both MFSK geometries.
        let mut rng = StdRng::seed_from_u64(41);
        for p in [profile("wide"), profile("narrow")] {
            let (mut t, mut d) = (TrackDetector::new(p), Detector::new(p));
            for n in [12_800, 9_600] {
                let secs = n as f64 / FS as f64;
                for (spacing, sym) in [(31.25, 320), (15.625, 640)] {
                    let (mut looks, mut watch, mut data) = (0, 0, 0);
                    for snr in [35.0, 10.0, 0.0, -5.0] {
                        for _ in 0..25 {
                            let x = mfsk(n, spacing, sym, snr, &mut rng);
                            let Some(h) = t.scan(&x).filter(|h| h.z >= TrackDetector::found(secs)) else {
                                continue;
                            };
                            let det = d.detect(&x);
                            looks += 1;
                            watch += usize::from(det.as_ref().is_some_and(|det| watch_confirms(&h, det)));
                            data += usize::from(looks_like_data(&h, det.as_ref()));
                        }
                    }
                    assert!(looks >= 80, "the scan passed only {looks} of 100 MFSK looks: test no longer needed?");
                    assert!(watch * 100 <= 4 * looks, "f_lo {} {secs} s {spacing} Hz: watch took {watch} of {looks} MFSK looks for whistles", p.f_lo);
                    let strong = 50; // 35, 10 and 0 dB
                    assert!(data * 100 >= 90 * strong.min(looks), "f_lo {} {secs} s {spacing} Hz: only {data} MFSK looks known for data", p.f_lo);
                }
            }
        }
    }

    #[test]
    fn whistles_are_not_data() {
        // Every whistle the scan finds, from −12 dB to +30 dB, a quarter of
        // them with a strong carrier inside the sweep: never taken for data,
        // and the watch confirms nearly all from −10 dB up (with the carrier
        // the matched filter often measures nothing: as before).
        let mut rng = StdRng::seed_from_u64(43);
        for p in [profile("wide"), profile("narrow")] {
            let (mut t, mut d) = (TrackDetector::new(p), Detector::new(p));
            for n in [12_800, 9_600] {
                let secs = n as f64 / FS as f64;
                let (mut looks, mut watch) = (0, 0);
                for snr in [30.0, 10.0, 0.0, -5.0, -10.0, -12.0] {
                    for k in 0..16 {
                        let x = whistle_in(&p, -150.0 + 19.0 * k as f64, snr, k % 4 == 0, 500 + 211 * k, n, &mut rng);
                        let Some(h) = t.scan(&x).filter(|h| h.z >= TrackDetector::found(secs)) else {
                            continue;
                        };
                        let det = d.detect(&x);
                        assert!(!looks_like_data(&h, det.as_ref()), "f_lo {} {secs} s {snr} dB #{k}: {h:?}, {det:?}", p.f_lo);
                        if snr >= -10.0 && k % 4 != 0 {
                            looks += 1;
                            watch += usize::from(det.as_ref().is_some_and(|det| watch_confirms(&h, det)));
                        }
                    }
                }
                assert!(watch * 100 >= 95 * looks, "f_lo {} {secs} s: watch confirmed {watch} of {looks}", p.f_lo);
            }
        }
    }

    /// Mercury's own signals: `mercury -x fifo -t -m <mode>` with `-o` a
    /// fifo read into `tx-<mode>.raw` (s32le, 8 kHz), in $WS_TX_DIR. The
    /// watch, looking every second as it does (a report: two looks in a
    /// row within 15 Hz), at several strengths.
    #[test]
    #[ignore]
    fn mercury_modes_are_not_whistles() {
        let dir = std::env::var("WS_TX_DIR").expect("WS_TX_DIR");
        let mut rng = StdRng::seed_from_u64(44);
        for p in [profile("wide"), profile("narrow")] {
            let (mut t, mut d) = (TrackDetector::new(p), Detector::new(p));
            for m in [11, 12, 0, 1, 3, 7, 9] {
                let Ok(raw) = std::fs::read(format!("{dir}/tx-{m}.raw")) else {
                    continue;
                };
                let x: Vec<f64> = raw.chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64 / 2e9).collect();
                let pw = x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64;
                for snr in [30.0, 10.0, 0.0, -5.0] {
                    let nd = Normal::new(0.0, (pw / 10f64.powf(snr / 10.0) * (FS as f64 / 2.0) / 2500.0).sqrt()).unwrap();
                    let y: Vec<f64> = x.iter().map(|v| v + nd.sample(&mut rng)).collect();
                    let y = ssb_channel(&y, 0.0, FS, PASSBAND);
                    let (mut found, mut data) = (0, 0);
                    let (mut before, mut now) = ((0, None::<f64>), (0, None::<f64>));
                    let report = |r: &mut (usize, Option<f64>), off: Option<f64>| {
                        if let (Some(a), Some(b)) = (r.1, off)
                            && (a - b).abs() <= 15.0
                        {
                            r.0 += 1;
                            r.1 = None;
                        } else {
                            r.1 = off;
                        }
                    };
                    for s in (0..y.len().saturating_sub(12_800)).step_by(8000) {
                        let w = &y[s..s + 12_800];
                        let h = t.scan(w).filter(|h| h.z >= TrackDetector::found(1.6));
                        let det = h.and_then(|_| d.detect(w));
                        found += usize::from(h.is_some());
                        data += usize::from(h.is_some_and(|h| looks_like_data(&h, det.as_ref())));
                        report(&mut before, det.as_ref().map(|d| d.offset_hz));
                        report(&mut now, det.as_ref().filter(|det| h.is_some_and(|h| watch_confirms(&h, det))).map(|d| d.offset_hz));
                    }
                    eprintln!("f_lo {} mode {m:2} {snr:5.1} dB: {found} looks pass the scan, {data} known for data; whistles reported {} before, {} now", p.f_lo, before.0, now.0);
                    assert_eq!(now.0, 0, "mode {m} at {snr} dB");
                }
            }
        }
    }

    #[test]
    fn track_detector_quiet_on_noise_and_carriers() {
        let mut rng = StdRng::seed_from_u64(22);
        let nd = Normal::new(0.0, 0.1).unwrap();
        let mut t = TrackDetector::new(WhistleParams::default());
        let mut over = 0;
        for i in 0..200 {
            let mut x: Vec<f64> = (0..4000).map(|_| nd.sample(&mut rng)).collect();
            if i % 2 == 0 {
                // A strong carrier and an RTTY-like pair.
                for (j, v) in x.iter_mut().enumerate() {
                    let tt = j as f64 / 8000.0;
                    *v += (2.0 * PI * 1234.0 * tt).sin()
                        + 0.5
                            * (2.0 * PI * if (j / 176) % 2 == 0 { 2125.0 } else { 2295.0 } * tt)
                                .sin();
                }
            }
            let x = ssb_channel(&x, 0.0, FS, PASSBAND);
            if t.scan(&x).is_some_and(|h| h.z > TrackDetector::found(0.5)) {
                over += 1;
            }
        }
        assert!(over <= 4, "{over} false stops in 200 dwells");
    }

    #[test]
    fn data_bursts_and_clicks_are_not_whistles() {
        // OFDM-like bursts with hard onsets (what Mercury's frames look like
        // to the detector) and isolated clicks.
        let mut rng = StdRng::seed_from_u64(11);
        let nd = Normal::new(0.0, 1.0).unwrap();
        let mut d = Detector::new(WhistleParams::default());
        for trial in 0..20 {
            let mut x = vec![0.0f64; 17600];
            let mut t = 800 + trial * 37;
            while t + 3000 < x.len() {
                for v in x[t..t + 2500].iter_mut() {
                    *v = nd.sample(&mut rng);
                }
                t += 4000 + trial * 13;
            }
            x[400] = 8.0;
            x[9000] = -8.0;
            let x = ssb_channel(&x, 0.0, FS, PASSBAND);
            assert!(d.detect(&x).is_none(), "trial {trial}: {:?}", d.detect(&x));
        }
    }

    #[test]
    fn whistle_is_whole_pairs() {
        let p = WhistleParams::default();
        assert_eq!(whistle(2.5, &p).len(), 7 * 2 * p.n_chirp());
        assert!((p.max_offset_hz() - 500.0).abs() < 1e-9);
        assert_eq!(p.period_s(), 0.4);
    }
}
