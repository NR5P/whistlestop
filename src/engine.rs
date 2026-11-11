//! The engine: owns the radio, Mercury and audio, and runs everything —
//! whistle watch, search, CQ, 1:1 link (chat, files), and Roundtables.
//!
//! It runs on a tokio runtime in its own thread. The GUI sends `Cmd`s and
//! reads `State` (behind a mutex), so the interface never blocks on the radio.

use crate::audio::{self, AudioSpectrum, LiveInput, LiveOutput, SharedRx, SimBand, SimSignal};
use crate::bandplan::{self, SearchScope};
use crate::config::Config;
use crate::e2e::{self, Channel, Notice};
use crate::fake_mercury::{FakeMercury, FakeOptions};
use crate::modes::{self, Bandwidth, Mode, ThroughputEstimator};
use crate::protocol::*;
use crate::rig::{Rig, RigStatus, RigctldClient};
use crate::sdr;
use crate::tnc::{Tnc, TncEvent, parse_connected};
use crate::whistle::{Detector, FS, TrackDetector, WhistleParams, whistle};
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
    /// Rig controls (only what the radio's capabilities list).
    RigLevel(String, f32),
    RigFunc(String, bool),
    RigOp(String),
    RigVfo(String),
    RigSplit(bool),
    RigOffset { xit: bool, hz: i32 },
    RigPassband(i64),
    /// The Radio Controls window is open: poll the extended controls.
    RigWatch(bool),
    /// Start the antenna tuner now.
    TuneAntenna,
    /// Speaker monitor on/off, and its volume.
    Monitor(bool),
    /// You're away (or back).
    SetAway(bool),
    /// Write Depot mail to a station.
    DepotPost { to: String, via: Option<String>, text: String },
    DepotDelete(u64),
    DepotRead(u64),
    /// Connect to a station now to deliver its mail (or collect yours).
    DepotConnect(String),
    /// Ask the linked station to move the link to this dial frequency.
    Qsy(f64),
    /// Browse the linked station's file library at this path ("" = top).
    LibList(String),
    /// Download this file from the linked station's library.
    LibFetch(String),
    /// Send a Station ID beacon now.
    BeaconNow,
    MonitorVolume(f32),
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
    /// End-to-end encryption on this link.
    pub e2e: E2eView,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum E2eView {
    /// Encryption is off in Settings.
    #[default]
    Off,
    /// Key exchange in progress; chat and files wait for it.
    Negotiating,
    /// Sealed with ChaCha20-Poly1305. Compare `sas` with the other operator.
    Encrypted { sas: String },
    /// Prefer was set, but this link is plain text.
    Plain { why: String },
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
    pub rig_desc: String,
    pub rig_caps: Option<crate::rig::RigCaps>,
    pub rig_ext: RigExt,
    /// The speaker monitor is playing.
    pub monitor_on: bool,
    pub depot: crate::depot::Depot,
    pub away: bool,
    /// Mail news to show: (station, what, since).
    pub mail_alerts: Vec<MailAlert>,
    /// A QSY in force: where we came from and where the link is now.
    pub qsy: Option<QsyView>,
    /// QSY in progress or just refused, for the link card.
    pub qsy_note: Option<String>,
    /// The linked station's library, as last listed.
    pub library: Option<LibView>,
    /// The schedule slot in charge, as text.
    pub schedule_now: Option<String>,
}

/// Radio controls beyond the basics, polled while their window is open,
/// plus transmit meters.
#[derive(Clone, Debug, Default)]
pub struct RigExt {
    pub levels: std::collections::BTreeMap<String, f32>,
    pub funcs: std::collections::BTreeMap<String, bool>,
    pub vfo: String,
    pub split: bool,
    pub rit: i32,
    pub xit: i32,
    /// SWR now (while transmitting) and the peak of the last transmission.
    pub swr: Option<f32>,
    pub swr_last_tx: Option<f32>,
    pub alc: Option<f32>,
    pub power_out: Option<f32>,
    /// What the tuner is doing ("Tuning…", "Tuned on 40m at 14:02").
    pub tuner_note: String,
    pub tune_pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct WideView {
    pub on: bool,
    pub auto: bool,
    pub note: String,
    /// (caller dial as the SDR sees it, score, seconds since last heard)
    pub hits: Vec<(f64, f64, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QsyView {
    pub home_hz: f64,
    pub at_hz: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibView {
    pub path: String,
    pub entries: Vec<crate::protocol::LibEntry>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MailAlert {
    pub call: String,
    /// We hold mail for them (true), or they hold mail for us (false).
    pub ours: bool,
    pub count: usize,
    pub at: Instant,
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
    /// Retuned onto a whistle the scan found (`coarse` Hz off the channel):
    /// measure it finely. `again`: this is the second listen.
    Verify { target: f64, from: u64, coarse: f64, again: bool },
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
    /// The fast scanning detector (see whistle::TrackDetector).
    tracker: TrackDetector,
    spectrum: AudioSpectrum,
    spec_from: u64,
    bw: Bandwidth,
    // link
    peer: Option<String>,
    dec: LinkDecoder,
    /// Security layer for the current link (dropped, keys and all, when it ends).
    e2e: Option<Channel>,
    /// Handshake packets: always sent at once, in the clear.
    ctl_queue: VecDeque<Vec<u8>>,
    /// Packets that arrived before Mercury's CONNECTED line.
    early: Vec<Vec<u8>>,
    /// Application packets, waiting for the modem (and for keys).
    hi_queue: VecDeque<Link>,
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
    rigctld: Option<crate::rigsetup::RigctldProcess>,
    /// Built-in rigctld server sharing a direct-CAT radio with Mercury.
    rig_server: Option<crate::rigserve::RigServer>,
    rig_caps: crate::rig::RigCaps,
    rig_watch: bool,
    last_ext_poll: Instant,
    tx_was: bool,
    swr_peak: f32,
    pending_tune: bool,
    last_autotune: Option<(Instant, Option<&'static str>)>,
    monitor: Option<Arc<Mutex<audio::Monitor>>>,
    mon_from: u64,
    /// The real-Mercury demo (its modems, band and K7BOT), while running.
    real_demo: Option<crate::realdemo::RealDemo>,
    depot: crate::depot::Depot,
    away: bool,
    last_beacon: Instant,
    last_link_activity: Instant,
    /// Why we connected automatically (deliver or collect), if we did.
    link_auto: Option<bool>,
    /// Per-station cooldown for notices and automatic connects.
    auto_cooldown: HashMap<String, Instant>,
    /// QSY in force: (home dial, dial now).
    qsy: Option<(f64, f64)>,
    /// Our QSY request awaiting an answer: (id, dial).
    qsy_out: Option<(u64, f64)>,
    /// Agreed QSY: move to this dial once the answer has crossed.
    qsy_move: Option<(f64, Instant, bool)>,
    /// Tuned the antenna since arriving (once per QSY).
    qsy_tuned: bool,
    qsy_tune_now: bool,
    /// SWR stayed high on the QSY frequency: end the link, go home.
    qsy_fail: bool,
    /// Library downloads we asked for (their offers are accepted at once).
    wanted: std::collections::HashSet<u64>,
    last_sched: Option<Instant>,
    sched_slot: Option<usize>,
    /// Roundtable host: the speed to switch to once the announcement is out.
    pending_speed: Option<(&'static Mode, Instant)>,
    last_query: Instant,
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
            tracker: TrackDetector::new(bw.whistle_profile()),
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
            e2e: None,
            ctl_queue: VecDeque::new(),
            early: vec![],
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
            rigctld: None,
            rig_server: None,
            rig_caps: Default::default(),
            rig_watch: false,
            last_ext_poll: Instant::now(),
            tx_was: false,
            swr_peak: 0.0,
            pending_tune: false,
            last_autotune: None,
            monitor: None,
            mon_from: 0,
            real_demo: None,
            // Demo keeps its own mailbox; tests can point anywhere.
            depot: crate::depot::Depot::load(std::env::var_os("WHISTLESTOP_DEPOT").map(PathBuf::from).or_else(|| (!cfg.depot_file.is_empty()).then(|| PathBuf::from(&cfg.depot_file))).unwrap_or_else(|| {
                if cfg!(test) {
                    std::env::temp_dir().join(format!("ws-depot-{}-{}.json", std::process::id(), cfg.callsign))
                } else if cfg.demo {
                    crate::depot::default_path().with_file_name("depot-demo.json")
                } else {
                    crate::depot::default_path()
                }
            })),
            away: false,
            last_beacon: Instant::now(),
            last_link_activity: Instant::now(),
            link_auto: None,
            auto_cooldown: HashMap::new(),
            qsy: None,
            qsy_out: None,
            qsy_move: None,
            qsy_tuned: false,
            qsy_tune_now: false,
            qsy_fail: false,
            wanted: Default::default(),
            last_sched: None,
            sched_slot: None,
            pending_speed: None,
            last_query: Instant::now(),
        };
        {
            let mut g = st.lock().unwrap();
            *g = State { running: true, demo: cfg.demo, mycall: cfg.callsign.to_uppercase(), bandwidth: Some(bw), ..Default::default() };
        }
        // Demo on real Mercury when it's installed; otherwise all simulated.
        let real = if cfg.demo && cfg.demo_real_mercury {
            match crate::realdemo::find_mercury(&cfg.mercury_bin) {
                Some(bin) => match crate::realdemo::start(&bin).await {
                    Ok(d) => Some(d),
                    Err(e) => {
                        s.log(format!("⚠ Demo couldn't start real Mercury ({e}); using the simulated modem"));
                        None
                    }
                },
                None => None,
            }
        } else {
            None
        };
        let real_ports = real.as_ref().map(|d| (d.rig_addr.clone(), d.audio_addr.clone(), d.mercury_port, d.bcast_port));
        s.log(match (&real, cfg.demo) {
            (Some(_), _) => "Whistlestop starting in Demo mode: two real Mercury modems on a simulated 20 m band, you and K7BOT".to_string(),
            (None, true) => "Whistlestop starting in Demo mode (simulated radio, band and Mercury; install Mercury for a real-Mercury demo)".to_string(),
            _ => "Whistlestop starting".to_string(),
        });

        // Radio.
        if cfg.demo && real.is_none() {
            s.rig = Some(Rig::sim(14_096_000.0, vfo.clone()));
            s.set(|g| g.rig_ok = true);
            s.set(|g| g.rig_desc = "Demo transceiver (simulated)".into());
        } else if cfg.rig_mode == crate::rigsetup::RigMode::Direct && real.is_none() && !cfg.demo {
            // Whistlestop's own CAT drivers: find the radio and its protocol.
            let cat = cfg.cat.clone();
            match tokio::task::spawn_blocking(move || crate::cat::connect(&cat)).await.map_err(|e| anyhow::anyhow!("{e}")).and_then(|r| r) {
                Ok(d) => {
                    let h = crate::cat::CatHandle::start(d);
                    let desc = h.desc.clone();
                    s.log(format!("Radio: {desc}"));
                    if cfg.cat.share_port > 0 {
                        match crate::rigserve::serve(h.clone(), cfg.cat.share_port).await {
                            Ok(srv) => {
                                s.log(format!("Radio shared for Mercury and other programs at {} (Hamlib model 2, NET rigctl)", srv.addr));
                                s.rig_server = Some(srv);
                            }
                            Err(e) => s.log(format!("⚠ {e}. Mercury won't be able to key the radio; change the sharing port in Settings.")),
                        }
                    }
                    s.rig = Some(Rig::Cat(h));
                    s.set(|g| {
                        g.rig_ok = true;
                        g.rig_desc = desc.clone();
                    });
                }
                Err(e) => s.log(format!("⚠ Radio (direct CAT): {e:#}")),
            }
        } else {
            let mut addr = real_ports.as_ref().map(|p| p.0.clone()).unwrap_or(cfg.rigctld.clone());
            let mut desc = if real_ports.is_some() { "Demo radio on the simulated band".to_string() } else { format!("rigctld at {addr}") };
            if cfg.rig_mode == crate::rigsetup::RigMode::Managed && !cfg.demo {
                match crate::rigsetup::start_rigctld(&cfg.managed_rig) {
                    Ok(p) => {
                        addr = p.addr.clone();
                        desc = p.description.clone();
                        s.log(format!("Radio: started rigctld for {} (listening on {addr}; point Mercury at it too)", p.description));
                        s.rigctld = Some(p);
                    }
                    Err(e) => s.log(format!("⚠ Radio: {e}")),
                }
            }
            // A freshly started rigctld needs a moment to open the radio.
            let mut last = None;
            for _ in 0..(if s.rigctld.is_some() { 30 } else { 1 }) {
                match RigctldClient::connect(&addr).await {
                    Ok(c) => {
                        s.rig = Some(Rig::Rigctld(c));
                        break;
                    }
                    Err(e) => {
                        last = Some(e);
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
            if s.rig.is_some() {
                s.set(|g| {
                    g.rig_ok = true;
                    g.rig_desc = desc.clone();
                });
                s.log(format!("Radio: {desc}"));
            } else if let Some(e) = last {
                s.log(format!("⚠ Radio: {e}. Start rigctld or RigHelm's rigctld server, or let Whistlestop run it (Settings → Radio)."));
            }
        }
        if let Some(r) = s.rig.as_mut() {
            // What this radio can do decides which controls are offered.
            let caps = tokio::time::timeout(Duration::from_secs(4), r.caps()).await.ok().and_then(|c| c.ok()).unwrap_or_default();
            if !caps.model.is_empty() {
                s.log(format!("Radio reports: {} ({} adjustable levels, {} functions{})", caps.model, caps.set_levels.len(), caps.set_funcs.len(), if caps.has_tuner() { ", antenna tuner" } else { "" }));
            }
            s.set(|g| g.rig_caps = Some(caps.clone()));
            s.rig_caps = caps;
        }

        // Audio.
        let audio_in = real_ports.as_ref().map(|p| format!("ether:{}", p.1)).unwrap_or(cfg.audio_in.clone());
        if cfg.demo && real.is_none() {
            let band = SimBand::new(
                vfo.clone(),
                vec![
                    SimSignal::Whistler { rf_hz: 14_103_150.0, snr_db: -2.0, whistle_s: 10.0, cycle_s: 30.0, start_s: 4.0 },
                    SimSignal::Carrier { rf_hz: 14_101_900.0, snr_db: 8.0 },
                    SimSignal::Ft8 { rf_hz: 14_097_350.0, snr_db: 0.0 },
                ],
            );
            audio::run_sim(band, rx.clone(), sim_stop.clone());
            s.set(|g| g.audio_ok = true);
        } else {
            if let Some(addr) = audio_in.strip_prefix("ether:") {
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
        let (host, port, bport) = if let Some(p) = &real_ports {
            ("127.0.0.1".to_string(), p.2, p.3)
        } else if cfg.demo {
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

        s.real_demo = real;
        let d = s.depot.clone();
        s.set(|g| g.depot = d);
        if cfg.monitor {
            s.set_monitor(true);
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

    /// Transmit meters, the SWR peak of each transmission, and whether the
    /// antenna tuner should be asked to tune afterwards.
    async fn poll_meters(&mut self) {
        let tx = self.st.lock().unwrap().tx;
        let caps = self.rig_caps.clone();
        let Some(r) = self.rig.as_mut() else { return };
        if tx {
            let swr = if caps.can_get_level("SWR") { r.get_level("SWR").await.ok().filter(|v| *v >= 1.0) } else { None };
            let alc = if caps.can_get_level("ALC") { r.get_level("ALC").await.ok() } else { None };
            let po = if caps.can_get_level("RFPOWER_METER") { r.get_level("RFPOWER_METER").await.ok() } else { None };
            if let Some(v) = swr {
                self.swr_peak = self.swr_peak.max(v);
            }
            self.set(|g| {
                g.rig_ext.swr = swr;
                g.rig_ext.alc = alc;
                g.rig_ext.power_out = po;
            });
        } else if self.tx_was {
            // A transmission just ended.
            let peak = self.swr_peak;
            self.swr_peak = 0.0;
            if peak >= 1.0 {
                let band = bandplan::band_of(self.dial());
                self.set(|g| g.rig_ext.swr_last_tx = Some(peak));
                if peak > self.cfg.swr_limit {
                    let recently = self.last_autotune.is_some_and(|(at, b)| b == band && at.elapsed() < Duration::from_secs(120));
                    let can = caps.has_tuner();
                    self.log(format!(
                        "⚠ High SWR: {peak:.1} on {} (limit {:.1}){}",
                        band.unwrap_or("this frequency"),
                        self.cfg.swr_limit,
                        if self.cfg.auto_tune && can && !recently {
                            "; the antenna tuner will tune when the band is quiet"
                        } else if !can {
                            "; this radio has no tuner control"
                        } else {
                            ""
                        }
                    ));
                    if self.qsy.is_some() {
                        // Moved for a QSY: tune once (if auto-tune is on);
                        // if it's still high, end the link so both go home.
                        if self.cfg.auto_tune && can && !self.qsy_tuned {
                            self.qsy_tuned = true;
                            self.qsy_tune_now = true;
                        } else if self.cfg.qsy_swr_home {
                            self.qsy_fail = true;
                        }
                    } else if self.cfg.auto_tune && can && !recently {
                        self.pending_tune = true;
                        self.set(|g| g.rig_ext.tune_pending = true);
                    }
                }
            }
        }
        if !tx && self.tx_was {
            self.set(|g| {
                g.rig_ext.swr = None;
                g.rig_ext.alc = None;
                g.rig_ext.power_out = None;
            });
        }
        self.tx_was = tx;
    }

    /// Everything the Radio Controls window shows that the radio reports.
    async fn poll_ext(&mut self) {
        const LEVELS: [&str; 10] = ["AF", "RF", "SQL", "MICGAIN", "COMP", "NR", "AGC", "PREAMP", "ATT", "RFPOWER"];
        const FUNCS: [&str; 7] = ["NB", "NR", "ANF", "COMP", "VOX", "MON", "TUNER"];
        let caps = self.rig_caps.clone();
        let Some(r) = self.rig.as_mut() else { return };
        let mut ext = self.st.lock().unwrap().rig_ext.clone();
        for l in LEVELS.iter().filter(|l| caps.can_get_level(l)) {
            if let Ok(v) = r.get_level(l).await {
                ext.levels.insert(l.to_string(), v);
            }
        }
        for f in FUNCS.iter().filter(|f| caps.get_funcs.iter().any(|x| x == *f)) {
            if let Ok(v) = r.get_func(f).await {
                ext.funcs.insert(f.to_string(), v);
            }
        }
        if let Ok(v) = r.vfo().await {
            ext.vfo = v;
        }
        if let Ok(v) = r.split().await {
            ext.split = v;
        }
        if let Ok(v) = r.offset(false).await {
            ext.rit = v;
        }
        if let Ok(v) = r.offset(true).await {
            ext.xit = v;
        }
        let (swr, swr_last, alc, po, note, pend) = {
            let g = self.st.lock().unwrap();
            (g.rig_ext.swr, g.rig_ext.swr_last_tx, g.rig_ext.alc, g.rig_ext.power_out, g.rig_ext.tuner_note.clone(), g.rig_ext.tune_pending)
        };
        ext.swr = swr;
        ext.swr_last_tx = swr_last;
        ext.alc = alc;
        ext.power_out = po;
        ext.tuner_note = note;
        ext.tune_pending = pend;
        self.set(|g| g.rig_ext = ext);
    }

    /// Ask the radio's antenna tuner to tune: switch it in, then start a
    /// tune cycle (the radio sends its own short carrier).
    async fn tune_antenna(&mut self, why: &str) -> anyhow::Result<()> {
        let caps = self.rig_caps.clone();
        let band = bandplan::band_of(self.dial());
        let Some(r) = self.rig.as_mut() else { anyhow::bail!("no radio") };
        if caps.can_func("TUNER") {
            r.set_func("TUNER", true).await?;
        }
        if caps.can_op("TUNE") {
            r.vfo_op("TUNE").await?;
        } else if !caps.can_func("TUNER") {
            anyhow::bail!("this radio doesn't offer tuner control through Hamlib");
        }
        self.last_autotune = Some((Instant::now(), band));
        self.pending_tune = false;
        let when = Clock::now().hhmm();
        self.set(|g| {
            g.rig_ext.tune_pending = false;
            g.rig_ext.tuner_note = format!("Tuned on {} at {when}", band.unwrap_or("this frequency"));
        });
        self.log(format!("Antenna tuner: tune started ({why})"));
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
                self.poll_meters().await;
                if self.rig_watch && self.last_ext_poll.elapsed() > Duration::from_millis(1500) {
                    self.last_ext_poll = Instant::now();
                    self.poll_ext().await;
                }
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
                self.tracker = TrackDetector::new(bw.whistle_profile());
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
                if let Some(m) = &self.monitor {
                    m.lock().unwrap().clear_tx();
                }
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
                    self.hi_queue.push_back(Link::Bye { text: "73".into() });
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
                self.hi_queue.push_back(Link::Chat { id, text: text.clone() });
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
                self.hi_queue.push_back(Link::Cancel { fid });
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
            Cmd::RigLevel(name, v) => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_level(&name, v).await?;
                }
                self.set(|g| {
                    g.rig_ext.levels.insert(name, v);
                });
            }
            Cmd::RigFunc(name, on) => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_func(&name, on).await?;
                }
                self.set(|g| {
                    g.rig_ext.funcs.insert(name, on);
                });
            }
            Cmd::RigOp(op) => {
                if let Some(r) = self.rig.as_mut() {
                    r.vfo_op(&op).await?;
                }
                self.last_ext_poll = Instant::now() - Duration::from_secs(10);
            }
            Cmd::RigVfo(v) => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_vfo(&v).await?;
                }
                self.set(|g| g.rig_ext.vfo = v);
            }
            Cmd::RigSplit(on) => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_split(on).await?;
                }
                self.set(|g| g.rig_ext.split = on);
            }
            Cmd::RigOffset { xit, hz } => {
                if let Some(r) = self.rig.as_mut() {
                    r.set_offset(xit, hz).await?;
                }
                self.set(|g| if xit { g.rig_ext.xit = hz } else { g.rig_ext.rit = hz });
            }
            Cmd::RigPassband(hz) => {
                let mode = self.st.lock().unwrap().rig.mode.clone();
                if let Some(r) = self.rig.as_mut() {
                    r.set_mode(if mode.is_empty() { "PKTUSB" } else { &mode }, hz).await?;
                }
            }
            Cmd::RigWatch(on) => {
                self.rig_watch = on;
                self.last_ext_poll = Instant::now() - Duration::from_secs(10);
            }
            Cmd::TuneAntenna => self.tune_antenna("by hand").await?,
            Cmd::SetAway(on) => {
                self.away = on;
                self.set(|g| g.away = on);
                self.log(if on { "🚪 Away: links get your away message; what they type waits in your Depot".into() } else { "Back at the radio".into() });
            }
            Cmd::DepotPost { to, via, text } => {
                let to = to.trim().to_uppercase();
                if to.is_empty() || text.trim().is_empty() {
                    anyhow::bail!("write a callsign and a message");
                }
                let me = self.mycall();
                let via = via.map(|v| v.trim().to_uppercase()).filter(|v| !v.is_empty() && *v != to);
                self.depot.post_via(&me, &to, via.as_deref(), text.trim());
                self.log(match &via {
                    Some(v) => format!("📮 Mail for {to} waits in Departures, to be handed to {v}"),
                    None => format!("📮 Mail for {to} waits in Departures"),
                });
                // Linked with them (or the relay) right now: send it at once.
                if let Some(p) = self.peer.clone()
                    && (p == to || via.as_deref() == Some(p.as_str())) {
                        let ids: Vec<u64> = self.depot.departures.iter().rev().take(1).map(|m| m.id).collect();
                        let mail: Vec<Link> = self.depot.departures.iter().filter(|m| ids.contains(&m.id)).map(|m| Link::Mail { id: m.id, from: m.from.clone(), to: m.to.clone(), created: m.created, hops: m.hops, text: m.text.clone() }).collect();
                        self.hi_queue.extend(mail);
                        self.pump().await?;
                    }
                self.sync_depot();
            }
            Cmd::DepotDelete(id) => {
                self.depot.delete(id);
                self.sync_depot();
            }
            Cmd::DepotRead(id) => {
                self.depot.mark_read(id);
                self.sync_depot();
            }
            Cmd::DepotConnect(call) => {
                let call = call.trim().to_uppercase();
                if self.peer.is_some() {
                    anyhow::bail!("already linked");
                }
                let me = self.mycall();
                self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
                self.set(|g| g.calling = Some(call.clone()));
                self.log(format!("Connecting to {call} for Depot mail…"));
            }
            Cmd::BeaconNow => self.send_beacon("by hand")?,
            Cmd::Qsy(hz) => {
                let Some(peer) = self.peer.clone() else { anyhow::bail!("not linked") };
                if bandplan::band_of(hz).is_none() {
                    anyhow::bail!("{:.3} kHz isn't in an amateur band", hz / 1e3);
                }
                let id = crate::depot::new_id();
                self.qsy_out = Some((id, hz));
                self.hi_queue.push_back(Link::Qsy { id, dial_hz: hz.round() as u64, why: String::new() });
                self.log(format!("↔ Asking {peer} to QSY to {:.3} kHz", hz / 1e3));
                self.set(|g| g.qsy_note = Some(format!("Asked {peer} to move to {:.3} kHz…", hz / 1e3)));
                self.pump().await?;
            }
            Cmd::LibList(path) => {
                if self.peer.is_none() {
                    anyhow::bail!("not linked");
                }
                self.hi_queue.push_back(Link::ListReq { id: crate::depot::new_id(), path });
                self.pump().await?;
            }
            Cmd::LibFetch(path) => {
                let Some(peer) = self.peer.clone() else { anyhow::bail!("not linked") };
                let id = crate::depot::new_id();
                self.wanted.insert(id);
                self.hi_queue.push_back(Link::Fetch { id, path: path.clone() });
                self.log(format!("📚 Asking {peer} for {path}"));
                self.pump().await?;
            }
            Cmd::Monitor(on) => {
                self.cfg.monitor = on;
                self.set_monitor(on);
            }
            Cmd::MonitorVolume(v) => {
                self.cfg.monitor_volume = v;
                if let Some(m) = &self.monitor {
                    m.lock().unwrap().set_volume(v);
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
        // Neighbouring channels overlap so every caller is within reach.
        self.cfg.step_hz.min(1.8 * self.whistle_params().scan_reach_hz())
    }

    /// The fine offset from a matched-filter measurement, choosing between
    /// it and its alias the one that agrees with the scan's coarse offset.
    fn fine_offset(&self, coarse: f64, measured: f64) -> f64 {
        let alias = self.whistle_params().alias_of(measured);
        if (alias - coarse).abs() < (measured - coarse).abs() { alias } else { measured }
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
        // A short dwell on every channel; a promising one gets up to 2 s;
        // a found whistle is measured over three of its 0.4 s patterns.
        let dwell = self.cfg.dwell_s;
        let n_dwell = (dwell * FS as f64) as usize;
        let hold_s = dwell.max(2.0);
        let n_hold = (hold_s * FS as f64) as usize;
        let n_verify = (3.0 * self.whistle_params().period_s() * FS as f64) as usize;
        let ch = s.channels[s.idx];
        let mut next_channel = false;
        let found = |z: f64, secs: f64| z >= TrackDetector::found(secs);
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
                    match self.tracker.scan(&x) {
                        Some(h) if found(h.z, dwell) => {
                            self.log(format!("🚂 Whistle on {:.1} kHz, {:+.0} Hz off (score {:.1}). Retuning onto it.", ch / 1e3, h.offset_hz, h.z));
                            s.phase = SearchPhase::Verify { target: ch + h.offset_hz, from: 0, coarse: h.offset_hz, again: false };
                        }
                        Some(h) if h.z >= TrackDetector::MAYBE && hold_s > dwell => s.phase = SearchPhase::Hold { from },
                        _ => next_channel = true,
                    }
                }
            }
            SearchPhase::Hold { from } => {
                let got = self.rx.lock().unwrap().range(*from, n_hold);
                if let Some(x) = got {
                    match self.tracker.scan(&x) {
                        Some(h) if found(h.z, hold_s) => {
                            self.log(format!("🚂 Whistle on {:.1} kHz, {:+.0} Hz off (score {:.1}, after listening longer). Retuning onto it.", ch / 1e3, h.offset_hz, h.z));
                            s.phase = SearchPhase::Verify { target: ch + h.offset_hz, from: 0, coarse: h.offset_hz, again: false };
                        }
                        _ => next_channel = true,
                    }
                }
            }
            SearchPhase::Verify { target, from, coarse, again } => {
                let (target, from, coarse, again) = (*target, *from, *coarse, *again);
                let got = if from == 0 { None } else { self.rx.lock().unwrap().range(from, n_verify) };
                if from == 0 {
                    self.tune(target).await?;
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    s.phase = SearchPhase::Verify { target, from: audio::now_count(&self.rx).max(1), coarse, again };
                } else if let Some(x) = got {
                    // Now within a few tens of hertz: the matched filter
                    // measures the rest to about a hertz.
                    let wide = s.scope == WIDE_SCOPE;
                    // The scan detector's estimate (unambiguous, good to
                    // about 10 Hz) decides; the matched filter refines it
                    // only when the two agree. It copes with a strong signal
                    // inside the sweep and an SDR hand-off's crystal error.
                    let secs = x.len() as f64 / FS as f64;
                    let reach = if wide { self.whistle_params().scan_reach_hz() } else { 150.0 };
                    let coarse_now = self.tracker.scan(&x).filter(|h| h.z >= TrackDetector::found(secs) && h.offset_hz.abs() <= reach);
                    let fine = self.detector.detect(&x).map(|d| d.offset_hz);
                    let v = match (coarse_now, fine) {
                        (Some(h), Some(f)) => {
                            let f = self.fine_offset(h.offset_hz, f);
                            Some(if (f - h.offset_hz).abs() <= 25.0 { f } else { h.offset_hz })
                        }
                        (Some(h), None) => Some(h.offset_hz),
                        (None, _) => self.detector.detect_near(&x, 60.0).map(|d| d.offset_hz),
                    };
                    let heard_something = coarse_now.is_some() || self.tracker.scan(&x).is_some_and(|h| h.z >= TrackDetector::MAYBE);
                    match v {
                        Some(off) => {
                            let fine = target + off;
                            if (fine - target).abs() >= 2.0 {
                                self.tune(fine).await?;
                            }
                            self.log(format!(
                                "Locked on {:.3} kHz (first estimate {coarse:+.0} Hz, fine {:+.1} Hz). Waiting for the CQ…",
                                fine / 1e3,
                                fine - target
                            ));
                            s.phase = SearchPhase::WaitCq { until: Instant::now() + Duration::from_secs(45) };
                        }
                        _ if !again && heard_something => {
                            // Faded, or the pattern straddled the window: listen once more.
                            s.phase = SearchPhase::Verify { target, from: audio::now_count(&self.rx).max(1), coarse, again: true };
                        }
                        _ => {
                            self.log("Whistle not confirmed after retuning; resuming search".into());
                            next_channel = true;
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
        let mon = if self.cfg.demo { self.monitor.clone() } else { None };
        self.ptt(true).await;
        tokio::spawn(async move {
            match out {
                Some(o) => {
                    let audio = whistle(secs, &params.with_fs(o.rate));
                    o.play(&audio).await;
                }
                None => {
                    // Demo: nothing is transmitted, but you can hear it.
                    if let Some(m) = mon {
                        let m = m.lock().unwrap();
                        m.push_tx(&whistle(secs, &params.with_fs(m.rate)));
                    }
                    tokio::time::sleep(Duration::from_secs_f64(secs)).await
                }
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
                if self.cfg.demo && self.real_demo.is_none()
                    && let Some(m) = &self.monitor {
                        let m = m.lock().unwrap();
                        let mut seed = 0x5eed_u64 ^ Instant::now().elapsed().as_nanos() as u64 ^ 0x9e37;
                        m.push_tx(&audio::ofdm_burst(m.rate as f64, 3.1, &mut seed));
                    }
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
                let bodies = self.dec.feed_bodies(&d)?;
                if self.peer.is_none() {
                    // Mercury's data port can beat the CONNECTED line on the
                    // control port: keep these for when it arrives.
                    self.early.extend(bodies);
                    self.early.truncate(64);
                } else {
                    self.on_bodies(bodies).await?;
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

    /// Link packets from the peer, through the security layer.
    async fn on_bodies(&mut self, bodies: Vec<Vec<u8>>) -> anyhow::Result<()> {
        for body in bodies {
            // None: the link is being dropped; ignore the rest.
            let Some(ch) = self.e2e.as_mut() else { break };
            match ch.on_body(&body) {
                Ok(step) => {
                    self.ctl_queue.extend(step.send);
                    if let Some(n) = step.notice {
                        self.on_e2e_notice(n);
                    }
                    for p in step.app {
                        self.on_link(p).await?;
                    }
                }
                Err(e) => {
                    self.e2e_drop(e.to_string()).await?;
                    break;
                }
            }
        }
        self.pump().await
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
                let hello = Link::Hello {
                    version: VERSION,
                    caps: CAP_RESUME | CAP_ZIP,
                    name: self.cfg.name.clone(),
                    grid: self.cfg.grid.clone(),
                    app: format!("Whistlestop {}", env!("CARGO_PKG_VERSION")),
                };
                let policy = self.cfg.e2e;
                let (ch, first) = Channel::open(policy, hello, Instant::now());
                self.e2e = Some(ch);
                self.ctl_queue = first.into();
                self.hi_queue.clear();
                if policy != e2e::Policy::Off {
                    self.set(|g| {
                        if let Some(l) = g.link.as_mut() {
                            l.e2e = E2eView::Negotiating;
                        }
                    });
                    let m = format!("Setting up encryption ({}); chat and files wait for it", policy.label());
                    self.log(m.clone());
                    self.system_line(m);
                }
                // Re-offer files queued before the link came up.
                for o in self.outgoing.iter() {
                    self.hi_queue.push_back(o.offer());
                }
                // Depot mail for this station goes across now.
                self.last_link_activity = Instant::now();
                self.queue_mail_for(&peer);
                let they_called = l.split_whitespace().nth(1).is_some_and(|c| !c.eq_ignore_ascii_case(&self.mycall()));
                if self.away && they_called {
                    let text = format!("[{}] {}", self.mycall(), self.cfg.away_message);
                    self.hi_queue.push_back(Link::Chat { id: 0, text: text.clone() });
                    self.system_line(format!("(away) sent: {text}"));
                }
                self.pump().await?;
                let early = std::mem::take(&mut self.early);
                self.on_bodies(early).await?;
            }
            "DISCONNECTED" => {
                self.link_auto = None;
                // Forget the session keys (they wipe themselves on drop).
                self.e2e = None;
                self.ctl_queue.clear();
                self.early.clear();
                self.dec = LinkDecoder::default();
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
                self.qsy_out = None;
                self.qsy_move = None;
                self.wanted.clear();
                self.set(|g| {
                    g.qsy_note = None;
                    g.library = None;
                });
                if let Some((home, at)) = self.qsy.take() {
                    self.tune(home).await?;
                    self.log(format!("↔ QSY over: back from {:.3} to {:.3} kHz", at / 1e3, home / 1e3));
                    self.set(|g| g.qsy = None);
                }
                for o in self.outgoing.iter_mut() {
                    o.next = None; // re-offer (with resume) on the next link
                }
                self.hi_queue.clear();
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
                self.heard_station(&call).await?;
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
                if let Some(r) = self.rig.as_mut().filter(|r| r.is_sim()) {
                    let _ = r.ptt(on).await;
                }
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
            // Mercury answers VERSION as VARA does ("VARA version 4.9.0
            // registered") so VARA programs accept it.
            _ if l.to_ascii_lowercase().contains("mercury") || word == "VERSION" || (word == "VARA" && l.to_ascii_lowercase().contains("version")) => {
                let v = version_text(&l);
                self.set(|g| g.modem_version = Some(v));
            }
            _ => {}
        }
        Ok(())
    }

    fn system_line(&self, text: String) {
        self.set(|g| {
            g.chat.push(ChatLine { at: Clock::now(), who: Who::System, from: String::new(), text, id: 0, delivered: true })
        });
    }

    fn on_e2e_notice(&mut self, n: Notice) {
        let peer = self.peer.clone().unwrap_or_default();
        let (view, line) = match n {
            Notice::Secure { sas } => (
                E2eView::Encrypted { sas: sas.clone() },
                format!(
                    "🔒 Encrypted with {peer} (X25519 + ChaCha20-Poly1305, keys for this link only). \
                     Code {sas}: read it to {peer} by voice or another channel. If theirs matches, \
                     no one is in the middle."
                ),
            ),
            Notice::Plain { why } => (E2eView::Plain { why: why.clone() }, format!("🔓 Not encrypted: {why}")),
        };
        self.log(line.clone());
        self.system_line(line);
        self.set(|g| {
            if let Some(l) = g.link.as_mut() {
                l.e2e = view;
            }
        });
    }

    /// The link can't be (or may no longer be) trusted: say why and hang up.
    /// Nothing more goes out except a plain BYE carrying the reason.
    async fn e2e_drop(&mut self, why: String) -> anyhow::Result<()> {
        self.e2e = None;
        self.hi_queue.clear();
        self.log(format!("⚠ Encryption: {why}. Disconnecting."));
        self.system_line(format!("⚠ Encryption: {why}. Disconnecting."));
        self.ctl_queue.push_back(Link::Bye { text: "encryption problem, disconnecting".into() }.encode());
        self.pump().await?;
        self.tnc()?.command("DISCONNECT").await?;
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
            Link::Qsy { id, dial_hz, why } => {
                let hz = dial_hz as f64;
                let why = if why.is_empty() { String::new() } else { format!(" ({why})") };
                match self.qsy_verdict(hz) {
                    Ok(()) => {
                        self.hi_queue.push_back(Link::QsyAnswer { id, accept: true, reason: String::new() });
                        self.qsy_move = Some((hz, Instant::now(), false));
                        self.log(format!("↔ {peer} asked to QSY to {:.3} kHz{why}: agreed", hz / 1e3));
                        self.system_line(format!("↔ {peer} asked to move to {:.3} kHz: agreed. Back here when the link ends.", hz / 1e3));
                    }
                    Err(reason) => {
                        self.hi_queue.push_back(Link::QsyAnswer { id, accept: false, reason: reason.clone() });
                        self.log(format!("↔ {peer} asked to QSY to {:.3} kHz{why}: declined ({reason})", hz / 1e3));
                        self.system_line(format!("↔ {peer} asked to move to {:.3} kHz: declined ({reason})", hz / 1e3));
                    }
                }
            }
            Link::QsyAnswer { id, accept, reason } => {
                if let Some((want, hz)) = self.qsy_out
                    && want == id
                {
                    self.qsy_out = None;
                    if accept {
                        self.system_line(format!("↔ {peer} agreed: moving to {:.3} kHz", hz / 1e3));
                        self.qsy_move = Some((hz, Instant::now(), true));
                    } else {
                        self.system_line(format!("↔ {peer} declined the QSY: {reason}"));
                        self.set(|g| g.qsy_note = None);
                    }
                }
            }
            Link::ListReq { id, path } => {
                let ans = self.library_answer(id, &path);
                if let Link::ListAns { note, entries, .. } = &ans {
                    self.log(format!("📚 {peer} browsed the library ({}): {}", if path.is_empty() { "top" } else { &path }, if note.is_empty() { format!("{} entries", entries.len()) } else { note.clone() }));
                }
                self.hi_queue.push_back(ans);
            }
            Link::ListAns { path, entries, note, .. } => self.set(|g| g.library = Some(LibView { path, entries, note })),
            Link::Fetch { id, path } => match self.library_file(&path) {
                Ok((name, data)) => {
                    self.log(format!("📚 Sending {path} from the library to {peer}"));
                    self.hi_queue.push_back(Link::FetchAnswer { id, ok: true, reason: String::new() });
                    let len = data.len() as u64;
                    self.offer_data(id, name, data, false, 1, len).await?;
                }
                Err(e) => {
                    self.log(format!("📚 {peer} asked for {path}: refused ({e})"));
                    self.hi_queue.push_back(Link::FetchAnswer { id, ok: false, reason: e.to_string() });
                }
            },
            Link::FetchAnswer { id, ok, reason } => {
                if !ok {
                    self.wanted.remove(&id);
                    self.system_line(format!("📚 {peer} couldn't send that: {reason}"));
                }
            }
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
                self.last_link_activity = Instant::now();
                if self.away && !peer.is_empty() {
                    let m = crate::depot::Mail { id: crate::depot::new_id(), from: peer.clone(), to: self.mycall(), created: crate::depot::now_s(), text: text.clone(), read: false, delivered_at: None, via: None, hops: 0, handed: None };
                    self.depot.arrive(m);
                    self.sync_depot();
                }
                self.hi_queue.push_back(Link::Received { id });
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
                // A library file we asked for: take it.
                if self.wanted.remove(&fid) {
                    self.answer_offer(fid, true).await?;
                }
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
                            self.hi_queue.push_back(Link::Done { fid, ok: true });
                            self.log(format!("✓ Received {name} → {}", path.display()));
                            self.xfer(fid, |t| t.state = XferState::Done(Some(path)));
                        }
                        Err(e) => {
                            self.hi_queue.push_back(Link::Done { fid, ok: false });
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
            // Handled by the e2e channel before we get here.
            Link::Mail { id, from, to, created, hops, text } => {
                self.last_link_activity = Instant::now();
                let m = crate::depot::Mail { id, from: from.clone(), to: to.clone(), created, text, read: false, delivered_at: None, via: None, hops, handed: None };
                if to.eq_ignore_ascii_case(&self.mycall()) {
                    self.hi_queue.push_back(Link::MailAck { id });
                    if self.depot.arrive(m) {
                        self.log(format!("📬 Mail from {from} arrived in your Depot"));
                        self.system_line(format!("📬 Mail from {from} is in your Depot (Arrivals)"));
                    }
                    self.set(|g| g.mail_alerts.retain(|a| !(a.call == from && !a.ours)));
                } else if self.cfg.relay {
                    // Store and forward: carry it on toward its station.
                    let fresh = self.depot.take_in_transit(crate::depot::Mail { hops: hops.saturating_add(1), ..m }, &peer);
                    let already = self.depot.transit.iter().chain(self.depot.delivered.iter()).any(|x| x.id == id);
                    if fresh || already {
                        self.hi_queue.push_back(Link::MailAck { id });
                    }
                    if fresh {
                        self.log(format!("🚚 Carrying mail from {from} for {to} (In Transit)"));
                        self.system_line(format!("🚚 {peer} handed you mail for {to}: it's In Transit until you reach {to}"));
                    } else if !already {
                        self.log(format!("Mail for {to} not taken: relayed too many times"));
                    }
                } else {
                    self.log(format!("{peer} offered mail for {to}; not carried (Settings → Automation → Carry mail for other stations is off)"));
                }
                self.sync_depot();
                self.pump().await?;
            }
            Link::MailAck { id } => {
                self.last_link_activity = Instant::now();
                if let Some(m) = self.depot.confirm(id, &peer).cloned() {
                    self.log(format!("✓ Mail delivered to {} (now in Delivered)", m.to));
                    let left = self.depot.waiting_for(&m.to).len();
                    if left == 0 {
                        self.set(|g| g.mail_alerts.retain(|a| !(a.call == m.to && a.ours)));
                    }
                }
                self.sync_depot();
            }
            Link::KeyCommit { .. } | Link::KeyX { .. } | Link::Sealed { .. } | Link::Unknown(_) => {}
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
        self.offer_data(fid, name, data, zipped, files, original).await
    }

    /// Offer data to the linked station (or as soon as one is linked).
    async fn offer_data(&mut self, fid: u64, name: String, data: Vec<u8>, zipped: bool, files: u64, original: u64) -> anyhow::Result<()> {
        let out = Outgoing::new(fid, name.clone(), data, zipped, files);
        let size = out.data.len() as u64;
        let eta = self.est.eta_s(file_wire_size(&name, size), self.bw);
        if self.peer.is_some() {
            self.hi_queue.push_back(out.offer());
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
        self.hi_queue.push_back(Link::Answer { fid, accept, resume_from: have });
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
        while let Some(p) = self.ctl_queue.pop_front() {
            self.mercury_buffer += p.len();
            self.tnc()?.send_data(&p).await?;
        }
        // Chat and files wait until the link is plain or keys are agreed.
        while let Some(p) = self.hi_queue.front() {
            let Some(bytes) = self.e2e.as_mut().and_then(|c| c.wrap(p)) else { return Ok(()) };
            self.hi_queue.pop_front();
            self.mercury_buffer += bytes.len();
            self.tnc()?.send_data(&bytes).await?;
        }
        if !self.e2e.as_ref().is_some_and(|c| c.ready()) {
            return Ok(());
        }
        while self.mercury_buffer < 2 * CHUNK {
            let Some(o) = self.outgoing.iter_mut().find(|o| o.next.is_some() && !o.finished_sending()) else { break };
            let fid = o.fid;
            let Some(chunk) = o.next_chunk() else { break };
            let sent = o.next.unwrap_or(0);
            let total = o.data.len();
            let Some(bytes) = self.e2e.as_mut().and_then(|c| c.wrap(&chunk)) else { break };
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
        match &fr.body {
            TableBody::Call { kind, table_name, .. } => {
                let what = if *kind == CALL_TABLE { format!("Roundtable \"{table_name}\"") } else { "CQ (chat)".into() };
                self.add_heard(&fr.from, what, (*kind == CALL_TABLE).then(|| table_name.clone()));
                self.heard_station(&fr.from).await?;
                return Ok(());
            }
            TableBody::Beacon { grid, away, mail_for } => {
                let me = self.mycall();
                let for_me = mail_for.iter().any(|c| c.eq_ignore_ascii_case(&me));
                let what = format!(
                    "Station ID {}{}{}",
                    grid,
                    if *away { " · away" } else { "" },
                    if mail_for.is_empty() { String::new() } else { format!(" · mail for {}", mail_for.join(" ")) }
                );
                self.add_heard(&fr.from, what, None);
                if for_me {
                    self.they_have_mail(&fr.from.clone(), 1).await?;
                }
                self.heard_station(&fr.from).await?;
                return Ok(());
            }
            TableBody::Notice { to, count } => {
                if to.eq_ignore_ascii_case(&self.mycall()) {
                    self.add_heard(&fr.from, format!("has {count} message(s) for you"), None);
                    self.they_have_mail(&fr.from.clone(), *count as usize).await?;
                }
                self.heard_station(&fr.from).await?;
                return Ok(());
            }
            _ => {}
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
            TableBody::Call { .. } | TableBody::Beacon { .. } | TableBody::Notice { .. } => {}
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
        self.step_monitor();
        self.step_wide().await?;
        self.step_automation().await?;
        self.step_qsy().await?;
        if self.pending_tune {
            let quiet = {
                let g = self.st.lock().unwrap();
                !g.tx && g.link.is_none() && g.calling.is_none()
            } && self.cq.is_none() && self.search.is_none() && self.table_queue.is_empty();
            if quiet
                && let Err(e) = self.tune_antenna("high SWR").await {
                    self.pending_tune = false;
                    self.set(|g| g.rig_ext.tune_pending = false);
                    self.log(format!("⚠ Antenna tuner: {e}"));
                }
        }
        if self.search.is_some() {
            self.step_search().await?;
        } else if self.cq.is_none() {
            self.step_watch();
        }
        self.step_cq().await?;
        self.step_table_tx().await?;

        if let Some(ch) = self.e2e.as_mut() {
            match ch.tick(Instant::now()) {
                Ok(Some(n)) => {
                    self.on_e2e_notice(n);
                    self.pump().await?;
                }
                Ok(None) => {}
                Err(e) => self.e2e_drop(e.to_string()).await?,
            }
        }

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
                self.hi_queue.push_back(Link::Report { snr_db: snr, mode_index: mi });
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
    fn sync_depot(&self) {
        let d = self.depot.clone();
        self.set(|g| g.depot = d);
    }

    fn queue_mail_for(&mut self, call: &str) {
        let mail: Vec<Link> = self
            .depot
            .waiting_for(call)
            .into_iter()
            .map(|m| Link::Mail { id: m.id, from: m.from.clone(), to: m.to.clone(), created: m.created, hops: m.hops, text: m.text.clone() })
            .collect();
        if !mail.is_empty() {
            self.log(format!("📮 Sending {} Depot message(s) to {call}", mail.len()));
        }
        self.hi_queue.extend(mail);
    }

    /// Is the radio inside an automatic-control digital segment (where an
    /// unattended station may transmit, 47 CFR 97.221(b))?
    fn auto_allowed(&self) -> bool {
        if !self.cfg.auto_in_segments_only {
            return true;
        }
        let d = self.dial();
        self.cfg.segments.iter().any(|s| d + 300.0 >= s.lo_hz && d + 2700.0 <= s.hi_hz)
    }

    fn qsy_verdict(&self, hz: f64) -> Result<(), String> {
        qsy_verdict(&self.cfg, self.qsy.map(|q| q.0).unwrap_or(self.dial()), hz)
    }

    async fn step_qsy(&mut self) -> anyhow::Result<()> {
        if let Some((hz, at, asked)) = self.qsy_move {
            // Both move a moment after the answer has crossed: the station
            // that asked, 3 s after it arrived (time for Mercury's ack);
            // the other, once Mercury has sent everything.
            let due = if asked { at.elapsed() > Duration::from_secs(3) } else { self.mercury_buffer == 0 && at.elapsed() > Duration::from_secs(2) };
            if self.peer.is_none() {
                self.qsy_move = None;
            } else if due {
                self.qsy_move = None;
                let home = self.qsy.map(|q| q.0).unwrap_or(self.dial());
                self.tune(hz).await?;
                self.qsy = Some((home, hz));
                self.qsy_tuned = false;
                self.qsy_fail = false;
                self.log(format!("↔ QSY: moved to {:.3} kHz (home {:.3} kHz)", hz / 1e3, home / 1e3));
                self.set(|g| {
                    g.qsy = Some(QsyView { home_hz: home, at_hz: hz });
                    g.qsy_note = None;
                });
            }
        }
        if self.qsy_tune_now {
            self.qsy_tune_now = false;
            if let Err(e) = self.tune_antenna("high SWR after a QSY").await {
                self.log(format!("⚠ Antenna tuner: {e}"));
                self.qsy_fail = self.cfg.qsy_swr_home;
            }
        }
        if self.qsy_fail && self.peer.is_some() {
            self.qsy_fail = false;
            self.log("⚠ SWR stays high on the QSY frequency: ending the link so both stations go home".into());
            self.hi_queue.push_back(Link::Chat { id: 0, text: "SWR is too high for me here; ending the link so we both go back. 73".into() });
            self.hi_queue.push_back(Link::Bye { text: "73".into() });
            self.pump().await?;
            self.tnc()?.command("DISCONNECT").await?;
        }
        Ok(())
    }

    fn library_answer(&self, id: u64, path: &str) -> Link {
        library_answer(&self.cfg, self.away, id, path)
    }

    fn library_file(&self, path: &str) -> anyhow::Result<(String, Vec<u8>)> {
        library_file(&self.cfg, self.away, path)
    }

    fn quiet(&self) -> bool {
        let g = self.st.lock().unwrap();
        !g.tx && g.link.is_none() && g.calling.is_none() && !g.incoming_call
    }

    fn cooled(&mut self, call: &str, secs: u64) -> bool {
        let ok = self.auto_cooldown.get(call).is_none_or(|t| t.elapsed() > Duration::from_secs(secs));
        if ok {
            self.auto_cooldown.insert(call.to_string(), Instant::now());
        }
        ok
    }

    /// We heard `call` (CQ, Station ID, a Roundtable frame…). If we hold
    /// mail for them: show it, tell them (if on), or deliver it (if away).
    async fn heard_station(&mut self, call: &str) -> anyhow::Result<()> {
        let call = call.to_uppercase();
        let n = self.depot.waiting_for(&call).len();
        if n == 0 || call == self.mycall() {
            return Ok(());
        }
        self.set(|g| {
            g.mail_alerts.retain(|a| !(a.call == call && a.ours));
            g.mail_alerts.push(MailAlert { call: call.clone(), ours: true, count: n, at: Instant::now() });
        });
        let quiet = self.quiet() && self.cq.is_none() && self.search.is_none();
        if self.away && self.cfg.auto_deliver && quiet && self.auto_allowed() && self.cooled(&format!("deliver {call}"), 900) {
            let me = self.mycall();
            self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
            self.link_auto = Some(true);
            self.set(|g| g.calling = Some(call.clone()));
            self.log(format!("🤖 Away: connecting to {call} to deliver {n} Depot message(s)"));
        } else if self.cfg.notify_mail && self.auto_allowed() && self.cooled(&format!("notice {call}"), 900) {
            let me = self.mycall();
            self.table_tx(TableFrame { table: 0, from: me, body: TableBody::Notice { to: call.clone(), count: n.min(255) as u8 } }.encode());
            self.log(format!("📣 Told {call}: {n} message(s) waiting here"));
        }
        Ok(())
    }

    /// `call` says it holds mail for us.
    async fn they_have_mail(&mut self, call: &str, count: usize) -> anyhow::Result<()> {
        let call = call.to_uppercase();
        self.set(|g| {
            g.mail_alerts.retain(|a| !(a.call == call && !a.ours));
            g.mail_alerts.push(MailAlert { call: call.clone(), ours: false, count, at: Instant::now() });
        });
        self.log(format!("📬 {call} has mail for you"));
        let quiet = self.quiet() && self.cq.is_none() && self.search.is_none();
        if self.away && self.cfg.auto_collect && quiet && self.auto_allowed() && self.cooled(&format!("collect {call}"), 900) {
            let me = self.mycall();
            self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
            self.link_auto = Some(false);
            self.set(|g| g.calling = Some(call.clone()));
            self.log(format!("🤖 Away: connecting to {call} to collect your mail"));
        }
        Ok(())
    }

    fn send_beacon(&mut self, why: &str) -> anyhow::Result<()> {
        if self.tnc.is_none() {
            anyhow::bail!("Mercury is not connected");
        }
        let me = self.mycall();
        let mut mail_for = self.depot.waiting_calls();
        // One modem frame (123 bytes on DATAC3): keep the list short.
        while (TableFrame { table: 0, from: me.clone(), body: TableBody::Beacon { grid: self.cfg.grid.clone(), away: self.away, mail_for: mail_for.clone() } }).encode().len() > 120 {
            mail_for.pop();
        }
        let f = TableFrame { table: 0, from: me, body: TableBody::Beacon { grid: self.cfg.grid.clone(), away: self.away, mail_for: mail_for.clone() } };
        self.table_tx(f.encode());
        self.last_beacon = Instant::now();
        self.log(format!(
            "📡 Station ID sent ({why}){}",
            if mail_for.is_empty() { String::new() } else { format!(": mail waiting for {}", mail_for.join(" ")) }
        ));
        Ok(())
    }

    async fn step_automation(&mut self) -> anyhow::Result<()> {
        // The schedule: where to listen at this time of day (UTC).
        if self.cfg.schedule_on && self.last_sched.is_none_or(|t| t.elapsed() > Duration::from_secs(20)) {
            self.last_sched = Some(Instant::now());
            let follow = !self.cfg.schedule_only_away || self.away;
            let slot = crate::schedule::active(&self.cfg.schedule, crate::depot::now_s()).map(|(i, s)| (i, s.clone()));
            let idle = self.quiet() && self.cq.is_none() && self.search.is_none() && self.peer.is_none() && self.qsy.is_none();
            match slot {
                Some((i, sl)) if follow => {
                    let hz = sl.khz * 1000.0;
                    if idle && (self.dial() - hz).abs() > 50.0 {
                        self.tune(hz).await?;
                        self.log(format!("🗓 Schedule: listening on {:.3} kHz ({}–{} UTC, {}){}", hz / 1e3, sl.from, sl.to, sl.days.label(), if sl.beacon { ", with Station ID" } else { "" }));
                    }
                    self.sched_slot = Some(i);
                    self.set(|g| g.schedule_now = Some(format!("{:.3} kHz until {} UTC{}", hz / 1e3, sl.to, if sl.beacon { " · Station ID" } else { "" })));
                }
                _ => {
                    self.sched_slot = None;
                    self.set(|g| g.schedule_now = None);
                }
            }
        } else if !self.cfg.schedule_on && self.sched_slot.is_some() {
            self.sched_slot = None;
            self.set(|g| g.schedule_now = None);
        }
        let sched_beacon = self.sched_slot.and_then(|i| self.cfg.schedule.get(i)).is_some_and(|s| s.beacon);
        // Station ID beacon, when due and the band is quiet.
        if (self.cfg.beacon || sched_beacon)
            && self.last_beacon.elapsed() > Duration::from_secs_f64(self.cfg.beacon_minutes.max(10.0) * 60.0)
            && self.quiet()
            && self.cq.is_none()
            && self.search.is_none()
            && self.table_queue.is_empty()
        {
            if self.auto_allowed() {
                self.send_beacon("scheduled")?;
            } else {
                self.last_beacon = Instant::now();
                self.log("Station ID skipped: outside the automatic-control segments".into());
            }
        }
        // Hang up quiet links when away, or after an automatic connect has
        // done its job, so others can get through.
        if let Some(peer) = self.peer.clone() {
            let idle = self.last_link_activity.elapsed().as_secs_f64();
            let done = match self.link_auto {
                Some(true) => self.depot.waiting_for(&peer).is_empty() && idle > 8.0,
                Some(false) => idle > 20.0,
                None => self.away && idle > self.cfg.away_idle_s,
            };
            if done && self.mercury_buffer == 0 {
                self.log(format!("Hanging up with {peer} (idle {idle:.0} s)"));
                self.link_auto = None;
                self.hi_queue.push_back(Link::Bye { text: "73".into() });
                self.pump().await?;
                self.tnc()?.command("DISCONNECT").await?;
                self.last_link_activity = Instant::now();
            }
        }
        Ok(())
    }

    fn set_monitor(&mut self, on: bool) {
        if !on {
            self.monitor = None;
            self.set(|g| g.monitor_on = false);
            return;
        }
        if self.monitor.is_some() {
            return;
        }
        match audio::Monitor::start(&self.cfg.monitor_device, self.cfg.monitor_volume) {
            Ok(m) => {
                self.mon_from = audio::now_count(&self.rx);
                self.monitor = Some(Arc::new(Mutex::new(m)));
                self.set(|g| g.monitor_on = true);
                self.log("🔊 Listening on the speakers".into());
            }
            Err(e) => {
                self.set(|g| g.monitor_on = false);
                self.log(format!("⚠ Speakers: {e}"));
            }
        }
    }

    /// Band audio to the speakers.
    fn step_monitor(&mut self) {
        let Some(m) = &self.monitor else { return };
        let x = {
            let rx = self.rx.lock().unwrap();
            if rx.count.saturating_sub(self.mon_from) > FS as u64 {
                self.mon_from = rx.count - (FS / 10) as u64; // fell behind: catch up
            }
            let n = (rx.count - self.mon_from) as usize;
            let v = rx.range(self.mon_from, n).unwrap_or_default();
            self.mon_from = rx.count;
            v
        };
        if !x.is_empty() {
            let f: Vec<f32> = x.iter().map(|v| *v as f32 * 3.0).collect();
            m.lock().unwrap().push_rx(&f);
        }
    }

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
            // The SDR's channels use the matched filter: keep each caller
            // inside its unambiguous range.
            step_hz: 1.8 * self.whistle_params().max_offset_hz(),
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
        let _ = score;
        self.search = Some(Search {
            channels: vec![dial],
            idx: 0,
            phase: SearchPhase::Verify { target: dial, from: 0, coarse: 0.0, again: false },
            scope: WIDE_SCOPE.into(),
        });
    }

    /// Parked on a frequency (not searching): listen for whistles anyway.
    fn step_watch(&mut self) {
        if !self.cfg.whistle_watch || self.last_watch.elapsed() < Duration::from_millis(1000) {
            return;
        }
        // Four whistle patterns, all heard on this frequency.
        let secs = 4.0 * self.whistle_params().period_s();
        if self.last_tune.elapsed() < Duration::from_secs_f64(secs + 0.3) {
            return;
        }
        self.last_watch = Instant::now();
        if self.st.lock().unwrap().tx || self.peer.is_some() {
            return;
        }
        let n = (secs * FS as f64) as usize;
        let x = self.rx.lock().unwrap().last(n);
        if x.len() < n {
            return;
        }
        // Found by the scan detector (unambiguous to ±1100 Hz), measured
        // by the matched filter.
        let Some(h) = self.tracker.scan(&x).filter(|h| h.z >= TrackDetector::found(secs)) else { return };
        if let Some(mut d) = self.detector.detect(&x) {
            d.offset_hz = self.fine_offset(h.offset_hz, d.offset_hz);
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

/// May a link move from `home` to `hz`? (Settings → QSY requests.)
pub fn qsy_verdict(cfg: &Config, home: f64, hz: f64) -> Result<(), String> {
    if !cfg.accept_qsy {
        return Err("QSY requests are turned off here".into());
    }
    let here = bandplan::band_of(home);
    let Some(there) = bandplan::band_of(hz) else { return Err("not an amateur band I can use".into()) };
    if cfg.qsy_same_band {
        if Some(there) != here {
            return Err(format!("I only QSY within {}", here.unwrap_or("this band")));
        }
    } else if !cfg.qsy_bands.iter().any(|b| b == there) {
        return Err(format!("{there} isn't one of my QSY bands"));
    }
    if cfg.qsy_max_khz > 0.0 && (hz - home).abs() > cfg.qsy_max_khz * 1000.0 {
        return Err(format!("more than {:.0} kHz from here", cfg.qsy_max_khz));
    }
    if cfg.qsy_segments_only && !cfg.segments.iter().any(|s| hz + 300.0 >= s.lo_hz && hz + 2700.0 <= s.hi_hz) {
        return Err("outside my digital segments".into());
    }
    Ok(())
}

fn library_root(cfg: &Config, away: bool) -> Result<PathBuf, String> {
    if !cfg.share_library || cfg.library_dir.trim().is_empty() {
        return Err("there's no file library here".into());
    }
    if cfg.library_only_away && !away {
        return Err("the library is open only while I'm away".into());
    }
    Ok(PathBuf::from(cfg.library_dir.trim()))
}

/// The answer to a library listing request, by the settings.
pub fn library_answer(cfg: &Config, away: bool, id: u64, path: &str) -> Link {
    let (entries, note) = match library_root(cfg, away) {
        Ok(root) => match crate::library::list(&root, path) {
            Ok(e) => (e, String::new()),
            Err(e) => (vec![], e.to_string()),
        },
        Err(n) => (vec![], n),
    };
    Link::ListAns { id, path: path.to_string(), entries, note }
}

/// A library file to send, by the settings (or why not).
pub fn library_file(cfg: &Config, away: bool, path: &str) -> anyhow::Result<(String, Vec<u8>)> {
    let root = library_root(cfg, away).map_err(|e| anyhow::anyhow!(e))?;
    crate::library::read(&root, path, (cfg.library_max_mb.max(0.0) * 1_048_576.0) as u64)
}

/// How to show the modem's VERSION reply.
pub fn version_text(line: &str) -> String {
    let l = line.trim().trim_start_matches("VERSION").trim();
    if let Some(rest) = l.strip_prefix("VARA version") {
        let v = rest.split_whitespace().next().unwrap_or("");
        return format!("Mercury (VARA-compatible {v})");
    }
    l.to_string()
}

#[test]
fn mercury_version_reply() {
    assert_eq!(version_text("VARA version 4.9.0 registered"), "Mercury (VARA-compatible 4.9.0)");
    assert_eq!(version_text("VERSION Mercury 1.9.23"), "Mercury 1.9.23");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Demo on two real Mercury modems (skipped when Mercury isn't
    /// installed): search finds K7BOT's real whistle, Mercury decodes its
    /// real CQFRAME, and the stations link and chat over real ARQ.
    #[test]
    fn real_mercury_demo_search_link_chat() {
        if crate::realdemo::find_mercury("").is_none() {
            return;
        }
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, callsign: "N0REAL".into(), auto_answer: true, ..Default::default() };
        cfg.sdr.enabled = false;
        cfg.sdr.wide_search = false;
        h.send(Cmd::Start(cfg));
        let wait = |what: &str, secs: u64, f: &dyn Fn(&State) -> bool| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(secs) {
                if f(&h.state.lock().unwrap()) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let g = h.state.lock().unwrap();
            let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
            panic!("timed out waiting for {what}; log:\n{}", log.join("\n"));
        };
        wait("real Mercury", 30, &|s| s.mercury_ok && s.log.iter().any(|l| l.1.contains("two real Mercury")));
        h.send(Cmd::StartSearch(SearchScope::CurrentBand));
        wait("link with K7BOT over real Mercury", 240, &|s| s.link.as_ref().is_some_and(|l| l.peer == "K7BOT"));
        let f = h.state.lock().unwrap().rig.freq_hz;
        assert!((f - crate::realdemo::BOT_DIAL).abs() <= 55.0, "locked {f}");
        wait("K7BOT's greeting", 90, &|s| s.chat.iter().any(|c| c.who == Who::Them));
        h.send(Cmd::Chat("hello over real Mercury".into()));
        wait("reply and delivery", 120, &|s| {
            s.chat.iter().any(|c| c.who == Who::Them && c.text.contains("hello over real Mercury")) && s.chat.iter().any(|c| c.who == Who::Me && c.delivered)
        });
        h.send(Cmd::Stop);
        std::thread::sleep(Duration::from_secs(1));
    }

    /// SWR watch: on 40 m the demo antenna shows SWR 2.8. With auto-tune
    /// off (the default) Whistlestop only warns; with it on, it tunes, but
    /// only once the CQ is over.
    #[test]
    fn swr_watch_and_optional_autotune() {
        for auto in [false, true] {
            let h = spawn(|| {});
            let mut cfg = Config { demo: true, demo_real_mercury: false, callsign: "N0SWR".into(), whistle_s: 3.0, cq_cycles: 2, cq_listen_s: 2.0, ..Default::default() };
            assert!(!cfg.auto_tune, "auto-tune must be off by default");
            cfg.auto_tune = auto;
            cfg.sdr.enabled = false;
            cfg.sdr.wide_search = false;
            h.send(Cmd::Start(cfg));
            let wait = |secs: u64, f: &dyn Fn(&State) -> bool| {
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(secs) {
                    if f(&h.state.lock().unwrap()) {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                false
            };
            assert!(wait(10, &|s| s.mercury_ok));
            h.send(Cmd::SetFreq(7_100_000.0));
            h.send(Cmd::CallCq { table: None });
            assert!(wait(30, &|s| s.log.iter().any(|l| l.1.contains("High SWR: 2.8 on 40m"))), "auto={auto}: no SWR warning");
            // Still calling: never tune in the middle of it.
            assert!(!h.state.lock().unwrap().log.iter().any(|l| l.1.contains("tune started")));
            h.send(Cmd::StopCq);
            let tuned = wait(8, &|s| s.log.iter().any(|l| l.1.contains("Antenna tuner: tune started (high SWR)")));
            assert_eq!(tuned, auto, "auto-tune {auto}");
            h.send(Cmd::Stop);
        }
    }

    /// Wide search: the (simulated) SDR spots the demo station anywhere in
    /// the segment, the radio tunes there and fine-locks on its own audio,
    /// hears the CQ and connects — without stepping through channels.
    #[test]
    fn demo_wide_search_finds_and_links() {
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, demo_real_mercury: false, callsign: "N0WIDE".into(), auto_answer: true, ..Default::default() };
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
        let mut cfg = Config { demo: true, demo_real_mercury: false, callsign: "N0TEST".into(), auto_answer: true, ..Default::default() };
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

    /// Depot in the demo: mail posted for K7BOT waits in Departures, goes
    /// across when linked and moves to Delivered on K7BOT's MAILACK; while
    /// away, what K7BOT types is saved to Arrivals.
    #[test]
    fn demo_depot_mail_delivered() {
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, demo_real_mercury: false, callsign: "N0MAIL".into(), ..Default::default() };
        cfg.sdr.enabled = false;
        cfg.whistle_watch = false;
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("ws-depot-{}-N0MAIL.json", std::process::id())));
        h.send(Cmd::Start(cfg));
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
        h.send(Cmd::DepotPost { to: "k7bot".into(), via: None, text: "See you on 40 m".into() });
        wait("mail in Departures", 5, &|s| s.depot.departures.len() == 1 && s.depot.departures[0].to == "K7BOT");
        h.send(Cmd::SetAway(true));
        wait("away", 5, &|s| s.away);
        h.send(Cmd::DepotConnect("K7BOT".into()));
        wait("link with K7BOT", 40, &|s| s.link.is_some());
        wait("mail delivered", 40, &|s| s.depot.departures.is_empty() && s.depot.delivered.iter().any(|m| m.to == "K7BOT" && m.handed.as_deref() == Some("K7BOT")));
        // While away, K7BOT's chat lands in Arrivals.
        wait("chat saved to Arrivals", 40, &|s| s.depot.arrivals.iter().any(|m| m.from == "K7BOT"));
        h.send(Cmd::Stop);
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("ws-depot-{}-N0MAIL.json", std::process::id())));
    }

    /// Direct CAT end to end: the engine finds a (simulated) IC-7300 over
    /// a network bridge, tunes it, and shares it on a rigctld port that a
    /// Hamlib-style client (as Mercury is) can key.
    #[test]
    fn direct_cat_controls_and_shares_the_radio() {
        let radio_port = crate::cat::sim::serve_tcp(crate::cat::sim::Kind::Icom(0x94));
        let share = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let h = spawn(|| {});
        let mut cfg = Config { demo: false, callsign: "N0CAT".into(), mercury_port: 1, mercury_bcast_port: 1, ..Default::default() };
        cfg.sdr.enabled = false;
        cfg.rig_mode = crate::rigsetup::RigMode::Direct;
        cfg.cat.port = format!("127.0.0.1:{radio_port}");
        cfg.cat.share_port = share;
        h.send(Cmd::Start(cfg));
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
        wait("radio", 15, &|s| s.rig_ok && s.rig_desc.contains("IC-7300"));
        wait("frequency read", 10, &|s| s.rig.freq_hz == 14_074_000.0);
        h.send(Cmd::SetFreq(7_074_000.0));
        wait("tuned", 10, &|s| s.rig.freq_hz == 7_074_000.0);
        // A NET rigctl client (Mercury's way) on the shared port.
        use std::io::{BufRead, BufReader, Write};
        let mut c = std::net::TcpStream::connect(("127.0.0.1", share)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut ask = |cmd: &str| {
            c.write_all(format!("{cmd}\n").as_bytes()).unwrap();
            let mut l = String::new();
            r.read_line(&mut l).unwrap();
            l.trim().to_string()
        };
        assert_eq!(ask("f"), "7074000");
        assert_eq!(ask("T 1"), "RPRT 0");
        assert_eq!(ask("t"), "1");
        wait("PTT seen by Whistlestop", 10, &|s| s.rig.ptt);
        assert_eq!(ask("T 0"), "RPRT 0");
        h.send(Cmd::Stop);
    }

    fn demo_station(call: &str, f: impl FnOnce(&mut Config)) -> (Handle, impl Fn(&Handle, &str, u64, &dyn Fn(&State) -> bool)) {
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, demo_real_mercury: false, callsign: call.into(), ..Default::default() };
        cfg.sdr.enabled = false;
        cfg.whistle_watch = false;
        cfg.download_dir = std::env::temp_dir().join(format!("ws-dl-{}-{call}", std::process::id()));
        f(&mut cfg);
        h.send(Cmd::Start(cfg));
        let wait = |h: &Handle, what: &str, secs: u64, f: &dyn Fn(&State) -> bool| {
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
        (h, wait)
    }

    /// QSY by request: K7BOT agrees, the radio moves, and goes home when
    /// the link ends; a QSY off 20 m is declined with K7BOT's reason.
    #[test]
    fn demo_qsy_there_and_back() {
        let (h, wait) = demo_station("N0QSY", |_| {});
        wait(&h, "Mercury", 10, &|s| s.mercury_ok);
        let home = h.state.lock().unwrap().rig.freq_hz;
        h.send(Cmd::Connect("K7BOT".into()));
        wait(&h, "link", 30, &|s| s.link.is_some());
        h.send(Cmd::Qsy(7_105_000.0));
        wait(&h, "refusal", 30, &|s| s.chat.iter().any(|c| c.text.contains("declined the QSY") && c.text.contains("stays on 20 m")));
        h.send(Cmd::Qsy(14_106_500.0));
        wait(&h, "moved", 30, &|s| s.rig.freq_hz == 14_106_500.0 && s.qsy.as_ref().is_some_and(|q| q.home_hz == home));
        h.send(Cmd::Disconnect);
        wait(&h, "home again", 30, &|s| s.link.is_none() && s.rig.freq_hz == home && s.qsy.is_none());
        h.send(Cmd::Stop);
    }

    /// The library: browse K7BOT's, fetch a file, and it's accepted and
    /// saved without asking (we asked for it).
    #[test]
    fn demo_library_browse_and_fetch() {
        let (h, wait) = demo_station("N0LIB", |_| {});
        wait(&h, "Mercury", 10, &|s| s.mercury_ok);
        h.send(Cmd::Connect("K7BOT".into()));
        wait(&h, "link", 30, &|s| s.link.is_some());
        h.send(Cmd::LibList(String::new()));
        wait(&h, "listing", 30, &|s| s.library.as_ref().is_some_and(|l| l.entries.iter().any(|e| e.name == "k7bot-qsl.txt")));
        h.send(Cmd::LibFetch("k7bot-qsl.txt".into()));
        wait(&h, "file saved", 60, &|s| s.transfers.iter().any(|t| t.dir == Dir::In && t.name == "k7bot-qsl.txt" && matches!(t.state, XferState::Done(_))));
        h.send(Cmd::LibFetch("nope.bin".into()));
        wait(&h, "refusal", 30, &|s| s.chat.iter().any(|c| c.text.contains("couldn't send that: no such file")));
        h.send(Cmd::Stop);
    }

    /// Our library, served: listing, sandboxing and size limit, by policy.
    #[test]
    fn library_policy() {
        let dir = std::env::temp_dir().join(format!("ws-libsrv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("small.txt"), "hi").unwrap();
        std::fs::write(dir.join("big.bin"), vec![0u8; 3 * 1_048_576]).unwrap();
        let mut cfg = Config { share_library: true, library_dir: dir.to_string_lossy().into(), library_max_mb: 1.0, ..Default::default() };
        match library_answer(&cfg, false, 1, "") {
            Link::ListAns { entries, note, .. } => assert!(note.is_empty() && entries.len() == 2, "{note} {entries:?}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(library_file(&cfg, false, "small.txt").unwrap().1, b"hi");
        assert!(library_file(&cfg, false, "big.bin").unwrap_err().to_string().contains("too big"));
        assert!(library_file(&cfg, false, "../etc/passwd").is_err());
        cfg.library_only_away = true;
        assert!(library_file(&cfg, false, "small.txt").unwrap_err().to_string().contains("only while"));
        assert!(library_file(&cfg, true, "small.txt").is_ok());
        cfg.share_library = false;
        assert!(library_file(&cfg, true, "small.txt").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// QSY rules: off by default; same band, distance, bands, segments.
    #[test]
    fn qsy_policy() {
        let home = 14_103_000.0;
        let mut c = Config::default();
        assert!(qsy_verdict(&c, home, 14_105_000.0).unwrap_err().contains("turned off"));
        c.accept_qsy = true;
        assert!(qsy_verdict(&c, home, 14_105_000.0).is_ok());
        assert!(qsy_verdict(&c, home, 7_102_000.0).unwrap_err().contains("only QSY within 20m"));
        assert!(qsy_verdict(&c, home, 14_200_000.0).unwrap_err().contains("kHz from here"));
        c.qsy_same_band = false;
        assert!(qsy_verdict(&c, home, 7_102_000.0).unwrap_err().contains("kHz from here"));
        c.qsy_max_khz = 0.0;
        assert!(qsy_verdict(&c, home, 7_102_000.0).is_ok(), "{:?}", qsy_verdict(&c, home, 7_102_000.0));
        assert!(qsy_verdict(&c, home, 28_120_000.0).unwrap_err().contains("isn't one of my QSY bands"));
        assert!(qsy_verdict(&c, home, 14_090_000.0).unwrap_err().contains("outside my digital segments"));
    }

    /// The schedule tunes the radio to the slot that's on now.
    #[test]
    fn demo_schedule_tunes_the_radio() {
        let (h, wait) = demo_station("N0SKD", |c| {
            c.schedule_on = true;
            c.schedule_only_away = false;
            c.schedule = vec![crate::schedule::Slot { from: "00:00".into(), to: "00:00".into(), khz: 14_108.5, ..Default::default() }];
        });
        wait(&h, "scheduled frequency", 30, &|s| s.rig.freq_hz == 14_108_500.0 && s.schedule_now.is_some());
        h.send(Cmd::Stop);
    }

    /// Demo with encryption required: link to K7BOT, see the 🔒 and the
    /// code, chat and send a file sealed; after hanging up and calling
    /// again, the new link has new keys (a different code).
    #[test]
    fn demo_encrypted_chat_and_file() {
        let h = spawn(|| {});
        let mut cfg = Config { demo: true, demo_real_mercury: false, callsign: "N0SEC".into(), e2e: e2e::Policy::Require, ..Default::default() };
        cfg.download_dir = std::env::temp_dir().join(format!("ws-e2e-{}", std::process::id()));
        cfg.sdr.enabled = false;
        cfg.whistle_watch = false;
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
        let sas = |s: &State| match s.link.as_ref().map(|l| &l.e2e) {
            Some(E2eView::Encrypted { sas }) => Some(sas.clone()),
            _ => None,
        };
        wait("Mercury", 10, &|s| s.mercury_ok);
        h.send(Cmd::Connect("K7BOT".into()));
        wait("encrypted link with K7BOT", 40, &|s| sas(s).is_some());
        let first = sas(&h.state.lock().unwrap()).unwrap();
        // The bot reads its code back: same as ours, so no one in the middle.
        wait("bot's code", 30, &|s| s.chat.iter().any(|c| c.who == Who::Them && c.text.contains(&first)));
        assert!(h.state.lock().unwrap().chat.iter().any(|c| c.who == Who::System && c.text.contains("🔒")));
        h.send(Cmd::Chat("hello sealed".into()));
        wait("bot reply", 30, &|s| s.chat.iter().any(|c| c.who == Who::Them && c.text.contains("hello sealed")));
        let dir = cfg.download_dir.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("log.txt"), "QSO log line\n".repeat(400)).unwrap();
        h.send(Cmd::SendFiles { paths: vec![dir.clone()], zip: true });
        wait("file delivered", 60, &|s| s.transfers.iter().any(|t| t.dir == Dir::Out && t.state == XferState::Done(None)));

        h.send(Cmd::Disconnect);
        wait("link down", 30, &|s| s.link.is_none());
        h.send(Cmd::Connect("K7BOT".into()));
        wait("second encrypted link", 40, &|s| sas(s).is_some());
        let second = sas(&h.state.lock().unwrap()).unwrap();
        assert_ne!(first, second, "a new link must have new keys");
        h.send(Cmd::Stop);
        std::fs::remove_dir_all(&cfg.download_dir).ok();
    }
}
