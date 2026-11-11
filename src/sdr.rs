//! Optional second receiver: an RTL-SDR dongle for a wide band waterfall.
//!
//! Same approach as RigHelm's band scope:
//! * pure-Rust USB driver (`rtlsdr-nusb`), nothing to install;
//! * **RX OUT** (antenna RF): the dongle samples HF directly (V3 Q-branch)
//!   or through its upconverter (Blog V4), and retunes to keep the radio's
//!   VFO in view;
//! * **IF OUT** (the radio's first IF): the dongle stays on the IF and the
//!   picture follows the VFO.
//!
//! Nothing here ever transmits. The simulator draws a band picture for Demo
//! mode.

use num_complex::Complex32;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SAMPLE_RATE: u32 = 2_048_000;
const FFT_SIZE: usize = 32_768; // 62.5 Hz bins
pub const COLS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Feed {
    #[default]
    RxOut,
    IfOut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SdrConfig {
    pub enabled: bool,
    pub feed: Feed,
    /// IF OUT: IF frequency matching the receive dial, and its direction.
    pub if_hz: u32,
    pub if_inverted: bool,
    pub ppm: i32,
    /// Width of the picture, Hz.
    pub span_hz: f64,
    /// Search the whole band segment for whistles on the SDR.
    pub wide_search: bool,
    /// ...and tune the main radio to what it finds.
    pub wide_auto: bool,
}

impl Default for SdrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            feed: Feed::RxOut,
            if_hz: 9_000_000,
            if_inverted: true,
            ppm: 0,
            span_hz: 100_000.0,
            wide_search: true,
            wide_auto: false,
        }
    }
}

/// One waterfall line: `levels[i]` is the dB level at
/// `start_hz + (i + 0.5) * bin_hz`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WideRow {
    pub start_hz: f64,
    pub bin_hz: f64,
    pub levels: Vec<f32>,
}

/// What the wide whistle search should watch (set by the engine; None = off).
#[derive(Clone, Debug, PartialEq)]
pub struct WideCfg {
    pub segments: Vec<crate::bandplan::Segment>,
    pub params: crate::whistle::WhistleParams,
    pub step_hz: f64,
}

#[derive(Default)]
pub struct SdrShared {
    pub rows: std::collections::VecDeque<WideRow>,
    pub status: String,
    pub running: bool,
    pub wide: Option<WideCfg>,
    /// Whistles found by the wide search, refreshed while they last.
    pub hits: Vec<(std::time::Instant, crate::widesearch::WideHit)>,
    /// e.g. "watching 20m (2 segments, 10 receivers)".
    pub wide_note: String,
}

// ------------------------------------------------------- wide whistle search

use crate::widesearch::{Decimator, WideWatch};

enum WideMsg {
    Configure(Vec<WideWatch>),
    Samples(usize, Vec<Complex32>),
}

/// The SDR thread's side of the wide search: decimates IQ per watched
/// segment and hands the result to a worker thread that scans it.
struct WideFront {
    cfg: Option<WideCfg>,
    ref_rf: f64,
    decs: Vec<Decimator>,
    /// (stream centre, rate) per watch, for the simulator.
    streams: Vec<(f64, u32)>,
    tx: std::sync::mpsc::Sender<WideMsg>,
    sim_phase: Vec<f64>,
    sim_t: f64,
    seed: u64,
}

impl WideFront {
    fn new(out: Shared) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<WideMsg>();
        std::thread::spawn(move || {
            let mut watches: Vec<WideWatch> = vec![];
            while let Ok(m) = rx.recv() {
                match m {
                    WideMsg::Configure(w) => watches = w,
                    WideMsg::Samples(i, y) => {
                        let Some(w) = watches.get_mut(i) else {
                            continue;
                        };
                        w.push(&y);
                        if !w.due() {
                            continue;
                        }
                        let found = w.scan();
                        let now = std::time::Instant::now();
                        let mut o = out.lock().unwrap();
                        for h in found {
                            match o
                                .hits
                                .iter_mut()
                                .find(|(_, x)| (x.dial_hz - h.dial_hz).abs() < 100.0)
                            {
                                Some(e) => *e = (now, h),
                                None => o.hits.push((now, h)),
                            }
                        }
                        o.hits
                            .retain(|(at, _)| at.elapsed() < Duration::from_secs(60));
                    }
                }
            }
        });
        Self {
            cfg: None,
            ref_rf: 0.0,
            decs: vec![],
            streams: vec![],
            tx,
            sim_phase: vec![],
            sim_t: 0.0,
            seed: 99,
        }
    }

    /// Rebuild the watches when the settings or the stream's reference
    /// frequency change. `half` = usable half-width of the capture.
    fn update(&mut self, out: &Shared, ref_rf: f64, half: f64) {
        let cfg = out.lock().unwrap().wide.clone();
        if cfg == self.cfg && (ref_rf - self.ref_rf).abs() < 1.0 {
            return;
        }
        self.cfg = cfg.clone();
        self.ref_rf = ref_rf;
        let mut watches = vec![];
        self.decs.clear();
        self.streams.clear();
        if let Some(c) = &cfg {
            for seg in c
                .segments
                .iter()
                .filter(|s| (s.lo_hz - ref_rf).abs() < half && (s.hi_hz - ref_rf).abs() < half)
            {
                let w = WideWatch::new(seg.clone(), c.params, c.step_hz);
                let decim = (SAMPLE_RATE / w.fs) as usize;
                self.decs.push(Decimator::new(
                    SAMPLE_RATE as f64,
                    decim,
                    w.center_rf - ref_rf,
                ));
                self.streams.push((w.center_rf, w.fs));
                watches.push(w);
            }
        }
        self.sim_phase = vec![0.0; watches.len()];
        let receivers: usize = watches.iter().map(|w| w.channels().len()).sum();
        let bands: Vec<String> = {
            let mut b: Vec<String> = watches.iter().map(|w| w.seg.band.clone()).collect();
            b.dedup();
            b
        };
        let mut o = out.lock().unwrap();
        o.wide_note = match (&cfg, watches.len()) {
            (None, _) => String::new(),
            (Some(_), 0) => "no digital segment inside the SDR's view".into(),
            (Some(_), n) => format!(
                "watching {} ({n} segment{}, {receivers} virtual receivers)",
                bands.join(", "),
                if n == 1 { "" } else { "s" }
            ),
        };
        o.hits.clear();
        let _ = self.tx.send(WideMsg::Configure(watches));
    }

    /// Dongle IQ at SAMPLE_RATE, already upright (IF inversion undone).
    fn feed(&mut self, x: &[Complex32]) {
        for (i, d) in self.decs.iter_mut().enumerate() {
            let mut y = Vec::with_capacity(x.len() / 8 + 1);
            d.process(x, &mut y);
            let _ = self.tx.send(WideMsg::Samples(i, y));
        }
    }

    /// Demo mode: synthesize each segment's decimated stream directly — the
    /// demo station's whistle, a carrier, and noise.
    fn feed_sim(&mut self, dt: f64) {
        let p = crate::whistle::WhistleParams::default();
        let t0 = self.sim_t;
        self.sim_t += dt;
        for i in 0..self.streams.len() {
            let (center, fs) = self.streams[i];
            let n = (dt * fs as f64) as usize;
            let mut y: Vec<Complex32> = (0..n)
                .map(|_| {
                    let mut g = || {
                        self.seed ^= self.seed << 13;
                        self.seed ^= self.seed >> 7;
                        self.seed ^= self.seed << 17;
                        (self.seed >> 11) as f32 / (1u64 << 53) as f32 - 0.5
                    };
                    Complex32::new(g(), g()) * 0.17
                })
                .collect();
            // Same schedule as the demo station on the radio-audio side. Like a
            // real dongle, this one's crystal is off (17 ppm: +240 Hz here),
            // which the main radio's fine lock has to take out.
            const CRYSTAL_ERR_HZ: f64 = 240.0;
            let (bot, start, whistle_s, cycle) = (14_103_150.0 + CRYSTAL_ERR_HZ, 4.0, 10.0, 30.0);
            let ph = (t0 - start).rem_euclid(cycle);
            if t0 >= start && ph < whistle_s {
                let w = crate::widesearch::complex_whistle(
                    fs as f64,
                    n,
                    bot - center,
                    ph,
                    &p,
                    &mut self.sim_phase[i],
                );
                y.iter_mut().zip(w).for_each(|(a, b)| *a += b * 0.03);
            }
            let fc = 14_101_900.0 + CRYSTAL_ERR_HZ - center;
            for (k, a) in y.iter_mut().enumerate() {
                let t = t0 + k as f64 / fs as f64;
                *a += Complex32::from_polar(0.05, (2.0 * std::f64::consts::PI * fc * t) as f32);
            }
            let _ = self.tx.send(WideMsg::Samples(i, y));
        }
    }
}

pub type Shared = Arc<Mutex<SdrShared>>;

/// Start the wide receiver thread. `vfo` is the radio's dial (shared),
/// `span` the picture width (shared, the GUI's zoom).
pub fn start(
    cfg: SdrConfig,
    vfo: Arc<Mutex<f64>>,
    span: Arc<Mutex<f64>>,
    sim: bool,
    out: Shared,
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        out.lock().unwrap().running = true;
        let r = if sim {
            run_sim(&vfo, &span, &out, &stop)
        } else {
            run_dongle(&cfg, &vfo, &span, &out, &stop)
        };
        let mut o = out.lock().unwrap();
        o.running = false;
        if let Err(e) = r {
            o.status = format!("Wide receiver stopped: {e}");
        }
    });
}

fn push_row(out: &Shared, row: WideRow) {
    let mut o = out.lock().unwrap();
    o.rows.push_back(row);
    while o.rows.len() > 64 {
        o.rows.pop_front();
    }
}

/// Reduce FFT bins (already fft-shifted, centre = `center_rf`) to COLS
/// columns covering `[lo, lo + span)`, taking the peak in each column.
fn to_row(bins_db: &[f32], center_rf: f64, inverted: bool, lo: f64, span: f64) -> WideRow {
    let n = bins_db.len();
    let bin_hz = SAMPLE_RATE as f64 / n as f64;
    let col_hz = span / COLS as f64;
    let mut levels = vec![f32::NEG_INFINITY; COLS];
    for (c, lv) in levels.iter_mut().enumerate() {
        let f0 = lo + c as f64 * col_hz;
        let f1 = f0 + col_hz;
        let (a, b) = {
            let sgn = if inverted { -1.0 } else { 1.0 };
            let x0 = (sgn * (f0 - center_rf)) / bin_hz + n as f64 / 2.0;
            let x1 = (sgn * (f1 - center_rf)) / bin_hz + n as f64 / 2.0;
            (x0.min(x1).floor() as i64, x0.max(x1).ceil() as i64)
        };
        for i in a.max(0)..b.min(n as i64).max(a.max(0) + 1) {
            if (i as usize) < n {
                *lv = lv.max(bins_db[i as usize]);
            }
        }
        if !lv.is_finite() {
            *lv = -120.0;
        }
    }
    WideRow {
        start_hz: lo,
        bin_hz: col_hz,
        levels,
    }
}

fn run_dongle(
    cfg: &SdrConfig,
    vfo: &Arc<Mutex<f64>>,
    span: &Arc<Mutex<f64>>,
    out: &Shared,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    use rtlsdr_nusb::{Device, DirectSampling, MaybeFuture};
    let list = Device::list().wait().map_err(|e| anyhow::anyhow!("{e}"))?;
    let desc = list
        .first()
        .ok_or_else(|| anyhow::anyhow!("no RTL-SDR dongle found"))?;
    let v4 = desc.product.as_deref() == Some("Blog V4");
    let direct = if v4 {
        DirectSampling::Off
    } else {
        DirectSampling::Q
    };
    let tune_hz = |vfo_hz: f64| -> u64 {
        match cfg.feed {
            Feed::RxOut => vfo_hz.max(0.0) as u64,
            Feed::IfOut => cfg.if_hz as u64,
        }
    };
    let mut center = *vfo.lock().unwrap();
    let mut dev = Device::builder()
        .index(desc.index)
        .frequency_hz(tune_hz(center))
        .sample_rate_hz(SAMPLE_RATE)
        .correction_ppm(cfg.ppm)
        .direct_sampling(direct)
        .open()
        .wait()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut rx = dev.rx_stream().map_err(|e| anyhow::anyhow!("{e}"))?;
    rx.start().wait().map_err(|e| anyhow::anyhow!("{e}"))?;
    out.lock().unwrap().status = format!(
        "RTL-SDR {} on {}",
        desc.product.clone().unwrap_or_default(),
        if cfg.feed == Feed::RxOut {
            "RX OUT"
        } else {
            "IF OUT"
        }
    );

    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
        .collect();
    let mut buf = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
    let mut wide = WideFront::new(out.clone());
    while !stop.load(Ordering::Relaxed) {
        let mut got = 0;
        while got < FFT_SIZE {
            got += rx
                .read(&mut buf[got..], Some(Duration::from_millis(500)))
                .wait()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        let vf = *vfo.lock().unwrap();
        let sp = *span.lock().unwrap();
        // RX OUT: retune when the VFO drifts out of the middle of the capture.
        if cfg.feed == Feed::RxOut && (vf - center).abs() > 0.25 * SAMPLE_RATE as f64 {
            center = vf;
            rx.stop().wait().ok();
            dev.set_frequency_hz(tune_hz(center))
                .wait()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            rx.start().wait().map_err(|e| anyhow::anyhow!("{e}"))?;
            continue;
        }
        // Wide whistle search, in upright radio frequencies.
        let (ref_rf, upright) = match cfg.feed {
            Feed::RxOut => (center, false),
            Feed::IfOut => (vf, cfg.if_inverted),
        };
        wide.update(out, ref_rf, 0.4 * SAMPLE_RATE as f64);
        if !wide.decs.is_empty() {
            if upright {
                let c: Vec<Complex32> = buf.iter().map(|z| z.conj()).collect();
                wide.feed(&c);
            } else {
                wide.feed(&buf);
            }
        }
        let mean = buf.iter().sum::<Complex32>() / FFT_SIZE as f32; // remove DC
        let mut x: Vec<Complex32> = buf
            .iter()
            .zip(&window)
            .map(|(s, w)| (s - mean) * *w)
            .collect();
        fft.process(&mut x);
        let n = FFT_SIZE;
        let db: Vec<f32> = (0..n)
            .map(|i| 10.0 * (x[(i + n / 2) % n].norm_sqr() + 1e-12).log10())
            .collect();
        let (center_rf, inverted) = match cfg.feed {
            Feed::RxOut => (center, false),
            // IF OUT: the IF's centre is the dial; inverted IFs mirror the band.
            Feed::IfOut => (vf, cfg.if_inverted),
        };
        let lo = vf - sp / 2.0;
        push_row(out, to_row(&db, center_rf, inverted, lo, sp));
    }
    let _ = rx.close().wait();
    let _ = dev.shutdown().wait();
    Ok(())
}

/// Demo mode: a believable band picture, drawn in the frequency domain.
fn run_sim(
    vfo: &Arc<Mutex<f64>>,
    span: &Arc<Mutex<f64>>,
    out: &Shared,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    out.lock().unwrap().status = "Simulated wide receiver (Demo)".into();
    let mut seed = 7u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f32 / (1u64 << 53) as f32
    };
    let t0 = std::time::Instant::now();
    // (rf, width, level dB, kind) kind: 0 steady, 1 FT8 cycle, 2 whistle, 3 CW keying
    let ft8: Vec<f64> = (0..28)
        .map(|i| 14_074_300.0 + (i as f64 * 97.0) % 2600.0)
        .collect();
    let mut wide = WideFront::new(out.clone());
    let mut last = 0.0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(60));
        let t = t0.elapsed().as_secs_f64();
        let vf = *vfo.lock().unwrap();
        // The simulated dongle sits on 14.2 MHz, like RX OUT on 20 m.
        wide.update(out, 14_200_000.0, 0.4 * SAMPLE_RATE as f64);
        wide.feed_sim(t - last);
        last = t;
        let sp = *span.lock().unwrap();
        let lo = vf - sp / 2.0;
        let col = sp / COLS as f64;
        let mut lv: Vec<f32> = (0..COLS).map(|_| -112.0 + 6.0 * rnd()).collect();
        let paint = |rf: f64, width: f64, db: f32, lv: &mut Vec<f32>| {
            let a = ((rf - width / 2.0 - lo) / col).floor() as i64;
            let b = ((rf + width / 2.0 - lo) / col).ceil() as i64;
            for i in a.max(0)..b.min(COLS as i64) {
                lv[i as usize] = lv[i as usize].max(db);
            }
        };
        for (k, f) in ft8.iter().enumerate() {
            if (t + k as f64 * 0.3).rem_euclid(15.0) < 12.6 && k % 3 != 0 {
                paint(*f, 50.0, -78.0 - (k % 5) as f32 * 4.0, &mut lv);
            }
        }
        // The demo station's whistle (14.10315 MHz dial + 500..2500 Hz).
        let cyc = t.rem_euclid(45.0);
        if cyc < 25.0 {
            let ph = cyc.rem_euclid(1.0);
            let af = if ph < 0.5 {
                500.0 + 4000.0 * ph
            } else {
                2500.0 - 4000.0 * (ph - 0.5)
            };
            paint(14_103_150.0 + af, 60.0, -74.0, &mut lv);
        } else if cyc < 28.1 {
            paint(14_103_150.0 + 1500.0, 200.0, -80.0, &mut lv);
        }
        // Other activity: CW, a Mercury/VARA-like wide data signal, carriers.
        for (i, f) in [14_021_000.0, 14_033_500.0, 14_046_200.0]
            .iter()
            .enumerate()
        {
            if ((t * (5.0 + i as f64)).sin() + (t * 3.1).cos()) > 0.2 {
                paint(*f, 40.0, -82.0, &mut lv);
            }
        }
        if t.rem_euclid(20.0) < 14.0 {
            paint(14_108_500.0, 2300.0, -88.0, &mut lv);
        }
        paint(14_100_000.0, 30.0, -76.0, &mut lv); // NCDXF beacon area
        push_row(
            out,
            WideRow {
                start_hz: lo,
                bin_hz: col,
                levels: lv,
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_mapping_rx_out_and_inverted_if() {
        // A peak 10 kHz above centre lands 10 kHz above centre (RX OUT) and
        // mirrored to 10 kHz below for an inverted IF.
        let n = FFT_SIZE;
        let bin_hz = SAMPLE_RATE as f64 / n as f64;
        let mut db = vec![-100.0f32; n];
        db[n / 2 + (10_000.0 / bin_hz) as usize] = 0.0;
        let r = to_row(&db, 14_000_000.0, false, 13_950_000.0, 100_000.0);
        let peak = r
            .levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let f = r.start_hz + (peak as f64 + 0.5) * r.bin_hz;
        assert!((f - 14_010_000.0).abs() < 200.0, "{f}");
        let r = to_row(&db, 14_000_000.0, true, 13_950_000.0, 100_000.0);
        let peak = r
            .levels
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let f = r.start_hz + (peak as f64 + 0.5) * r.bin_hz;
        assert!((f - 13_990_000.0).abs() < 200.0, "{f}");
    }
}
