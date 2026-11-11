//! The engine: owns the radio, Mercury and audio, and runs everything —
//! whistle watch, search, CQ, 1:1 link (chat, files), and Roundtables.
//!
//! It runs on a tokio runtime in its own thread. The GUI sends `Cmd`s and
//! reads `State` (behind a mutex), so the interface never blocks on the radio.

use crate::audio::{self, AudioSpectrum, LiveInput, LiveOutput, SharedRx, SimBand, SimSignal};
use crate::bandplan::{self, SearchScope};
use crate::config::Config;
use crate::fake_mercury::{FakeMercury, FakeOptions};
use crate::modes::{self, Bandwidth, Mode, ThroughputEstimator};
use crate::protocol::*;
use crate::rig::{Rig, RigStatus, RigctldClient};
use crate::sdr;
use crate::tnc::{Tnc, TncEvent, parse_connected};
use crate::whistle::{Detector, FS, WhistleParams, whistle};
use crate::ziputil;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// ------------------------------------------------------------------ API ---

#[derive(Debug, Clone)]
pub enum Cmd {
    Start(Config),
    Stop,
    /// New settings that don't need a reconnect (CQ, search, ramp...).
    Update(Config),
    SetFreq(f64),
    SetMode(String),
    SetPower(f32),
    Tune(bool),
    SetBandwidth(Bandwidth),
    StartSearch(SearchScope),
    StopSearch,
    /// Tune by this many Hz (e.g. onto a whistle heard while parked).
    Nudge(f64),
    CallCq { table: Option<String> },
    StopCq,
    Connect(String),
    Disconnect,
    Chat(String),
    SendFiles { paths: Vec<PathBuf>, zip: bool },
    AnswerOffer { fid: u64, accept: bool },
    Cancel(u64),
    JoinTable(String),
    LeaveTable,
    TableSay(String),
    PassFloor(String),
    SetSdr(bool),
    SetSdrSpan(f64),
    /// Wide whistle search on the SDR; `auto` tunes the radio to hits.
    WideSearch { on: bool, auto: bool },
    /// Tune to an SDR hit and fine-lock on it with the radio's own audio.
    LockOn(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Who {
    Me,
    Them,
    System,
}

#[derive(Clone, Debug)]
pub struct ChatLine {
    pub at: chrono_lite::Clock,
    pub who: Who,
    pub from: String,
    pub text: String,
    pub id: u64,
    /// The other station confirmed it arrived (a RECEIVED packet).
    pub delivered: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Dir {
    Out,
    In,
}

#[derive(Clone, Debug, PartialEq)]
pub enum XferState {
    Offered,
    WaitingForYou,
    Sending,
    Receiving,
    Done(Option<PathBuf>),
    Failed(String),
    Declined,
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub fid: u64,
    pub dir: Dir,
    pub name: String,
    pub size: u64,
    pub done: u64,
    pub zipped: bool,
    pub files: u64,
    pub original: u64,
    pub state: XferState,
    pub eta_s: f64,
    pub started: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct Heard {
    pub at: chrono_lite::Clock,
    pub call: String,
    pub dial_hz: f64,
    pub what: String,
    pub table: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct LinkInfo {
    pub peer: String,
    pub bw: u32,
    pub since: Option<Instant>,
    pub peer_name: String,
    pub peer_grid: String,
    pub peer_app: String,
    /// How the peer hears us (their REPORT).
    pub their_report_db: Option<i8>,
    pub mode: Option<&'static Mode>,
    pub bps: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Member {
    pub snr_of_host: Option<i8>,
    pub last: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct TableState {
    pub name: String,
    pub id: u16,
    pub host: bool,
    pub mode: &'static Mode,
    pub members: HashMap<String, Member>,
    pub floor: Option<String>,
    pub lines: Vec<(chrono_lite::Clock, String, String)>,
    /// Frames still to transmit, with their air time.
    pub sending_eta_s: f64,
}

#[derive(Clone, Debug, Default)]
pub struct SearchView {
    pub active: bool,
    pub scope: String,
    pub index: usize,
    pub total: usize,
    pub channel_hz: f64,
    pub note: String,
}

#[derive(Clone, Debug, Default)]
pub struct CqView {
    pub active: bool,
    pub cycle: u32,
    pub cycles: u32,
    pub phase: String,
    pub table: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WhistleSeen {
    pub at: Instant,
    pub dial_hz: f64,
    pub offset_hz: f64,
    pub score: f64,
}

#[derive(Default)]
pub struct State {
    pub running: bool,
    pub demo: bool,
    pub mycall: String,
    pub rig_ok: bool,
    pub mercury_ok: bool,
    pub audio_ok: bool,
    pub rig: RigStatus,
    pub tx: bool,
    pub snr_db: Option<f64>,
    pub bandwidth: Option<Bandwidth>,
    pub link: Option<LinkInfo>,
    pub incoming_call: bool,
    pub calling: Option<String>,
    pub chat: Vec<ChatLine>,
    pub transfers: Vec<Transfer>,
    pub heard: Vec<Heard>,
    pub whistle: Option<WhistleSeen>,
    pub search: SearchView,
    pub cq: CqView,
    pub table: Option<TableState>,
    pub log: VecDeque<(chrono_lite::Clock, String)>,
    /// Radio-audio waterfall rows (dB per bin, 0..4 kHz), newest last.
    pub audio_rows: VecDeque<Vec<f32>>,
    pub listen_mode: Option<&'static Mode>,
    pub est_bps: f64,
    pub wide: WideView,
    /// What Mercury says about itself (VERSION) and its current speed
    /// (BITRATE, raw bits/s of the mode on the air).
    pub modem_version: Option<String>,
    pub modem_bps: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct WideView {
    pub on: bool,
    pub auto: bool,
    pub note: String,
    /// (caller dial as the SDR sees it, score, seconds since last heard)
    pub hits: Vec<(f64, f64, f64)>,
}

pub const WIDE_SCOPE: &str = "SDR wide search";

pub type SharedState = Arc<Mutex<State>>;

pub struct Handle {
    pub tx: mpsc::UnboundedSender<Cmd>,
    pub state: SharedState,
    pub sdr: sdr::Shared,
}

impl Handle {
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

/// A tiny wall-clock time for display, without a date/time crate.
pub mod chrono_lite {
    #[derive(Clone, Copy, Debug)]
    pub struct Clock(pub u64);
    impl Clock {
        pub fn now() -> Self {
            Clock(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
        }
        /// UTC hh:mm (hams log in UTC).
        pub fn hhmm(&self) -> String {
            let s = self.0 % 86400;
            format!("{:02}:{:02}Z", s / 3600, (s % 3600) / 60)
        }
    }
}
use chrono_lite::Clock;

/// Start the engine thread. `repaint` is called whenever state changes.
pub fn spawn(repaint: impl Fn() + Send + Sync + 'static) -> Handle {
    let (tx, rx) = mpsc::unbounded_channel();
    let state: SharedState = Default::default();
    let sdr_shared: sdr::Shared = Default::default();
    let (st, sd) = (state.clone(), sdr_shared.clone());
    let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(repaint);
    std::thread::Builder::new()
        .name("whistlestop-engine".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
            rt.block_on(run(rx, st, sd, repaint));
        })
        .unwrap();
    Handle { tx, state, sdr: sdr_shared }
}

// ---------------------------------------------------------------- engine ---

enum SearchPhase {
    Settle { until: Instant },
    Dwell { from: u64 },
    Hold { from: u64 },
    /// `alt`: already trying the alias candidate (see WhistleParams::alias_of).
    Verify { target: f64, from: u64, first: crate::whistle::Detection, alt: bool },
    WaitCq { until: Instant },
}

struct Search {
    channels: Vec<f64>,
    idx: usize,
    phase: SearchPhase,
    scope: String,
}

enum CqPhase {
    Whistle { done: Arc<AtomicBool> },
    Frame { until: Instant },
    Listen { until: Instant },
}

struct Cq {
    table: Option<String>,
    cycle: u32,
    phase: CqPhase,
}

struct Station {
    cfg: Config,
    st: SharedState,
    sdr: sdr::Shared,
    _repaint: Arc<dyn Fn() + Send + Sync>,
    rig: Option<Rig>,
    tnc: Option<Tnc>,
    tnc_rx: Option<mpsc::UnboundedReceiver<TncEvent>>,
    rx: SharedRx,
    _input: Option<LiveInput>,
    output: Option<Arc<LiveOutput>>,
    sim_stop: Arc<AtomicBool>,
    sdr_stop: Arc<AtomicBool>,
    vfo: Arc<Mutex<f64>>,
    sdr_span: Arc<Mutex<f64>>,
    detector: Detector,
    spectrum: AudioSpectrum,
    spec_from: u64,
    bw: Bandwidth,
    // link
    peer: Option<String>,
    dec: LinkDecoder,
    hi_queue: VecDeque<Vec<u8>>,
    outgoing: Vec<Outgoing>,
    incoming: HashMap<u64, Incoming>,
    mercury_buffer: usize,
    next_fid: u64,
    next_chat_id: u64,
    est: ThroughputEstimator,
    last_report: Instant,
    // roundtable
    table_asm: TableAssembler,
    table_sent: HashMap<u64, Vec<Vec<u8>>>,
    table_msg_id: u64,
    table_queue: VecDeque<Vec<u8>>,
    table_busy_until: Instant,
    last_ramp: Instant,
    last_downgrade: Instant,
    last_again: Instant,
    // activities
    search: Option<Search>,
    cq: Option<Cq>,
    last_watch: Instant,
    wide_on: bool,
    wide_auto: bool,
    /// Hits already handed to the radio, so one caller isn't chased forever.
    wide_tried: Vec<(f64, Instant)>,
    /// When the radio last changed frequency: audio from before then
    /// belongs to another frequency.
    last_tune: Instant,
    /// Roundtable host: the speed to switch to once the announcement is out.
    pending_speed: Option<(&'static Mode, Instant)>,
    last_query: Instant,
    /// Link data that arrived before Mercury's CONNECTED line.
    prelink: Vec<u8>,
}

async fn run(mut cmds: mpsc::UnboundedReceiver<Cmd>, st: SharedState, sdr: sdr::Shared, repaint: Arc<dyn Fn() + Send + Sync>) {
    let mut station: Option<Station> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut poll = tokio::time::interval(Duration::from_millis(700));
    loop {
        let ev = async {
            match station.as_mut().and_then(|s| s.tnc_rx.as_mut()) {
                Some(rx) => rx.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            c = cmds.recv() => {
                let Some(c) = c else { return };
                match c {
                    Cmd::Start(cfg) => {
                        if let Some(s) = station.take() { s.shutdown().await; }
                        station = Some(Station::start(cfg, st.clone(), sdr.clone(), repaint.clone()).await);
                    }
                    Cmd::Stop => {
                        if let Some(s) = station.take() { s.shutdown().await; }
                        st.lock().unwrap().running = false;
                    }
                    other => if let Some(s) = station.as_mut()
                        && let Err(e) = s.command(other).await { s.log(format!("⚠ {e}")); },
                }
                repaint();
            }
            e = ev => {
                if let (Some(s), Some(e)) = (station.as_mut(), e) {
                    if let Err(e) = s.on_tnc(e).await { s.log(format!("⚠ {e}")); }
                    repaint();
                }
            }
            _ = tick.tick() => {
                if let Some(s) = station.as_mut() {
                    if let Err(e) = s.tick().await { s.log(format!("⚠ {e}")); }
                    repaint();
                }
            }
            _ = poll.tick() => {
                if let Some(s) = station.as_mut() { s.poll_rig().await; }
            }
        }
    }
}

impl Station {
    async fn start(cfg: Config, st: SharedState, sdr: sdr::Shared, repaint: Arc<dyn Fn() + Send + Sync>) -> Station {
        let vfo = Arc::new(Mutex::new(14_096_000.0));
        let rx: SharedRx = Default::default();
        let sim_stop = Arc::new(AtomicBool::new(false));
        let bw = cfg.bandwidth;
        let mut s = Station {
            detector: Detector::new(bw.whistle_profile()),
            spectrum: AudioSpectrum::new(512),
            spec_from: 0,
            bw,
            cfg: cfg.clone(),
            st: st.clone(),
            sdr,
            _repaint: repaint,
            rig: None,
            tnc: None,
            tnc_rx: None,
            rx: rx.clone(),
            _input: None,
            output: None,
            sim_stop: sim_stop.clone(),
            sdr_stop: Arc::new(AtomicBool::new(false)),
            vfo: vfo.clone(),
            sdr_span: Arc::new(Mutex::new(cfg.sdr.span_hz)),
            peer: None,
            dec: LinkDecoder::default(),
            hi_queue: VecDeque::new(),
            outgoing: vec![],
            incoming: HashMap::new(),
            mercury_buffer: 0,
            next_fid: 1,
            next_chat_id: 1,
            est: ThroughputEstimator::default(),
            last_report: Instant::now(),
            table_asm: TableAssembler::default(),
            table_sent: HashMap::new(),
            table_msg_id: 0,
            table_queue: VecDeque::new(),
            table_busy_until: Instant::now(),
            last_ramp: Instant::now(),
            last_downgrade: Instant::now() - Duration::from_secs(600),
            last_again: Instant::now(),
            search: None,
            cq: None,
            last_watch: Instant::now(),
            wide_on: cfg.sdr.wide_search,
            wide_auto: cfg.sdr.wide_auto,
            wide_tried: vec![],
            last_tune: Instant::now(),
            pending_speed: None,
            last_query: Instant::now(),
            prelink: vec![],
        };
        {
            let mut g = st.lock().unwrap();
            *g = State { running: true, demo: cfg.demo, mycall: cfg.callsign.to_uppercase(), bandwidth: Some(bw), ..Default::default() };
        }
        s.log(format!("Whistlestop starting{}", if cfg.demo { " in Demo mode (simulated radio, band and Mercury)" } else { "" }));

        // Radio.
        if cfg.demo {
            s.rig = Some(Rig::sim(14_096_000.0, vfo.clone()));
            s.set(|g| g.rig_ok = true);
        } else {
            match RigctldClient::connect(&cfg.rigctld).await {
                Ok(c) => {
                    s.rig = Some(Rig::Rigctld(c));
                    s.set(|g| g.rig_ok = true);
                    s.log(format!("Radio: rigctld at {}", cfg.rigctld));
                }
                Err(e) => s.log(format!("⚠ Radio: {e}. Start rigctld or RigHelm's rigctld server.")),
            }
        }

        // Audio.
        if cfg.demo {
            let band = SimBand::new(
                vfo.clone(),
                vec![
                    SimSignal::Whistler { rf_hz: 14_103_150.0, snr_db: -2.0, whistle_s: 25.0, cycle_s: 45.0, start_s: 4.0 },
                    SimSignal::Carrier { rf_hz: 14_101_900.0, snr_db: 8.0 },
                    SimSignal::Ft8 { rf_hz: 14_097_350.0, snr_db: 0.0 },
                ],
            );
            audio::run_sim(band, rx.clone(), sim_stop.clone());
            s.set(|g| g.audio_ok = true);
        } else {
            if let Some(addr) = cfg.audio_in.strip_prefix("ether:") {
                // Test bench: audio to and from the ether over TCP.
                match std::net::TcpStream::connect(addr) {
                    Ok(t) => {
                        t.set_nodelay(true).ok();
                        s._input = Some(LiveInput::ether(t.try_clone().expect("clone"), rx.clone()));
                        s.output = Some(Arc::new(LiveOutput::ether(t)));
                        s.set(|g| g.audio_ok = true);
                        s.log(format!("Audio: test ether at {addr}"));
                    }
                    Err(e) => s.log(format!("⚠ Audio: ether at {addr}: {e}")),
                }
            } else {
            match LiveInput::start(&cfg.audio_in, rx.clone()) {
                Ok(i) => {
                    s._input = Some(i);
                    s.set(|g| g.audio_ok = true);
                }
                Err(e) => s.log(format!("⚠ Audio input: {e}")),
            }
            match LiveOutput::start(&cfg.audio_out) {
                Ok(o) => s.output = Some(Arc::new(o)),
                Err(e) => s.log(format!("⚠ Audio output: {e} (CQ whistle disabled)")),
            }
            }
        }

        // Mercury.
        let (host, port, bport) = if cfg.demo {
            match FakeMercury::start(FakeOptions { rig_freq: Some(vfo.clone()), ..Default::default() }).await {
                Ok(f) => ("127.0.0.1".to_string(), f.port, f.bcast_port),
                Err(e) => {
                    s.log(format!("⚠ demo Mercury: {e}"));
                    ("127.0.0.1".into(), 0, 0)
                }
            }
        } else {
            (cfg.mercury_host.clone(), cfg.mercury_port, cfg.mercury_bcast_port)
        };
        let (etx, erx) = mpsc::unbounded_channel();
        match Tnc::connect(&host, port, Some(bport), etx).await {
            Ok(mut t) => match t.setup(&cfg.callsign, bw.mercury_arq_bw()).await {
                Ok(()) => {
                    let _ = t.command("MODE 1").await; // calling mode: DATAC3
                    let _ = t.send_line("VERSION").await;
                    s.tnc = Some(t);
                    s.tnc_rx = Some(erx);
                    s.set(|g| {
                        g.mercury_ok = true;
                        g.listen_mode = modes::mode_by_name("DATAC3");
                    });
                    s.log(format!("Mercury: connected ({host}:{port}), listening as {}", cfg.callsign.to_uppercase()));
                }
                Err(e) => s.log(format!("⚠ Mercury setup: {e}")),
            },
            Err(e) => s.log(format!("⚠ {e}")),
        }

        s.push_wide_cfg();
        if cfg.sdr.enabled || cfg.demo {
            s.start_sdr();
        }
        s.poll_rig().await;
        s
    }

    async fn shutdown(mut self) {
        self.sim_stop.store(true, Ordering::Relaxed);
        self.sdr_stop.store(true, Ordering::Relaxed);
        if let Some(r) = self.rig.as_mut() {
            let _ = r.ptt(false).await;
        }
        if let Some(t) = self.tnc.as_mut() {
            let _ = t.command("LISTEN OFF").await;
        }
        for i in self.incoming.values() {
            i.save_partial();
        }
    }

    fn start_sdr(&mut self) {
        self.sdr_stop.store(true, Ordering::Relaxed);
        self.sdr_stop = Arc::new(AtomicBool::new(false));
        sdr::start(self.cfg.sdr.clone(), self.vfo.clone(), self.sdr_span.clone(), self.cfg.demo, self.sdr.clone(), self.sdr_stop.clone());
    }

    fn set(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.st.lock().unwrap());
    }

    fn log(&self, msg: String) {
        let mut g = self.st.lock().unwrap();
        g.log.push_back((Clock::now(), msg));
        while g.log.len() > 500 {
            g.log.pop_front();
        }
    }

    fn mycall(&self) -> String {
        self.cfg.callsign.to_uppercase()
    }

    fn dial(&self) -> f64 {
        *self.vfo.lock().unwrap()
    }

    async fn tune(&mut self, hz: f64) -> anyhow::Result<()> {
        if let Some(r) = self.rig.as_mut() {
            r.set_freq(hz).await?;
        }
        *self.vfo.lock().unwrap() = hz;
        self.last_tune = Instant::now();
        self.set(|g| g.rig.freq_hz = hz);
        Ok(())
    }

    async fn poll_rig(&mut self) {
        let Some(r) = self.rig.as_mut() else { return };
        match r.poll().await {
            Ok(status) => {
                let moved = {
                    let mut v = self.vfo.lock().unwrap();
                    let moved = (*v - status.freq_hz).abs() > 1.0;
                    *v = status.freq_hz;
                    moved
                };
                if moved {
                    self.last_tune = Instant::now(); // turned by hand on the radio
                }
                let tx = status.ptt;
                self.set(|g| {
                    g.rig = status;
                    g.rig_ok = true;
                    if tx {
                        g.tx = true;
                    }
                });
            }
            Err(e) => {
                self.set(|g| g.rig_ok = false);
                self.log(format!("⚠ Radio: {e}"));
            }
        }
    }

    // ------------------------------------------------------------ commands

    async fn command(&mut self, c: Cmd) -> anyhow::Result<()> {
        match c {
            Cmd::Start(_) | Cmd::Stop => {}
            Cmd::Update(c) => {
                let keep_bw = self.bw;
                self.cfg = c;
                self.cfg.bandwidth = keep_bw;
            }
            Cmd::SetFreq(hz) => {
                self.stop_search("tuned by hand");
                self.tune(hz).await?;
            }
            Cmd::Nudge(d) => {
                let f = self.dial() + d;
                self.tune(f).await?;
                self.log(format!("Tuned {d:+.0} Hz to {:.3} kHz", f / 1e3));
                self.set(|g| g.whistle = None);
            }
            Cmd::SetMode(m) => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_mode(&m, 3000).await?;
                }
            }
            Cmd::SetPower(p) => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_power(p).await?;
                }
            }
            Cmd::Tune(on) => {
                if let Some(t) = self.tnc.as_mut() {
                    t.command(if on { "TUNE -15" } else { "TUNE OFF" }).await?;
                }
            }
            Cmd::SetBandwidth(bw) => {
                self.bw = bw;
                self.detector = Detector::new(bw.whistle_profile());
                if let Some(t) = self.tnc.as_mut() {
                    t.command(&format!("BW{}", bw.mercury_arq_bw())).await?;
                }
                let table_mode = self.st.lock().unwrap().table.as_ref().map(|t| t.mode);
                if let Some(m) = table_mode
                    && m.bandwidth_hz > bw.hz() {
                        self.table_set_speed(modes::best_mode(bw, -30.0, 0.0)).await?;
                    }
                self.set(|g| g.bandwidth = Some(bw));
                self.push_wide_cfg();
                self.log(format!(
                    "Bandwidth {bw}: 1:1 links use Mercury BW{}, Roundtable up to {}",
                    bw.mercury_arq_bw(),
                    bw.modes().last().map(|m| m.name).unwrap_or("-")
                ));
            }
            Cmd::StartSearch(scope) => self.start_search(scope).await?,
            Cmd::StopSearch => self.stop_search("stopped"),
            Cmd::CallCq { table } => self.start_cq(table).await?,
            Cmd::StopCq => {
                if let Some(Cq { phase: CqPhase::Whistle { .. }, .. }) = &self.cq
                    && let Some(o) = &self.output {
                        o.abort();
                    }
                self.cq = None;
                self.ptt(false).await;
                self.set(|g| g.cq = CqView::default());
                self.log("CQ stopped".into());
            }
            Cmd::Connect(call) => {
                let call = call.trim().to_uppercase();
                self.stop_search("connecting");
                let me = self.mycall();
                self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
                self.set(|g| g.calling = Some(call.clone()));
                self.log(format!("Calling {call}…"));
            }
            Cmd::Disconnect => {
                if self.peer.is_some() {
                    self.hi_queue.push_back(Link::Bye { text: "73".into() }.encode());
                    self.pump().await?;
                }
                self.tnc()?.command("DISCONNECT").await?;
                self.set(|g| g.calling = None);
            }
            Cmd::Chat(text) => {
                if self.peer.is_none() {
                    anyhow::bail!("not linked to anyone: connect first (or use a Roundtable)");
                }
                let id = self.next_chat_id;
                self.next_chat_id += 1;
                self.hi_queue.push_back(Link::Chat { id, text: text.clone() }.encode());
                let me = self.mycall();
                self.set(|g| {
                    g.chat.push(ChatLine { at: Clock::now(), who: Who::Me, from: me, text, id, delivered: false })
                });
                self.pump().await?;
            }
            Cmd::SendFiles { paths, zip } => self.queue_files(paths, zip).await?,
            Cmd::AnswerOffer { fid, accept } => self.answer_offer(fid, accept).await?,
            Cmd::Cancel(fid) => {
                self.outgoing.retain(|o| o.fid != fid);
                if let Some(i) = self.incoming.remove(&fid) {
                    i.save_partial();
                }
                self.hi_queue.push_back(Link::Cancel { fid }.encode());
                self.xfer(fid, |t| t.state = XferState::Failed("cancelled".into()));
                self.pump().await?;
            }
            Cmd::JoinTable(name) => self.join_table(name, false).await?,
            Cmd::LeaveTable => {
                if let Some(id) = self.table_id() {
                    let me = self.mycall();
                    self.table_tx(TableFrame { table: id, from: me, body: TableBody::Leave }.encode());
                }
                self.set(|g| g.table = None);
                self.log("Left the Roundtable".into());
            }
            Cmd::TableSay(text) => self.table_say(text)?,
            Cmd::PassFloor(to) => {
                let Some(id) = self.table_id() else { anyhow::bail!("not at a Roundtable") };
                let me = self.mycall();
                self.table_tx(TableFrame { table: id, from: me, body: TableBody::Floor { to: to.to_uppercase() } }.encode());
                self.set(|g| {
                    if let Some(t) = g.table.as_mut() {
                        t.floor = Some(to.to_uppercase());
                    }
                });
            }
            Cmd::SetSdr(on) => {
                self.cfg.sdr.enabled = on;
                if on {
                    self.start_sdr();
                } else {
                    self.sdr_stop.store(true, Ordering::Relaxed);
                }
            }
            Cmd::SetSdrSpan(hz) => *self.sdr_span.lock().unwrap() = hz.clamp(5_000.0, 1_600_000.0),
            Cmd::LockOn(dial) => {
                if self.cq.is_some() || self.peer.is_some() {
                    anyhow::bail!("busy: stop the CQ or link first");
                }
                self.stop_search("for a wide-search hit");
                self.wide_handoff(dial, 0.0);
            }
            Cmd::WideSearch { on, auto } => {
                self.wide_on = on;
                self.wide_auto = auto;
                self.cfg.sdr.wide_search = on;
                self.cfg.sdr.wide_auto = auto;
                self.push_wide_cfg();
                self.log(if on {
                    format!("Wide whistle search on{}", if auto { ": the radio will tune to whistles it finds" } else { "" })
                } else {
                    "Wide whistle search off".into()
                });
            }
        }
        Ok(())
    }

    fn tnc(&mut self) -> anyhow::Result<&mut Tnc> {
        self.tnc.as_mut().ok_or_else(|| anyhow::anyhow!("Mercury is not connected"))
    }

    async fn ptt(&mut self, on: bool) {
        if let Some(r) = self.rig.as_mut()
            && let Err(e) = r.ptt(on).await {
                self.log(format!("⚠ PTT: {e}"));
            }
        self.set(|g| g.tx = on);
    }

    // -------------------------------------------------------------- search

    fn whistle_params(&self) -> WhistleParams {
        self.bw.whistle_profile()
    }

    fn step_hz(&self) -> f64 {
        // Neighbouring channels overlap so every caller is within capture.
        self.cfg.step_hz.min(1.8 * self.whistle_params().max_offset_hz())
    }

    async fn start_search(&mut self, scope: SearchScope) -> anyhow::Result<()> {
        if self.cq.is_some() {
            anyhow::bail!("stop calling CQ first");
        }
        let segs = bandplan::segments_for(&scope, &self.cfg.segments, self.dial());
        if segs.is_empty() {
            anyhow::bail!("no digital segment configured for that band (the radio is on {:.3} kHz)", self.dial() / 1e3);
        }
        let p = self.whistle_params();
        let (alo, ahi) = (p.f_lo - 200.0, p.f_hi + 200.0);
        let channels: Vec<f64> = segs.iter().flat_map(|s| bandplan::channels(s, self.step_hz(), alo, ahi)).collect();
        let label = match &scope {
            SearchScope::CurrentBand => format!("{} digital segment", bandplan::band_of(self.dial()).unwrap_or("?")),
            SearchScope::Band(b) => format!("{b} digital segment"),
            SearchScope::AllBands => "all bands".into(),
        };
        let cycle = channels.len() as f64 * (self.cfg.dwell_s + 0.15);
        self.log(format!("Search: {label}, {} channels, {:.0} s per pass", channels.len(), cycle));
        let first = channels[0];
        self.search = Some(Search { channels, idx: 0, phase: SearchPhase::Settle { until: Instant::now() }, scope: label.clone() });
        self.tune(first).await?;
        self.set_settle();
        self.set(|g| g.search = SearchView { active: true, scope: label, ..Default::default() });
        Ok(())
    }

    fn set_settle(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.phase = SearchPhase::Settle { until: Instant::now() + Duration::from_millis(150) };
        }
    }

    fn stop_search(&mut self, why: &str) {
        if self.search.take().is_some() {
            self.log(format!("Search {why}"));
        }
        self.set(|g| g.search.active = false);
    }

    async fn step_search(&mut self) -> anyhow::Result<()> {
        let Some(mut s) = self.search.take() else { return Ok(()) };
        let n_dwell = (self.cfg.dwell_s * FS as f64) as usize;
        let n_hold = (1.5 * FS as f64) as usize;
        let ch = s.channels[s.idx];
        let mut next_channel = false;
        match &s.phase {
            SearchPhase::Settle { until } => {
                if Instant::now() >= *until {
                    s.phase = SearchPhase::Dwell { from: audio::now_count(&self.rx) };
                }
            }
            SearchPhase::Dwell { from } => {
                let from = *from;
                let got = self.rx.lock().unwrap().range(from, n_dwell);
                if let Some(x) = got {
                    match self.detector.detect(&x) {
                        Some(d) => {
                            self.log(format!(
                                "🚂 Whistle on {:.1} kHz, {:+.0} Hz off (score {:.2}). Retuning onto it.",
                                ch / 1e3,
                                d.offset_hz,
                                d.score
                            ));
                            let target = ch + d.offset_hz;
                            s.phase = SearchPhase::Verify { target, from: 0, first: d, alt: false };
                        }
                        None if self.detector.peak_level(&x) >= self.detector.threshold => {
                            s.phase = SearchPhase::Hold { from };
                        }
                        None => next_channel = true,
                    }
                }
            }
            SearchPhase::Hold { from } => {
                let got = self.rx.lock().unwrap().range(*from, n_dwell + n_hold);
                if let Some(x) = got {
                    match self.detector.detect(&x) {
                        Some(d) => {
                            let target = ch + d.offset_hz;
                            s.phase = SearchPhase::Verify { target, from: 0, first: d, alt: false };
                        }
                        None => next_channel = true,
                    }
                }
            }
            SearchPhase::Verify { target, from, first, alt } => {
                let (target, first, from, alt) = (*target, first.clone(), *from, *alt);
                let got = if from == 0 { None } else { self.rx.lock().unwrap().range(from, n_dwell) };
                if from == 0 {
                    self.tune(target).await?;
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    s.phase = SearchPhase::Verify { target, from: audio::now_count(&self.rx).max(1), first, alt };
                } else if let Some(x) = got {
                    let p = self.whistle_params();
                    match self.detector.detect(&x) {
                        Some(v) if v.offset_hz.abs() > 50.0 && s.scope != WIDE_SCOPE => {
                            self.log(format!("Verify found {:+.0} Hz (aliased); resuming search", v.offset_hz));
                            next_channel = true;
                        }
                        // Nothing where the whistle should be: the caller may
                        // be beyond the unambiguous range. Try the alias.
                        None if !alt => {
                            let alias = p.alias_of(first.offset_hz);
                            self.log(format!(
                                "Nothing at {:+.0} Hz; checking the alias at {alias:+.0} Hz",
                                first.offset_hz
                            ));
                            let mut f = first.clone();
                            f.offset_hz = alias;
                            s.phase = SearchPhase::Verify { target: ch + alias, from: 0, first: f, alt: true };
                        }
                        None => {
                            self.log("Whistle not confirmed at either candidate; resuming search".into());
                            next_channel = true;
                        }
                        v => {
                            let fine = target + v.as_ref().map(|v| v.offset_hz).unwrap_or(0.0);
                            if (fine - target).abs() >= 2.0 {
                                self.tune(fine).await?;
                            }
                            self.log(format!(
                                "Locked on {:.3} kHz (first estimate {:+.1} Hz{}). Waiting for the CQ…",
                                fine / 1e3,
                                first.offset_hz,
                                v.map(|v| format!(", fine {:+.1} Hz", v.offset_hz)).unwrap_or_default()
                            ));
                            s.phase = SearchPhase::WaitCq { until: Instant::now() + Duration::from_secs(45) };
                        }
                    }
                }
            }
            SearchPhase::WaitCq { until } => {
                if Instant::now() >= *until {
                    self.log("No CQ followed the whistle; resuming search".into());
                    next_channel = true;
                }
            }
        }
        if next_channel && s.scope == WIDE_SCOPE {
            // A wide-search hand-off has one channel: give up on it.
            self.set(|g| g.search.active = false);
            return Ok(());
        }
        if next_channel {
            s.idx = (s.idx + 1) % s.channels.len();
            let f = s.channels[s.idx];
            s.phase = SearchPhase::Settle { until: Instant::now() + Duration::from_millis(150) };
            self.tune(f).await?;
        }
        let note = match &s.phase {
            SearchPhase::Settle { .. } | SearchPhase::Dwell { .. } => "listening".into(),
            SearchPhase::Hold { .. } => "something whistle-like… holding".into(),
            SearchPhase::Verify { .. } => "whistle! measuring".into(),
            SearchPhase::WaitCq { until } => {
                format!("locked on, waiting for CQ ({}s)", until.saturating_duration_since(Instant::now()).as_secs())
            }
        };
        let view = SearchView { active: true, scope: s.scope.clone(), index: s.idx, total: s.channels.len(), channel_hz: s.channels[s.idx], note };
        self.set(|g| g.search = view);
        self.search = Some(s);
        Ok(())
    }

    // ------------------------------------------------------------------ CQ

    async fn start_cq(&mut self, table: Option<String>) -> anyhow::Result<()> {
        self.stop_search("for CQ");
        if self.peer.is_some() {
            anyhow::bail!("already linked; disconnect first");
        }
        if let Some(t) = &table {
            self.join_table(t.clone(), true).await?;
        }
        self.cq = Some(Cq { table: table.clone(), cycle: 0, phase: CqPhase::Frame { until: Instant::now() } });
        self.begin_cq_cycle().await;
        self.log(format!(
            "Calling CQ{} on {:.3} kHz: {:.0} s whistle, then CQFRAME",
            table.as_ref().map(|t| format!(" for Roundtable \"{t}\"")).unwrap_or_default(),
            self.dial() / 1e3,
            self.cfg.whistle_s
        ));
        Ok(())
    }

    async fn begin_cq_cycle(&mut self) {
        let Some(cq) = self.cq.as_mut() else { return };
        cq.cycle += 1;
        let done = Arc::new(AtomicBool::new(false));
        cq.phase = CqPhase::Whistle { done: done.clone() };
        let secs = self.cfg.whistle_s;
        let params = self.whistle_params();
        let out = self.output.clone();
        self.ptt(true).await;
        tokio::spawn(async move {
            match out {
                Some(o) => {
                    let audio = whistle(secs, &params.with_fs(o.rate));
                    o.play(&audio).await;
                }
                None => tokio::time::sleep(Duration::from_secs_f64(secs)).await, // Demo
            }
            done.store(true, Ordering::Relaxed);
        });
    }

    async fn step_cq(&mut self) -> anyhow::Result<()> {
        let Some(cq) = self.cq.as_ref() else { return Ok(()) };
        let cycles = self.cfg.cq_cycles;
        let (cycle, table) = (cq.cycle, cq.table.clone());
        let phase = match &cq.phase {
            CqPhase::Whistle { done } if done.load(Ordering::Relaxed) => {
                self.ptt(false).await;
                let me = self.mycall();
                let arq_bw = self.bw.mercury_arq_bw();
                self.tnc()?.command(&format!("CQFRAME {me} {arq_bw}")).await?;
                let kind = if table.is_some() { CALL_TABLE } else { CALL_CHAT };
                let call = TableFrame {
                    table: table.as_deref().map(table_id).unwrap_or(0),
                    from: me,
                    body: TableBody::Call {
                        kind,
                        table_name: table.clone().unwrap_or_default(),
                        bw_code: Bandwidth::ALL.iter().position(|b| *b == self.bw).unwrap_or(4) as u8,
                        grid: self.cfg.grid.clone(),
                    },
                };
                self.table_tx(call.encode());
                Some(CqPhase::Frame { until: Instant::now() + Duration::from_secs(4) })
            }
            CqPhase::Frame { until } if Instant::now() >= *until => {
                Some(CqPhase::Listen { until: Instant::now() + Duration::from_secs_f64(self.cfg.cq_listen_s) })
            }
            CqPhase::Listen { until } if Instant::now() >= *until => {
                if cycle >= cycles {
                    self.log("No answer to CQ".into());
                    self.cq = None;
                    self.set(|g| g.cq = CqView::default());
                    return Ok(());
                }
                self.begin_cq_cycle().await;
                None
            }
            _ => None,
        };
        if let (Some(p), Some(cq)) = (phase, self.cq.as_mut()) {
            cq.phase = p;
        }
        if let Some(cq) = self.cq.as_ref() {
            let phase = match &cq.phase {
                CqPhase::Whistle { .. } => "whistling".to_string(),
                CqPhase::Frame { .. } => "sending CQFRAME".into(),
                CqPhase::Listen { until } => {
                    format!("listening for answers ({}s)", until.saturating_duration_since(Instant::now()).as_secs())
                }
            };
            let v = CqView { active: true, cycle: cq.cycle, cycles, phase, table: cq.table.clone() };
            self.set(|g| g.cq = v);
        }
        Ok(())
    }

    // -------------------------------------------------------------- events

    async fn on_tnc(&mut self, e: TncEvent) -> anyhow::Result<()> {
        match e {
            TncEvent::Line(l) => self.on_line(l).await?,
            TncEvent::Data(d) => {
                // Mercury reports CONNECTED on the control port and delivers
                // data on the data port: the first bytes can win the race.
                // Hold them until the link is confirmed.
                if self.peer.is_none() {
                    self.prelink.extend_from_slice(&d);
                    self.prelink.truncate(1 << 20);
                    return Ok(());
                }
                for p in self.dec.feed(&d)? {
                    self.on_link(p).await?;
                }
            }
            TncEvent::Broadcast(f) => self.on_table_frame(&f).await?,
            TncEvent::Closed(which) => {
                self.log(format!("⚠ Mercury closed the {which} connection"));
                self.tnc = None;
                self.tnc_rx = None;
                self.set(|g| g.mercury_ok = false);
            }
        }
        Ok(())
    }

    async fn on_line(&mut self, l: String) -> anyhow::Result<()> {
        let word = l.split_whitespace().next().unwrap_or("").to_string();
        match word.as_str() {
            "CONNECTED" => {
                let Some((peer, bw)) = parse_connected(&l, &self.mycall()) else { return Ok(()) };
                self.stop_search("— linked");
                if self.cq.take().is_some() {
                    if let Some(o) = &self.output {
                        o.abort();
                    }
                    self.ptt(false).await;
                    self.set(|g| g.cq = CqView::default());
                }
                self.peer = Some(peer.clone());
                self.dec = LinkDecoder::default();
                let early = std::mem::take(&mut self.prelink);
                self.est = ThroughputEstimator::default();
                self.mercury_buffer = 0;
                self.set(|g| {
                    g.incoming_call = false;
                    g.calling = None;
                    g.link = Some(LinkInfo { peer: peer.clone(), bw, since: Some(Instant::now()), ..Default::default() });
                    g.chat.push(ChatLine {
                        at: Clock::now(),
                        who: Who::System,
                        from: String::new(),
                        text: format!("Linked with {peer} ({bw} Hz)"),
                        id: 0,
                        delivered: true,
                    });
                });
                self.log(format!("Linked with {peer}, {bw} Hz"));
                self.hi_queue.push_back(
                    Link::Hello {
                        version: VERSION,
                        caps: CAP_RESUME | CAP_ZIP,
                        name: self.cfg.name.clone(),
                        grid: self.cfg.grid.clone(),
                        app: format!("Whistlestop {}", env!("CARGO_PKG_VERSION")),
                    }
                    .encode(),
                );
                // Re-offer files queued before the link came up.
                for o in self.outgoing.iter() {
                    self.hi_queue.push_back(o.offer().encode());
                }
                self.pump().await?;
                if !early.is_empty() {
                    for p in self.dec.feed(&early)? {
                        self.on_link(p).await?;
                    }
                }
            }
            "DISCONNECTED" => {
                if let Some(p) = self.peer.take() {
                    self.log(format!("Link with {p} ended"));
                    self.set(|g| {
                        g.chat.push(ChatLine {
                            at: Clock::now(),
                            who: Who::System,
                            from: String::new(),
                            text: format!("Link with {p} ended"),
                            id: 0,
                            delivered: true,
                        })
                    });
                }
                for i in self.incoming.values() {
                    i.save_partial();
                }
                self.incoming.clear();
                for o in self.outgoing.iter_mut() {
                    o.next = None; // re-offer (with resume) on the next link
                }
                self.hi_queue.clear();
                self.prelink.clear();
                self.dec = LinkDecoder::default();
                self.set(|g| {
                    g.link = None;
                    g.tx = false;
                    g.calling = None;
                    g.incoming_call = false;
                    for t in g.transfers.iter_mut() {
                        if matches!(t.state, XferState::Sending | XferState::Receiving | XferState::Offered) {
                            t.state = XferState::Failed("link ended — will resume next time".into());
                        }
                    }
                });
            }
            "PENDING" => {
                if self.cq.is_none() {
                    self.set(|g| g.incoming_call = true);
                    // Hold the search: a connect is arriving here.
                    if let Some(s) = self.search.as_mut() {
                        s.phase = SearchPhase::WaitCq { until: Instant::now() + Duration::from_secs(60) };
                    }
                }
            }
            "CANCELPENDING" => self.set(|g| g.incoming_call = false),
            "CQFRAME" => {
                let call = l.split_whitespace().nth(1).unwrap_or("?").to_string();
                let dial = self.dial();
                self.log(format!("📣 CQ from {call} on {:.3} kHz", dial / 1e3));
                self.add_heard(&call, "CQ".into(), None);
                let locked = matches!(self.search.as_ref().map(|s| &s.phase), Some(SearchPhase::WaitCq { .. }));
                if locked {
                    self.stop_search(&format!("found {call}"));
                    if self.cfg.auto_answer && self.peer.is_none() {
                        let me = self.mycall();
                        self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
                        self.log(format!("Auto-answering {call}"));
                    }
                }
            }
            "PTT" => {
                let on = l.ends_with("ON");
                self.set(|g| g.tx = on);
            }
            "BUFFER" => {
                let n = l[6..].trim().parse().unwrap_or(0);
                self.mercury_buffer = n;
                self.est.on_buffer(Instant::now(), n);
                self.pump().await?;

            }
            "BITRATE" => {
                self.est.on_bitrate_line(&l);
                let (bps, mode) = (self.est.reported_bps, self.est.reported_mode);
                self.set(|g| {
                    g.modem_bps = bps;
                    if let (Some(m), Some(link)) = (mode, g.link.as_mut()) {
                        link.mode = Some(m);
                    }
                });
            }
            "SN" => {
                self.est.on_snr_line(&l);
                let v = self.est.snr_db;
                self.set(|g| g.snr_db = v);
            }
            _ if l.to_ascii_lowercase().contains("mercury") || word == "VERSION" => {
                let v = l.trim_start_matches("VERSION").trim().to_string();
                self.set(|g| g.modem_version = Some(v));
            }
            _ => {}
        }
        Ok(())
    }

    fn add_heard(&self, call: &str, what: String, table: Option<String>) {
        let dial = self.dial();
        self.set(|g| {
            g.heard.retain(|h| !(h.call == call && h.what == what));
            g.heard.insert(0, Heard { at: Clock::now(), call: call.to_string(), dial_hz: dial, what, table });
            g.heard.truncate(30);
        });
    }

    async fn on_link(&mut self, p: Link) -> anyhow::Result<()> {
        let peer = self.peer.clone().unwrap_or_default();
        match p {
            Link::Hello { name, grid, app, .. } => {
                self.set(|g| {
                    if let Some(l) = g.link.as_mut() {
                        l.peer_name = name;
                        l.peer_grid = grid;
                        l.peer_app = app;
                    }
                });
            }
            Link::Chat { id, text } => {
                self.hi_queue.push_back(Link::Received { id }.encode());
                self.set(|g| {
                    g.chat.push(ChatLine { at: Clock::now(), who: Who::Them, from: peer, text, id, delivered: true })
                });
                self.pump().await?;
            }
            Link::Received { id } => self.set(|g| {
                for c in g.chat.iter_mut().filter(|c| c.who == Who::Me && c.id == id) {
                    c.delivered = true;
                }
            }),
            Link::Offer { fid, name, size, sha, zipped, files } => {
                let inc = Incoming::new(&self.cfg.download_dir, fid, name.clone(), size, sha, zipped);
                let have = inc.have();
                self.incoming.insert(fid, inc);
                let eta = self.est.eta_s((size - have) as usize, self.bw);
                self.set(|g| {
                    g.transfers.push(Transfer {
                        fid,
                        dir: Dir::In,
                        name: name.clone(),
                        size,
                        done: have,
                        zipped,
                        files,
                        original: size,
                        state: XferState::WaitingForYou,
                        eta_s: eta,
                        started: None,
                    })
                });
                self.log(format!(
                    "{peer} offers {name} ({}){}, about {}",
                    modes::human_bytes(size as usize),
                    if have > 0 { format!(", {} already here — will resume", modes::human_bytes(have as usize)) } else { String::new() },
                    modes::human_duration(eta)
                ));
            }
            Link::Answer { fid, accept, resume_from } => {
                if let Some(o) = self.outgoing.iter_mut().find(|o| o.fid == fid) {
                    if accept {
                        o.next = Some(resume_from.min(o.data.len() as u64));
                        if resume_from > 0 {
                            self.log(format!("{peer} already has {} — resuming", modes::human_bytes(resume_from as usize)));
                        }
                        self.xfer(fid, |t| {
                            t.state = XferState::Sending;
                            t.done = resume_from;
                            t.started = Some(Instant::now());
                        });
                    } else {
                        self.outgoing.retain(|o| o.fid != fid);
                        self.xfer(fid, |t| t.state = XferState::Declined);
                    }
                }
                self.pump().await?;
            }
            Link::Chunk { fid, offset, data } => {
                let mut finished = None;
                let mut progress = None;
                if let Some(inc) = self.incoming.get_mut(&fid) {
                    inc.chunk(offset, &data);
                    let have = inc.have();
                    if have % (16 * CHUNK as u64) < CHUNK as u64 {
                        inc.save_partial();
                    }
                    progress = Some((have, inc.size - have, inc.complete()));
                }
                if let Some((have, left, complete)) = progress {
                    let eta = self.est.eta_s(left as usize, self.bw);
                    self.xfer(fid, |t| {
                        t.done = have;
                        t.eta_s = eta;
                    });
                    if complete {
                        finished = self.incoming.remove(&fid);
                    }
                }
                if let Some(inc) = finished {
                    let name = inc.name.clone();
                    match inc.finish(&self.cfg.download_dir) {
                        Ok(path) => {
                            self.hi_queue.push_back(Link::Done { fid, ok: true }.encode());
                            self.log(format!("✓ Received {name} → {}", path.display()));
                            self.xfer(fid, |t| t.state = XferState::Done(Some(path)));
                        }
                        Err(e) => {
                            self.hi_queue.push_back(Link::Done { fid, ok: false }.encode());
                            self.xfer(fid, |t| t.state = XferState::Failed(e.to_string()));
                        }
                    }
                    self.pump().await?;
                }
            }
            Link::Done { fid, ok } => {
                self.outgoing.retain(|o| o.fid != fid);
                self.xfer(fid, |t| {
                    t.state = if ok { XferState::Done(None) } else { XferState::Failed("checksum failed at the other end".into()) };
                    t.done = t.size;
                });
            }
            Link::Cancel { fid } => {
                self.outgoing.retain(|o| o.fid != fid);
                if let Some(i) = self.incoming.remove(&fid) {
                    i.save_partial();
                }
                self.xfer(fid, |t| t.state = XferState::Failed(format!("cancelled by {peer}")));
            }
            Link::Report { snr_db, mode_index } => self.set(|g| {
                if let Some(l) = g.link.as_mut() {
                    l.their_report_db = Some(snr_db);
                    if let Some(m) = modes::mode_by_index(mode_index) {
                        l.mode = Some(m);
                    }
                }
            }),
            Link::Bye { text } => {
                self.set(|g| {
                    g.chat.push(ChatLine { at: Clock::now(), who: Who::Them, from: peer, text, id: 0, delivered: true })
                });
            }
            Link::Unknown(_) => {}
        }
        Ok(())
    }

    fn xfer(&self, fid: u64, f: impl FnOnce(&mut Transfer)) {
        let mut g = self.st.lock().unwrap();
        if let Some(t) = g.transfers.iter_mut().rev().find(|t| t.fid == fid) {
            f(t);
        }
    }

    // ------------------------------------------------------------- files

    async fn queue_files(&mut self, paths: Vec<PathBuf>, zip: bool) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let single_file = paths.len() == 1 && paths[0].is_file();
        let (name, data, zipped, files, original) = if zip || !single_file {
            let z = tokio::task::spawn_blocking(move || ziputil::zip_paths(&paths, None)).await??;
            self.log(format!(
                "Zipped {} file(s): {} → {} ({:.0}% of original)",
                z.files,
                modes::human_bytes(z.original_bytes as usize),
                modes::human_bytes(z.data.len()),
                100.0 * z.ratio()
            ));
            (z.name.clone(), z.data, true, z.files as u64, z.original_bytes)
        } else {
            let data = std::fs::read(&paths[0])?;
            let len = data.len() as u64;
            (paths[0].file_name().unwrap().to_string_lossy().into_owned(), data, false, 1, len)
        };
        let fid = self.next_fid;
        self.next_fid += 1;
        let out = Outgoing::new(fid, name.clone(), data, zipped, files);
        let size = out.data.len() as u64;
        let eta = self.est.eta_s(file_wire_size(&name, size), self.bw);
        if self.peer.is_some() {
            self.hi_queue.push_back(out.offer().encode());
        }
        self.outgoing.push(out);
        self.set(|g| {
            g.transfers.push(Transfer {
                fid,
                dir: Dir::Out,
                name: name.clone(),
                size,
                done: 0,
                zipped,
                files,
                original,
                state: XferState::Offered,
                eta_s: eta,
                started: None,
            })
        });
        self.log(format!(
            "Offering {name} ({}), about {} at the current speed{}",
            modes::human_bytes(size as usize),
            modes::human_duration(eta),
            if self.peer.is_none() { " — will offer when linked" } else { "" }
        ));
        self.pump().await
    }

    async fn answer_offer(&mut self, fid: u64, accept: bool) -> anyhow::Result<()> {
        let have = self.incoming.get(&fid).map(|i| i.have()).unwrap_or(0);
        if !accept {
            self.incoming.remove(&fid);
        }
        self.hi_queue.push_back(Link::Answer { fid, accept, resume_from: have }.encode());
        self.xfer(fid, |t| {
            t.state = if accept { XferState::Receiving } else { XferState::Declined };
            t.started = Some(Instant::now());
        });
        self.pump().await
    }

    /// Hand data to Mercury: everything urgent first, then file chunks, but
    /// never more than two chunks waiting in Mercury's buffer, so a chat
    /// line typed during a transfer waits for at most two chunks.
    async fn pump(&mut self) -> anyhow::Result<()> {
        if self.peer.is_none() || self.tnc.is_none() {
            return Ok(());
        }
        while let Some(p) = self.hi_queue.pop_front() {
            self.mercury_buffer += p.len();
            self.tnc()?.send_data(&p).await?;
        }
        while self.mercury_buffer < 2 * CHUNK {
            let Some(o) = self.outgoing.iter_mut().find(|o| o.next.is_some() && !o.finished_sending()) else { break };
            let fid = o.fid;
            let Some(chunk) = o.next_chunk() else { break };
            let sent = o.next.unwrap_or(0);
            let total = o.data.len();
            let bytes = chunk.encode();
            self.mercury_buffer += bytes.len();
            self.tnc()?.send_data(&bytes).await?;
            let eta = self.est.eta_s(total - sent as usize + self.mercury_buffer, self.bw);
            self.xfer(fid, |t| {
                t.done = sent.saturating_sub(self.mercury_buffer as u64);
                t.eta_s = eta;
            });
        }
        Ok(())
    }

    // -------------------------------------------------------- roundtable

    fn table_id(&self) -> Option<u16> {
        self.st.lock().unwrap().table.as_ref().map(|t| t.id)
    }

    async fn join_table(&mut self, name: String, host: bool) -> anyhow::Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("give the Roundtable a name");
        }
        let id = table_id(&name);
        let mode = modes::mode_by_name("DATAC3").unwrap();
        self.tnc()?.command(&format!("MODE {}", mode.index)).await?;
        self.set(|g| {
            g.listen_mode = Some(mode);
            g.table = Some(TableState {
                name: name.clone(),
                id,
                host,
                mode,
                members: HashMap::new(),
                floor: None,
                lines: vec![],
                sending_eta_s: 0.0,
            })
        });
        if !host {
            let me = self.mycall();
            let snr = self.est.snr_db.unwrap_or(0.0).round() as i8;
            self.table_tx(TableFrame { table: id, from: me, body: TableBody::Join { table_name: name.clone(), snr_of_host: snr } }.encode());
            self.log(format!("Joined Roundtable \"{name}\" on {:.3} kHz", self.dial() / 1e3));
        } else {
            self.log(format!("Hosting Roundtable \"{name}\" on {:.3} kHz", self.dial() / 1e3));
        }
        Ok(())
    }

    fn table_tx(&mut self, frame: Vec<u8>) {
        self.table_queue.push_back(frame);
    }

    fn table_say(&mut self, text: String) -> anyhow::Result<()> {
        let (id, mode) = {
            let g = self.st.lock().unwrap();
            let t = g.table.as_ref().ok_or_else(|| anyhow::anyhow!("not at a Roundtable: join or host one first"))?;
            (t.id, t.mode)
        };
        self.table_msg_id += 1;
        let me = self.mycall();
        let frames = table_text_frames(id, &me, self.table_msg_id, &text, mode.broadcast_bytes())?;
        let eta = frames.len() as f64 * mode.frame_s;
        self.table_sent.insert(self.table_msg_id, frames.clone());
        self.table_queue.extend(frames);
        self.set(|g| {
            if let Some(t) = g.table.as_mut() {
                t.lines.push((Clock::now(), me, text));
                t.sending_eta_s += eta;
            }
        });
        Ok(())
    }

    /// Transmit queued table frames one modem frame at a time.
    async fn step_table_tx(&mut self) -> anyhow::Result<()> {
        if let Some((m, at)) = self.pending_speed {
            if Instant::now() < at {
                return Ok(()); // hold table traffic until the switch
            }
            self.pending_speed = None;
            self.apply_table_mode(m).await?;
            self.log(format!("Roundtable speed → {} ({} bps)", m.name, m.payload_bps));
        }
        if Instant::now() < self.table_busy_until || self.tnc.is_none() {
            return Ok(());
        }
        let Some(f) = self.table_queue.pop_front() else {
            self.set(|g| {
                if let Some(t) = g.table.as_mut() {
                    t.sending_eta_s = 0.0;
                }
            });
            return Ok(());
        };
        let mode = self.st.lock().unwrap().listen_mode.unwrap_or(&modes::MODES[4]);
        self.tnc()?.send_broadcast(&f).await?;
        self.table_busy_until = Instant::now() + Duration::from_secs_f64(mode.frame_s + 0.4);
        let left = self.table_queue.len() as f64 * mode.frame_s;
        self.set(|g| {
            if let Some(t) = g.table.as_mut() {
                t.sending_eta_s = left;
            }
        });
        Ok(())
    }

    async fn on_table_frame(&mut self, f: &[u8]) -> anyhow::Result<()> {
        let Some(fr) = TableFrame::decode(f) else { return Ok(()) };
        if let TableBody::Call { kind, table_name, .. } = &fr.body {
            let what = if *kind == CALL_TABLE { format!("Roundtable \"{table_name}\"") } else { "CQ (chat)".into() };
            self.add_heard(&fr.from, what, (*kind == CALL_TABLE).then(|| table_name.clone()));
            return Ok(());
        }
        let Some(my_id) = self.table_id() else { return Ok(()) };
        if fr.table != my_id {
            return Ok(());
        }
        let from = fr.from.clone();
        match fr.body {
            TableBody::Text { id, part, total, data } => {
                if let Some(text) = self.table_asm.add(&from, id, part, total, &data) {
                    self.set(|g| {
                        if let Some(t) = g.table.as_mut() {
                            t.lines.push((Clock::now(), from.clone(), text));
                            t.members.entry(from).or_default().last = Some(Instant::now());
                        }
                    });
                }
            }
            TableBody::Join { snr_of_host, .. } => {
                self.set(|g| {
                    if let Some(t) = g.table.as_mut() {
                        let m = t.members.entry(from.clone()).or_default();
                        m.snr_of_host = Some(snr_of_host);
                        m.last = Some(Instant::now());
                        t.lines.push((Clock::now(), String::new(), format!("{from} joined (hears the host at {snr_of_host:+} dB)")));
                    }
                });
                self.ramp().await?;
            }
            TableBody::Speed { mode_index } => {
                if let Some(m) = modes::mode_by_index(mode_index) {
                    self.apply_table_mode(m).await?;
                    self.log(format!("Roundtable speed → {} ({} bps), set by {from}", m.name, m.payload_bps));
                }
            }
            TableBody::Again { of, id, missing } => {
                if of == self.mycall()
                    && let Some(frames) = self.table_sent.get(&id).cloned() {
                        for (i, fr) in frames.into_iter().enumerate() {
                            if missing & (1 << i) != 0 {
                                self.table_queue.push_back(fr);
                            }
                        }
                    }
            }
            TableBody::Floor { to } => self.set(|g| {
                if let Some(t) = g.table.as_mut() {
                    t.floor = Some(to.clone());
                    t.lines.push((Clock::now(), String::new(), format!("{from} passes the floor to {to}")));
                }
            }),
            TableBody::Leave => self.set(|g| {
                if let Some(t) = g.table.as_mut() {
                    t.members.remove(&from);
                    t.lines.push((Clock::now(), String::new(), format!("{from} left")));
                }
            }),
            TableBody::Call { .. } => {}
        }
        Ok(())
    }

    async fn apply_table_mode(&mut self, m: &'static Mode) -> anyhow::Result<()> {
        self.tnc()?.command(&format!("MODE {}", m.index)).await?;
        self.set(|g| {
            g.listen_mode = Some(m);
            if let Some(t) = g.table.as_mut() {
                t.mode = m;
            }
        });
        Ok(())
    }

    async fn table_set_speed(&mut self, m: &'static Mode) -> anyhow::Result<()> {
        let Some(id) = self.table_id() else { return Ok(()) };
        let me = self.mycall();
        let f = TableFrame { table: id, from: me, body: TableBody::Speed { mode_index: m.index } }.encode();
        // Announce twice in the old mode, and switch only once Mercury has
        // actually sent both (switching sooner sends them in the new mode,
        // which members still on the old one cannot hear).
        let old = self.st.lock().unwrap().listen_mode.unwrap_or(&modes::MODES[4]);
        self.tnc()?.send_broadcast(&f).await?;
        self.tnc()?.send_broadcast(&f).await?;
        let at = Instant::now() + Duration::from_secs_f64(2.0 * old.frame_s + 3.0);
        self.pending_speed = Some((m, at));
        self.log(format!("Roundtable: announcing speed change {} → {} ({} bps)", old.name, m.name, m.payload_bps));
        Ok(())
    }

    /// Host only: move the table to the fastest mode everyone can copy.
    async fn ramp(&mut self) -> anyhow::Result<()> {
        if !self.cfg.auto_ramp || self.pending_speed.is_some() {
            return Ok(());
        }
        let (host, worst, current) = {
            let g = self.st.lock().unwrap();
            let Some(t) = g.table.as_ref() else { return Ok(()) };
            let recent = |m: &Member| m.last.is_some_and(|l| l.elapsed() < Duration::from_secs(300));
            let worst = t.members.values().filter(|m| recent(m)).filter_map(|m| m.snr_of_host).min();
            (t.host, worst, t.mode)
        };
        let Some(worst) = worst else { return Ok(()) };
        if !host {
            return Ok(());
        }
        let target = modes::best_mode(self.bw, worst as f64, self.cfg.ramp_margin_db);
        self.last_ramp = Instant::now();
        if target.payload_bps < current.payload_bps {
            self.last_downgrade = Instant::now();
            self.table_set_speed(target).await?;
        } else if target.payload_bps > current.payload_bps && self.last_downgrade.elapsed() > Duration::from_secs(120) {
            self.table_set_speed(target).await?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- tick

    async fn tick(&mut self) -> anyhow::Result<()> {
        self.step_waterfall();
        self.step_wide().await?;
        if self.search.is_some() {
            self.step_search().await?;
        } else if self.cq.is_none() {
            self.step_watch();
        }
        self.step_cq().await?;
        self.step_table_tx().await?;

        if self.peer.is_some() {
            let bps = self.est.bps(self.bw);
            let mode = self.est.reported_mode;
            self.set(|g| {
                g.est_bps = bps;
                if let Some(l) = g.link.as_mut() {
                    l.bps = bps;
                    if mode.is_some() {
                        l.mode = mode;
                    }
                }
            });
            if self.last_query.elapsed() > Duration::from_secs(5) {
                self.last_query = Instant::now();
                if let Some(t) = self.tnc.as_mut() {
                    let _ = t.send_line("BITRATE").await;
                    let _ = t.send_line("SN").await;
                }
            }
            if self.last_report.elapsed() > Duration::from_secs(30) {
                self.last_report = Instant::now();
                if let Some(t) = self.tnc.as_mut() {
                    let _ = t.send_line("SN").await;
                }
                let snr = self.est.snr_db.unwrap_or(0.0).round().clamp(-60.0, 60.0) as i8;
                let mi = self.est.reported_mode.map(|m| m.index).unwrap_or(1);
                self.hi_queue.push_back(Link::Report { snr_db: snr, mode_index: mi }.encode());
                self.pump().await?;
            }
        } else {
            let bps = self.est.bps(self.bw);
            self.set(|g| g.est_bps = bps);
        }

        // Roundtable housekeeping: ask again for missing parts; host ramps.
        if self.table_id().is_some() {
            if self.last_again.elapsed() > Duration::from_secs(20) {
                self.last_again = Instant::now();
                let id = self.table_id().unwrap();
                let me = self.mycall();
                for (of, mid, missing) in self.table_asm.missing() {
                    self.table_tx(TableFrame { table: id, from: me.clone(), body: TableBody::Again { of, id: mid, missing } }.encode());
                }
            }
            if self.last_ramp.elapsed() > Duration::from_secs(30) {
                self.ramp().await?;
            }
        }
        Ok(())
    }

    /// Radio-audio waterfall rows from the shared RX buffer.
    fn step_waterfall(&mut self) {
        let n = self.spectrum.n;
        let hop = n; // 15.6 rows/s at 8 kHz
        let mut rows = vec![];
        {
            let rx = self.rx.lock().unwrap();
            if self.spec_from == 0 || rx.count.saturating_sub(self.spec_from) > (FS * 2) as u64 {
                self.spec_from = rx.count.saturating_sub(n as u64);
            }
            while let Some(x) = rx.range(self.spec_from, n) {
                rows.push(self.spectrum.row(&x));
                self.spec_from += hop as u64;
            }
        }
        if !rows.is_empty() {
            self.set(|g| {
                g.audio_rows.extend(rows);
                while g.audio_rows.len() > 64 {
                    g.audio_rows.pop_front();
                }
            });
        }
    }

    /// Tell the SDR thread what to watch.
    fn push_wide_cfg(&mut self) {
        let wide = self.wide_on.then(|| sdr::WideCfg {
            segments: self.cfg.segments.clone(),
            params: self.whistle_params(),
            step_hz: self.step_hz(),
        });
        self.sdr.lock().unwrap().wide = wide;
    }

    /// Wide-search hits: show them, and (auto) hand the best new one to the
    /// main radio, whose own audio fine-tunes the lock (the dongle's crystal
    /// can be hundreds of Hz off).
    async fn step_wide(&mut self) -> anyhow::Result<()> {
        let (hits, note) = {
            let o = self.sdr.lock().unwrap();
            (o.hits.clone(), o.wide_note.clone())
        };
        let view = WideView {
            on: self.wide_on,
            auto: self.wide_auto,
            note,
            hits: hits.iter().map(|(at, h)| (h.dial_hz, h.score, at.elapsed().as_secs_f64())).collect(),
        };
        self.set(|g| g.wide = view);
        self.wide_tried.retain(|(_, at)| at.elapsed() < Duration::from_secs(120));
        let busy = self.search.is_some() || self.cq.is_some() || self.peer.is_some() || self.st.lock().unwrap().calling.is_some();
        if !self.wide_on || !self.wide_auto || busy {
            return Ok(());
        }
        let fresh = hits
            .iter()
            .filter(|(at, h)| at.elapsed() < Duration::from_secs(4) && !self.wide_tried.iter().any(|(f, _)| (f - h.dial_hz).abs() < 200.0))
            .max_by(|a, b| a.1.score.partial_cmp(&b.1.score).unwrap());
        if let Some((_, h)) = fresh {
            let (dial, score) = (h.dial_hz, h.score);
            self.wide_handoff(dial, score);
        }
        Ok(())
    }

    fn wide_handoff(&mut self, dial: f64, score: f64) {
        self.wide_tried.push((dial, Instant::now()));
        self.log(format!("🚂 SDR whistle at {:.3} kHz; tuning the radio there to fine-lock", dial / 1e3));
        let first = crate::whistle::Detection { offset_hz: 0.0, score, n_pairs: 0, spread_hz: 0.0 };
        self.search = Some(Search {
            channels: vec![dial],
            idx: 0,
            phase: SearchPhase::Verify { target: dial, from: 0, first, alt: true },
            scope: WIDE_SCOPE.into(),
        });
    }

    /// Parked on a frequency (not searching): listen for whistles anyway.
    fn step_watch(&mut self) {
        if !self.cfg.whistle_watch || self.last_watch.elapsed() < Duration::from_millis(1000) {
            return;
        }
        // The window must hold only audio heard on this frequency.
        if self.last_tune.elapsed() < Duration::from_secs_f64(self.cfg.dwell_s + 0.3) {
            return;
        }
        self.last_watch = Instant::now();
        if self.st.lock().unwrap().tx || self.peer.is_some() {
            return;
        }
        let n = (self.cfg.dwell_s * FS as f64) as usize;
        let x = self.rx.lock().unwrap().last(n);
        if x.len() < n {
            return;
        }
        if let Some(d) = self.detector.detect(&x) {
            let dial = self.dial();
            let fresh = self.st.lock().unwrap().whistle.as_ref().is_none_or(|w| w.at.elapsed() > Duration::from_secs(20));
            if fresh {
                self.log(format!(
                    "🚂 Whistle heard {:+.0} Hz from here (caller at {:.3} kHz)",
                    d.offset_hz,
                    (dial + d.offset_hz) / 1e3
                ));
            }
            self.set(|g| g.whistle = Some(WhistleSeen { at: Instant::now(), dial_hz: dial, offset_hz: d.offset_hz, score: d.score }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wide search: the (simulated) SDR spots the demo station anywhere in
    /// the segment, the radio tunes there and fine-locks on its own audio,
    /// hears the CQ and connects — without stepping through channels.
    #[test]
    fn demo_wide_search_finds_and_links() {
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, callsign: "N0WIDE".into(), auto_answer: true, ..Default::default() };
        cfg.sdr.wide_search = true;
        cfg.sdr.wide_auto = true;
        cfg.download_dir = std::env::temp_dir().join(format!("ws-wide-{}", std::process::id()));
        h.send(Cmd::Start(cfg));
        let t = Instant::now();
        let mut saw_hit = None;
        loop {
            {
                let g = h.state.lock().unwrap();
                if saw_hit.is_none() && !g.wide.hits.is_empty() {
                    saw_hit = Some((t.elapsed(), g.wide.hits[0].0));
                }
                if g.link.as_ref().is_some_and(|l| l.peer == "K7BOT") {
                    assert!((g.rig.freq_hz - 14_103_150.0).abs() <= 55.0, "{}", g.rig.freq_hz);
                    assert!(g.search.scope == WIDE_SCOPE || !g.search.active);
                    break;
                }
            }
            if t.elapsed() > Duration::from_secs(90) {
                let g = h.state.lock().unwrap();
                let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
                panic!("no link; wide {:?}\n{}", g.wide, log.join("\n"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // The link must carry data, both ways.
        h.send(Cmd::Chat("hello over the wide-search link".into()));
        let t2 = Instant::now();
        loop {
            {
                let g = h.state.lock().unwrap();
                let reply = g.chat.iter().any(|c| c.who == Who::Them && c.text.contains("wide-search link"));
                let delivered = g.chat.iter().any(|c| c.who == Who::Me && c.delivered);
                if reply && delivered {
                    break;
                }
                if t2.elapsed() > Duration::from_secs(40) {
                    let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
                    panic!("no chat over the link; chat {:?}\n{}", g.chat.iter().map(|c| (&c.who, &c.text, c.delivered)).collect::<Vec<_>>(), log.join("\n"));
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let (when, dial) = saw_hit.expect("SDR hit");
        // The SDR's reading carries its crystal error (+240 Hz in the demo);
        // the lock above is within Mercury's window anyway.
        assert!((dial - 14_103_390.0).abs() < 5.0, "SDR saw {dial}");
        eprintln!("SDR hit after {:.1} s at {dial:.1}; linked after {:.1} s", when.as_secs_f64(), t.elapsed().as_secs_f64());
        h.send(Cmd::Stop);
    }

    /// The whole demo flow, headless: search the 20m segment, find the
    /// whistling demo station, lock on within Mercury's +/-55 Hz window,
    /// hear its CQFRAME, connect, chat, and send a zipped file.
    #[test]
    fn demo_search_connect_chat_file() {
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, callsign: "N0TEST".into(), auto_answer: true, ..Default::default() };
        cfg.download_dir = std::env::temp_dir().join(format!("ws-engine-{}", std::process::id()));
        cfg.sdr.enabled = false;
        h.send(Cmd::Start(cfg.clone()));
        let wait = |what: &str, secs: u64, f: &dyn Fn(&State) -> bool| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(secs) {
                if f(&h.state.lock().unwrap()) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let g = h.state.lock().unwrap();
            let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
            panic!("timed out waiting for {what}; log:\n{}", log.join("\n"));
        };
        wait("Mercury", 10, &|s| s.mercury_ok);
        h.send(Cmd::StartSearch(SearchScope::CurrentBand));
        wait("link with K7BOT", 150, &|s| s.link.as_ref().is_some_and(|l| l.peer == "K7BOT"));
        let f = h.state.lock().unwrap().rig.freq_hz;
        assert!((f - 14_103_150.0).abs() <= 55.0, "locked {f}");
        wait("bot greeting", 30, &|s| s.chat.iter().any(|c| c.who == Who::Them));
        h.send(Cmd::Chat("hello from the test".into()));
        wait("bot reply", 30, &|s| s.chat.iter().any(|c| c.who == Who::Them && c.text.contains("hello from the test")));
        let dir = cfg.download_dir.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("log.txt"), "QSO log line\n".repeat(400)).unwrap();
        h.send(Cmd::SendFiles { paths: vec![dir.clone()], zip: true });
        wait("file delivered", 60, &|s| s.transfers.iter().any(|t| t.dir == Dir::Out && t.state == XferState::Done(None)));
        h.send(Cmd::Stop);
        std::fs::remove_dir_all(&cfg.download_dir).ok();
    }
}
