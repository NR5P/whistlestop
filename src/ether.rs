//! The ether: a virtual HF band for testing stations on one computer.
//!
//! Each station gets
//! * a **virtual radio**: a rigctld server (dial frequency, PTT, S-meter);
//! * a **Mercury audio link** using Mercury's own `-x sock` lockstep protocol
//!   (audioio/sock_wire.h): every 20 ms the ether sends the station what its
//!   receiver hears and gets back what its modem transmits, and its PTT;
//! * an **app audio link** (TCP, f32 LE at 8 kHz both ways) so Whistlestop
//!   can send its whistle and hear the band.
//!
//! Between stations it models an HF channel: each transmission is heard
//! shifted by the difference of the dial frequencies (USB), with band noise,
//! optional slow fading, and the receiver's 300-2700 Hz passband. A station's
//! receiver is muted while it transmits.

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const FS: f64 = 8000.0;
pub const BLOCK: usize = 160; // 20 ms

#[derive(Clone, Debug)]
pub struct StationCfg {
    pub name: String,
    pub sock_path: PathBuf,
    pub rig_port: u16,
    pub audio_port: u16,
    pub dial_hz: f64,
    /// While this station transmits, send its own transmitted audio to its
    /// app instead of silence (so a listener hears their own signal).
    pub hear_own_tx: bool,
}

/// Knobs a test can turn while the ether runs.
#[derive(Clone, Debug)]
pub struct Channel {
    /// Band noise in the 2500 Hz passband, dB relative to full scale.
    pub noise_dbfs: f64,
    /// Path loss applied to every transmission, dB.
    pub path_loss_db: f64,
    /// Slow fading depth, 0 = none, 1 = deep.
    pub fading: f64,
}

impl Default for Channel {
    fn default() -> Self {
        Self { noise_dbfs: -45.0, path_loss_db: 0.0, fading: 0.0 }
    }
}

#[derive(Default, Debug, Clone)]
pub struct StationStats {
    pub mercury_connected: bool,
    pub app_connected: bool,
    pub dial_hz: f64,
    pub rig_ptt: bool,
    pub mercury_ptt: bool,
    /// Seconds transmitted, and the TX level while transmitting.
    pub tx_seconds: f64,
    pub tx_rms: f64,
    pub retunes: u64,
}

struct Station {
    _cfg: StationCfg,
    stats: StationStats,
    app_tx: VecDeque<f32>,
    app_out: Option<TcpStream>,
}

pub struct Ether {
    pub channel: Arc<Mutex<Channel>>,
    /// (rigctld port, app audio port) per station, as actually bound (a
    /// port of 0 in the config picks a free one).
    pub ports: Vec<(u16, u16)>,
    stations: Arc<Vec<Mutex<Station>>>,
    stop: Arc<AtomicBool>,
}

impl Ether {
    pub fn stats(&self) -> Vec<StationStats> {
        self.stations.iter().map(|s| s.lock().unwrap().stats.clone()).collect()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn start(cfgs: Vec<StationCfg>, channel: Channel) -> std::io::Result<Ether> {
        let stop = Arc::new(AtomicBool::new(false));
        let stations: Arc<Vec<Mutex<Station>>> = Arc::new(
            cfgs.iter()
                .map(|c| {
                    Mutex::new(Station {
                        _cfg: c.clone(),
                        stats: StationStats { dial_hz: c.dial_hz, ..Default::default() },
                        app_tx: VecDeque::new(),
                        app_out: None,
                    })
                })
                .collect(),
        );
        let channel = Arc::new(Mutex::new(channel));
        let mut mercury_listeners = vec![];
        let mut ports = vec![];
        for (i, c) in cfgs.iter().enumerate() {
            let _ = std::fs::remove_file(&c.sock_path);
            let ul = UnixListener::bind(&c.sock_path)?;
            ul.set_nonblocking(true)?;
            mercury_listeners.push(ul);
            let rl = TcpListener::bind(("127.0.0.1", c.rig_port))?;
            let al = TcpListener::bind(("127.0.0.1", c.audio_port))?;
            ports.push((rl.local_addr()?.port(), al.local_addr()?.port()));
            spawn_rig(rl, stations.clone(), i, stop.clone());
            spawn_app_audio(al, stations.clone(), i, stop.clone());
        }
        {
            let (stations, channel, stop) = (stations.clone(), channel.clone(), stop.clone());
            std::thread::Builder::new().name("ether".into()).spawn(move || run(stations, channel, mercury_listeners, stop))?;
        }
        Ok(Ether { channel, ports, stations, stop })
    }
}

// ----------------------------------------------------------- virtual rigs

fn spawn_rig(l: TcpListener, st: Arc<Vec<Mutex<Station>>>, i: usize, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        for conn in l.incoming() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(s) = conn else { continue };
            let st = st.clone();
            std::thread::spawn(move || {
                let mut w = s.try_clone().unwrap();
                for line in BufReader::new(s).lines() {
                    let Ok(line) = line else { return };
                    let mut p = line.split_whitespace();
                    let cmd = p.next().unwrap_or("");
                    let reply = {
                        let mut g = st[i].lock().unwrap();
                        match cmd {
                            "F" | "\\set_freq" => {
                                if let Some(f) = p.next().and_then(|v| v.parse::<f64>().ok()) {
                                    if (f - g.stats.dial_hz).abs() > 0.5 {
                                        g.stats.retunes += 1;
                                    }
                                    g.stats.dial_hz = f;
                                }
                                "RPRT 0\n".to_string()
                            }
                            "f" => format!("{}\n", g.stats.dial_hz.round() as i64),
                            "m" => "PKTUSB\n3000\n".into(),
                            "M" | "L" => "RPRT 0\n".into(),
                            "T" => {
                                g.stats.rig_ptt = p.next() == Some("1");
                                "RPRT 0\n".into()
                            }
                            "t" => format!("{}\n", g.stats.rig_ptt as u8),
                            "l" => match p.next() {
                                Some("STRENGTH") => "-20\n".into(),
                                Some("RFPOWER") => "0.5\n".into(),
                                _ => "RPRT -4\n".into(),
                            },
                            _ => "RPRT -4\n".into(),
                        }
                    };
                    if w.write_all(reply.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
}

fn spawn_app_audio(l: TcpListener, st: Arc<Vec<Mutex<Station>>>, i: usize, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        for conn in l.incoming() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(mut s) = conn else { continue };
            s.set_nodelay(true).ok();
            {
                let mut g = st[i].lock().unwrap();
                g.app_out = s.try_clone().ok();
                g.stats.app_connected = true;
            }
            let st = st.clone();
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 8192];
                let mut carry: Vec<u8> = vec![];
                loop {
                    let n = match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    carry.extend_from_slice(&buf[..n]);
                    let whole = carry.len() / 4 * 4;
                    let samples: Vec<f32> = carry[..whole].chunks(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
                    carry.drain(..whole);
                    st[i].lock().unwrap().app_tx.extend(samples);
                }
                st[i].lock().unwrap().stats.app_connected = false;
            });
        }
    });
}

// ------------------------------------------------------------ channel DSP

fn hilbert_taps(n: usize) -> Vec<f64> {
    let m = (n / 2) as i64;
    (0..n)
        .map(|i| {
            let k = i as i64 - m;
            // Blackman window: the image of the opposite sideband stays
            // ~70 dB down, as in a real receiver.
            let x = 2.0 * PI * i as f64 / (n - 1) as f64;
            let w = 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos();
            if k % 2 == 0 { 0.0 } else { 2.0 / (PI * k as f64) * w }
        })
        .collect()
}

fn bandpass_taps(n: usize, lo: f64, hi: f64) -> Vec<f64> {
    let m = (n - 1) as f64 / 2.0;
    (0..n)
        .map(|i| {
            let x = i as f64 - m;
            let lp = |fc: f64| if x == 0.0 { 2.0 * fc / FS } else { (2.0 * PI * fc / FS * x).sin() / (PI * x) };
            let w = 0.54 - 0.46 * (2.0 * PI * i as f64 / (n - 1) as f64).cos();
            (lp(hi) - lp(lo)) * w
        })
        .collect()
}

/// Streaming FIR with history.
struct Fir {
    taps: Vec<f64>,
    hist: VecDeque<f64>,
}

impl Fir {
    fn new(taps: Vec<f64>) -> Self {
        let n = taps.len();
        Self { taps, hist: VecDeque::from(vec![0.0; n]) }
    }
    fn step(&mut self, x: f64) -> f64 {
        self.hist.pop_front();
        self.hist.push_back(x);
        self.hist.iter().zip(self.taps.iter().rev()).map(|(a, b)| a * b).sum()
    }
}

/// Complex streaming FIR (for the receiver's one-sided passband).
struct CFir {
    re: Vec<f64>,
    im: Vec<f64>,
    hist: VecDeque<(f64, f64)>,
}

impl CFir {
    /// Passes +300…+2700 Hz only: a 1200 Hz low-pass moved up to 1500 Hz.
    /// Like a real USB receiver it rejects the opposite sideband, and the
    /// fold-over of anything shifted past the top of the band.
    fn usb_passband() -> Self {
        let lp = bandpass_taps(127, 0.0, 1200.0);
        let m = (lp.len() - 1) as f64 / 2.0;
        let (re, im) = lp.iter().enumerate().map(|(i, h)| {
            let ph = 2.0 * PI * 1500.0 * (i as f64 - m) / FS;
            (h * ph.cos(), h * ph.sin())
        }).unzip();
        Self { re, im, hist: VecDeque::from(vec![(0.0, 0.0); 127]) }
    }
    fn step(&mut self, x: (f64, f64)) -> (f64, f64) {
        self.hist.pop_front();
        self.hist.push_back(x);
        let (mut a, mut b) = (0.0, 0.0);
        for ((xr, xi), (hr, hi)) in self.hist.iter().zip(self.re.iter().rev().zip(self.im.iter().rev())) {
            a += xr * hr - xi * hi;
            b += xr * hi + xi * hr;
        }
        (a, b)
    }
}

/// One transmitter as heard by one receiver: frequency-shifted by `delta`.
struct Path {
    hilbert: Fir,
    delay: VecDeque<f64>,
    phase: f64,
    fade_phase: f64,
    rx: CFir,
}

const HILBERT_N: usize = 511;

impl Path {
    fn new(seed: f64) -> Self {
        Self { hilbert: Fir::new(hilbert_taps(HILBERT_N)), delay: VecDeque::from(vec![0.0; HILBERT_N / 2]), phase: 0.0, fade_phase: seed, rx: CFir::usb_passband() }
    }
    fn process(&mut self, x: &[f64], delta_hz: f64, gain: f64, fading: f64, out: &mut [f64]) {
        let dphi = 2.0 * PI * delta_hz / FS;
        for (o, &v) in out.iter_mut().zip(x) {
            let q = self.hilbert.step(v);
            self.delay.push_back(v);
            let i = self.delay.pop_front().unwrap();
            // Analytic (i + jq) shifted by delta, then the receiver's
            // one-sided passband; the real part is what the speaker hears.
            let (c, sn) = (self.phase.cos(), self.phase.sin());
            let s = self.rx.step((i * c - q * sn, i * sn + q * c)).0;
            self.phase = (self.phase + dphi) % (2.0 * PI);
            self.fade_phase += 2.0 * PI * 0.11 / FS;
            let fade = 1.0 - fading * 0.5 * (1.0 + (self.fade_phase.sin() * (0.37 * self.fade_phase).cos()));
            *o += s * gain * fade.max(0.02);
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (a, b) = (self.next().max(1e-12), self.next());
        (-2.0 * a.ln()).sqrt() * (2.0 * PI * b).cos()
    }
}

// ------------------------------------------------------------- main loop

fn read_exact_timeout(s: &mut UnixStream, buf: &mut [u8]) -> std::io::Result<()> {
    s.read_exact(buf)
}

fn run(stations: Arc<Vec<Mutex<Station>>>, channel: Arc<Mutex<Channel>>, listeners: Vec<UnixListener>, stop: Arc<AtomicBool>) {
    let n = stations.len();
    let mut merc: Vec<Option<UnixStream>> = (0..n).map(|_| None).collect();
    let mut tx_prev: Vec<Vec<f64>> = vec![vec![0.0; BLOCK]; n];
    let mut paths: Vec<Vec<Path>> = (0..n).map(|r| (0..n).map(|t| Path::new((r * 7 + t * 3) as f64)).collect()).collect();
    let mut bpf: Vec<Fir> = (0..n).map(|_| Fir::new(bandpass_taps(127, 300.0, 2700.0))).collect();
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let start = Instant::now();
    let mut seq = 0u64;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        // Accept Mercury connections as they arrive.
        for (i, l) in listeners.iter().enumerate() {
            if merc[i].is_none()
                && let Ok((s, _)) = l.accept() {
                    s.set_nonblocking(false).ok();
                    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    merc[i] = Some(s);
                    stations[i].lock().unwrap().stats.mercury_connected = true;
                }
        }
        let ch = channel.lock().unwrap().clone();
        let noise_rms = 10f64.powf(ch.noise_dbfs / 20.0) * (FS / 2.0 / 2500.0).sqrt();
        let gain = 10f64.powf(-ch.path_loss_db / 20.0);
        let (dials, keyed): (Vec<f64>, Vec<bool>) = stations
            .iter()
            .map(|s| {
                let g = s.lock().unwrap();
                (g.stats.dial_hz, g.stats.rig_ptt || g.stats.mercury_ptt)
            })
            .unzip();

        // What each receiver hears this block (from last block's TX).
        let mut rx: Vec<Vec<f64>> = vec![vec![0.0; BLOCK]; n];
        for r in 0..n {
            for t in 0..n {
                if t == r {
                    continue;
                }
                let delta = dials[t] - dials[r]; // caller above us -> higher audio
                if delta.abs() < 4000.0 && tx_prev[t].iter().any(|v| *v != 0.0) {
                    let (path, out) = (&mut paths[r][t], &mut rx[r]);
                    path.process(&tx_prev[t], delta, gain, ch.fading, out);
                }
            }
            for v in rx[r].iter_mut() {
                let x = *v + noise_rms * rng.gauss();
                *v = bpf[r].step(x);
            }
            if keyed[r] {
                rx[r].iter_mut().for_each(|v| *v = 0.0); // muted on TX
            }
        }
        // Stations that want to hear themselves get last block's TX on
        // their app link (never on Mercury's: its receiver stays muted).
        let mut app_rx = rx.clone();
        for r in 0..n {
            let own = stations[r].lock().unwrap()._cfg.hear_own_tx;
            if own && keyed[r] {
                app_rx[r] = tx_prev[r].iter().map(|v| v * 0.6).collect();
            }
        }

        // Exchange with each station: RX out, TX in.
        let vnow = start.elapsed().as_millis() as u64;
        let mut tx_now: Vec<Vec<f64>> = vec![vec![0.0; BLOCK]; n];
        for i in 0..n {
            // Mercury, over its lockstep socket.
            let mut drop_it = false;
            if let Some(s) = merc[i].as_mut() {
                let mut f = Vec::with_capacity(4 + 18 + BLOCK * 2);
                f.extend_from_slice(&((18 + BLOCK * 2) as u32).to_le_bytes());
                f.extend_from_slice(&seq.to_le_bytes());
                f.extend_from_slice(&vnow.to_le_bytes());
                f.extend_from_slice(&(BLOCK as u16).to_le_bytes());
                for v in &rx[i] {
                    f.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                }
                let ok = s.write_all(&f).is_ok() && {
                    let mut len = [0u8; 4];
                    let mut hdr = [0u8; 11];
                    if read_exact_timeout(s, &mut len).is_ok() && read_exact_timeout(s, &mut hdr).is_ok() {
                        let ptt = hdr[8] == 1;
                        let cnt = u16::from_le_bytes([hdr[9], hdr[10]]) as usize;
                        let mut pay = vec![0u8; cnt * 2];
                        if read_exact_timeout(s, &mut pay).is_ok() {
                            for (k, b) in pay.chunks(2).enumerate().take(BLOCK) {
                                tx_now[i][k] = i16::from_le_bytes([b[0], b[1]]) as f64 / 32768.0;
                            }
                            stations[i].lock().unwrap().stats.mercury_ptt = ptt;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                drop_it = !ok;
            }
            if drop_it {
                merc[i] = None;
                stations[i].lock().unwrap().stats.mercury_connected = false;
            }
            // The app: send what the radio hears; its whistle goes out only
            // while the virtual radio is keyed.
            let mut g = stations[i].lock().unwrap();
            if let Some(o) = g.app_out.as_mut() {
                let bytes: Vec<u8> = app_rx[i].iter().flat_map(|v| (*v as f32).to_le_bytes()).collect();
                if o.write_all(&bytes).is_err() {
                    g.app_out = None;
                }
            }
            let take = g.app_tx.len().min(BLOCK);
            let app: Vec<f32> = g.app_tx.drain(..take).collect();
            if g.stats.rig_ptt {
                for (k, v) in app.iter().enumerate() {
                    tx_now[i][k] += *v as f64;
                }
            }
            let p: f64 = tx_now[i].iter().map(|v| v * v).sum::<f64>() / BLOCK as f64;
            if p > 1e-8 {
                g.stats.tx_seconds += BLOCK as f64 / FS;
                g.stats.tx_rms = 0.9 * g.stats.tx_rms + 0.1 * p.sqrt();
            }
        }
        tx_prev = tx_now;
        seq += 1;

        next += Duration::from_secs_f64(BLOCK as f64 / FS);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(200) {
            next = now; // fell behind; don't try to catch up in a burst
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_shifts_frequency() {
        // A 1000 Hz tone heard 400 Hz up comes out at 1400 Hz.
        let mut p = Path::new(0.0);
        let x: Vec<f64> = (0..8000).map(|i| (2.0 * PI * 1000.0 * i as f64 / FS).sin()).collect();
        let mut y = vec![0.0; 8000];
        p.process(&x, 400.0, 1.0, 0.0, &mut y);
        let y = &y[1000..];
        let mut best = (0.0, 0.0);
        for f in (500..2500).step_by(10) {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in y.iter().enumerate() {
                let ph = 2.0 * PI * f as f64 * k as f64 / FS;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            let m = (re * re + im * im).sqrt();
            if m > best.1 {
                best = (f as f64, m);
            }
        }
        assert!((best.0 - 1400.0).abs() <= 10.0, "{}", best.0);
    }

    #[test]
    fn opposite_sideband_and_fold_over_are_rejected() {
        // Like a real USB receiver: a station 2600 Hz below the dial (in the
        // opposite sideband) and one 3450 Hz above (past the top of the
        // audio band) are not heard; one 400 Hz above is.
        let x: Vec<f64> = (0..8000).map(|i| (2.0 * PI * 1500.0 * i as f64 / FS).sin()).collect();
        let rms = |delta: f64| {
            let mut p = Path::new(0.0);
            let mut y = vec![0.0; 8000];
            p.process(&x, delta, 1.0, 0.0, &mut y);
            (y[1000..].iter().map(|v| v * v).sum::<f64>() / 7000.0).sqrt()
        };
        let heard = rms(400.0);
        assert!(heard > 0.5, "{heard}");
        assert!(rms(-2600.0) < 0.02 * heard, "opposite sideband leaked: {}", rms(-2600.0));
        assert!(rms(3450.0) < 0.02 * heard, "fold-over leaked: {}", rms(3450.0));
    }

    #[test]
    fn mirror_image_is_far_down() {
        // A strong 1500 Hz tone shifted 3450 Hz up must not leave a mirror
        // image at 3450 − 1500 = 1950 Hz (a receiver's image rejection).
        let x: Vec<f64> = (0..16000).map(|i| (2.0 * PI * 1500.0 * i as f64 / FS).sin()).collect();
        let level = |delta: f64, f: f64| {
            let mut p = Path::new(0.0);
            let mut y = vec![0.0; x.len()];
            p.process(&x, delta, 1.0, 0.0, &mut y);
            let y = &y[2000..];
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in y.iter().enumerate() {
                let ph = 2.0 * PI * f * k as f64 / FS;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            (re * re + im * im).sqrt() / y.len() as f64
        };
        let wanted = level(400.0, 1900.0);
        let image = level(3450.0, 1950.0);
        let db = 20.0 * (image / wanted).log10();
        assert!(db < -65.0, "image only {db:.1} dB down");
    }
}
