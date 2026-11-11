//! Whistle lab: which attention signal lets a scanning receiver find a
//! caller fastest without passing over it?
//!
//!   cargo run --release --bin whistlelab [-- quick]
//!
//! Every candidate is a constant-envelope signal at the same power, sent
//! over a simulated HF path (two-path Rayleigh fading, ITU "moderate":
//! ~1 Hz Doppler spread, 1 ms delay) into a busy band (FT8, RTTY, PSK31,
//! carriers, OFDM modems) and heard by a scanning SSB receiver
//! (300–2700 Hz) that dwells a short time on each channel, not knowing the
//! caller's offset or where in the pattern it is.
//!
//! Each design gets the detector that suits it (a time–frequency "track"
//! detector, non-coherent so it survives fading), and every detector's
//! threshold is set from busy-band dwells with no whistle so that all
//! designs false-alarm on the same 1 % of dwells. Then: probability of
//! detection vs dwell, SNR and offset; the widest channel step that still
//! catches ≥ 90 %; and so the scan speed and how long a whistle must last
//! for a scanner to sweep a 12 kHz segment.

use num_complex::Complex64 as C;
use rustfft::FftPlanner;
use std::f64::consts::PI;
use std::sync::Arc;

const FS: f64 = 8000.0;
const PB: (f64, f64) = (300.0, 2700.0);

// ---------------------------------------------------------------- random

#[derive(Clone)]
struct Rng(u64);
impl Rng {
    fn u(&mut self) -> f64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.u()
    }
    fn gauss(&mut self) -> f64 {
        let (u1, u2) = (self.u().max(1e-300), self.u());
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
    fn cgauss(&mut self) -> C {
        C::new(self.gauss(), self.gauss()) / 2f64.sqrt()
    }
}

// ---------------------------------------------------------------- designs

/// Instantaneous audio frequency (Hz, centred on 0 = 1500 Hz audio) at time
/// t within one period, or None when the signal is off.
#[derive(Clone)]
enum Shape {
    /// Up then down linear chirps: span Hz, each leg `leg` s.
    UpDown { span: f64, leg: f64 },
    /// Up chirps only (sawtooth): span Hz, `leg` s each.
    Up { span: f64, leg: f64 },
    Tone,
    /// Costas tone pattern: symbol s, tone spacing Hz.
    Costas { sym: f64, spacing: f64 },
    /// Tone on for `on` s, off for `off` s.
    Pulse { on: f64, off: f64 },
}

const COSTAS7: [usize; 7] = [3, 1, 4, 0, 6, 5, 2]; // FT8's

impl Shape {
    fn period(&self) -> f64 {
        match self {
            Shape::UpDown { leg, .. } => 2.0 * leg,
            Shape::Up { leg, .. } => *leg,
            Shape::Tone => 0.128,
            Shape::Costas { sym, .. } => 7.0 * sym,
            Shape::Pulse { on, off } => on + off,
        }
    }
    fn freq(&self, t: f64) -> Option<f64> {
        let t = t.rem_euclid(self.period());
        Some(match self {
            Shape::UpDown { span, leg } => {
                if t < *leg {
                    -span / 2.0 + span * t / leg
                } else {
                    span / 2.0 - span * (t - leg) / leg
                }
            }
            Shape::Up { span, leg } => -span / 2.0 + span * t / leg,
            Shape::Tone => 0.0,
            Shape::Costas { sym, spacing } => {
                let i = ((t / sym) as usize).min(6);
                (COSTAS7[i] as f64 - 3.0) * spacing
            }
            Shape::Pulse { on, .. } => {
                if t < *on {
                    0.0
                } else {
                    return None;
                }
            }
        })
    }
    fn width(&self) -> f64 {
        match self {
            Shape::UpDown { span, .. } | Shape::Up { span, .. } => *span,
            Shape::Costas { spacing, .. } => 7.0 * spacing,
            _ => 10.0,
        }
    }
}

#[derive(Clone)]
struct Design {
    key: &'static str,
    name: &'static str,
    shape: Shape,
    /// STFT frame (samples); hop is half of it.
    frame: usize,
    /// Whiten each frequency bin by its own median (removes steady
    /// carriers) — wrong for a steady tone, which would whiten itself away.
    per_bin: bool,
    /// Use Whistlestop's own coherent matched filter (today's detector).
    production: bool,
}

fn designs() -> Vec<Design> {
    vec![
        Design { key: "A", name: "today: wide chirps 2000 Hz, 0.5 s up/down", shape: Shape::UpDown { span: 2000.0, leg: 0.5 }, frame: 128, per_bin: true, production: false },
        Design { key: "A*", name: "today, with today's coherent matched filter", shape: Shape::UpDown { span: 2000.0, leg: 0.5 }, frame: 128, per_bin: true, production: true },
        Design { key: "B", name: "fast wide chirps 2000 Hz, 0.1 s up/down", shape: Shape::UpDown { span: 2000.0, leg: 0.1 }, frame: 64, per_bin: true, production: false },
        Design { key: "C", name: "fast narrow warble 400 Hz, 0.1 s up/down", shape: Shape::UpDown { span: 400.0, leg: 0.1 }, frame: 128, per_bin: true, production: false },
        Design { key: "D", name: "slow narrow warble 400 Hz, 0.5 s up/down", shape: Shape::UpDown { span: 400.0, leg: 0.5 }, frame: 320, per_bin: true, production: false },
        Design { key: "E", name: "steady tone", shape: Shape::Tone, frame: 1024, per_bin: false, production: false },
        Design { key: "F", name: "Costas 7-tone pulse, 40 ms, 25 Hz (175 Hz wide)", shape: Shape::Costas { sym: 0.04, spacing: 25.0 }, frame: 320, per_bin: true, production: false },
        Design { key: "G", name: "fast up-sweeps only 2000 Hz, 0.1 s", shape: Shape::Up { span: 2000.0, leg: 0.1 }, frame: 64, per_bin: true, production: false },
        Design { key: "I", name: "medium chirps 1000 Hz, 0.2 s up/down", shape: Shape::UpDown { span: 1000.0, leg: 0.2 }, frame: 128, per_bin: true, production: false },
        Design { key: "J", name: "Costas 7-tone pulse, 20 ms, 50 Hz (350 Hz wide)", shape: Shape::Costas { sym: 0.02, spacing: 50.0 }, frame: 160, per_bin: true, production: false },
        Design { key: "H", name: "tone pulses 0.1 s on / 0.1 s off", shape: Shape::Pulse { on: 0.1, off: 0.1 }, frame: 320, per_bin: false, production: false },
    ]
}

/// Complex audio of the design, centred at 1500 Hz + `offset`, starting
/// `t0` s into its pattern.
fn transmit(d: &Design, n: usize, offset: f64, t0: f64, rng: &mut Rng) -> Vec<C> {
    let mut ph = rng.range(0.0, 2.0 * PI);
    (0..n)
        .map(|i| {
            let t = t0 + i as f64 / FS;
            match d.shape.freq(t) {
                Some(f) => {
                    ph += 2.0 * PI * (1500.0 + offset + f) / FS;
                    C::from_polar(1.0, ph)
                }
                None => C::new(0.0, 0.0),
            }
        })
        .collect()
}

// ---------------------------------------------------------------- channel

/// A Rayleigh gain process (sum of sinusoids), E|g|² = power.
struct Fader {
    w: Vec<(f64, f64)>,
    power: f64,
}
impl Fader {
    fn new(fd: f64, power: f64, rng: &mut Rng) -> Fader {
        let m = 12;
        Fader { w: (0..m).map(|_| (2.0 * PI * fd * rng.range(0.0, 2.0 * PI).cos(), rng.range(0.0, 2.0 * PI))).collect(), power }
    }
    fn at(&self, t: f64) -> C {
        let s: C = self.w.iter().map(|(w, p)| C::from_polar(1.0, w * t + p)).sum();
        s * (self.power / self.w.len() as f64).sqrt()
    }
}

#[derive(Clone, Copy)]
enum Path {
    Awgn,
    /// ITU-R moderate-ish: two paths, each ~1 Hz Doppler spread, 1 ms apart.
    Moderate,
}

fn channel(x: &[C], path: Path, rng: &mut Rng) -> Vec<C> {
    match path {
        Path::Awgn => x.to_vec(),
        Path::Moderate => {
            let (g1, g2) = (Fader::new(0.5, 0.5, rng), Fader::new(0.5, 0.5, rng));
            let d = (0.001 * FS) as usize;
            (0..x.len()).map(|i| {
                let t = i as f64 / FS;
                g1.at(t) * x[i] + if i >= d { g2.at(t) * x[i - d] } else { C::new(0.0, 0.0) }
            }).collect()
        }
    }
}

/// A busy digital segment: FT8s, maybe a carrier, RTTY, PSK31, an OFDM
/// modem burst, each at a random level (SNR −15…+10 dB in 2500 Hz).
fn clutter(n: usize, rng: &mut Rng) -> Vec<C> {
    let mut y = vec![C::new(0.0, 0.0); n];
    let amp = |rng: &mut Rng| 10f64.powf(rng.range(-15.0, 10.0) / 20.0);
    let add_fsk = |y: &mut Vec<C>, base: f64, spacing: f64, sym: f64, tones: usize, a: f64, rng: &mut Rng| {
        let mut ph = 0.0;
        let mut tone = 0;
        let mut next = 0.0;
        for (i, v) in y.iter_mut().enumerate() {
            let t = i as f64 / FS;
            if t >= next {
                tone = (rng.u() * tones as f64) as usize;
                next += sym;
            }
            ph += 2.0 * PI * (base + tone as f64 * spacing) / FS;
            *v += C::from_polar(a, ph);
        }
    };
    for _ in 0..(1 + (rng.u() * 3.0) as usize) {
        let (b, a) = (rng.range(PB.0, PB.1 - 50.0), amp(rng));
        add_fsk(&mut y, b, 6.25, 0.16, 8, a, rng); // FT8
    }
    if rng.u() < 0.5 {
        let (b, a) = (rng.range(PB.0, PB.1), amp(rng));
        add_fsk(&mut y, b, 0.0, 1.0, 1, a, rng); // carrier
    }
    if rng.u() < 0.3 {
        let (b, a) = (rng.range(PB.0, PB.1 - 170.0), amp(rng));
        add_fsk(&mut y, b, 170.0, 0.022, 2, a, rng); // RTTY
    }
    if rng.u() < 0.3 {
        // PSK31: a tone with phase reversals.
        let (f, a) = (rng.range(PB.0, PB.1), amp(rng));
        let mut sign = 1.0;
        for (i, v) in y.iter_mut().enumerate() {
            if i % 256 == 0 && rng.u() < 0.5 {
                sign = -sign;
            }
            *v += C::from_polar(a * sign, 2.0 * PI * f * i as f64 / FS);
        }
    }
    if rng.u() < 0.3 {
        // OFDM modem (Mercury/VARA-like): 16–33 carriers, 62.5 Hz apart.
        let nc = 16 + (rng.u() * 17.0) as usize;
        let f0 = rng.range(PB.0, PB.1 - nc as f64 * 62.5);
        let a = amp(rng) / (nc as f64).sqrt();
        let mut phs: Vec<f64> = (0..nc).map(|_| rng.range(0.0, 2.0 * PI)).collect();
        for (i, v) in y.iter_mut().enumerate() {
            if i % 128 == 0 {
                for p in phs.iter_mut() {
                    *p = (rng.u() * 4.0).floor() * PI / 2.0;
                }
            }
            let t = i as f64 / FS;
            for (k, p) in phs.iter().enumerate() {
                *v += C::from_polar(a, 2.0 * PI * (f0 + k as f64 * 62.5) * t + p);
            }
        }
    }
    y
}

/// The receiver: noise at `snr_db` (for a unit signal, in 2500 Hz), then the
/// SSB filter.
fn receive(mut x: Vec<C>, snr_db: f64, rng: &mut Rng, planner: &mut FftPlanner<f64>) -> Vec<C> {
    let sigma2 = FS / (2500.0 * 10f64.powf(snr_db / 10.0));
    let s = sigma2.sqrt();
    for v in x.iter_mut() {
        *v += rng.cgauss() * s;
    }
    let n = x.len();
    let f = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    f.process(&mut x);
    for (k, v) in x.iter_mut().enumerate() {
        let hz = if k <= n / 2 { k as f64 * FS / n as f64 } else { (k as f64 - n as f64) * FS / n as f64 };
        if hz < PB.0 || hz > PB.1 {
            *v = C::new(0.0, 0.0);
        }
    }
    inv.process(&mut x);
    x.iter_mut().for_each(|v| *v /= n as f64);
    x
}

// ---------------------------------------------------------------- detector

/// Non-coherent time–frequency track detector: STFT, whiten, then for every
/// (pattern phase, frequency offset) add up the cells along the design's
/// track; returns the best z-score.
fn detect(d: &Design, x: &[C], max_off: f64, planner: &mut FftPlanner<f64>) -> f64 {
    let l = d.frame;
    let hop = l / 2;
    let bw = FS / l as f64;
    let fft = planner.plan_fft_forward(l);
    let win: Vec<f64> = (0..l).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / l as f64).cos()).collect();
    let (k0, k1) = ((PB.0 / bw).ceil() as usize, (PB.1 / bw).floor() as usize);
    let nb = k1 - k0 + 1;
    let mut rows: Vec<Vec<f64>> = vec![];
    let mut buf = vec![C::new(0.0, 0.0); l];
    let mut start = 0;
    while start + l <= x.len() {
        for i in 0..l {
            buf[i] = x[start + i] * win[i];
        }
        fft.process(&mut buf);
        rows.push((k0..=k1).map(|k| buf[k].norm_sqr()).collect());
        start += hop;
    }
    let nt = rows.len();
    if nt == 0 {
        return 0.0;
    }
    // Whiten to mean-1 exponential cells.
    let med = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2].max(1e-30) / std::f64::consts::LN_2
    };
    if d.per_bin && nt >= 5 {
        for b in 0..nb {
            let m = med(rows.iter().map(|r| r[b]).collect());
            rows.iter_mut().for_each(|r| r[b] /= m);
        }
    } else {
        let m = med(rows.iter().flatten().copied().collect());
        rows.iter_mut().flatten().for_each(|c| *c /= m);
    }
    // Prefix sums per row for range sums.
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
    let period_frames = ((d.shape.period() * FS) / hop as f64).round().max(1.0) as usize;
    let hop_s = hop as f64 / FS;
    let nd = (max_off / bw).ceil() as i64;
    let mut best = f64::MIN;
    for dt in 0..period_frames {
        for df in -nd..=nd {
            let (mut s, mut n) = (0.0, 0.0);
            for (t, p) in pre.iter().enumerate() {
                let tc = (t + dt) as f64 * hop_s;
                // Chirps sweep across the frame; tones are read at its centre.
                let chirp = matches!(d.shape, Shape::UpDown { .. } | Shape::Up { .. });
                let (fa, fb) = if chirp {
                    match (d.shape.freq(tc), d.shape.freq(tc + l as f64 / FS)) {
                        (Some(a), Some(b)) => (a, b),
                        _ => continue,
                    }
                } else {
                    match d.shape.freq(tc + l as f64 / FS / 2.0) {
                        Some(f) => (f, f),
                        None => continue,
                    }
                };
                let (lo, hi) = (fa.min(fb), fa.max(fb));
                // Wrapped sawtooth inside one frame: take the start leg only.
                let (lo, hi) = if hi - lo > d.shape.width() * 0.6 { (fa, fa) } else { (lo, hi) };
                let blo = ((1500.0 + lo) / bw).round() as i64 + df - k0 as i64;
                let bhi = ((1500.0 + hi) / bw).round() as i64 + df - k0 as i64;
                let (blo, bhi) = (blo.max(0), bhi.min(nb as i64 - 1));
                if blo > bhi {
                    continue;
                }
                s += p[bhi as usize + 1] - p[blo as usize];
                n += (bhi - blo + 1) as f64;
            }
            // Ignore hypotheses that put most of the track out of band.
            if n < 0.3 * nt as f64 {
                continue;
            }
            let z = (s - n) / n.sqrt();
            if z > best {
                best = z;
            }
        }
    }
    best
}

// ---------------------------------------------------------------- runs

struct Trial<'a> {
    d: &'a Design,
    dwell: f64,
    snr: f64,
    offset: f64,
    signal: bool,
    path: Path,
    seed: u64,
}

fn run_trial(t: &Trial, planner: &mut FftPlanner<f64>) -> f64 {
    let mut rng = Rng(t.seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let n = (t.dwell * FS) as usize;
    let mut x = clutter(n, &mut rng);
    if t.signal {
        let t0 = rng.range(0.0, t.d.shape.period());
        let s = channel(&transmit(t.d, n, t.offset, t0, &mut rng), t.path, &mut rng);
        x.iter_mut().zip(s).for_each(|(a, b)| *a += b);
    }
    let y = receive(x, t.snr, &mut rng, planner);
    if t.d.production {
        let real: Vec<f64> = y.iter().map(|c| c.re * 2f64.sqrt()).collect();
        let mut det = whistlestop::whistle::Detector::new(whistlestop::whistle::WhistleParams::default());
        return det.peak_level(&real);
    }
    detect(t.d, &y, 1200.0, planner)
}

fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T, &mut FftPlanner<f64>) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let hs: Vec<_> = items
            .chunks(chunk)
            .map(|c| {
                let f = &f;
                s.spawn(move || {
                    let mut p = FftPlanner::new();
                    c.iter().map(|i| f(i, &mut p)).collect::<Vec<R>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

/// Busy-band dwells through the app's own scan detector (whistle v2), for
/// setting its thresholds: score quantiles with no whistle, and how often
/// a whistle is found at each threshold.
fn calibrate() {
    use whistlestop::whistle::{TrackDetector, WhistleParams};
    let p = WhistleParams::default();
    let v2 = Design { key: "V2", name: "whistle v2", shape: Shape::UpDown { span: p.f_hi - p.f_lo, leg: p.chirp_s }, frame: 128, per_bin: true, production: false };
    let score = |d: &Design, dwell: f64, snr: f64, off: f64, signal: bool, seed: u64, planner: &mut FftPlanner<f64>| -> f64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let n = (dwell * FS) as usize;
        let mut x = clutter(n, &mut rng);
        if signal {
            let t0 = rng.range(0.0, d.shape.period());
            let s = channel(&transmit(d, n, off, t0, &mut rng), Path::Moderate, &mut rng);
            x.iter_mut().zip(s).for_each(|(a, b)| *a += b);
        }
        let y = receive(x, snr, &mut rng, planner);
        let real: Vec<f64> = y.iter().map(|c| c.re * 2f64.sqrt()).collect();
        TrackDetector::new(p).scan(&real).map(|h| h.z).unwrap_or(0.0)
    };
    for dwell in [0.5, 2.0] {
        let seeds: Vec<u64> = (0..2000).collect();
        let mut h0 = par_map(&seeds, |&i, pl| score(&v2, dwell, -10.0, 0.0, false, 7_000_000 + i, pl));
        h0.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |f: f64| h0[((h0.len() as f64 * f) as usize).min(h0.len() - 1)];
        println!("dwell {dwell} s, busy band, no whistle: z50 {:.2}  z90 {:.2}  z99 {:.2}  z99.5 {:.2}  z99.9 {:.2}", q(0.5), q(0.9), q(0.99), q(0.995), q(0.999));
        for snr in [-15.0, -10.0, -5.0] {
            for off in [0.0, 1000.0] {
                let seeds: Vec<u64> = (0..300).collect();
                let z = par_map(&seeds, |&i, pl| score(&v2, dwell, snr, if i % 2 == 0 { off } else { -off }, true, (i << 16) ^ off as u64, pl));
                let at = |t: f64| z.iter().filter(|v| **v > t).count() as f64 / z.len() as f64 * 100.0;
                println!(
                    "   snr {snr:>5} off ±{off:>4}: z>{:.2}(25% empty) {:5.1} %   z>{:.2}(15%) {:5.1} %   z>{:.2}(10%) {:5.1} %   z>{:.2}(1%) {:5.1} %",
                    q(0.75), at(q(0.75)), q(0.85), at(q(0.85)), q(0.9), at(q(0.9)), q(0.99), at(q(0.99))
                );
            }
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "calibrate") {
        return calibrate();
    }
    let quick = std::env::args().any(|a| a == "quick");
    let ds = Arc::new(designs());
    let dwells = [0.15, 0.25, 0.5, 1.0, 2.0];
    let snrs = [-15.0, -10.0, -5.0, 0.0];
    let offsets = [0.0, 200.0, 400.0, 600.0, 800.0, 1000.0, 1200.0];
    let (n_h0, n_h1) = if quick { (100, 20) } else { (500, 80) };

    // Thresholds: 99th percentile of busy-band dwells with no whistle.
    let mut thr = std::collections::HashMap::new();
    for d in ds.iter() {
        for &dw in &dwells {
            let trials: Vec<Trial> = (0..n_h0).map(|i| Trial { d, dwell: dw, snr: -15.0, offset: 0.0, signal: false, path: Path::Moderate, seed: 1_000_000 + i as u64 }).collect();
            let mut v = par_map(&trials, run_trial);
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let th = v[(v.len() as f64 * 0.99) as usize - 1];
            thr.insert((d.key, (dw * 1000.0) as u32), th);
        }
    }

    // Detection probability grid.
    println!("design,dwell_s,snr_db,offset_hz,path,pd,threshold");
    let mut pd = std::collections::HashMap::new();
    for path in [Path::Moderate, Path::Awgn] {
        let pname = match path {
            Path::Moderate => "moderate",
            Path::Awgn => "awgn",
        };
        for d in ds.iter() {
            for &dw in &dwells {
                let th = thr[&(d.key, (dw * 1000.0) as u32)];
                for &snr in &snrs {
                    for &off in &offsets {
                        let trials: Vec<Trial> = (0..n_h1)
                            .map(|i| Trial { d, dwell: dw, snr, offset: if i % 2 == 0 { off } else { -off }, signal: true, path, seed: (i as u64) << 20 ^ (off as u64) << 8 ^ (snr.abs() as u64) })
                            .collect();
                        let hits = par_map(&trials, run_trial).iter().filter(|z| **z > th).count();
                        let p = hits as f64 / n_h1 as f64;
                        println!("{},{dw},{snr},{off},{pname},{p:.3},{th:.2}", d.key);
                        pd.insert((d.key, (dw * 1000.0) as u32, snr as i32, off as u32, pname), p);
                    }
                }
            }
        }
    }

    // Summary: widest step with Pd ≥ 0.9 at every offset within ±step/2,
    // scan speed, and whistle length to sweep 12 kHz.
    eprintln!("\n## Scan speed at Pd ≥ 90 % per dwell, 1 % false stops, moderate fading\n");
    for &snr in &snrs {
        eprintln!("### SNR {snr} dB (2500 Hz)\n");
        eprintln!("| | design | best dwell | channel step | scan speed | whistle needed for 12 kHz |");
        eprintln!("|---|---|---|---|---|---|");
        for d in ds.iter() {
            let mut best: Option<(f64, f64, f64)> = None; // (speed, dwell, step)
            for &dw in &dwells {
                let mut reach = None;
                for &off in &offsets {
                    if pd[&(d.key, (dw * 1000.0) as u32, snr as i32, off as u32, "moderate")] >= 0.9 {
                        reach = Some(off);
                    } else {
                        break;
                    }
                }
                if let Some(r) = reach {
                    let step = (2.0 * r).max(100.0);
                    let speed = step / dw;
                    if best.is_none_or(|b| speed > b.0) {
                        best = Some((speed, dw, step));
                    }
                }
            }
            match best {
                Some((sp, dw, st)) => eprintln!("| {} | {} | {dw} s | {st:.0} Hz | {:.1} kHz/s | {:.0} s |", d.key, d.name, sp / 1000.0, 12000.0 / sp + dw),
                None => eprintln!("| {} | {} | — | — | not found reliably | — |", d.key, d.name),
            }
        }
        eprintln!();
    }
}
