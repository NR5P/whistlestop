//! Audio: the radio's sound card (cpal), a simulated band for Demo mode,
//! the shared receive buffer everything reads from, and the waterfall FFT.

use crate::whistle::{FS, WhistleParams, ssb_channel, whistle};
use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use num_complex::Complex32;
use rustfft::FftPlanner;
use std::collections::VecDeque;
use std::f64::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ------------------------------------------------------------- shared RX ---

/// The last minute of received audio at 8 kHz, plus a running sample count.
/// The audio thread writes it; search, whistle watch and the waterfall read.
#[derive(Default)]
pub struct RxBuffer {
    samples: VecDeque<f32>,
    /// Total samples ever written.
    pub count: u64,
    pub level: f32,
}

pub type SharedRx = Arc<Mutex<RxBuffer>>;

const KEEP: usize = FS as usize * 60;

impl RxBuffer {
    pub fn push(&mut self, x: &[f32]) {
        self.samples.extend(x.iter().copied());
        self.count += x.len() as u64;
        if self.samples.len() > KEEP {
            let extra = self.samples.len() - KEEP;
            self.samples.drain(..extra);
        }
        if !x.is_empty() {
            self.level = (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        }
    }

    /// Samples `[from, from + n)` by absolute count, if all present.
    pub fn range(&self, from: u64, n: usize) -> Option<Vec<f64>> {
        let first = self.count - self.samples.len() as u64;
        if from < first || from + n as u64 > self.count {
            return None;
        }
        let i = (from - first) as usize;
        Some(self.samples.range(i..i + n).map(|&v| v as f64).collect())
    }

    /// The most recent `n` samples.
    pub fn last(&self, n: usize) -> Vec<f64> {
        let n = n.min(self.samples.len());
        self.samples.range(self.samples.len() - n..).map(|&v| v as f64).collect()
    }
}

/// Wait until `n` samples after `from` have been received; return them.
pub async fn wait_samples(rx: &SharedRx, from: u64, n: usize) -> Vec<f64> {
    loop {
        if let Some(v) = rx.lock().unwrap().range(from, n) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

pub fn now_count(rx: &SharedRx) -> u64 {
    rx.lock().unwrap().count
}

// ------------------------------------------------------------- resampling --

fn lowpass(taps: usize, cutoff: f64, fs: f64) -> Vec<f32> {
    let m = (taps - 1) as f64 / 2.0;
    let mut h: Vec<f64> = (0..taps)
        .map(|i| {
            let n = i as f64 - m;
            let x = 2.0 * cutoff / fs * n;
            let sinc = if n == 0.0 { 1.0 } else { (PI * x).sin() / (PI * x) };
            sinc * (0.54 - 0.46 * (2.0 * PI * i as f64 / (taps - 1) as f64).cos())
        })
        .collect();
    let s: f64 = h.iter().sum();
    h.iter_mut().for_each(|v| *v /= s);
    h.into_iter().map(|v| v as f32).collect()
}

/// Device rate -> 8 kHz: FIR low-pass, then fractional decimation.
struct Downsampler {
    h: Vec<f32>,
    hist: Vec<f32>,
    step: f64,
    pos: f64,
}

impl Downsampler {
    fn new(in_rate: u32) -> Self {
        let h = lowpass(127, 3400.0, in_rate as f64);
        Self { hist: vec![0.0; h.len()], h, step: in_rate as f64 / FS as f64, pos: 0.0 }
    }

    fn process(&mut self, x: &[f32], out: &mut Vec<f32>) {
        let l = self.h.len();
        for &v in x {
            self.hist.rotate_left(1);
            self.hist[l - 1] = v;
            self.pos += 1.0;
            if self.pos >= self.step {
                self.pos -= self.step;
                out.push(self.hist.iter().zip(&self.h).map(|(a, b)| a * b).sum());
            }
        }
    }
}

// ------------------------------------------------------------ sound cards --

pub fn list_devices() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let names = |it: Option<Box<dyn Iterator<Item = cpal::Device>>>| -> Vec<String> {
        it.map(|i| i.filter_map(|d| d.description().ok().map(|n| n.name().to_string())).collect()).unwrap_or_default()
    };
    let ins = names(host.input_devices().ok().map(|i| Box::new(i) as Box<dyn Iterator<Item = _>>));
    let outs = names(host.output_devices().ok().map(|i| Box::new(i) as Box<dyn Iterator<Item = _>>));
    (ins, outs)
}

fn find_device(name: &str, input: bool) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if name.is_empty() {
        return if input { host.default_input_device() } else { host.default_output_device() }
            .ok_or_else(|| anyhow!("no default {} device", if input { "input" } else { "output" }));
    }
    let mut devs: Box<dyn Iterator<Item = cpal::Device>> =
        if input { Box::new(host.input_devices()?) } else { Box::new(host.output_devices()?) };
    devs.find(|d| d.description().map(|n| n.name().to_lowercase().contains(&name.to_lowercase())).unwrap_or(false))
        .ok_or_else(|| anyhow!("no audio device matching {name:?}"))
}

/// Runs a capture stream on its own thread (cpal streams aren't Send on
/// every platform) and feeds the shared RX buffer at 8 kHz.
pub struct LiveInput {
    stop: Arc<AtomicBool>,
}

impl LiveInput {
    pub fn start(device_name: &str, rx: SharedRx) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let name = device_name.to_string();
        let st = stop.clone();
        std::thread::spawn(move || {
            let build = || -> Result<cpal::Stream> {
                let dev = find_device(&name, true)?;
                let cfg = dev.default_input_config()?;
                let rate = cfg.sample_rate();
                let ch = cfg.channels() as usize;
                let mut ds = Downsampler::new(rate);
                let mut out = Vec::new();
                let stream = dev.build_input_stream::<f32, _, _>(
                    cfg.config(),
                    move |data: &[f32], _| {
                        let mono: Vec<f32> = data.chunks(ch).map(|f| f[0]).collect();
                        out.clear();
                        ds.process(&mono, &mut out);
                        rx.lock().unwrap().push(&out);
                    },
                    |e| eprintln!("audio input error: {e}"),
                    None,
                )?;
                stream.play()?;
                Ok(stream)
            };
            match build() {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    while !st.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("audio input did not start"))??;
        Ok(Self { stop })
    }
}

impl LiveInput {
    /// Receive audio from the test ether (f32 LE at 8 kHz over TCP).
    pub fn ether(mut stream: std::net::TcpStream, rx: SharedRx) -> Self {
        use std::io::Read;
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 8192];
            let mut carry: Vec<u8> = vec![];
            while !st.load(Ordering::Relaxed) {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                carry.extend_from_slice(&buf[..n]);
                let whole = carry.len() / 4 * 4;
                let x: Vec<f32> = carry[..whole].chunks(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
                carry.drain(..whole);
                rx.lock().unwrap().push(&x);
            }
        });
        Self { stop }
    }
}

impl Drop for LiveInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Plays audio (the whistle) to the radio. Owns an output stream on its own
/// thread; `play` queues samples at the device rate and waits until done.
pub struct LiveOutput {
    queue: Arc<Mutex<VecDeque<f32>>>,
    pub rate: u32,
    stop: Arc<AtomicBool>,
}

impl LiveOutput {
    pub fn start(device_name: &str) -> Result<Self> {
        let queue: Arc<Mutex<VecDeque<f32>>> = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32>>();
        let (q, st, name) = (queue.clone(), stop.clone(), device_name.to_string());
        std::thread::spawn(move || {
            let build = || -> Result<(cpal::Stream, u32)> {
                let dev = find_device(&name, false)?;
                let cfg = dev.default_output_config()?;
                let rate = cfg.sample_rate();
                let ch = cfg.channels() as usize;
                let stream = dev.build_output_stream::<f32, _, _>(
                    cfg.config(),
                    move |data: &mut [f32], _| {
                        let mut q = q.lock().unwrap();
                        for frame in data.chunks_mut(ch) {
                            let v = q.pop_front().unwrap_or(0.0);
                            frame.iter_mut().for_each(|s| *s = v);
                        }
                    },
                    |e| eprintln!("audio output error: {e}"),
                    None,
                )?;
                stream.play()?;
                Ok((stream, rate))
            };
            match build() {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    while !st.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        let rate = ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("audio output did not start"))??;
        Ok(Self { queue, rate, stop })
    }

    /// Send audio to the test ether, paced in real time at 8 kHz.
    pub fn ether(mut stream: std::net::TcpStream) -> Self {
        use std::io::Write;
        let queue: Arc<Mutex<VecDeque<f32>>> = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (q, st) = (queue.clone(), stop.clone());
        std::thread::spawn(move || {
            let block = FS as usize / 50;
            let mut next = std::time::Instant::now();
            while !st.load(Ordering::Relaxed) {
                let chunk: Vec<f32> = {
                    let mut q = q.lock().unwrap();
                    let n = q.len().min(block);
                    q.drain(..n).collect()
                };
                if !chunk.is_empty() {
                    let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                    if stream.write_all(&bytes).is_err() {
                        return;
                    }
                }
                next += Duration::from_millis(20);
                if let Some(w) = next.checked_duration_since(std::time::Instant::now()) {
                    std::thread::sleep(w);
                } else {
                    next = std::time::Instant::now();
                }
            }
        });
        Self { queue, rate: FS, stop }
    }

    pub async fn play(&self, samples: &[f32]) {
        self.queue.lock().unwrap().extend(samples.iter().copied());
        while !self.queue.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(60)).await; // device buffer
    }

    pub fn abort(&self) {
        self.queue.lock().unwrap().clear();
    }
}

impl Drop for LiveOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------- speaker monitor

/// Plays what the radio hears (and, in Demo mode, what you send) through
/// the computer's speakers. Two streams are mixed: the band (kept under half
/// a second behind, so it stays live) and your own transmissions (played in
/// full).
pub struct Monitor {
    rx: Arc<Mutex<VecDeque<f32>>>,
    tx: Arc<Mutex<VecDeque<f32>>>,
    volume: Arc<std::sync::atomic::AtomicU32>,
    pub rate: u32,
    stop: Arc<AtomicBool>,
    up_pos: f64,
    up_prev: f32,
}

impl Monitor {
    pub fn start(device_name: &str, volume: f32) -> Result<Self> {
        let rx: Arc<Mutex<VecDeque<f32>>> = Default::default();
        let tx: Arc<Mutex<VecDeque<f32>>> = Default::default();
        let vol = Arc::new(std::sync::atomic::AtomicU32::new(volume.to_bits()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32>>();
        let (r, t, v, st, name) = (rx.clone(), tx.clone(), vol.clone(), stop.clone(), device_name.to_string());
        std::thread::spawn(move || {
            let build = || -> Result<(cpal::Stream, u32)> {
                let dev = find_device(&name, false)?;
                let cfg = dev.default_output_config()?;
                let rate = cfg.sample_rate();
                let ch = cfg.channels() as usize;
                let stream = dev.build_output_stream::<f32, _, _>(
                    cfg.config(),
                    move |data: &mut [f32], _| {
                        let g = f32::from_bits(v.load(Ordering::Relaxed));
                        let (mut r, mut t) = (r.lock().unwrap(), t.lock().unwrap());
                        for frame in data.chunks_mut(ch) {
                            let s = (r.pop_front().unwrap_or(0.0) + t.pop_front().unwrap_or(0.0)) * g;
                            frame.iter_mut().for_each(|x| *x = s.clamp(-1.0, 1.0));
                        }
                    },
                    |e| eprintln!("speaker monitor error: {e}"),
                    None,
                )?;
                stream.play()?;
                Ok((stream, rate))
            };
            match build() {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    while !st.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        let rate = ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("speaker output did not start"))??;
        Ok(Self { rx, tx, volume: vol, rate, stop, up_pos: 0.0, up_prev: 0.0 })
    }

    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
    }

    /// Band audio at 8 kHz: resampled to the speaker rate, kept live.
    pub fn push_rx(&mut self, x: &[f32]) {
        let step = FS as f64 / self.rate as f64;
        let mut out = Vec::with_capacity((x.len() as f64 / step) as usize + 2);
        for &s in x {
            // Linear interpolation from the previous sample to this one.
            while self.up_pos < 1.0 {
                out.push(self.up_prev + (s - self.up_prev) * self.up_pos as f32);
                self.up_pos += step;
            }
            self.up_pos -= 1.0;
            self.up_prev = s;
        }
        let mut q = self.rx.lock().unwrap();
        q.extend(out);
        let max = self.rate as usize / 2;
        if q.len() > max {
            let extra = q.len() - max / 2;
            q.drain(..extra);
        }
    }

    /// Your own transmission, already at the speaker rate: played in full.
    pub fn push_tx(&self, x: &[f32]) {
        self.tx.lock().unwrap().extend(x.iter().copied());
    }

    pub fn clear_tx(&self) {
        self.tx.lock().unwrap().clear();
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What a Mercury CQ burst roughly sounds like (DATAC16 is a ~3 s OFDM
/// burst about 200 Hz wide): 8 carriers around 1500 Hz, random QPSK each
/// 40 ms symbol. For listening only; Mercury makes the real one.
pub fn ofdm_burst(fs: f64, secs: f64, seed: &mut u64) -> Vec<f32> {
    let mut rnd = move || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 61) as usize
    };
    let carriers = 8;
    let ns = (0.04 * fs) as usize;
    let n_sym = (secs / 0.04) as usize;
    let mut out = Vec::with_capacity(n_sym * ns);
    for k in 0..n_sym {
        let ph: Vec<f64> = (0..carriers).map(|c| if k < 2 { (c % 2) as f64 * PI } else { (rnd() % 4) as f64 * PI / 2.0 }).collect();
        for i in 0..ns {
            let t = (k * ns + i) as f64 / fs;
            let v: f64 = ph.iter().enumerate().map(|(c, p)| (2.0 * PI * (1500.0 + (c as f64 - 3.5) * 25.0) * t + p).cos()).sum();
            let edge = (i.min(ns - 1 - i) as f64 / (0.002 * fs)).min(1.0);
            out.push((v / carriers as f64 * 0.9 * (0.6 + 0.4 * edge)) as f32);
        }
    }
    out
}

// -------------------------------------------------------------- simulator --

/// Something on the simulated band, at an RF frequency.
#[derive(Clone)]
pub enum SimSignal {
    /// A station calling CQ: whistle for `whistle_s` out of every `cycle_s`,
    /// then a Mercury-style data burst (its CQFRAME).
    Whistler { rf_hz: f64, snr_db: f64, whistle_s: f64, cycle_s: f64, start_s: f64 },
    /// An FT8-like 50 Hz-wide burst every 15 s.
    Ft8 { rf_hz: f64, snr_db: f64 },
    /// A steady carrier (someone tuning up).
    Carrier { rf_hz: f64, snr_db: f64 },
}

/// A simulated HF band heard through a USB receiver tuned to `rig_freq`.
pub struct SimBand {
    pub signals: Vec<SimSignal>,
    pub rig_freq: Arc<Mutex<f64>>,
    pub noise_rms: f64,
    t: f64,
    whistle: Vec<f32>,
    seed: u64,
}

impl SimBand {
    pub fn new(rig_freq: Arc<Mutex<f64>>, signals: Vec<SimSignal>) -> Self {
        Self { signals, rig_freq, noise_rms: 0.03, t: 0.0, whistle: whistle(30.0, &WhistleParams::default()), seed: 1 }
    }

    fn rand(&mut self) -> f64 {
        // xorshift: fast, deterministic, good enough for band noise.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }

    fn gauss(&mut self) -> f64 {
        let (u1, u2) = (self.rand().max(1e-12), self.rand());
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }

    fn gain(&self, snr_db: f64) -> f64 {
        let noise_inband = self.noise_rms * (2500.0 / (FS as f64 / 2.0)).sqrt();
        noise_inband * 10f64.powf(snr_db / 20.0) / (0.5 / 2f64.sqrt())
    }

    /// Next `n` samples of receiver audio.
    pub fn next(&mut self, n: usize) -> Vec<f32> {
        let dial = *self.rig_freq.lock().unwrap();
        let fs = FS as f64;
        let mut y = vec![0.0f64; n];
        for sig in self.signals.clone() {
            match sig {
                SimSignal::Whistler { rf_hz, snr_db, whistle_s, cycle_s, start_s } => {
                    let off = rf_hz - dial;
                    if off.abs() > 3500.0 {
                        continue;
                    }
                    let g = self.gain(snr_db);
                    let mut seg = vec![0.0f64; n];
                    let mut any = false;
                    for (i, s) in seg.iter_mut().enumerate() {
                        let t = self.t + i as f64 / fs - start_s;
                        if t < 0.0 {
                            continue;
                        }
                        let ph = t.rem_euclid(cycle_s);
                        let k = (ph * fs) as usize;
                        if ph < whistle_s && k < self.whistle.len() {
                            *s = self.whistle[k] as f64 * g;
                            any = true;
                        } else if ph >= whistle_s && ph < whistle_s + 3.1 {
                            // CQFRAME burst: noise-like, 1100-1300 Hz audio.
                            let tt = t;
                            *s = g * 0.4 * ((2.0 * PI * 1200.0 * tt).sin() * (2.0 * PI * 37.0 * tt).sin());
                            any = true;
                        }
                    }
                    if any {
                        let sh = ssb_channel(&seg, off, FS, (0.0, 4000.0));
                        y.iter_mut().zip(sh).for_each(|(a, b)| *a += b);
                    }
                }
                SimSignal::Ft8 { rf_hz, snr_db } => {
                    let af = rf_hz - dial;
                    if !(200.0..3000.0).contains(&af) {
                        continue;
                    }
                    let g = self.gain(snr_db) * 0.7;
                    for (i, s) in y.iter_mut().enumerate() {
                        let t = self.t + i as f64 / fs;
                        if t.rem_euclid(15.0) < 12.6 {
                            let tone = ((t * 6.25) as u64 * 2654435761 % 8) as f64 * 6.25;
                            *s += g * (2.0 * PI * (af + tone) * t).sin();
                        }
                    }
                }
                SimSignal::Carrier { rf_hz, snr_db } => {
                    let af = rf_hz - dial;
                    if !(200.0..3000.0).contains(&af) {
                        continue;
                    }
                    let g = self.gain(snr_db) * 0.7;
                    for (i, s) in y.iter_mut().enumerate() {
                        let t = self.t + i as f64 / fs;
                        *s += g * (2.0 * PI * af * t).sin();
                    }
                }
            }
        }
        for v in y.iter_mut() {
            *v += self.noise_rms * self.gauss();
        }
        let y = ssb_channel(&y, 0.0, FS, crate::whistle::PASSBAND);
        self.t += n as f64 / fs;
        y.into_iter().map(|v| v as f32).collect()
    }
}

/// Feed the shared RX buffer from the simulator in real time.
pub fn run_sim(mut band: SimBand, rx: SharedRx, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let block = FS as usize / 20; // 50 ms
        let start = std::time::Instant::now();
        let mut sent = 0u64;
        while !stop.load(Ordering::Relaxed) {
            let x = band.next(block);
            rx.lock().unwrap().push(&x);
            sent += block as u64;
            let due = Duration::from_secs_f64(sent as f64 / FS as f64);
            if let Some(wait) = due.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
        }
    });
}

// --------------------------------------------------------------- waterfall --

/// Power spectrum of the radio audio for the waterfall: 0..fs/2 in `bins`.
pub struct AudioSpectrum {
    fft: Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    pub n: usize,
}

impl AudioSpectrum {
    pub fn new(n: usize) -> Self {
        let window = (0..n).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()) as f32).collect();
        Self { fft: FftPlanner::new().plan_fft_forward(n), window, n }
    }

    /// dB per bin, `n/2` bins covering 0..4000 Hz.
    pub fn row(&self, x: &[f64]) -> Vec<f32> {
        let mut buf: Vec<Complex32> =
            x.iter().zip(&self.window).map(|(&v, &w)| Complex32::new(v as f32 * w, 0.0)).collect();
        buf.resize(self.n, Complex32::new(0.0, 0.0));
        self.fft.process(&mut buf);
        buf[..self.n / 2].iter().map(|c| 10.0 * (c.norm_sqr() + 1e-12).log10()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whistle::Detector;

    #[test]
    fn downsampler_keeps_tones_below_3k() {
        let mut ds = Downsampler::new(48000);
        let x: Vec<f32> = (0..48000).map(|i| (2.0 * PI * 1500.0 * i as f64 / 48000.0).sin() as f32).collect();
        let mut out = vec![];
        ds.process(&x, &mut out);
        assert!((out.len() as i64 - 8000).abs() <= 1);
        let rms = (out[1000..].iter().map(|v| v * v).sum::<f32>() / (out.len() - 1000) as f32).sqrt();
        assert!((rms - 0.707).abs() < 0.02, "rms {rms}");
    }

    #[test]
    fn sim_band_whistler_is_detected_with_offset() {
        let rf = Arc::new(Mutex::new(14_100_000.0));
        let mut band = SimBand::new(
            rf.clone(),
            vec![
                SimSignal::Whistler { rf_hz: 14_100_000.0 + 340.0, snr_db: 0.0, whistle_s: 25.0, cycle_s: 40.0, start_s: 0.0 },
                SimSignal::Whistler { rf_hz: 14_100_000.0 - 640.0 + 50_000.0, snr_db: 0.0, whistle_s: 25.0, cycle_s: 40.0, start_s: 0.0 },
                SimSignal::Carrier { rf_hz: 14_102_400.0, snr_db: 10.0 },
            ],
        );
        let x: Vec<f64> = band.next(9600).iter().map(|&v| v as f64).collect();
        let det = Detector::new(WhistleParams::default()).detect(&x).expect("whistle");
        assert!((det.offset_hz - 340.0).abs() < 3.0, "{}", det.offset_hz);
        let hit = crate::whistle::TrackDetector::new(WhistleParams::default()).scan(&x[..4000]).unwrap();
        assert!((hit.offset_hz - 340.0).abs() < 70.0 && hit.z > crate::whistle::TrackDetector::found(0.5), "{hit:?}");
        // 640 Hz off: beyond the matched filter's ±500, still found by the scan.
        *rf.lock().unwrap() = 14_150_000.0;
        let x: Vec<f64> = band.next(4000).iter().map(|&v| v as f64).collect();
        let hit = crate::whistle::TrackDetector::new(WhistleParams::default()).scan(&x).unwrap();
        assert!((hit.offset_hz + 640.0).abs() < 70.0 && hit.z > crate::whistle::TrackDetector::found(0.5), "{hit:?}");
    }
}
