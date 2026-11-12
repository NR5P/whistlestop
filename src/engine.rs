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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    CallCq {
        table: Option<String>,
    },
    StopCq,
    Connect(String),
    Disconnect,
    Chat(String),
    SendFiles {
        paths: Vec<PathBuf>,
        zip: bool,
    },
    AnswerOffer {
        fid: u64,
        accept: bool,
    },
    /// Stop a transfer. Both directions are named: the two stations number
    /// their own files independently, so a fid alone can match one of each.
    Cancel {
        fid: u64,
        dir: Dir,
    },
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
    RigOffset {
        xit: bool,
        hz: i32,
    },
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
    DepotPost {
        to: String,
        via: Option<String>,
        text: String,
    },
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
    /// Connect to Mercury again if not connected, keeping the radio, its
    /// shared rigctld port and everything else as they are. For a Mercury
    /// that opened its ports after the station started (the phone apps').
    ReconnectModem,
    MonitorVolume(f32),
    /// Wide whistle search on the SDR; `auto` tunes the radio to hits.
    WideSearch {
        on: bool,
        auto: bool,
    },
    /// Tune to an SDR hit and fine-lock on it with the radio's own audio.
    LockOn(f64),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Who {
    Me,
    Them,
    System,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ChatLine {
    pub at: chrono_lite::Clock,
    pub who: Who,
    pub from: String,
    pub text: String,
    pub id: u64,
    /// The other station confirmed it arrived (a RECEIVED packet).
    pub delivered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dir {
    Out,
    In,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum XferState {
    Offered,
    WaitingForYou,
    Sending,
    Receiving,
    Done(Option<PathBuf>),
    Failed(String),
    Declined,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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
    #[serde(with = "ser::opt_instant")]
    pub started: Option<Instant>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Heard {
    pub at: chrono_lite::Clock,
    pub call: String,
    pub dial_hz: f64,
    pub what: String,
    pub table: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LinkInfo {
    pub peer: String,
    pub bw: u32,
    #[serde(with = "ser::opt_instant")]
    pub since: Option<Instant>,
    pub peer_name: String,
    pub peer_grid: String,
    pub peer_app: String,
    /// How the peer hears us (their REPORT).
    pub their_report_db: Option<i8>,
    #[serde(with = "ser::opt_mode")]
    pub mode: Option<&'static Mode>,
    pub bps: f64,
    /// End-to-end encryption on this link.
    pub e2e: E2eView,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Member {
    pub snr_of_host: Option<i8>,
    #[serde(with = "ser::opt_instant")]
    pub last: Option<Instant>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TableState {
    pub name: String,
    pub id: u16,
    pub host: bool,
    #[serde(with = "ser::mode")]
    pub mode: &'static Mode,
    pub members: HashMap<String, Member>,
    /// Who hosts it (a member learns it from the host's CALL or SPEED).
    #[serde(default)]
    pub host_call: Option<String>,
    pub floor: Option<String>,
    pub lines: Vec<(chrono_lite::Clock, String, String)>,
    /// Frames still to transmit, with their air time.
    pub sending_eta_s: f64,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SearchView {
    pub active: bool,
    pub scope: String,
    pub index: usize,
    pub total: usize,
    pub channel_hz: f64,
    pub note: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct CqView {
    pub active: bool,
    pub cycle: u32,
    pub cycles: u32,
    pub phase: String,
    pub table: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WhistleSeen {
    #[serde(with = "ser::instant")]
    pub at: Instant,
    pub dial_hz: f64,
    pub offset_hz: f64,
    pub score: f64,
}

/// A Roundtable member re-reports how well it hears the host this often.
const TABLE_REPORT_EVERY: Duration = Duration::from_secs(120);
/// An unattended transmission (CQ, Station ID) waits at most this long for a
/// busy channel to clear, then goes anyway.
const BUSY_WAIT_MAX: Duration = Duration::from_secs(60);
/// How long an automatic deliver/collect call waits for a clear channel
/// (past BUSY_WAIT_MAX, so a stuck detector still lets it go).
const AUTO_PENDING_MAX: Duration = Duration::from_secs(90);
/// A member not heard for this long no longer counts for the speed.
const TABLE_MEMBER_TIMEOUT: Duration = Duration::from_secs(420);
/// A sped-up table opens a DATAC3 join window this often...
const JOIN_WINDOW_EVERY: Duration = Duration::from_secs(180);
/// ...for this long (after the switch to DATAC3).
const JOIN_WINDOW_FOR: Duration = Duration::from_secs(20);
/// A host with nothing to say re-announces the table's speed this often, so
/// its members can tell they still hear it.
const TABLE_HEARTBEAT: Duration = Duration::from_secs(90);
/// A member that hears nothing of its table for this long (two heartbeats
/// and a join window) has lost it: it listens in another of the table's
/// likely speeds until it hears it again.
const TABLE_LOST_AFTER: Duration = Duration::from_secs(240);
/// A member whose report says it can't copy the table's speed expects the
/// host to slow down; if no announcement comes in this time, it slows down
/// by itself (it would never copy the one sent at the faster speed).
const SLOWDOWN_WAIT: Duration = Duration::from_secs(40);
/// Roundtable: at most this many AGAIN requests every 20 s.
const AGAIN_PER_ROUND: usize = 3;

/// An automatic call (auto-answer, deliver, collect) that hasn't linked by
/// then is abandoned.
const AUTO_CALL_GIVE_UP: Duration = Duration::from_secs(60);

/// Chat lines kept on screen (older ones scroll away for good).
pub const MAX_CHAT_LINES: usize = 500;
/// Finished transfer rows kept (those still going are never dropped).
pub const MAX_DONE_TRANSFERS: usize = 200;

impl State {
    /// Keep the history bounded: an unattended station runs for days, and
    /// the whole State goes to a remote operator several times a second.
    pub fn trim_history(&mut self) {
        if self.chat.len() > MAX_CHAT_LINES {
            let excess = self.chat.len() - MAX_CHAT_LINES;
            self.chat.drain(..excess);
        }
        let finished = |t: &Transfer| {
            matches!(
                t.state,
                XferState::Done(_) | XferState::Failed(_) | XferState::Declined
            )
        };
        let mut excess = self
            .transfers
            .iter()
            .filter(|t| finished(t))
            .count()
            .saturating_sub(MAX_DONE_TRANSFERS);
        if excess > 0 {
            self.transfers.retain(|t| {
                if excess > 0 && finished(t) {
                    excess -= 1;
                    false
                } else {
                    true
                }
            });
        }
        // Mail news older than the window the screen shows (30 min).
        self.mail_alerts
            .retain(|a| a.at.elapsed() < Duration::from_secs(1800));
    }
}

/// Fields missing from a serialized State take their defaults, so a remote
/// client can still show a radio computer running another version.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct State {
    /// Mercury's busy detector reads the channel as occupied.
    pub channel_busy: bool,
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
    /// Radio-audio waterfall rows (dB per bin, 0..4 kHz), newest last
    /// (the last 64; `audio_seq` counts every row ever added). Sent to a
    /// remote computer separately, only the new ones.
    #[serde(skip)]
    pub audio_rows: VecDeque<Vec<f32>>,
    #[serde(skip)]
    pub audio_seq: u64,
    #[serde(with = "ser::opt_mode")]
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
    /// A Roundtable we'll join as soon as Mercury is free (a link is
    /// still closing).
    pub table_pending: Option<String>,
}

/// Radio controls beyond the basics, polled while their window is open,
/// plus transmit meters.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
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

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct WideView {
    pub on: bool,
    pub auto: bool,
    pub note: String,
    /// (caller dial as the SDR sees it, score, seconds since last heard)
    pub hits: Vec<(f64, f64, f64)>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QsyView {
    pub home_hz: f64,
    pub at_hz: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LibView {
    pub path: String,
    pub entries: Vec<crate::protocol::LibEntry>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MailAlert {
    pub call: String,
    /// We hold mail for them (true), or they hold mail for us (false).
    pub ours: bool,
    pub count: usize,
    #[serde(with = "ser::instant")]
    pub at: Instant,
}

pub const WIDE_SCOPE: &str = "SDR wide search";

pub type SharedState = Arc<Mutex<State>>;

/// Serde helpers for state that crosses the network (remote operation):
/// an `Instant` travels as "seconds ago", a Mercury mode by its name.
pub mod ser {
    use crate::modes::{MODES, Mode};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, Instant};

    fn ago(t: &Instant) -> f64 {
        t.elapsed().as_secs_f64()
    }
    fn back(secs: f64) -> Instant {
        Instant::now()
            .checked_sub(Duration::from_secs_f64(secs.max(0.0)))
            .unwrap_or_else(Instant::now)
    }
    fn find(name: &str) -> Result<&'static Mode, String> {
        MODES
            .iter()
            .find(|m| m.name == name)
            .ok_or_else(|| format!("unknown mode {name}"))
    }

    pub mod instant {
        use super::*;
        pub fn serialize<S: Serializer>(t: &Instant, s: S) -> Result<S::Ok, S::Error> {
            ago(t).serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Instant, D::Error> {
            Ok(back(f64::deserialize(d)?))
        }
    }
    pub mod opt_instant {
        use super::*;
        pub fn serialize<S: Serializer>(t: &Option<Instant>, s: S) -> Result<S::Ok, S::Error> {
            t.as_ref().map(ago).serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Instant>, D::Error> {
            Ok(Option::<f64>::deserialize(d)?.map(back))
        }
    }
    pub mod mode {
        use super::*;
        pub fn serialize<S: Serializer>(m: &&'static Mode, s: S) -> Result<S::Ok, S::Error> {
            m.name.serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<&'static Mode, D::Error> {
            find(&String::deserialize(d)?).map_err(serde::de::Error::custom)
        }
    }
    pub mod opt_mode {
        use super::*;
        pub fn serialize<S: Serializer>(
            m: &Option<&'static Mode>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            m.map(|m| m.name).serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<&'static Mode>, D::Error> {
            match Option::<String>::deserialize(d)? {
                Some(n) => find(&n).map(Some).map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        }
    }
}

#[derive(Clone)]
pub struct Handle {
    pub tx: mpsc::UnboundedSender<Cmd>,
    pub state: SharedState,
    pub sdr: sdr::Shared,
    /// The radio's received audio, for remote listeners (filled only while
    /// one is listening).
    pub pcm: PcmTap,
}

/// Received audio (8 kHz) waiting for remote listeners.
#[derive(Default)]
pub struct PcmBuf {
    pub on: bool,
    pub samples: VecDeque<f32>,
}
pub type PcmTap = Arc<Mutex<PcmBuf>>;

impl Handle {
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

/// A tiny wall-clock time for display, without a date/time crate.
pub mod chrono_lite {
    #[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
    pub struct Clock(pub u64);
    impl Clock {
        pub fn now() -> Self {
            Clock(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            )
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
    let pcm: PcmTap = Default::default();
    let (st, sd, pc) = (state.clone(), sdr_shared.clone(), pcm.clone());
    let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(repaint);
    std::thread::Builder::new()
        .name("whistlestop-engine".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .unwrap();
            rt.block_on(run(rx, st, sd, pc, repaint));
        })
        .unwrap();
    Handle {
        tx,
        state,
        sdr: sdr_shared,
        pcm,
    }
}

// ---------------------------------------------------------------- engine ---

enum SearchPhase {
    Settle {
        until: Instant,
    },
    Dwell {
        from: u64,
    },
    Hold {
        from: u64,
    },
    /// Retuned onto a whistle the scan found (`coarse` Hz off the channel):
    /// measure it finely. `again`: this is the second listen.
    Verify {
        target: f64,
        from: u64,
        coarse: f64,
        again: bool,
        hops: u8,
    },
    WaitCq {
        until: Instant,
    },
}

struct Search {
    channels: Vec<f64>,
    idx: usize,
    phase: SearchPhase,
    scope: String,
    /// A wide-search hand-off that interrupted a stepping scan keeps the
    /// scan here and resumes it (on the same channel) if the hit comes to
    /// nothing.
    resume: Option<Box<Search>>,
}

impl Search {
    /// A stepping scan still going channel to channel: nothing found yet,
    /// so an SDR hit may cut in.
    fn interruptible(&self) -> bool {
        self.scope != WIDE_SCOPE
            && matches!(
                self.phase,
                SearchPhase::Settle { .. } | SearchPhase::Dwell { .. } | SearchPhase::Hold { .. }
            )
    }
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
    pcm: PcmTap,
    pcm_from: u64,
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
    early: Vec<(Instant, Vec<u8>)>,
    /// Application packets, waiting for the modem (and for keys).
    hi_queue: VecDeque<Link>,
    outgoing: Vec<Outgoing>,
    incoming: HashMap<u64, Incoming>,
    mercury_buffer: usize,
    /// Mercury's last BUFFER figure (see the BUFFER handler).
    mercury_reported: usize,
    /// When we last wrote to Mercury's data port.
    last_data_write: Instant,
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
    /// Left a Roundtable: go back to the calling mode (DATAC3) once the
    /// LEAVE has gone out in the table's own mode.
    restore_calling_mode: bool,
    last_ramp: Instant,
    last_downgrade: Instant,
    last_again: Instant,
    // activities
    search: Option<Search>,
    cq: Option<Cq>,
    last_watch: Instant,
    /// Whistle watch rests until then: Mercury's connect and disconnect
    /// bursts are wideband and can look like a whistle to the detector.
    watch_quiet_until: Instant,
    /// A first detection waiting for a second, agreeing one before it is
    /// reported (when, offset).
    watch_pending: Option<(Instant, f64)>,
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
    /// Where Mercury's TNC is: (host, control port, broadcast port).
    modem: (String, u16, u16),
    depot: crate::depot::Depot,
    away: bool,
    last_beacon: Instant,
    last_link_activity: Instant,
    /// Why we connected automatically (deliver or collect), if we did.
    link_auto: Option<bool>,
    /// Mail arrived on this link (a collect link can end soon after).
    link_got_mail: bool,
    /// When we placed a call nobody pressed a button for (auto-answer,
    /// deliver, collect): given up after AUTO_CALL_GIVE_UP.
    auto_call: Option<Instant>,
    /// We asked Mercury for a CQFRAME (when; whether its PENDING has come).
    own_cq_pending: Option<(Instant, bool)>,
    /// Demo mode's simulated Mercury.
    fake: Option<FakeMercury>,
    /// Settings problems already logged (each is logged once).
    cfg_notes: Vec<String>,
    /// Mercury's last SN figure, and when.
    last_sn: Option<(Instant, f64)>,
    /// Mercury's busy detector says someone is transmitting (BUSY ON/OFF).
    chan_busy: bool,
    /// Since when an unattended transmission has been waiting for the
    /// channel to clear (and when it last asked: a gap starts it over).
    busy_wait: Option<(Instant, Instant)>,
    /// An automatic deliver (true) or collect (false) call waiting for the
    /// channel to clear: the Station ID or CQ that set it off has only just
    /// ended, and the busy detector still reads it.
    auto_pending: Option<(String, bool, Instant)>,
    /// How well we hear the host of the table we're at (from its frames).
    host_snr: Option<f64>,
    /// When we last reported to the table's host (members re-report).
    last_table_report: Instant,
    /// Host: a join window is open (when it ends, the speed to go back to).
    join_window: Option<(Instant, &'static Mode)>,
    /// When we last heard a frame of our table, and last sent one.
    last_table_heard: Instant,
    last_table_sent: Instant,
    /// The table's speed as last announced to us (members).
    table_announced: Option<&'static Mode>,
    /// A member not yet known to be in step with the host (just joined, or
    /// lost the table): it re-sends its JOIN when it hears the host's CALL.
    table_unsynced: bool,
    /// Member: our report said we can't copy the current speed (when).
    expect_slowdown: Option<Instant>,
    /// Host: when the last join window ended (or the table started).
    last_join_window: Instant,
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
    /// Link over after a QSY: go home (to, from, since).
    qsy_home: Option<(f64, f64, Instant)>,
    /// BYE sent (or received): disconnect once Mercury has sent it.
    hangup_at: Option<Instant>,
    /// The other station said BYE at this moment.
    bye_at: Option<Instant>,
    /// When we started ending the link (BYE queued or DISCONNECT sent);
    /// cleared by DISCONNECTED. See `leaving()`.
    leaving_since: Option<Instant>,
    /// When the last link or connect attempt ended. Mercury's closing
    /// frames can still be on the air for a few seconds after.
    link_ended_at: Option<Instant>,
    /// A Roundtable to join once Mercury is free: (name, asked, last try).
    pending_join: Option<(String, Instant, Instant)>,
    /// An auto-answer waiting for the CQ station to stop transmitting:
    /// (callsign, when to CONNECT).
    answer_at: Option<(String, Instant)>,
    /// Library downloads we asked for (their offers are accepted at once).
    wanted: std::collections::HashSet<u64>,
    /// Library files we're sending on this link (fids): dropped when it ends.
    library_out: std::collections::HashSet<u64>,
    last_sched: Option<Instant>,
    sched_slot: Option<usize>,
    /// Roundtable host: the speed to switch to once the announcement is out.
    pending_speed: Option<(&'static Mode, Instant)>,
    last_query: Instant,
}

async fn run(
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    st: SharedState,
    sdr: sdr::Shared,
    pcm: PcmTap,
    repaint: Arc<dyn Fn() + Send + Sync>,
) {
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
                        // Out-of-range values (a hand-edited file, a remote
                        // client) are clamped before they reach the station.
                        let (cfg, notes) = cfg.sanitized();
                        let mut s = Station::start(cfg, st.clone(), sdr.clone(), pcm.clone(), repaint.clone()).await;
                        for n in &notes {
                            s.log(format!("⚠ Settings: {n}"));
                        }
                        s.cfg_notes = notes;
                        station = Some(s);
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
    async fn start(
        cfg: Config,
        st: SharedState,
        sdr: sdr::Shared,
        pcm: PcmTap,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Station {
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
            pcm,
            pcm_from: 0,
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
            mercury_reported: 0,
            last_data_write: Instant::now(),
            // Each station numbers its own files. Starting at a random point
            // keeps the two stations' numbers apart, so a CANCEL (which
            // either side may send) can't name the wrong file.
            next_fid: rand::random_range(1..1u64 << 20),
            next_chat_id: 1,
            est: ThroughputEstimator::default(),
            last_report: Instant::now(),
            table_asm: TableAssembler::default(),
            table_sent: HashMap::new(),
            table_msg_id: 0,
            table_queue: VecDeque::new(),
            table_busy_until: Instant::now(),
            restore_calling_mode: false,
            last_ramp: Instant::now(),
            last_downgrade: Instant::now() - Duration::from_secs(600),
            last_again: Instant::now(),
            search: None,
            cq: None,
            last_watch: Instant::now(),
            watch_quiet_until: Instant::now(),
            watch_pending: None,
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
            modem: (String::new(), 0, 0),
            // Demo keeps its own mailbox; tests can point anywhere.
            depot: crate::depot::Depot::load(
                std::env::var_os("WHISTLESTOP_DEPOT")
                    .map(PathBuf::from)
                    .or_else(|| {
                        (!cfg.depot_file.is_empty()).then(|| PathBuf::from(&cfg.depot_file))
                    })
                    .unwrap_or_else(|| {
                        if cfg!(test) {
                            std::env::temp_dir().join(format!(
                                "ws-depot-{}-{}.json",
                                std::process::id(),
                                cfg.callsign
                            ))
                        } else if cfg.demo {
                            crate::depot::default_path().with_file_name("depot-demo.json")
                        } else {
                            crate::depot::default_path()
                        }
                    }),
            ),
            away: false,
            last_beacon: Instant::now(),
            last_link_activity: Instant::now(),
            link_auto: None,
            link_got_mail: false,
            auto_call: None,
            own_cq_pending: None,
            fake: None,
            cfg_notes: vec![],
            last_sn: None,
            chan_busy: false,
            busy_wait: None,
            auto_pending: None,
            host_snr: None,
            last_table_report: Instant::now(),
            join_window: None,
            last_join_window: Instant::now(),
            last_table_heard: Instant::now(),
            last_table_sent: Instant::now(),
            table_announced: None,
            table_unsynced: false,
            expect_slowdown: None,
            auto_cooldown: HashMap::new(),
            qsy: None,
            qsy_out: None,
            qsy_move: None,
            qsy_tuned: false,
            qsy_tune_now: false,
            qsy_fail: false,
            qsy_home: None,
            hangup_at: None,
            bye_at: None,
            leaving_since: None,
            link_ended_at: None,
            pending_join: None,
            answer_at: None,
            wanted: Default::default(),
            library_out: Default::default(),
            last_sched: None,
            sched_slot: None,
            pending_speed: None,
            last_query: Instant::now(),
        };
        {
            let mut g = st.lock().unwrap();
            *g = State {
                running: true,
                demo: cfg.demo,
                mycall: cfg.callsign.to_uppercase(),
                bandwidth: Some(bw),
                ..Default::default()
            };
        }
        // Demo on real Mercury when it's installed; otherwise all simulated.
        let real = if cfg.demo && cfg.demo_real_mercury {
            match crate::realdemo::find_mercury(&cfg.mercury_bin) {
                Some(bin) => {
                    match crate::realdemo::start(&bin).await {
                        Ok(d) => Some(d),
                        Err(e) => {
                            s.log(format!("⚠ Demo couldn't start real Mercury ({e}); using the simulated modem"));
                            None
                        }
                    }
                }
                None => None,
            }
        } else {
            None
        };
        let real_ports = real.as_ref().map(|d| {
            (
                d.rig_addr.clone(),
                d.audio_addr.clone(),
                d.mercury_port,
                d.bcast_port,
            )
        });
        s.log(match (&real, cfg.demo) {
            (Some(_), _) => "Whistlestop starting in Demo mode: two real Mercury modems on a simulated 20 m band, you and K7BOT".to_string(),
            (None, true) => "Whistlestop starting in Demo mode (simulated radio, band and Mercury; install Mercury for a real-Mercury demo)".to_string(),
            _ => "Whistlestop starting".to_string(),
        });
        if let Some(p) = s.depot.load_problem.take() {
            s.log(format!("⚠ {p}"));
        }
        for p in &cfg.load_problems {
            s.log(format!("⚠ Settings file: {p}"));
        }

        // Radio.
        if cfg.demo && real.is_none() {
            s.rig = Some(Rig::sim(14_096_000.0, vfo.clone()));
            s.set(|g| g.rig_ok = true);
            s.set(|g| g.rig_desc = "Demo transceiver (simulated)".into());
        } else if cfg.rig_mode == crate::rigsetup::RigMode::Direct && real.is_none() && !cfg.demo {
            // Whistlestop's own CAT drivers: find the radio and its protocol.
            let cat = cfg.cat.clone();
            match tokio::task::spawn_blocking(move || crate::cat::connect(&cat))
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
                .and_then(|r| r)
            {
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
            let mut addr = real_ports
                .as_ref()
                .map(|p| p.0.clone())
                .unwrap_or(cfg.rigctld.clone());
            let mut desc = if real_ports.is_some() {
                "Demo radio on the simulated band".to_string()
            } else {
                format!("rigctld at {addr}")
            };
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
            // "none" (the phones' Radio → None): no CAT at all, keyed by VOX.
            let no_cat = real_ports.is_none() && s.rigctld.is_none() && addr.trim().eq_ignore_ascii_case("none");
            if no_cat {
                s.log("Radio: none (no CAT). Whistlestop can't read or tune the dial; key the radio with VOX.".into());
            }
            // A freshly started rigctld needs a moment to open the radio.
            let mut last = None;
            for _ in 0..(if no_cat { 0 } else if s.rigctld.is_some() { 30 } else { 1 }) {
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
            let caps = tokio::time::timeout(Duration::from_secs(4), r.caps())
                .await
                .ok()
                .and_then(|c| c.ok())
                .unwrap_or_default();
            if !caps.model.is_empty() {
                s.log(format!(
                    "Radio reports: {} ({} adjustable levels, {} functions{})",
                    caps.model,
                    caps.set_levels.len(),
                    caps.set_funcs.len(),
                    if caps.has_tuner() {
                        ", antenna tuner"
                    } else {
                        ""
                    }
                ));
            }
            s.set(|g| g.rig_caps = Some(caps.clone()));
            s.rig_caps = caps;
        }

        // Audio.
        let audio_in = real_ports
            .as_ref()
            .map(|p| format!("ether:{}", p.1))
            .unwrap_or(cfg.audio_in.clone());
        if cfg.demo && real.is_none() {
            let band = SimBand::new(
                vfo.clone(),
                vec![
                    SimSignal::Whistler {
                        rf_hz: 14_103_150.0,
                        snr_db: -2.0,
                        whistle_s: 10.0,
                        cycle_s: 30.0,
                        start_s: 4.0,
                    },
                    SimSignal::Carrier {
                        rf_hz: 14_101_900.0,
                        snr_db: 8.0,
                    },
                    SimSignal::Ft8 {
                        rf_hz: 14_097_350.0,
                        snr_db: 0.0,
                    },
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
                        s._input =
                            Some(LiveInput::ether(t.try_clone().expect("clone"), rx.clone()));
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
            match FakeMercury::start(FakeOptions {
                rig_freq: Some(vfo.clone()),
                ..Default::default()
            })
            .await
            {
                Ok(f) => {
                    let r = ("127.0.0.1".to_string(), f.port, f.bcast_port);
                    // Stopped with the station (each restart used to leave
                    // the old one running).
                    s.fake = Some(f);
                    r
                }
                Err(e) => {
                    s.log(format!("⚠ demo Mercury: {e}"));
                    ("127.0.0.1".into(), 0, 0)
                }
            }
        } else {
            (
                cfg.mercury_host.clone(),
                cfg.mercury_port,
                cfg.mercury_bcast_port,
            )
        };
        s.modem = (host, port, bport);
        s.connect_modem().await;

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

    /// Connect to Mercury's TNC (`self.modem`). Mercury serves one control
    /// client, so this is tried at start and then only on request
    /// (`Cmd::ReconnectModem`), never polled.
    async fn connect_modem(&mut self) -> bool {
        let (host, port, bport) = self.modem.clone();
        self.chan_busy = false;
        self.busy_wait = None;
        self.set(|g| g.channel_busy = false);
        let (etx, erx) = mpsc::unbounded_channel();
        match Tnc::connect(&host, port, Some(bport), etx).await {
            Ok(mut t) => match t.setup(&self.mycall(), self.bw.mercury_arq_bw()).await {
                Ok(()) => {
                    let _ = t.command("MODE 1").await; // calling mode: DATAC3
                    // Listen before unattended transmissions (CQ, Station ID,
                    // automatic calls). Off during links: Mercury's link also
                    // holds back while the detector says busy.
                    if self.cfg.busy_detect {
                        let _ = t.command("BUSYDETECT ON").await;
                    }
                    let _ = t.send_line("VERSION").await;
                    self.tnc = Some(t);
                    self.tnc_rx = Some(erx);
                    self.set(|g| {
                        g.mercury_ok = true;
                        g.listen_mode = modes::mode_by_name("DATAC3");
                    });
                    let me = self.mycall();
                    if crate::protocol::valid_call(&me) {
                        self.log(format!("Mercury: connected ({host}:{port}), listening as {me}"));
                    } else {
                        self.log(format!(
                            "⚠ Mercury: connected ({host}:{port}), but {me:?} isn't a callsign: nothing will be sent until you set yours (Settings → Station)"
                        ));
                    }
                    true
                }
                Err(e) => {
                    self.log(format!("⚠ Mercury setup: {e}"));
                    false
                }
            },
            Err(e) => {
                self.log(format!("⚠ {e}"));
                false
            }
        }
    }

    async fn shutdown(mut self) {
        if let Some(f) = self.fake.take() {
            f.stop();
        }
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
        sdr::start(
            self.cfg.sdr.clone(),
            self.vfo.clone(),
            self.sdr_span.clone(),
            self.cfg.demo,
            self.sdr.clone(),
            self.sdr_stop.clone(),
        );
    }

    fn set(&self, f: impl FnOnce(&mut State)) {
        let mut g = self.st.lock().unwrap();
        f(&mut g);
        g.trim_history();
    }

    fn log(&self, msg: String) {
        let mut g = self.st.lock().unwrap();
        g.log.push_back((Clock::now(), msg));
        while g.log.len() > 500 {
            g.log.pop_front();
        }
    }

    fn mycall(&self) -> String {
        self.cfg.callsign.trim().to_uppercase()
    }

    fn dial(&self) -> f64 {
        *self.vfo.lock().unwrap()
    }

    /// Mercury holds a 1:1 link only within about ±55 Hz: moving the dial
    /// by hand mid-link drops it silently and leaves both modems polling for
    /// minutes. (A QSY moves both stations together and doesn't come here.)
    fn no_retune_while_linked(&self) -> anyhow::Result<()> {
        if self.peer.is_some() || self.st.lock().unwrap().calling.is_some() {
            anyhow::bail!("not while linked: retuning would break the link. Disconnect first");
        }
        Ok(())
    }

    /// Rest the whistle watch for 8 s: Mercury's connect bursts can look
    /// like a whistle.
    fn quiet_watch(&mut self) {
        self.quiet_watch_for(8.0);
    }

    fn quiet_watch_for(&mut self, secs: f64) {
        let until = Instant::now() + Duration::from_secs_f64(secs.max(0.0));
        if until > self.watch_quiet_until {
            self.watch_quiet_until = until;
        }
        self.watch_pending = None;
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
            let swr = if caps.can_get_level("SWR") {
                r.get_level("SWR").await.ok().filter(|v| *v >= 1.0)
            } else {
                None
            };
            let alc = if caps.can_get_level("ALC") {
                r.get_level("ALC").await.ok()
            } else {
                None
            };
            let po = if caps.can_get_level("RFPOWER_METER") {
                r.get_level("RFPOWER_METER").await.ok()
            } else {
                None
            };
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
                    let recently = self.last_autotune.is_some_and(|(at, b)| {
                        b == band && at.elapsed() < Duration::from_secs(120)
                    });
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
        const LEVELS: [&str; 10] = [
            "AF", "RF", "SQL", "MICGAIN", "COMP", "NR", "AGC", "PREAMP", "ATT", "RFPOWER",
        ];
        const FUNCS: [&str; 7] = ["NB", "NR", "ANF", "COMP", "VOX", "MON", "TUNER"];
        let caps = self.rig_caps.clone();
        let Some(r) = self.rig.as_mut() else { return };
        let mut ext = self.st.lock().unwrap().rig_ext.clone();
        for l in LEVELS.iter().filter(|l| caps.can_get_level(l)) {
            if let Ok(v) = r.get_level(l).await {
                ext.levels.insert(l.to_string(), v);
            }
        }
        for f in FUNCS
            .iter()
            .filter(|f| caps.get_funcs.iter().any(|x| x == *f))
        {
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
            (
                g.rig_ext.swr,
                g.rig_ext.swr_last_tx,
                g.rig_ext.alc,
                g.rig_ext.power_out,
                g.rig_ext.tuner_note.clone(),
                g.rig_ext.tune_pending,
            )
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
        let Some(r) = self.rig.as_mut() else {
            anyhow::bail!("no radio")
        };
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
            g.rig_ext.tuner_note =
                format!("Tuned on {} at {when}", band.unwrap_or("this frequency"));
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
                let (c, notes) = c.sanitized();
                for n in notes.iter().filter(|n| !self.cfg_notes.contains(n)) {
                    self.log(format!("⚠ Settings: {n}"));
                }
                self.cfg_notes = notes;
                let keep_bw = self.bw;
                let old_call = self.mycall();
                let busy_was = self.cfg.busy_detect;
                let busy_now = c.busy_detect;
                if busy_was && !busy_now {
                    // Off: a BUSY it latched must not hold anything back.
                    self.busy_detect(false).await;
                }
                self.cfg = c;
                if !busy_was && busy_now && self.peer.is_none() {
                    self.busy_detect(true).await;
                }
                self.cfg.bandwidth = keep_bw;
                // A new callsign reaches Mercury now (not linked: a link
                // carries on under the call it was made with).
                let call = self.mycall();
                if call != old_call && self.peer.is_none() && crate::protocol::valid_call(&call) {
                    if let Some(t) = self.tnc.as_mut() {
                        t.command(&format!("MYCALL {call}")).await?;
                    }
                    self.set(|g| g.mycall = call.clone());
                    self.log(format!("Callsign now {call}"));
                }
            }
            Cmd::SetFreq(hz) => {
                self.no_retune_while_linked()?;
                self.stop_search("tuned by hand");
                self.tune(hz).await?;
            }
            Cmd::Nudge(d) => {
                self.no_retune_while_linked()?;
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
                // The link's bandwidth was agreed when it came up; Mercury
                // isn't asked to change it mid-link.
                if self.peer.is_some() {
                    anyhow::bail!("bandwidth can't change during a link: it applies from the next one");
                }
                self.bw = bw;
                self.detector = Detector::new(bw.whistle_profile());
                self.tracker = TrackDetector::new(bw.whistle_profile());
                if let Some(t) = self.tnc.as_mut() {
                    t.command(&format!("BW{}", bw.mercury_arq_bw())).await?;
                }
                let table_mode = self.st.lock().unwrap().table.as_ref().map(|t| t.mode);
                if let Some(m) = table_mode
                    && m.bandwidth_hz > bw.hz()
                {
                    self.table_set_speed(modes::best_mode(bw, -30.0, 0.0))
                        .await?;
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
            Cmd::StopCq => self.stop_cq("CQ stopped").await,
            Cmd::Connect(call) => {
                let call = call.trim().to_uppercase();
                self.ready_to_call(&call).await?;
                self.stop_search("connecting");
                self.cancel_pending_join("calling instead");
                // A Connect by hand replaces a pending auto-answer (and its
                // "Calling", set again below once this CONNECT is out).
                if self.answer_at.take().is_some() {
                    self.set(|g| g.calling = None);
                }
                let me = self.mycall();
                self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
                self.set(|g| g.calling = Some(call.clone()));
                self.quiet_watch();
                self.log(format!("Calling {call}…"));
            }
            Cmd::Disconnect => {
                // Nothing automatic stays armed: a waiting auto-answer would
                // still call, and an automatic call's give-up timer would
                // ABORT whatever is called by hand next.
                self.answer_at = None;
                self.auto_pending = None;
                if self.peer.is_none() {
                    self.auto_call = None;
                }
                if self.peer.is_some() {
                    self.hang_up("73").await?;
                } else {
                    self.disconnect_now().await?;
                }
                self.quiet_watch();
                self.set(|g| g.calling = None);
            }
            Cmd::Chat(text) => {
                if self.peer.is_none() {
                    anyhow::bail!("not linked to anyone: connect first (or use a Roundtable)");
                }
                let id = self.next_chat_id;
                self.next_chat_id += 1;
                self.hi_queue.push_back(Link::Chat {
                    id,
                    text: text.clone(),
                });
                let me = self.mycall();
                self.set(|g| {
                    g.chat.push(ChatLine {
                        at: Clock::now(),
                        who: Who::Me,
                        from: me,
                        text,
                        id,
                        delivered: false,
                    })
                });
                self.pump().await?;
            }
            Cmd::SendFiles { paths, zip } => self.queue_files(paths, zip).await?,
            Cmd::AnswerOffer { fid, accept } => self.answer_offer(fid, accept).await?,
            Cmd::Cancel { fid, dir } => {
                match dir {
                    Dir::Out => self.outgoing.retain(|o| o.fid != fid),
                    Dir::In => {
                        if let Some(i) = self.incoming.remove(&fid) {
                            i.save_partial();
                        }
                    }
                }
                self.hi_queue.push_back(Link::Cancel { fid });
                self.xfer(dir, fid, |t| t.state = XferState::Failed("cancelled".into()));
                self.pump().await?;
            }
            Cmd::JoinTable(name) => self.request_join(name).await?,
            Cmd::LeaveTable => {
                if self.pending_join.take().is_some() {
                    self.set(|g| g.table_pending = None);
                    self.log("Roundtable join cancelled".into());
                }
                if let Some(id) = self.table_id() {
                    let me = self.mycall();
                    self.table_tx(
                        TableFrame {
                            table: id,
                            from: me,
                            body: TableBody::Leave,
                        }
                        .encode(),
                    );
                }
                self.set(|g| g.table = None);
                // A table that ramped up (say to DATAC1) left Mercury
                // listening there, deaf to the DATAC3 calls everyone else uses.
                self.restore_calling_mode = true;
                self.log("Left the Roundtable".into());
            }
            Cmd::TableSay(text) => self.table_say(text)?,
            Cmd::PassFloor(to) => {
                let Some(id) = self.table_id() else {
                    anyhow::bail!("not at a Roundtable")
                };
                let me = self.mycall();
                self.table_tx(
                    TableFrame {
                        table: id,
                        from: me,
                        body: TableBody::Floor {
                            to: to.to_uppercase(),
                        },
                    }
                    .encode(),
                );
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
                self.set(|g| {
                    if xit {
                        g.rig_ext.xit = hz
                    } else {
                        g.rig_ext.rit = hz
                    }
                });
            }
            Cmd::RigPassband(hz) => {
                let mode = self.st.lock().unwrap().rig.mode.clone();
                if let Some(r) = self.rig.as_mut() {
                    r.set_mode(if mode.is_empty() { "PKTUSB" } else { &mode }, hz)
                        .await?;
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
                self.log(if on {
                    "🚪 Away: links get your away message; what they type waits in your Depot"
                        .into()
                } else {
                    "Back at the radio".into()
                });
            }
            Cmd::DepotPost { to, via, text } => {
                let to = to.trim().to_uppercase();
                if to.is_empty() || text.trim().is_empty() {
                    anyhow::bail!("write a callsign and a message");
                }
                let me = self.mycall();
                let via = via
                    .map(|v| v.trim().to_uppercase())
                    .filter(|v| !v.is_empty() && *v != to);
                self.depot.post_via(&me, &to, via.as_deref(), text.trim())?;
                self.log(match &via {
                    Some(v) => format!("📮 Mail for {to} waits in Departures, to be handed to {v}"),
                    None => format!("📮 Mail for {to} waits in Departures"),
                });
                // Linked with them (or the relay) right now: send it at once.
                if let Some(p) = self.peer.clone()
                    && (p == to || via.as_deref() == Some(p.as_str()))
                {
                    let ids: Vec<u64> = self
                        .depot
                        .departures
                        .iter()
                        .rev()
                        .take(1)
                        .map(|m| m.id)
                        .collect();
                    let mail: Vec<Link> = self
                        .depot
                        .departures
                        .iter()
                        .filter(|m| ids.contains(&m.id))
                        .map(|m| Link::Mail {
                            id: m.id,
                            from: m.from.clone(),
                            to: m.to.clone(),
                            created: m.created,
                            hops: m.hops,
                            text: m.text.clone(),
                        })
                        .collect();
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
                self.ready_to_call(&call).await?;
                self.stop_search("connecting");
                let me = self.mycall();
                self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
                self.set(|g| g.calling = Some(call.clone()));
                self.quiet_watch();
                self.log(format!("Connecting to {call} for Depot mail…"));
            }
            Cmd::BeaconNow => self.send_beacon("by hand")?,
            Cmd::ReconnectModem => {
                if self.tnc.is_none() {
                    self.connect_modem().await;
                }
            }
            Cmd::Qsy(hz) => {
                let Some(peer) = self.peer.clone() else {
                    anyhow::bail!("not linked")
                };
                if bandplan::band_of(hz).is_none() {
                    anyhow::bail!("{:.3} kHz isn't in an amateur band", hz / 1e3);
                }
                let id = crate::depot::new_id();
                self.qsy_out = Some((id, hz));
                self.hi_queue.push_back(Link::Qsy {
                    id,
                    dial_hz: hz.round() as u64,
                    why: String::new(),
                });
                self.log(format!("↔ Asking {peer} to QSY to {:.3} kHz", hz / 1e3));
                self.set(|g| {
                    g.qsy_note = Some(format!("Asked {peer} to move to {:.3} kHz…", hz / 1e3))
                });
                self.pump().await?;
            }
            Cmd::LibList(path) => {
                if self.peer.is_none() {
                    anyhow::bail!("not linked");
                }
                self.hi_queue.push_back(Link::ListReq {
                    id: crate::depot::new_id(),
                    path,
                });
                self.pump().await?;
            }
            Cmd::LibFetch(path) => {
                let Some(peer) = self.peer.clone() else {
                    anyhow::bail!("not linked")
                };
                let id = crate::depot::new_id();
                self.wanted.insert(id);
                self.hi_queue.push_back(Link::Fetch {
                    id,
                    path: path.clone(),
                });
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
                self.wide_handoff(dial, 0.0);
            }
            Cmd::WideSearch { on, auto } => {
                self.wide_on = on;
                self.wide_auto = auto;
                self.cfg.sdr.wide_search = on;
                self.cfg.sdr.wide_auto = auto;
                self.push_wide_cfg();
                self.log(if on {
                    format!(
                        "Wide whistle search on{}",
                        if auto {
                            ": the radio will tune to whistles it finds"
                        } else {
                            ""
                        }
                    )
                } else {
                    "Wide whistle search off".into()
                });
            }
        }
        Ok(())
    }

    fn tnc(&mut self) -> anyhow::Result<&mut Tnc> {
        self.tnc
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Mercury is not connected"))
    }

    async fn ptt(&mut self, on: bool) {
        if let Some(r) = self.rig.as_mut()
            && let Err(e) = r.ptt(on).await
        {
            self.log(format!("⚠ PTT: {e}"));
        }
        self.set(|g| g.tx = on);
    }

    // -------------------------------------------------------------- search

    fn whistle_params(&self) -> WhistleParams {
        self.bw.whistle_profile()
    }

    fn step_hz(&self) -> f64 {
        // Neighbouring channels overlap so every caller is within reach. A
        // step from a hand-edited settings file is kept sane (0 would make
        // infinitely many channels).
        let reach = 1.8 * self.whistle_params().scan_reach_hz();
        let s = self.cfg.step_hz;
        if s.is_finite() { s.clamp(100.0, reach) } else { reach }
    }

    /// The fine offset from a matched-filter measurement, choosing between
    /// it and its alias the one that agrees with the scan's coarse offset.
    fn fine_offset(&self, coarse: f64, measured: f64) -> f64 {
        let alias = self.whistle_params().alias_of(measured);
        if (alias - coarse).abs() < (measured - coarse).abs() {
            alias
        } else {
            measured
        }
    }

    async fn start_search(&mut self, scope: SearchScope) -> anyhow::Result<()> {
        if self.cq.is_some() {
            anyhow::bail!("stop calling CQ first");
        }
        self.no_retune_while_linked()?;
        let segs = bandplan::segments_for(&scope, &self.cfg.segments, self.dial());
        if segs.is_empty() {
            anyhow::bail!(
                "no digital segment configured for that band (the radio is on {:.3} kHz)",
                self.dial() / 1e3
            );
        }
        let p = self.whistle_params();
        let (alo, ahi) = (p.f_lo - 200.0, p.f_hi + 200.0);
        let channels: Vec<f64> = segs
            .iter()
            .flat_map(|s| bandplan::channels(s, self.step_hz(), alo, ahi))
            .collect();
        let label = match &scope {
            SearchScope::CurrentBand => format!(
                "{} digital segment",
                bandplan::band_of(self.dial()).unwrap_or("?")
            ),
            SearchScope::Band(b) => format!("{b} digital segment"),
            SearchScope::AllBands => "all bands".into(),
        };
        let cycle = channels.len() as f64 * (self.cfg.dwell_s + 0.15);
        self.log(format!(
            "Search: {label}, {} channels, {:.0} s per pass",
            channels.len(),
            cycle
        ));
        let first = channels[0];
        self.search = Some(Search {
            channels,
            idx: 0,
            phase: SearchPhase::Settle {
                until: Instant::now(),
            },
            scope: label.clone(),
            resume: None,
        });
        self.tune(first).await?;
        self.set_settle();
        self.set(|g| {
            g.search = SearchView {
                active: true,
                scope: label,
                ..Default::default()
            }
        });
        Ok(())
    }

    fn set_settle(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.phase = SearchPhase::Settle {
                until: Instant::now() + Duration::from_millis(150),
            };
        }
    }

    fn stop_search(&mut self, why: &str) {
        if self.search.take().is_some() {
            self.log(format!("Search {why}"));
        }
        self.set(|g| g.search.active = false);
    }

    async fn step_search(&mut self) -> anyhow::Result<()> {
        let Some(mut s) = self.search.take() else {
            return Ok(());
        };
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
                    s.phase = SearchPhase::Dwell {
                        from: audio::now_count(&self.rx),
                    };
                }
            }
            SearchPhase::Dwell { from } => {
                let from = *from;
                let got = self.rx.lock().unwrap().range(from, n_dwell);
                if let Some(x) = got {
                    match self.tracker.scan(&x) {
                        Some(h) if found(h.z, dwell) => {
                            self.log(format!("🚂 Whistle on {:.1} kHz, {:+.0} Hz off (score {:.1}). Retuning onto it.", ch / 1e3, h.offset_hz, h.z));
                            s.phase = SearchPhase::Verify {
                                target: ch + h.offset_hz,
                                from: 0,
                                coarse: h.offset_hz,
                                again: false,
                                hops: 0,
                            };
                        }
                        Some(h) if h.z >= TrackDetector::MAYBE && hold_s > dwell => {
                            s.phase = SearchPhase::Hold { from }
                        }
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
                            s.phase = SearchPhase::Verify {
                                target: ch + h.offset_hz,
                                from: 0,
                                coarse: h.offset_hz,
                                again: false,
                                hops: 0,
                            };
                        }
                        _ => next_channel = true,
                    }
                }
            }
            SearchPhase::Verify {
                target,
                from,
                coarse,
                again,
                hops,
            } => {
                let (target, from, coarse, again, hops) = (*target, *from, *coarse, *again, *hops);
                let got = if from == 0 {
                    None
                } else {
                    self.rx.lock().unwrap().range(from, n_verify)
                };
                if from == 0 {
                    self.tune(target).await?;
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    s.phase = SearchPhase::Verify {
                        target,
                        from: audio::now_count(&self.rx).max(1),
                        coarse,
                        again,
                        hops,
                    };
                } else if let Some(x) = got {
                    // Now within a few tens of hertz: the matched filter
                    // measures the rest to about a hertz.
                    let wide = s.scope == WIDE_SCOPE;
                    // The scan detector's estimate (unambiguous, good to
                    // about 10 Hz) decides; the matched filter refines it
                    // only when the two agree. It copes with a strong signal
                    // inside the sweep and an SDR hand-off's crystal error.
                    let secs = x.len() as f64 / FS as f64;
                    let reach = if wide {
                        self.whistle_params().scan_reach_hz()
                    } else {
                        150.0
                    };
                    let coarse_now = self.tracker.scan(&x).filter(|h| {
                        h.z >= TrackDetector::found(secs) && h.offset_hz.abs() <= reach
                    });
                    let fine = self.detector.detect(&x).map(|d| d.offset_hz);
                    let v = match (coarse_now, fine) {
                        (Some(h), Some(f)) => {
                            let f = self.fine_offset(h.offset_hz, f);
                            Some(if (f - h.offset_hz).abs() <= 25.0 {
                                f
                            } else {
                                h.offset_hz
                            })
                        }
                        (Some(h), None) => Some(h.offset_hz),
                        (None, _) => self.detector.detect_near(&x, 60.0).map(|d| d.offset_hz),
                    };
                    let heard_something = coarse_now.is_some()
                        || self
                            .tracker
                            .scan(&x)
                            .is_some_and(|h| h.z >= TrackDetector::MAYBE);
                    // Clearly there, but further off than expected (a first
                    // look that only caught the bottom of a sweep at the
                    // passband edge): walk to it, up to two more hops.
                    let walk = if v.is_none() && hops < 2 {
                        self.tracker.scan(&x).filter(|h| {
                            h.z >= TrackDetector::found(secs)
                                && h.offset_hz.abs() > 150.0
                                && h.offset_hz.abs() <= self.whistle_params().scan_reach_hz()
                        })
                    } else {
                        None
                    };
                    if let Some(h) = walk {
                        self.log(format!(
                            "🚂 The whistle is {:+.0} Hz further; retuning again",
                            h.offset_hz
                        ));
                        s.phase = SearchPhase::Verify {
                            target: target + h.offset_hz,
                            from: 0,
                            coarse: coarse + h.offset_hz,
                            again: false,
                            hops: hops + 1,
                        };
                    } else {
                        match v {
                            // Freshly retuned onto it, the whistle should be
                            // within the scan detector's ~10 Hz. Further off
                            // means the look likely caught audio from before
                            // the retune (a radio or sound card slow to
                            // follow). Such audio shows the whistle where it
                            // was before the retune, so moving by it would
                            // retune twice: listen here once more first.
                            Some(off) if off.abs() >= RECHECK_HZ && hops < 3 && !again => {
                                self.log(format!(
                                    "🚂 Measured {off:+.0} Hz off after retuning; listening again here"
                                ));
                                s.phase = SearchPhase::Verify {
                                    target,
                                    from: audio::now_count(&self.rx).max(1),
                                    coarse,
                                    again: true,
                                    // Counts as a hop: if the whistle has
                                    // ended meanwhile, wait for its CQ here.
                                    hops: hops + 1,
                                };
                            }
                            // Two looks agree it's that far off: go there and
                            // measure once more before locking.
                            Some(off) if off.abs() >= RECHECK_HZ && hops < 3 => {
                                self.log(format!(
                                    "🚂 Measured {off:+.0} Hz off again; checking there"
                                ));
                                s.phase = SearchPhase::Verify {
                                    target: target + off,
                                    from: 0,
                                    coarse,
                                    again: false,
                                    hops: hops + 1,
                                };
                            }
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
                                s.phase = SearchPhase::WaitCq {
                                    until: Instant::now() + Duration::from_secs(45),
                                };
                            }
                            _ if !again && heard_something => {
                                // Faded, or the pattern straddled the window: listen once more.
                                s.phase = SearchPhase::Verify {
                                    target,
                                    from: audio::now_count(&self.rx).max(1),
                                    coarse,
                                    again: true,
                                    hops,
                                };
                            }
                            // Reached by walking (two looks agreed where it is),
                            // and now silent: the whistle has likely ended and its
                            // CQ is about to follow. Wait for it here.
                            _ if hops > 0 => {
                                self.log(format!("Whistle ended; staying on {:.3} kHz (the scan's estimate) for the CQ…", target / 1e3));
                                s.phase = SearchPhase::WaitCq {
                                    until: Instant::now() + Duration::from_secs(20),
                                };
                            }
                            _ => {
                                self.log(
                                    "Whistle not confirmed after retuning; resuming search".into(),
                                );
                                next_channel = true;
                            }
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
            // A wide-search hand-off has one channel: give up on it, and
            // go back to the scan it interrupted, if any.
            match s.resume.take() {
                Some(mut r) => {
                    let f = r.channels[r.idx];
                    self.log(format!(
                        "Back to the {} search from {:.1} kHz",
                        r.scope,
                        f / 1e3
                    ));
                    r.phase = SearchPhase::Settle {
                        until: Instant::now() + Duration::from_millis(150),
                    };
                    self.tune(f).await?;
                    self.search = Some(*r);
                }
                None => self.set(|g| g.search.active = false),
            }
            return Ok(());
        }
        if next_channel {
            s.idx = (s.idx + 1) % s.channels.len();
            let f = s.channels[s.idx];
            s.phase = SearchPhase::Settle {
                until: Instant::now() + Duration::from_millis(150),
            };
            self.tune(f).await?;
        }
        let note = match &s.phase {
            SearchPhase::Settle { .. } | SearchPhase::Dwell { .. } => "listening".into(),
            SearchPhase::Hold { .. } => "something whistle-like… holding".into(),
            SearchPhase::Verify { .. } => "whistle! measuring".into(),
            SearchPhase::WaitCq { until } => {
                format!(
                    "locked on, waiting for CQ ({}s)",
                    until.saturating_duration_since(Instant::now()).as_secs()
                )
            }
        };
        let view = SearchView {
            active: true,
            scope: s.scope.clone(),
            index: s.idx,
            total: s.channels.len(),
            channel_hz: s.channels[s.idx],
            note,
        };
        self.set(|g| g.search = view);
        self.search = Some(s);
        Ok(())
    }

    // ------------------------------------------------------------------ CQ

    async fn start_cq(&mut self, table: Option<String>) -> anyhow::Result<()> {
        if !crate::protocol::valid_call(&self.mycall()) {
            anyhow::bail!("set your own callsign first (Settings → Station): a CQ identifies you");
        }
        self.stop_search("for CQ");
        self.cancel_pending_join("calling CQ instead");
        if self.peer.is_some() {
            anyhow::bail!("already linked; disconnect first");
        }
        if let Some(t) = &table {
            self.join_table(t.clone(), true).await?;
        }
        self.cq = Some(Cq {
            table: table.clone(),
            cycle: 0,
            phase: CqPhase::Frame {
                until: Instant::now(),
            },
        });
        if self.clear_to_send("CQ") {
            self.begin_cq_cycle().await;
        } else if let Some(cq) = self.cq.as_mut() {
            // step_cq starts it once the channel clears.
            cq.phase = CqPhase::Listen {
                until: Instant::now(),
            };
        }
        self.log(format!(
            "Calling CQ{} on {:.3} kHz: {:.0} s whistle, then CQFRAME",
            table
                .as_ref()
                .map(|t| format!(" for Roundtable \"{t}\""))
                .unwrap_or_default(),
            self.dial() / 1e3,
            self.cfg.whistle_s
        ));
        Ok(())
    }

    async fn stop_cq(&mut self, why: &str) {
        if self.cq.is_none() {
            return;
        }
        if let Some(m) = &self.monitor {
            m.lock().unwrap().clear_tx();
        }
        if let Some(Cq {
            phase: CqPhase::Whistle { .. },
            ..
        }) = &self.cq
            && let Some(o) = &self.output
        {
            o.abort();
        }
        self.cq = None;
        self.ptt(false).await;
        self.set(|g| g.cq = CqView::default());
        self.log(why.into());
    }

    /// A station is connecting to us (Mercury said PENDING): the CQ goes
    /// quiet, so no whistle or CQFRAME is sent on top of Mercury's accept
    /// handshake (that cancels the incoming call). It stays held until the
    /// call connects (the CQ ends) or is cancelled (the CQ carries on).
    async fn hold_cq(&mut self) {
        let Some(cq) = self.cq.as_mut() else { return };
        let whistling = matches!(cq.phase, CqPhase::Whistle { .. });
        // Mercury gives up on an incoming call after about 45 s.
        cq.phase = CqPhase::Listen {
            until: Instant::now() + Duration::from_secs(60),
        };
        if whistling {
            if let Some(o) = &self.output {
                o.abort();
            }
            if let Some(m) = &self.monitor {
                m.lock().unwrap().clear_tx();
            }
            self.ptt(false).await;
        }
        self.log("Incoming call: CQ on hold".into());
    }

    async fn begin_cq_cycle(&mut self) {
        let Some(cq) = self.cq.as_mut() else { return };
        cq.cycle += 1;
        let done = Arc::new(AtomicBool::new(false));
        cq.phase = CqPhase::Whistle { done: done.clone() };
        let secs = self.cfg.whistle_s;
        let params = self.whistle_params();
        let out = self.output.clone();
        let mon = if self.cfg.demo {
            self.monitor.clone()
        } else {
            None
        };
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
        // The next cycle is due (listening over, table traffic out): it
        // still waits for a clear channel.
        let next_due = matches!(
            self.cq.as_ref().map(|c| &c.phase),
            Some(CqPhase::Listen { until }) if Instant::now() >= *until
        ) && self.table_queue.is_empty()
            && Instant::now() >= self.table_busy_until;
        let may_go = next_due && self.clear_to_send("CQ");
        let Some(cq) = self.cq.as_ref() else {
            return Ok(());
        };
        let cycles = self.cfg.cq_cycles;
        let (cycle, table) = (cq.cycle, cq.table.clone());
        let phase = match &cq.phase {
            CqPhase::Whistle { done } if done.load(Ordering::Relaxed) => {
                self.ptt(false).await;
                let me = self.mycall();
                let arq_bw = self.bw.mercury_arq_bw();
                // Mercury answers its own CQFRAME transmission with PENDING
                // (on air) and CANCELPENDING (done), VARA-style: not a call.
                self.own_cq_pending = Some((Instant::now(), false));
                self.tnc()?
                    .command(&format!("CQFRAME {me} {arq_bw}"))
                    .await?;
                if self.cfg.demo
                    && self.real_demo.is_none()
                    && let Some(m) = &self.monitor
                {
                    let m = m.lock().unwrap();
                    let mut seed = 0x5eed_u64 ^ Instant::now().elapsed().as_nanos() as u64 ^ 0x9e37;
                    m.push_tx(&audio::ofdm_burst(m.rate as f64, 3.1, &mut seed));
                }
                let kind = if table.is_some() {
                    CALL_TABLE
                } else {
                    CALL_CHAT
                };
                let call = TableFrame {
                    table: table.as_deref().map(table_id).unwrap_or(0),
                    from: me,
                    body: TableBody::Call {
                        kind,
                        table_name: table.clone().unwrap_or_default(),
                        bw_code: Bandwidth::ALL
                            .iter()
                            .position(|b| *b == self.bw)
                            .unwrap_or(4) as u8,
                        grid: self.cfg.grid.clone(),
                    },
                };
                self.table_tx(call.encode());
                Some(CqPhase::Frame {
                    until: Instant::now() + Duration::from_secs(4),
                })
            }
            CqPhase::Frame { until } if Instant::now() >= *until => Some(CqPhase::Listen {
                until: Instant::now() + Duration::from_secs_f64(self.cfg.cq_listen_s),
            }),
            // The next whistle waits for a Roundtable host's table traffic,
            // so the two never go out on top of each other. (An incoming
            // call holds the CQ through `until`: see hold_cq.)
            CqPhase::Listen { .. } if may_go =>
            {
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
                // The next cycle is due but someone is on the frequency.
                CqPhase::Listen { until } if Instant::now() >= *until && self.chan_busy => {
                    let waited = self.busy_wait.map(|(t, _)| t.elapsed().as_secs()).unwrap_or(0);
                    format!(
                        "waiting for a clear channel ({}s)",
                        BUSY_WAIT_MAX.as_secs().saturating_sub(waited)
                    )
                }
                CqPhase::Listen { until } => {
                    format!(
                        "listening for answers ({}s)",
                        until.saturating_duration_since(Instant::now()).as_secs()
                    )
                }
            };
            let v = CqView {
                active: true,
                cycle: cq.cycle,
                cycles,
                phase,
                table: cq.table.clone(),
            };
            self.set(|g| g.cq = v);
        }
        Ok(())
    }

    // -------------------------------------------------------------- events

    async fn on_tnc(&mut self, e: TncEvent) -> anyhow::Result<()> {
        match e {
            TncEvent::Line(l) => self.on_line(l).await?,
            TncEvent::Data(d) => {
                // A file coming in moves only a piece at a time (one 1 KB
                // piece can take over a minute on a slow mode): its bytes
                // keep the link from counting as idle, though the signal
                // reports alone don't.
                if self.peer.is_some() && !self.incoming.is_empty() && !d.is_empty() {
                    self.last_link_activity = Instant::now();
                }
                let bodies = match self.dec.feed_bodies(&d) {
                    Ok(b) => b,
                    // The stream has no resync point: everything after a
                    // broken length is the middle of a packet. End the link.
                    Err(e) if self.peer.is_some() => {
                        self.log(format!("⚠ The link's data stream is corrupt ({e}); hanging up"));
                        self.tnc()?.command("DISCONNECT").await?;
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                };
                if self.peer.is_none() {
                    // Mercury's data port can beat the CONNECTED line on the
                    // control port: keep these for when it arrives.
                    let now = Instant::now();
                    self.early.extend(bodies.into_iter().map(|b| (now, b)));
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
                let Some((peer, bw)) = parse_connected(&l, &self.mycall()) else {
                    return Ok(());
                };
                self.quiet_watch();
                self.leaving_since = None;
                self.auto_call = None;
                self.busy_detect(false).await;
                self.stop_search("— linked");
                self.link_got_mail = false;
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
                self.mercury_reported = 0;
                self.set(|g| {
                    g.incoming_call = false;
                    g.calling = None;
                    g.link = Some(LinkInfo {
                        peer: peer.clone(),
                        bw,
                        since: Some(Instant::now()),
                        ..Default::default()
                    });
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
                    let m = format!(
                        "Setting up encryption ({}); chat and files wait for it",
                        policy.label()
                    );
                    self.log(m.clone());
                    self.system_line(m);
                }
                // Re-offer files queued before the link came up: those for
                // this station (resuming), and those picked while not linked.
                // A file meant for someone else waits for them.
                let mut waiting = 0;
                for o in self.outgoing.iter_mut() {
                    match &o.to {
                        None => o.to = Some(peer.clone()),
                        Some(t) if *t == peer => {}
                        Some(_) => {
                            waiting += 1;
                            continue;
                        }
                    }
                    self.hi_queue.push_back(o.offer());
                }
                if waiting > 0 {
                    self.log(format!("{waiting} file(s) wait for the station they were sent to"));
                }
                // Depot mail for this station goes across now.
                self.last_link_activity = Instant::now();
                self.queue_mail_for(&peer);
                let they_called = l
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|c| !c.eq_ignore_ascii_case(&self.mycall()));
                if self.away && they_called {
                    let text = format!("[{}] {}", self.mycall(), self.cfg.away_message);
                    self.hi_queue.push_back(Link::Chat {
                        id: 0,
                        text: text.clone(),
                    });
                    self.system_line(format!("(away) sent: {text}"));
                }
                self.pump().await?;
                // Only what came just before this CONNECTED belongs to this
                // link: older bytes are the last link's leftovers (its
                // replies can arrive after its DISCONNECTED).
                let early: Vec<Vec<u8>> = std::mem::take(&mut self.early)
                    .into_iter()
                    .filter(|(at, _)| at.elapsed() < Duration::from_secs(3))
                    .map(|(_, b)| b)
                    .collect();
                self.on_bodies(early).await?;
            }
            "DISCONNECTED" => {
                // The other modem can keep sending teardown bursts for a
                // while (up to ~25 s seen on the bench).
                self.quiet_watch_for(30.0);
                self.link_auto = None;
                self.leaving_since = None;
                self.link_ended_at = Some(Instant::now());
                // Forget the session keys (they wipe themselves on drop).
                self.e2e = None;
                self.ctl_queue.clear();
                self.early.clear();
                self.dec = LinkDecoder::default();
                self.mercury_reported = 0;
                self.hangup_at = None;
                self.bye_at = None;
                self.auto_call = None;
                self.busy_detect(true).await;
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
                    // Stay a few seconds, so Mercury's disconnect finishes
                    // here and the other station hears it too.
                    self.qsy_home = Some((home, at, Instant::now()));
                }
                // Library files were for that station only.
                let lib = std::mem::take(&mut self.library_out);
                self.outgoing.retain(|o| !lib.contains(&o.fid));
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
                        let library = t.dir == Dir::Out && lib.contains(&t.fid);
                        if matches!(
                            t.state,
                            XferState::Sending | XferState::Receiving | XferState::Offered
                        ) {
                            t.state = XferState::Failed(if library {
                                "link ended".into()
                            } else {
                                "link ended — will resume next time".into()
                            });
                        } else if t.state == XferState::WaitingForYou {
                            // Nothing to answer any more: the sender offers it
                            // again on the next link.
                            t.state = XferState::Failed("link ended before you answered".into());
                        }
                    }
                });
            }
            "PENDING" => {
                // Our own CQFRAME going out (see step_cq), not a station
                // calling us.
                if let Some((at, seen)) = self.own_cq_pending.as_mut()
                    && !*seen
                    && at.elapsed() < Duration::from_secs(10)
                {
                    *seen = true;
                    return Ok(());
                }
                self.quiet_watch();
                self.set(|g| g.incoming_call = true);
                if self.cq.is_some() {
                    self.hold_cq().await;
                } else if let Some(s) = self.search.as_mut() {
                    // Hold the search: a connect is arriving here.
                    s.phase = SearchPhase::WaitCq {
                        until: Instant::now() + Duration::from_secs(60),
                    };
                }
            }
            "CANCELPENDING" => {
                // Our own CQFRAME is done.
                if let Some((_, true)) = self.own_cq_pending {
                    self.own_cq_pending = None;
                    return Ok(());
                }
                self.set(|g| g.incoming_call = false);
                // A held CQ carries on with its next cycle.
                if let Some(Cq {
                    phase: CqPhase::Listen { until },
                    ..
                }) = self.cq.as_mut()
                {
                    *until = Instant::now();
                }
            }
            "CQFRAME" => {
                let call = l.split_whitespace().nth(1).unwrap_or("?").to_string();
                if !crate::protocol::valid_call(&call) {
                    return Ok(());
                }
                let dial = self.dial();
                self.log(format!("📣 CQ from {call} on {:.3} kHz", dial / 1e3));
                self.add_heard(&call, "CQ".into(), None);
                self.heard_station(&call).await?;
                let locked = matches!(
                    self.search.as_ref().map(|s| &s.phase),
                    Some(SearchPhase::WaitCq { .. })
                );
                if locked {
                    self.stop_search(&format!("found {call}"));
                    if self.cfg.auto_answer && self.peer.is_none() {
                        // Not at once: the CQ station keys straight on with
                        // its CALL packet (~3.7 s), and a CONNECT now puts
                        // Mercury's first fast CALL on top of it, unheard.
                        // The CONNECT goes when that packet has been decoded,
                        // or after ANSWER_AFTER_CQ_S (`step_answer`).
                        self.answer_at = Some((
                            call.clone(),
                            Instant::now() + Duration::from_secs_f64(ANSWER_AFTER_CQ_S),
                        ));
                        // Calling, like a Connect by hand: nothing else may
                        // retune, beacon or connect while Mercury links up.
                        self.set(|g| g.calling = Some(call.clone()));
                        self.quiet_watch();
                        self.log(format!("Auto-answering {call} once its CQ is off the air"));
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
                // Mercury reports its backlog after every read of the data
                // socket (128 bytes at a time), so a count can trail what's
                // been written and is still in the socket. Taken as the whole
                // backlog, every low count let another chunk go, and whole
                // files were handed over at once (no cancel, chat behind
                // 8 KB, wrong progress). So bytes are counted when written,
                // and only Mercury's decreases (bytes sent) are taken off.
                let n: usize = l[6..].trim().parse().unwrap_or(0);
                let now = Instant::now();
                let prev = std::mem::replace(&mut self.mercury_reported, n);
                if n < prev {
                    let sent = prev - n;
                    self.mercury_buffer = self.mercury_buffer.saturating_sub(sent);
                    self.est.on_drain(now, sent);
                }
                self.mercury_buffer = self.mercury_buffer.max(n);
                // Drift guard: empty, and nothing written for a moment.
                if n == 0 && self.last_data_write.elapsed() > Duration::from_secs(2) {
                    self.mercury_buffer = 0;
                }
                if self.mercury_buffer == 0 {
                    self.est.on_idle();
                }
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
            // Mercury's busy detector (on while not linked, see busy_detect).
            "BUSY" => {
                let on = l.split_whitespace().nth(1) == Some("ON");
                self.chan_busy = on;
                self.set(|g| g.channel_busy = on);
            }
            "SN" => {
                self.est.on_snr_line(&l);
                let v = self.est.snr_db;
                self.last_sn = v.map(|s| (Instant::now(), s));
                self.set(|g| g.snr_db = v);
            }
            // Mercury answers VERSION as VARA does ("VARA version 4.9.0
            // registered") so VARA programs accept it.
            _ if l.to_ascii_lowercase().contains("mercury")
                || word == "VERSION"
                || (word == "VARA" && l.to_ascii_lowercase().contains("version")) =>
            {
                let v = version_text(&l);
                self.set(|g| g.modem_version = Some(v));
            }
            _ => {}
        }
        Ok(())
    }

    fn system_line(&self, text: String) {
        self.set(|g| {
            g.chat.push(ChatLine {
                at: Clock::now(),
                who: Who::System,
                from: String::new(),
                text,
                id: 0,
                delivered: true,
            })
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
            Notice::Plain { why } => (
                E2eView::Plain { why: why.clone() },
                format!("🔓 Not encrypted: {why}"),
            ),
            Notice::Skipped { why } => {
                self.log(format!("⚠ Skipped a packet from {peer} that didn't make sense ({why})"));
                return;
            }
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
        // Tell the other station why, in its own terms.
        let text = if why.contains("requires it") {
            "this station requires encryption: turn it on (Settings → Encryption) to link"
        } else {
            "encryption problem, disconnecting"
        };
        self.ctl_queue.push_back(Link::Bye { text: text.into() }.encode());
        self.pump().await?;
        self.disconnect_now().await?;
        Ok(())
    }

    fn add_heard(&self, call: &str, what: String, table: Option<String>) {
        let dial = self.dial();
        self.set(|g| {
            g.heard.retain(|h| !(h.call == call && h.what == what));
            g.heard.insert(
                0,
                Heard {
                    at: Clock::now(),
                    call: call.to_string(),
                    dial_hz: dial,
                    what,
                    table,
                },
            );
            g.heard.truncate(30);
        });
    }

    async fn on_link(&mut self, p: Link) -> anyhow::Result<()> {
        let peer = self.peer.clone().unwrap_or_default();
        // Real traffic (a file piece too) keeps the link from counting as
        // idle; the signal reports both stations send every 30 s don't.
        if !matches!(p, Link::Report { .. }) {
            self.last_link_activity = Instant::now();
        }
        match p {
            Link::Qsy { id, dial_hz, why } => {
                let hz = dial_hz as f64;
                let why = if why.is_empty() {
                    String::new()
                } else {
                    format!(" ({why})")
                };
                match self.qsy_verdict(hz) {
                    Ok(()) => {
                        self.hi_queue.push_back(Link::QsyAnswer {
                            id,
                            accept: true,
                            reason: String::new(),
                        });
                        self.qsy_move = Some((hz, Instant::now(), false));
                        self.log(format!(
                            "↔ {peer} asked to QSY to {:.3} kHz{why}: agreed",
                            hz / 1e3
                        ));
                        self.system_line(format!("↔ {peer} asked to move to {:.3} kHz: agreed. Back here when the link ends.", hz / 1e3));
                    }
                    Err(reason) => {
                        self.hi_queue.push_back(Link::QsyAnswer {
                            id,
                            accept: false,
                            reason: reason.clone(),
                        });
                        self.log(format!(
                            "↔ {peer} asked to QSY to {:.3} kHz{why}: declined ({reason})",
                            hz / 1e3
                        ));
                        self.system_line(format!(
                            "↔ {peer} asked to move to {:.3} kHz: declined ({reason})",
                            hz / 1e3
                        ));
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
                    self.log(format!(
                        "📚 {peer} browsed the library ({}): {}",
                        if path.is_empty() { "top" } else { &path },
                        if note.is_empty() {
                            format!("{} entries", entries.len())
                        } else {
                            note.clone()
                        }
                    ));
                }
                self.hi_queue.push_back(ans);
            }
            Link::ListAns {
                path,
                entries,
                note,
                ..
            } => self.set(|g| {
                g.library = Some(LibView {
                    path,
                    entries,
                    note,
                })
            }),
            // The reply is offered under the asker's id. One that matches a
            // file already going out would mix the two up.
            Link::Fetch { id, .. } if self.outgoing.iter().any(|o| o.fid == id) => {
                self.hi_queue.push_back(Link::FetchAnswer {
                    id,
                    ok: false,
                    reason: "busy: ask again".into(),
                });
            }
            Link::Fetch { id, path } => match self.library_file(&path) {
                Ok((name, data)) => {
                    // Only for this link: never offered to whoever links next.
                    self.library_out.insert(id);
                    self.log(format!("📚 Sending {path} from the library to {peer}"));
                    self.hi_queue.push_back(Link::FetchAnswer {
                        id,
                        ok: true,
                        reason: String::new(),
                    });
                    let len = data.len() as u64;
                    self.offer_data(id, name, data, false, 1, len).await?;
                }
                Err(e) => {
                    self.log(format!("📚 {peer} asked for {path}: refused ({e})"));
                    self.hi_queue.push_back(Link::FetchAnswer {
                        id,
                        ok: false,
                        reason: e.to_string(),
                    });
                }
            },
            Link::FetchAnswer { id, ok, reason } => {
                if !ok {
                    self.wanted.remove(&id);
                    self.system_line(format!("📚 {peer} couldn't send that: {reason}"));
                }
            }
            Link::Hello {
                name, grid, app, ..
            } => {
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
                    let m = crate::depot::Mail {
                        id: crate::depot::new_id(),
                        from: peer.clone(),
                        to: self.mycall(),
                        created: crate::depot::now_s(),
                        text: text.clone(),
                        read: false,
                        delivered_at: None,
                        via: None,
                        hops: 0,
                        handed: None,
                    };
                    self.depot.arrive(m);
                    self.sync_depot();
                }
                self.hi_queue.push_back(Link::Received { id });
                self.set(|g| {
                    g.chat.push(ChatLine {
                        at: Clock::now(),
                        who: Who::Them,
                        from: peer,
                        text,
                        id,
                        delivered: true,
                    })
                });
                self.pump().await?;
            }
            Link::Received { id } => self.set(|g| {
                for c in g.chat.iter_mut().filter(|c| c.who == Who::Me && c.id == id) {
                    c.delivered = true;
                }
            }),
            Link::Offer {
                fid,
                name,
                size,
                sha,
                zipped,
                files,
            } => {
                let inc =
                    Incoming::new(&self.cfg.download_dir, fid, name.clone(), size, sha, zipped);
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
                    if have > 0 {
                        format!(
                            ", {} already here — will resume",
                            modes::human_bytes(have as usize)
                        )
                    } else {
                        String::new()
                    },
                    modes::human_duration(eta)
                ));
                // A library file we asked for: take it.
                if self.wanted.remove(&fid) {
                    self.answer_offer(fid, true).await?;
                }
            }
            Link::Answer {
                fid,
                accept,
                resume_from,
            } => {
                if let Some(o) = self.outgoing.iter_mut().find(|o| o.fid == fid) {
                    if accept {
                        o.next = Some(resume_from.min(o.data.len() as u64));
                        if resume_from > 0 {
                            self.log(format!(
                                "{peer} already has {} — resuming",
                                modes::human_bytes(resume_from as usize)
                            ));
                        }
                        self.xfer(Dir::Out, fid, |t| {
                            t.state = XferState::Sending;
                            t.done = resume_from;
                            t.started = Some(Instant::now());
                        });
                    } else {
                        self.outgoing.retain(|o| o.fid != fid);
                        self.xfer(Dir::Out, fid, |t| t.state = XferState::Declined);
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
                    progress = Some((have, inc.size.saturating_sub(have), inc.complete()));
                }
                if let Some((have, left, complete)) = progress {
                    let eta = self.est.eta_s(left as usize, self.bw);
                    self.xfer(Dir::In, fid, |t| {
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
                            self.xfer(Dir::In, fid, |t| t.state = XferState::Done(Some(path)));
                        }
                        Err(e) => {
                            self.hi_queue.push_back(Link::Done { fid, ok: false });
                            self.xfer(Dir::In, fid, |t| t.state = XferState::Failed(e.to_string()));
                        }
                    }
                    self.pump().await?;
                }
            }
            Link::Done { fid, ok } => {
                self.outgoing.retain(|o| o.fid != fid);
                self.xfer(Dir::Out, fid, |t| {
                    // Cancelled here before their DONE arrived: stays so.
                    if matches!(t.state, XferState::Failed(_)) {
                        return;
                    }
                    t.state = if ok {
                        XferState::Done(None)
                    } else {
                        XferState::Failed("checksum failed at the other end".into())
                    };
                    t.done = t.size;
                });
            }
            Link::Cancel { fid } => {
                // Either side may cancel, so the fid is the sender's (a file
                // coming here) or ours (a file going there). A file coming
                // here takes precedence: our own numbers start at a random
                // point, so the two hardly ever meet.
                let dir = if let Some(i) = self.incoming.remove(&fid) {
                    i.save_partial();
                    Dir::In
                } else {
                    self.outgoing.retain(|o| o.fid != fid);
                    Dir::Out
                };
                self.xfer(dir, fid, |t| {
                    t.state = XferState::Failed(format!("cancelled by {peer}"))
                });
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
                    g.chat.push(ChatLine {
                        at: Clock::now(),
                        who: Who::Them,
                        from: peer,
                        text,
                        id: 0,
                        delivered: true,
                    })
                });
                // They're leaving and will disconnect (Mercury's disconnect
                // takes ~20 s to arrive). If it never does (say, after a
                // QSY), hang up from this end after 30 s; sooner would
                // collide with theirs.
                if self.bye_at.is_none() {
                    self.bye_at = Some(Instant::now());
                }
            }
            // Handled by the e2e channel before we get here.
            Link::Mail {
                id,
                from,
                to,
                created,
                hops,
                text,
            } => {
                self.last_link_activity = Instant::now();
                self.link_got_mail = true;
                let m = crate::depot::Mail {
                    id,
                    from: from.clone(),
                    to: to.clone(),
                    created,
                    text,
                    read: false,
                    delivered_at: None,
                    via: None,
                    hops,
                    handed: None,
                };
                if to.eq_ignore_ascii_case(&self.mycall()) {
                    self.hi_queue.push_back(Link::MailAck { id });
                    if self.depot.arrive(m) {
                        self.log(format!("📬 Mail from {from} arrived in your Depot"));
                        self.system_line(format!(
                            "📬 Mail from {from} is in your Depot (Arrivals)"
                        ));
                    }
                    self.set(|g| g.mail_alerts.retain(|a| !(a.call == from && !a.ours)));
                } else if self.cfg.relay {
                    // Store and forward: carry it on toward its station.
                    let fresh = self.depot.take_in_transit(
                        crate::depot::Mail {
                            hops: hops.saturating_add(1),
                            ..m
                        },
                        &peer,
                    );
                    let already = self
                        .depot
                        .transit
                        .iter()
                        .chain(self.depot.delivered.iter())
                        .any(|x| x.id == id);
                    if fresh || already {
                        self.hi_queue.push_back(Link::MailAck { id });
                    }
                    if fresh {
                        self.log(format!(
                            "🚚 Carrying mail from {from} for {to} (In Transit)"
                        ));
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
                    if m.to.eq_ignore_ascii_case(&peer) {
                        self.log(format!("✓ Mail delivered to {} (now in Delivered)", m.to));
                    } else {
                        self.log(format!(
                            "✓ Mail for {} handed to {peer}, who carries it on (now in Delivered)",
                            m.to
                        ));
                    }
                    let left = self.depot.waiting_for(&m.to).len();
                    if left == 0 {
                        self.set(|g| g.mail_alerts.retain(|a| !(a.call == m.to && a.ours)));
                    }
                }
                self.sync_depot();
            }
            Link::KeyCommit { .. } | Link::KeyX { .. } | Link::Sealed { .. } | Link::Unknown(_) => {
            }
        }
        Ok(())
    }

    /// Update the newest transfer row with this direction and fid. Both are
    /// needed: incoming rows carry the sender's numbers, outgoing rows ours.
    fn xfer(&self, dir: Dir, fid: u64, f: impl FnOnce(&mut Transfer)) {
        let mut g = self.st.lock().unwrap();
        if let Some(t) = g
            .transfers
            .iter_mut()
            .rev()
            .find(|t| t.fid == fid && t.dir == dir)
        {
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
            (
                z.name.clone(),
                z.data,
                true,
                z.files as u64,
                z.original_bytes,
            )
        } else {
            let data = std::fs::read(&paths[0])?;
            let len = data.len() as u64;
            (
                paths[0].file_name().unwrap().to_string_lossy().into_owned(),
                data,
                false,
                1,
                len,
            )
        };
        let fid = self.next_fid;
        self.next_fid += 1;
        self.offer_data(fid, name, data, zipped, files, original)
            .await
    }

    /// Offer data to the linked station (or as soon as one is linked).
    async fn offer_data(
        &mut self,
        fid: u64,
        name: String,
        data: Vec<u8>,
        zipped: bool,
        files: u64,
        original: u64,
    ) -> anyhow::Result<()> {
        let mut out = Outgoing::new(fid, name.clone(), data, zipped, files);
        out.to = self.peer.clone();
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
            if self.peer.is_none() {
                " — will offer when linked"
            } else {
                ""
            }
        ));
        self.pump().await
    }

    async fn answer_offer(&mut self, fid: u64, accept: bool) -> anyhow::Result<()> {
        // Only an offer on the current link can be answered; an answer queued
        // now would otherwise go out on whatever link comes next.
        let Some(have) = self
            .incoming
            .get(&fid)
            .map(|i| i.have())
            .filter(|_| self.peer.is_some())
        else {
            self.xfer(Dir::In, fid, |t| {
                if t.state == XferState::WaitingForYou {
                    t.state = XferState::Failed("link ended before you answered".into());
                }
            });
            anyhow::bail!("that offer is no longer open: the link it came on has ended");
        };
        if !accept {
            self.incoming.remove(&fid);
        }
        self.hi_queue.push_back(Link::Answer {
            fid,
            accept,
            resume_from: have,
        });
        self.xfer(Dir::In, fid, |t| {
            t.state = if accept {
                XferState::Receiving
            } else {
                XferState::Declined
            };
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
            self.last_data_write = Instant::now();
        }
        // Chat and files wait until the link is plain or keys are agreed.
        while let Some(p) = self.hi_queue.front() {
            let Some(bytes) = self.e2e.as_mut().and_then(|c| c.wrap(p)) else {
                return Ok(());
            };
            self.hi_queue.pop_front();
            self.mercury_buffer += bytes.len();
            self.tnc()?.send_data(&bytes).await?;
            self.last_data_write = Instant::now();
        }
        if !self.e2e.as_ref().is_some_and(|c| c.ready()) {
            return Ok(());
        }
        while self.mercury_buffer < 2 * CHUNK {
            let Some(o) = self
                .outgoing
                .iter_mut()
                .find(|o| o.next.is_some() && !o.finished_sending())
            else {
                break;
            };
            let fid = o.fid;
            let Some(chunk) = o.next_chunk() else { break };
            let sent = o.next.unwrap_or(0);
            let total = o.data.len();
            let Some(bytes) = self.e2e.as_mut().and_then(|c| c.wrap(&chunk)) else {
                break;
            };
            self.mercury_buffer += bytes.len();
            self.tnc()?.send_data(&bytes).await?;
            self.last_data_write = Instant::now();
            self.last_link_activity = Instant::now();
            let eta = self
                .est
                .eta_s(total - sent as usize + self.mercury_buffer, self.bw);
            self.xfer(Dir::Out, fid, |t| {
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

    /// Join here, now if Mercury is free. While a link is closing (or
    /// Mercury still says BUSY after it), the join waits and is retried,
    /// for up to `JOIN_WAIT_S`.
    async fn request_join(&mut self, name: String) -> anyhow::Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("give the Roundtable a name");
        }
        if let Some(peer) = self.peer.clone() {
            if !(self.leaving() || self.bye_at.is_some()) {
                anyhow::bail!("still linked with {peer}: end the link first, then join");
            }
            self.queue_join(name, "the link is still closing");
            return Ok(());
        }
        match self.join_table(name.clone(), false).await {
            Err(e) if crate::tnc::is_busy(&e) => {
                self.queue_join(name, "Mercury is still busy closing the last link");
                Ok(())
            }
            r => r,
        }
    }

    fn queue_join(&mut self, name: String, why: &str) {
        let now = Instant::now();
        self.pending_join = Some((name.clone(), now, now));
        self.set(|g| g.table_pending = Some(name.clone()));
        let m = format!("Joining Roundtable \"{name}\" as soon as Mercury is free ({why})");
        self.log(m.clone());
        self.system_line(m);
    }

    /// Send a waiting auto-answer's CONNECT once the CQ is off the air,
    /// unless the call was cancelled or another link came first.
    async fn step_answer(&mut self) -> anyhow::Result<()> {
        let Some((call, at)) = self.answer_at.clone() else {
            return Ok(());
        };
        if Instant::now() < at {
            return Ok(());
        }
        self.answer_at = None;
        let still = self.st.lock().unwrap().calling.as_deref() == Some(call.as_str());
        if still && self.peer.is_none() {
            let cmd = format!("CONNECT {} {call}", self.mycall());
            let sent = match self.tnc() {
                Ok(t) => t.command(&cmd).await.map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = sent {
                // Not calling after all: "Calling" would otherwise stay up
                // (and keep the watch, beacons and schedule off) for good.
                self.set(|g| g.calling = None);
                return Err(e.context(format!("couldn't answer {call}")));
            }
            self.auto_call = Some(Instant::now());
            self.log(format!("Calling {call}…"));
        }
        Ok(())
    }

    /// Try a waiting Roundtable join again (every 2 s once unlinked).
    /// Drop a waiting Roundtable join: the operator has moved on (a call,
    /// a CQ of their own), and a late join would cut into it.
    fn cancel_pending_join(&mut self, why: &str) {
        if let Some((name, ..)) = self.pending_join.take() {
            self.set(|g| g.table_pending = None);
            self.log(format!("Roundtable join \"{name}\" cancelled ({why})"));
        }
    }

    async fn step_pending_join(&mut self) -> anyhow::Result<()> {
        let Some((name, asked, last)) = self.pending_join.clone() else {
            return Ok(());
        };
        if asked.elapsed() > Duration::from_secs_f64(JOIN_WAIT_S) {
            self.pending_join = None;
            self.set(|g| g.table_pending = None);
            let m = format!(
                "Couldn't join Roundtable \"{name}\": Mercury stayed busy for {JOIN_WAIT_S:.0} s. Try Join here again."
            );
            self.log(format!("⚠ {m}"));
            self.system_line(m);
            return Ok(());
        }
        let calling = self.st.lock().unwrap().calling.is_some();
        if self.peer.is_some() || calling || last.elapsed() < Duration::from_secs(2) {
            return Ok(());
        }
        self.pending_join = Some((name.clone(), asked, Instant::now()));
        match self.join_table(name, false).await {
            Ok(()) => {
                self.pending_join = None;
                self.set(|g| g.table_pending = None);
            }
            Err(e) if crate::tnc::is_busy(&e) => {}
            Err(e) => {
                self.pending_join = None;
                self.set(|g| g.table_pending = None);
                return Err(e);
            }
        }
        Ok(())
    }

    async fn join_table(&mut self, name: String, host: bool) -> anyhow::Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("give the Roundtable a name");
        }
        // A Roundtable switches Mercury to broadcast modes, which would cut
        // a 1:1 link off mid-sentence.
        if self.peer.is_some() {
            anyhow::bail!("you're in a 1:1 link: disconnect first, then join the Roundtable");
        }
        let id = table_id(&name);
        self.restore_calling_mode = false;
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
                host_call: host.then(|| self.mycall()),
                floor: None,
                lines: vec![],
                sending_eta_s: 0.0,
            })
        });
        self.join_window = None;
        self.last_join_window = Instant::now();
        self.last_table_report = Instant::now();
        self.last_table_heard = Instant::now();
        self.last_table_sent = Instant::now();
        self.table_announced = Some(mode);
        self.table_unsynced = !host;
        self.expect_slowdown = None;
        if !host {
            self.host_snr = None;
            let me = self.mycall();
            let snr = self.est.snr_db.unwrap_or(0.0).round() as i8;
            self.table_tx(
                TableFrame {
                    table: id,
                    from: me,
                    body: TableBody::Join {
                        table_name: name.clone(),
                        snr_of_host: snr,
                    },
                }
                .encode(),
            );
            self.log(format!(
                "Joined Roundtable \"{name}\" on {:.3} kHz",
                self.dial() / 1e3
            ));
        } else {
            self.log(format!(
                "Hosting Roundtable \"{name}\" on {:.3} kHz",
                self.dial() / 1e3
            ));
        }
        Ok(())
    }

    fn table_tx(&mut self, frame: Vec<u8>) {
        self.table_queue.push_back(frame);
    }

    fn table_say(&mut self, text: String) -> anyhow::Result<()> {
        let (id, mode) = {
            let g = self.st.lock().unwrap();
            let t = g
                .table
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("not at a Roundtable: join or host one first"))?;
            (t.id, t.mode)
        };
        self.table_msg_id += 1;
        let me = self.mycall();
        let frames = table_text_frames(id, &me, self.table_msg_id, &text, mode.broadcast_bytes())?;
        let eta = frames.len() as f64 * mode.frame_s;
        self.table_sent.insert(self.table_msg_id, frames.clone());
        // Repeats are asked for within minutes: keep the last 64 messages.
        let oldest = self.table_msg_id.saturating_sub(64);
        self.table_sent.retain(|id, _| *id > oldest);
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
            self.log(format!(
                "Roundtable speed → {} ({} bps)",
                m.name, m.payload_bps
            ));
        }
        if Instant::now() < self.table_busy_until || self.tnc.is_none() {
            return Ok(());
        }
        // Our own CQ whistle is on the air: a table frame now would be sent
        // on top of it and nobody would copy either.
        if matches!(
            self.cq.as_ref().map(|c| &c.phase),
            Some(CqPhase::Whistle { .. })
        ) {
            return Ok(());
        }
        // Our CQFRAME is going out (until Mercury's CANCELPENDING): a frame
        // handed over now would go out in the CQFRAME's mode, cut short, and
        // nobody could decode it.
        if self
            .own_cq_pending
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(10))
        {
            return Ok(());
        }
        let Some(f) = self.table_queue.pop_front() else {
            if self.restore_calling_mode
                && self.peer.is_none()
                && self.st.lock().unwrap().table.is_none()
            {
                self.restore_calling_mode = false;
                let calling = modes::mode_by_name("DATAC3").expect("DATAC3");
                self.tnc()?
                    .command(&format!("MODE {}", calling.index))
                    .await?;
                self.set(|g| g.listen_mode = Some(calling));
            }
            self.set(|g| {
                if let Some(t) = g.table.as_mut() {
                    t.sending_eta_s = 0.0;
                }
            });
            return Ok(());
        };
        let mode = self
            .st
            .lock()
            .unwrap()
            .listen_mode
            .unwrap_or(&modes::MODES[4]);
        self.tnc()?.send_broadcast(&f).await?;
        self.last_table_sent = Instant::now();
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
        let Some(fr) = TableFrame::decode(f) else {
            return Ok(());
        };
        match &fr.body {
            TableBody::Call {
                kind, table_name, ..
            } => {
                // The CALL packet that follows a CQFRAME: the air is about
                // to be free for a waiting auto-answer.
                if let Some((call, at)) = self.answer_at.as_mut()
                    && *call == fr.from
                {
                    *at = (*at).min(Instant::now() + Duration::from_millis(300));
                }
                let what = if *kind == CALL_TABLE {
                    format!("Roundtable \"{table_name}\"")
                } else {
                    "CQ (chat)".into()
                };
                self.add_heard(
                    &fr.from,
                    what,
                    (*kind == CALL_TABLE).then(|| table_name.clone()),
                );
                // At this table: its CALL comes from the host (also during a
                // join window).
                if *kind == CALL_TABLE && self.table_id() == Some(fr.table) {
                    let from = fr.from.clone();
                    self.set(|g| {
                        if let Some(t) = g.table.as_mut()
                            && !t.host
                        {
                            t.host_call = Some(from);
                        }
                    });
                    if let Some((at, snr)) = self.last_sn
                        && at.elapsed() < Duration::from_secs(2)
                    {
                        self.host_snr = Some(snr);
                    }
                    self.last_table_heard = Instant::now();
                    // A join window: a member the host may not know yet
                    // (its JOIN went out while the table ran faster, or it
                    // lost the table) says so now, while the host listens
                    // in DATAC3.
                    let is_member = self.st.lock().unwrap().table.as_ref().is_some_and(|t| !t.host);
                    if is_member
                        && self.table_unsynced
                        && self.last_table_report.elapsed() >= Duration::from_secs(15)
                    {
                        self.send_table_report();
                    }
                }
                self.heard_station(&fr.from).await?;
                return Ok(());
            }
            TableBody::Beacon {
                grid,
                away,
                mail_for,
            } => {
                let me = self.mycall();
                let for_me = mail_for.iter().any(|c| c.eq_ignore_ascii_case(&me));
                let what = format!(
                    "Station ID {}{}{}",
                    grid,
                    if *away { " · away" } else { "" },
                    if mail_for.is_empty() {
                        String::new()
                    } else {
                        format!(" · mail for {}", mail_for.join(" "))
                    }
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
                    self.they_have_mail(&fr.from.clone(), *count as usize)
                        .await?;
                }
                self.heard_station(&fr.from).await?;
                return Ok(());
            }
            _ => {}
        }
        let Some(my_id) = self.table_id() else {
            return Ok(());
        };
        if fr.table != my_id {
            return Ok(());
        }
        self.last_table_heard = Instant::now();
        let from = fr.from.clone();
        // Only the host sends SPEED. A member reports how well it hears the
        // host, so it keeps the SN Mercury gave for the host's own frames.
        if matches!(fr.body, TableBody::Speed { .. }) {
            self.set(|g| {
                if let Some(t) = g.table.as_mut()
                    && !t.host
                {
                    t.host_call = Some(from.clone());
                }
            });
        }
        let is_host = self
            .st
            .lock()
            .unwrap()
            .table
            .as_ref()
            .and_then(|t| t.host_call.clone())
            .is_some_and(|h| h == from);
        if is_host
            && let Some((at, snr)) = self.last_sn
            && at.elapsed() < Duration::from_secs(2)
        {
            self.host_snr = Some(snr);
        }
        match fr.body {
            TableBody::Text {
                id,
                part,
                total,
                data,
            } => {
                if let Some(text) = self.table_asm.add(&from, id, part, total, &data) {
                    self.set(|g| {
                        if let Some(t) = g.table.as_mut() {
                            t.lines.push((Clock::now(), from.clone(), text));
                            t.members.entry(from).or_default().last = Some(Instant::now());
                        }
                    });
                }
            }
            // A JOIN, or a member's periodic report (the same frame).
            TableBody::Join { snr_of_host, .. } => {
                self.set(|g| {
                    if let Some(t) = g.table.as_mut() {
                        let new = !t.members.contains_key(&from);
                        let m = t.members.entry(from.clone()).or_default();
                        m.snr_of_host = Some(snr_of_host);
                        m.last = Some(Instant::now());
                        if new {
                            t.lines.push((
                                Clock::now(),
                                String::new(),
                                format!("{from} joined (hears the host at {snr_of_host:+} dB)"),
                            ));
                        }
                    }
                });
                self.ramp().await?;
            }
            // A speed change, or the host's heartbeat (the same speed).
            TableBody::Speed { mode_index, last_id } => {
                // Its newest message: one we never heard is asked for.
                self.table_asm.note_latest(&from, last_id, true);
                if let Some(m) = modes::mode_by_index(mode_index) {
                    self.table_unsynced = false;
                    self.expect_slowdown = None;
                    self.table_announced = Some(m);
                    let now = self.st.lock().unwrap().table.as_ref().map(|t| t.mode.index);
                    if now != Some(m.index) {
                        self.apply_table_mode(m).await?;
                        self.log(format!(
                            "Roundtable speed → {} ({} bps), set by {from}",
                            m.name, m.payload_bps
                        ));
                    }
                }
            }
            TableBody::Again { of, id, missing } => {
                if of == self.mycall()
                    && let Some(frames) = self.table_sent.get(&id).cloned()
                {
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
                    t.lines.push((
                        Clock::now(),
                        String::new(),
                        format!("{from} passes the floor to {to}"),
                    ));
                }
            }),
            TableBody::Leave => self.set(|g| {
                if let Some(t) = g.table.as_mut() {
                    t.members.remove(&from);
                    t.lines
                        .push((Clock::now(), String::new(), format!("{from} left")));
                }
            }),
            TableBody::Call { .. } | TableBody::Beacon { .. } | TableBody::Notice { .. } => {}
        }
        Ok(())
    }

    /// Roundtable timers. A member re-reports how well it hears the host
    /// (every TABLE_REPORT_EVERY). The host of a table that has sped up past
    /// DATAC3 opens a join window every JOIN_WINDOW_EVERY: the table drops to
    /// DATAC3 for JOIN_WINDOW_FOR and sends its CALL, so latecomers (who call
    /// and listen in DATAC3) can join; then it goes back to speed.
    async fn step_table_timers(&mut self) -> anyhow::Result<()> {
        let Some((host, mode, id, name)) = self
            .st
            .lock()
            .unwrap()
            .table
            .as_ref()
            .map(|t| (t.host, t.mode, t.id, t.name.clone()))
        else {
            self.join_window = None;
            return Ok(());
        };
        if self.tnc.is_none() || self.pending_speed.is_some() {
            return Ok(());
        }
        let me = self.mycall();
        let calling = modes::mode_by_name("DATAC3").expect("DATAC3");
        if !host {
            if self.last_table_report.elapsed() >= TABLE_REPORT_EVERY {
                self.send_table_report();
            }
            // Our report said we can't copy this speed and no slow-down
            // came (it was announced at this speed: we missed it).
            if self.expect_slowdown.is_some_and(|t| t.elapsed() >= SLOWDOWN_WAIT) {
                self.expect_slowdown = None;
                if mode.payload_bps > calling.payload_bps {
                    self.log(format!(
                        "Roundtable: {} is too fast to copy here and no slow-down was heard; listening in DATAC3",
                        mode.name
                    ));
                    self.table_lost_to(calling).await?;
                }
            }
            // Nothing heard of the table for a long while: it moved without
            // us. Try the speed it last announced, DATAC3 (join windows),
            // then the most robust mode, in turn.
            if self.last_table_heard.elapsed() >= TABLE_LOST_AFTER {
                let robust = self
                    .bw
                    .modes()
                    .iter()
                    .min_by(|a, b| a.min_snr_db.partial_cmp(&b.min_snr_db).unwrap())
                    .copied()
                    .unwrap_or(calling);
                let next = match self.table_announced {
                    Some(a) if a.index != mode.index => a,
                    _ if mode.index != calling.index => calling,
                    _ => robust,
                };
                self.log(format!(
                    "Roundtable: nothing heard for {} s at {}; listening in {}",
                    TABLE_LOST_AFTER.as_secs(),
                    mode.name,
                    next.name
                ));
                self.table_lost_to(next).await?;
            }
            return Ok(());
        }
        // A host with nothing to say: a heartbeat at the table's speed.
        if self.join_window.is_none()
            && self.table_queue.is_empty()
            && self.last_table_sent.elapsed() >= TABLE_HEARTBEAT
        {
            self.last_table_sent = Instant::now();
            self.table_tx(
                TableFrame {
                    table: id,
                    from: me.clone(),
                    body: TableBody::Speed {
                        mode_index: mode.index,
                        last_id: self.table_msg_id,
                    },
                }
                .encode(),
            );
        }
        if let Some((end, back)) = self.join_window {
            if Instant::now() < end {
                return Ok(());
            }
            self.join_window = None;
            self.last_join_window = Instant::now();
            // Back to speed: what it was, or slower if a newcomer (or a
            // member's new report) needs it.
            let worst = self
                .st
                .lock()
                .unwrap()
                .table
                .as_ref()
                .and_then(|t| {
                    t.members
                        .values()
                        .filter(|m| m.last.is_some_and(|l| l.elapsed() < TABLE_MEMBER_TIMEOUT))
                        .filter_map(|m| m.snr_of_host)
                        .min()
                });
            let target = worst
                .map(|w| modes::best_mode(self.bw, w as f64, self.cfg.ramp_margin_db))
                .filter(|m| m.payload_bps < back.payload_bps)
                .unwrap_or(back);
            if target.payload_bps > calling.payload_bps {
                self.log(format!("Roundtable: join window closed; back to {}", target.name));
                self.table_set_speed(target).await?;
            } else {
                self.log("Roundtable: join window closed; staying at DATAC3".into());
            }
            return Ok(());
        }
        if mode.payload_bps > calling.payload_bps
            && self.last_join_window.elapsed() >= JOIN_WINDOW_EVERY
            && self.table_queue.is_empty()
        {
            self.table_set_speed(calling).await?;
            let switched = self.pending_speed.map(|(_, at)| at).unwrap_or_else(Instant::now);
            self.join_window = Some((switched + JOIN_WINDOW_FOR, mode));
            // Its CALL, in DATAC3 once the table has switched (table traffic
            // waits for the switch), for stations looking for it.
            let call = TableFrame {
                table: id,
                from: me,
                body: TableBody::Call {
                    kind: CALL_TABLE,
                    table_name: name,
                    bw_code: Bandwidth::ALL
                        .iter()
                        .position(|b| *b == self.bw)
                        .unwrap_or(4) as u8,
                    grid: self.cfg.grid.clone(),
                },
            };
            self.table_tx(call.encode());
            self.log(format!(
                "Roundtable: join window: DATAC3 for {} s so others can join",
                JOIN_WINDOW_FOR.as_secs()
            ));
        }
        Ok(())
    }

    /// Member: our periodic report (a JOIN frame with how well we hear the
    /// host). If it says we can't copy the table's speed, expect the host to
    /// slow down.
    fn send_table_report(&mut self) {
        let Some((id, name, mode)) =
            self.st.lock().unwrap().table.as_ref().map(|t| (t.id, t.name.clone(), t.mode))
        else {
            return;
        };
        self.last_table_report = Instant::now();
        let snr = self.host_snr.or(self.est.snr_db).unwrap_or(0.0).round() as i8;
        if self.host_snr.is_some()
            && (snr as f64) < mode.min_snr_db + self.cfg.ramp_margin_db
            && self.expect_slowdown.is_none()
        {
            self.expect_slowdown = Some(Instant::now());
        }
        let me = self.mycall();
        self.table_tx(
            TableFrame {
                table: id,
                from: me,
                body: TableBody::Join {
                    table_name: name,
                    snr_of_host: snr,
                },
            }
            .encode(),
        );
    }

    /// Member: listen for the table in `m`, and tell the host where we are
    /// at the next chance (now, and again at its next join window).
    async fn table_lost_to(&mut self, m: &'static Mode) -> anyhow::Result<()> {
        self.apply_table_mode(m).await?;
        self.last_table_heard = Instant::now();
        self.table_unsynced = true;
        self.send_table_report();
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
        let Some(id) = self.table_id() else {
            return Ok(());
        };
        let me = self.mycall();
        let f = TableFrame {
            table: id,
            from: me,
            body: TableBody::Speed {
                mode_index: m.index,
                last_id: self.table_msg_id,
            },
        }
        .encode();
        // Announce twice in the old mode, and switch only once Mercury has
        // actually sent both (switching sooner sends them in the new mode,
        // which members still on the old one cannot hear).
        let old = self
            .st
            .lock()
            .unwrap()
            .listen_mode
            .unwrap_or(&modes::MODES[4]);
        // Calls for the table go out in its mode; newcomers listen in DATAC3
        // and couldn't copy them any more, so the CQ for the table ends, and
        // first, so the announcements don't go out over a whistle.
        if m.name != "DATAC3" && self.cq.as_ref().is_some_and(|c| c.table.is_some()) {
            self.stop_cq(&format!(
                "CQ for the Roundtable ends: the table now runs at {}, which stations still calling at DATAC3 can't copy",
                m.name
            ))
            .await;
        }
        self.tnc()?.send_broadcast(&f).await?;
        self.tnc()?.send_broadcast(&f).await?;
        self.last_table_sent = Instant::now();
        if old.name == "DATAC3" && m.payload_bps > old.payload_bps && self.join_window.is_none() {
            // The first join window a full period after leaving DATAC3.
            self.last_join_window = Instant::now();
        }
        let at = Instant::now() + Duration::from_secs_f64(2.0 * old.frame_s + 3.0);
        self.pending_speed = Some((m, at));
        self.log(format!(
            "Roundtable: announcing speed change {} → {} ({} bps)",
            old.name, m.name, m.payload_bps
        ));
        Ok(())
    }

    /// Host only: move the table to the fastest mode everyone can copy.
    async fn ramp(&mut self) -> anyhow::Result<()> {
        if !self.cfg.auto_ramp || self.pending_speed.is_some() || self.join_window.is_some() {
            return Ok(());
        }
        let (host, worst, current) = {
            let g = self.st.lock().unwrap();
            let Some(t) = g.table.as_ref() else {
                return Ok(());
            };
            let recent = |m: &Member| {
                // Members report every 2 min: after three missed reports a
                // member no longer holds the table back.
                m.last
                    .is_some_and(|l| l.elapsed() < TABLE_MEMBER_TIMEOUT)
            };
            let worst = t
                .members
                .values()
                .filter(|m| recent(m))
                .filter_map(|m| m.snr_of_host)
                .min();
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
        } else if target.payload_bps > current.payload_bps
            && self.last_downgrade.elapsed() > Duration::from_secs(120)
        {
            self.table_set_speed(target).await?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- tick

    async fn tick(&mut self) -> anyhow::Result<()> {
        // An automatic call that hasn't linked: another station probably won
        // the CQ, or the station is gone. Stop calling (Mercury would go on
        // for minutes, on top of whoever is on the frequency).
        if let Some(at) = self.auto_call
            && self.peer.is_none()
            && at.elapsed() > AUTO_CALL_GIVE_UP
        {
            self.auto_call = None;
            self.link_auto = None;
            let who = self.st.lock().unwrap().calling.take();
            if let Some(t) = self.tnc.as_mut() {
                let _ = t.command("ABORT").await;
            }
            self.log(format!(
                "Gave up calling {} after {} s (no link)",
                who.as_deref().unwrap_or("the station"),
                AUTO_CALL_GIVE_UP.as_secs()
            ));
        }
        self.step_waterfall();
        self.step_monitor();
        self.step_pcm();
        self.step_wide().await?;
        self.step_automation().await?;
        self.step_auto_pending().await?;
        self.step_qsy().await?;
        self.step_pending_join().await?;
        self.step_answer().await?;
        if self.pending_tune {
            let quiet = {
                let g = self.st.lock().unwrap();
                !g.tx && g.link.is_none() && g.calling.is_none()
            } && self.cq.is_none()
                && self.search.is_none()
                && self.table_queue.is_empty();
            if quiet && let Err(e) = self.tune_antenna("high SWR").await {
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
        self.step_table_timers().await?;
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
                self.hi_queue.push_back(Link::Report {
                    snr_db: snr,
                    mode_index: mi,
                });
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
                // A few per round, oldest first: each is a frame on the air,
                // and a long gap (or a sender gone for good) mustn't keep
                // this station transmitting.
                let mut wanted = self.table_asm.missing();
                wanted.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
                for (of, mid, missing) in wanted.into_iter().take(AGAIN_PER_ROUND) {
                    self.table_tx(
                        TableFrame {
                            table: id,
                            from: me.clone(),
                            body: TableBody::Again {
                                of,
                                id: mid,
                                missing,
                            },
                        }
                        .encode(),
                    );
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
            .map(|m| Link::Mail {
                id: m.id,
                from: m.from.clone(),
                to: m.to.clone(),
                created: m.created,
                hops: m.hops,
                text: m.text.clone(),
            })
            .collect();
        if !mail.is_empty() {
            self.log(format!(
                "📮 Sending {} Depot message(s) to {call}",
                mail.len()
            ));
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
        self.cfg
            .segments
            .iter()
            .any(|s| d + 300.0 >= s.lo_hz && d + 2700.0 <= s.hi_hz)
    }

    fn qsy_verdict(&self, hz: f64) -> Result<(), String> {
        qsy_verdict(&self.cfg, self.qsy.map(|q| q.0).unwrap_or(self.dial()), hz)?;
        // Unattended, the new frequency must be one where automatic control
        // is allowed too, like everywhere else the away station transmits.
        if self.away
            && self.cfg.auto_in_segments_only
            && !self
                .cfg
                .segments
                .iter()
                .any(|s| hz + 300.0 >= s.lo_hz && hz + 2700.0 <= s.hi_hz)
        {
            return Err("I'm away: only within the automatic-control segments".into());
        }
        Ok(())
    }

    /// Ending a link, and Mercury hasn't confirmed yet. Gives up after
    /// 45 s, in case a DISCONNECT with nothing to end never gets an answer.
    fn leaving(&self) -> bool {
        self.leaving_since
            .is_some_and(|t| t.elapsed() < Duration::from_secs(45))
    }

    /// Tell Mercury to end the link (or the call in progress) now.
    async fn disconnect_now(&mut self) -> anyhow::Result<()> {
        self.leaving_since = Some(Instant::now());
        self.tnc()?.command("DISCONNECT").await
    }

    /// End the link politely: BYE, then DISCONNECT once Mercury has sent it.
    async fn hang_up(&mut self, text: &str) -> anyhow::Result<()> {
        self.leaving_since = Some(Instant::now());
        self.hi_queue.push_back(Link::Bye { text: text.into() });
        self.pump().await?;
        self.hangup_at = Some(Instant::now());
        Ok(())
    }

    async fn step_qsy(&mut self) -> anyhow::Result<()> {
        if let Some(at) = self.bye_at {
            if self.peer.is_none() {
                self.bye_at = None;
            } else if at.elapsed() > Duration::from_secs(30) {
                self.bye_at = None;
                self.log("The other station said 73 but its disconnect didn't arrive; hanging up from here".into());
                // ABORT, not DISCONNECT: they've gone (after a QSY, maybe to
                // another frequency), and Mercury would otherwise hold a
                // DISCONNECT "until delivered", polling nobody for minutes.
                self.leaving_since = Some(Instant::now());
                self.tnc()?.command("ABORT").await?;
            }
        }
        if let Some(at) = self.hangup_at {
            if self.peer.is_none() {
                self.hangup_at = None;
            } else if (self.mercury_buffer == 0
                && self.hi_queue.is_empty()
                && at.elapsed() > Duration::from_millis(500))
                || at.elapsed() > Duration::from_secs(6)
            {
                self.hangup_at = None;
                self.disconnect_now().await?;
            }
        }
        if let Some((home, at, since)) = self.qsy_home
            && (since.elapsed() > Duration::from_secs(8) || self.peer.is_some())
        {
            self.qsy_home = None;
            if self.peer.is_none() {
                self.tune(home).await?;
                self.log(format!(
                    "↔ QSY over: back from {:.3} to {:.3} kHz",
                    at / 1e3,
                    home / 1e3
                ));
                self.set(|g| g.qsy = None);
            } else {
                // A new link came up here first: still away from home, so go
                // back when this one ends.
                self.qsy = Some((home, at));
            }
        }
        if let Some((hz, at, asked)) = self.qsy_move {
            // Both move a moment after the answer has crossed: the station
            // that asked, 3 s after it arrived (time for Mercury's ack);
            // the other, once Mercury has sent everything.
            // (Mercury doesn't always report an empty buffer, so the station
            // that agreed moves after 6 s at the latest.)
            let due = if asked {
                at.elapsed() > Duration::from_secs(3)
            } else {
                (self.mercury_buffer == 0 && at.elapsed() > Duration::from_secs(2))
                    || at.elapsed() > Duration::from_secs(6)
            };
            if self.peer.is_none() {
                self.qsy_move = None;
            } else if due {
                self.qsy_move = None;
                let home = self.qsy.map(|q| q.0).unwrap_or(self.dial());
                self.tune(hz).await?;
                self.qsy = Some((home, hz));
                self.qsy_tuned = false;
                self.qsy_fail = false;
                self.log(format!(
                    "↔ QSY: moved to {:.3} kHz (home {:.3} kHz)",
                    hz / 1e3,
                    home / 1e3
                ));
                self.set(|g| {
                    g.qsy = Some(QsyView {
                        home_hz: home,
                        at_hz: hz,
                    });
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
            self.log(
                "⚠ SWR stays high on the QSY frequency: ending the link so both stations go home"
                    .into(),
            );
            self.hi_queue.push_back(Link::Chat {
                id: 0,
                text: "SWR is too high for me here; ending the link so we both go back.".into(),
            });
            self.hang_up("73").await?;
        }
        Ok(())
    }

    fn library_answer(&self, id: u64, path: &str) -> Link {
        library_answer(&self.cfg, self.away, id, path)
    }

    fn library_file(&self, path: &str) -> anyhow::Result<(String, Vec<u8>)> {
        library_file(&self.cfg, self.away, path)
    }

    /// At a Roundtable, or waiting to join one.
    fn at_table_or_joining(&self) -> bool {
        self.pending_join.is_some() || self.st.lock().unwrap().table.is_some()
    }

    fn quiet(&self) -> bool {
        let g = self.st.lock().unwrap();
        !g.tx && g.link.is_none() && g.calling.is_none() && !g.incoming_call
    }

    /// Mercury's busy detector on (no link) or off (linked), if the
    /// settings use it.
    async fn busy_detect(&mut self, on: bool) {
        if !self.cfg.busy_detect {
            return;
        }
        if !on {
            self.chan_busy = false;
            self.busy_wait = None;
            self.set(|g| g.channel_busy = false);
        }
        if let Some(t) = self.tnc.as_mut() {
            let _ = t.command(if on { "BUSYDETECT ON" } else { "BUSYDETECT OFF" }).await;
        }
    }

    /// May an unattended transmission go now? Not while the channel is busy,
    /// but after BUSY_WAIT_MAX of waiting it goes anyway (a detector that
    /// reads busy all the time, behind a narrow filter say, mustn't silence
    /// the station).
    fn clear_to_send(&mut self, what: &str) -> bool {
        if !self.chan_busy {
            self.busy_wait = None;
            return true;
        }
        let now = Instant::now();
        // Not asked for a few seconds: whatever waited gave up meanwhile.
        if self
            .busy_wait
            .is_some_and(|(_, last)| now.duration_since(last) > Duration::from_secs(3))
        {
            self.busy_wait = None;
        }
        match self.busy_wait {
            None => {
                self.busy_wait = Some((now, now));
                self.log(format!("{what} waits: someone is transmitting on this frequency"));
                false
            }
            Some((t, _)) if t.elapsed() >= BUSY_WAIT_MAX => {
                self.busy_wait = None;
                self.log(format!(
                    "{what}: the channel has read busy for {} s; sending anyway",
                    BUSY_WAIT_MAX.as_secs()
                ));
                true
            }
            Some((t, _)) => {
                self.busy_wait = Some((t, now));
                false
            }
        }
    }

    /// Before calling a station: a real call of ours and theirs, not linked
    /// or at a Roundtable already; a running CQ stops (its whistle and
    /// CQFRAME would go out on top of the call).
    async fn ready_to_call(&mut self, call: &str) -> anyhow::Result<()> {
        if !crate::protocol::valid_call(call) {
            anyhow::bail!("{call:?} isn't a callsign");
        }
        if !crate::protocol::valid_call(&self.mycall()) {
            anyhow::bail!("set your own callsign first (Settings → Station)");
        }
        let calling = self.st.lock().unwrap().calling.is_some() && self.answer_at.is_none();
        if self.peer.is_some() || calling {
            anyhow::bail!("already linked (or calling): disconnect first");
        }
        if self.st.lock().unwrap().table.is_some() {
            anyhow::bail!("you're at a Roundtable: Leave it first, then call");
        }
        self.stop_cq("CQ stopped: calling a station").await;
        // This call replaces automatic ones: a waiting auto-answer or
        // deliver/collect, and the give-up timer of an earlier one (which
        // would otherwise ABORT this call).
        if self.answer_at.take().is_some() {
            self.set(|g| g.calling = None);
        }
        self.auto_pending = None;
        self.auto_call = None;
        self.link_auto = None;
        Ok(())
    }

    fn cooled(&mut self, call: &str, secs: u64) -> bool {
        // Forget stations not heard from in a day (the map would grow).
        if self.auto_cooldown.len() > 256 {
            self.auto_cooldown
                .retain(|_, t| t.elapsed() < Duration::from_secs(86_400));
        }
        let ok = self
            .auto_cooldown
            .get(call)
            .is_none_or(|t| t.elapsed() > Duration::from_secs(secs));
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
            g.mail_alerts.push(MailAlert {
                call: call.clone(),
                ours: true,
                count: n,
                at: Instant::now(),
            });
        });
        let quiet = self.quiet() && self.cq.is_none() && self.search.is_none();
        if self.away
            && self.cfg.auto_deliver
            && quiet
            && self.auto_allowed()
            && self.cooled(&format!("deliver {call}"), 900)
        {
            self.auto_pending = Some((call, true, Instant::now()));
            self.step_auto_pending().await?;
        } else if self.cfg.notify_mail
            && self.auto_allowed()
            && self.cooled(&format!("notice {call}"), 900)
        {
            let me = self.mycall();
            self.table_tx(
                TableFrame {
                    table: 0,
                    from: me,
                    body: TableBody::Notice {
                        to: call.clone(),
                        count: n.min(255) as u8,
                    },
                }
                .encode(),
            );
            self.log(format!("📣 Told {call}: {n} message(s) waiting here"));
        }
        Ok(())
    }

    /// `call` says it holds mail for us.
    async fn they_have_mail(&mut self, call: &str, count: usize) -> anyhow::Result<()> {
        let call = call.to_uppercase();
        self.set(|g| {
            g.mail_alerts.retain(|a| !(a.call == call && !a.ours));
            g.mail_alerts.push(MailAlert {
                call: call.clone(),
                ours: false,
                count,
                at: Instant::now(),
            });
        });
        self.log(format!("📬 {call} has mail for you"));
        let quiet = self.quiet() && self.cq.is_none() && self.search.is_none();
        if self.away
            && self.cfg.auto_collect
            && quiet
            && self.auto_allowed()
            && self.cooled(&format!("collect {call}"), 900)
        {
            self.auto_pending = Some((call, false, Instant::now()));
            self.step_auto_pending().await?;
        }
        Ok(())
    }

    /// Place the waiting automatic deliver/collect call once the channel is
    /// clear (the frame that set it off has just ended, and the detector
    /// holds BUSY a moment longer). Dropped if things change meanwhile.
    async fn step_auto_pending(&mut self) -> anyhow::Result<()> {
        let Some((call, deliver, at)) = self.auto_pending.clone() else {
            return Ok(());
        };
        let still = self.away
            && if deliver { self.cfg.auto_deliver } else { self.cfg.auto_collect }
            && self.pending_join.is_none()
            && self.quiet()
            && self.peer.is_none()
            && self.cq.is_none()
            && self.search.is_none()
            && self.st.lock().unwrap().table.is_none()
            && (!deliver || !self.depot.waiting_for(&call).is_empty());
        if !still {
            self.auto_pending = None;
            return Ok(());
        }
        if at.elapsed() > AUTO_PENDING_MAX {
            self.auto_pending = None;
            self.log(format!(
                "🤖 Away: no clear channel to call {call} in {} s; not calling",
                AUTO_PENDING_MAX.as_secs()
            ));
            return Ok(());
        }
        let what = if deliver { "Automatic delivery" } else { "Automatic collection" };
        if !self.clear_to_send(what) {
            return Ok(());
        }
        self.auto_pending = None;
        let me = self.mycall();
        self.tnc()?.command(&format!("CONNECT {me} {call}")).await?;
        self.auto_call = Some(Instant::now());
        self.quiet_watch();
        self.link_auto = Some(deliver);
        self.set(|g| g.calling = Some(call.clone()));
        if deliver {
            let n = self.depot.waiting_for(&call).len();
            self.log(format!(
                "🤖 Away: connecting to {call} to deliver {n} Depot message(s)"
            ));
        } else {
            self.log(format!("🤖 Away: connecting to {call} to collect your mail"));
        }
        Ok(())
    }

    fn send_beacon(&mut self, why: &str) -> anyhow::Result<()> {
        if self.tnc.is_none() || !crate::protocol::valid_call(&self.mycall()) {
            // Counts as an attempt: otherwise the automation step would retry
            // (and fail, skipping the rest of the tick) ten times a second.
            self.last_beacon = Instant::now();
            anyhow::bail!("Mercury is not connected, or no callsign is set");
        }
        let me = self.mycall();
        let mut mail_for = self.depot.waiting_calls();
        // One modem frame (123 bytes on DATAC3): keep the list short.
        while (TableFrame {
            table: 0,
            from: me.clone(),
            body: TableBody::Beacon {
                grid: self.cfg.grid.clone(),
                away: self.away,
                mail_for: mail_for.clone(),
            },
        })
        .encode()
        .len()
            > 120
        {
            mail_for.pop();
        }
        let f = TableFrame {
            table: 0,
            from: me,
            body: TableBody::Beacon {
                grid: self.cfg.grid.clone(),
                away: self.away,
                mail_for: mail_for.clone(),
            },
        };
        self.table_tx(f.encode());
        self.last_beacon = Instant::now();
        self.log(format!(
            "📡 Station ID sent ({why}){}",
            if mail_for.is_empty() {
                String::new()
            } else {
                format!(": mail waiting for {}", mail_for.join(" "))
            }
        ));
        Ok(())
    }

    async fn step_automation(&mut self) -> anyhow::Result<()> {
        // The schedule: where to listen at this time of day (UTC).
        if self.cfg.schedule_on
            && self
                .last_sched
                .is_none_or(|t| t.elapsed() > Duration::from_secs(20))
        {
            self.last_sched = Some(Instant::now());
            let follow = !self.cfg.schedule_only_away || self.away;
            let slot = crate::schedule::active(&self.cfg.schedule, crate::depot::now_s())
                .map(|(i, s)| (i, s.clone()));
            // Not at (or joining) a Roundtable either: retuning would leave
            // it; nor with an automatic call about to go out here.
            let idle = self.quiet()
                && self.cq.is_none()
                && self.search.is_none()
                && self.peer.is_none()
                && self.qsy.is_none()
                && !self.at_table_or_joining()
                && self.auto_pending.is_none();
            match slot {
                Some((i, sl)) if follow => {
                    let hz = sl.khz * 1000.0;
                    if idle && (self.dial() - hz).abs() > 50.0 {
                        self.tune(hz).await?;
                        self.log(format!(
                            "🗓 Schedule: listening on {:.3} kHz ({}–{} UTC, {}){}",
                            hz / 1e3,
                            sl.from,
                            sl.to,
                            sl.days.label(),
                            if sl.beacon { ", with Station ID" } else { "" }
                        ));
                    }
                    self.sched_slot = Some(i);
                    self.set(|g| {
                        g.schedule_now = Some(format!(
                            "{:.3} kHz until {} UTC{}",
                            hz / 1e3,
                            sl.to,
                            if sl.beacon { " · Station ID" } else { "" }
                        ))
                    });
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
        let sched_beacon = self
            .sched_slot
            .and_then(|i| self.cfg.schedule.get(i))
            .is_some_and(|s| s.beacon);
        // Station ID beacon, when due and the band is quiet.
        if (self.cfg.beacon || sched_beacon)
            && self.last_beacon.elapsed()
                > Duration::from_secs_f64(self.cfg.beacon_minutes.max(10.0) * 60.0)
            && self.quiet()
            && self.cq.is_none()
            && self.search.is_none()
            && self.table_queue.is_empty()
            // At a Roundtable it would go out at the table's speed, in the
            // middle of its traffic: it waits until the table is left.
            && !self.at_table_or_joining()
            && self.clear_to_send("Station ID")
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
                // Collecting: once mail has come, 10 s of quiet ends it; until
                // then wait up to a minute (it took 23 s to cross on the bench).
                Some(false) => idle > if self.link_got_mail { 10.0 } else { 60.0 },
                None => self.away && idle > self.cfg.away_idle_s,
            };
            if done && self.mercury_buffer == 0 {
                self.log(format!("Hanging up with {peer} (idle {idle:.0} s)"));
                self.link_auto = None;
                self.hang_up("73").await?;
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

    /// Feed remote listeners the received audio (at most 2 s waiting).
    fn step_pcm(&mut self) {
        let mut tap = self.pcm.lock().unwrap();
        if !tap.on {
            self.pcm_from = 0;
            return;
        }
        let rx = self.rx.lock().unwrap();
        if self.pcm_from == 0 || rx.count.saturating_sub(self.pcm_from) > FS as u64 {
            self.pcm_from = rx.count.saturating_sub((FS / 10) as u64);
        }
        let n = (rx.count - self.pcm_from) as usize;
        if let Some(v) = rx.range(self.pcm_from, n) {
            tap.samples.extend(v.iter().map(|x| (*x * 3.0) as f32));
        }
        self.pcm_from = rx.count;
        let excess = tap.samples.len().saturating_sub(2 * FS as usize);
        tap.samples.drain(..excess);
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
                g.audio_seq += rows.len() as u64;
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
            hits: hits
                .iter()
                .map(|(at, h)| (h.dial_hz, h.score, at.elapsed().as_secs_f64()))
                .collect(),
        };
        self.set(|g| g.wide = view);
        self.wide_tried
            .retain(|(_, at)| at.elapsed() < Duration::from_secs(120));
        // A stepping scan that hasn't found anything yet gives way to an
        // SDR hit (auto-tune or not: searching already means "go to
        // whistles"). One that is measuring a whistle or waiting for its
        // CQ is left alone.
        let scanning = self.search.as_ref().is_some_and(|s| s.interruptible());
        let busy = (self.search.is_some() && !scanning)
            || self.cq.is_some()
            || self.peer.is_some()
            || self.st.lock().unwrap().calling.is_some();
        if !self.wide_on || !(self.wide_auto || scanning) || busy {
            return Ok(());
        }
        let fresh = hits
            .iter()
            .filter(|(at, h)| {
                at.elapsed() < Duration::from_secs(4)
                    && !self
                        .wide_tried
                        .iter()
                        .any(|(f, _)| (f - h.dial_hz).abs() < 200.0)
            })
            .max_by(|a, b| a.1.score.partial_cmp(&b.1.score).unwrap());
        if let Some((_, h)) = fresh {
            let (dial, score) = (h.dial_hz, h.score);
            self.wide_handoff(dial, score);
        }
        Ok(())
    }

    fn wide_handoff(&mut self, dial: f64, score: f64) {
        self.wide_tried.push((dial, Instant::now()));
        let _ = score;
        // Pause a stepping scan (or keep the one an earlier hand-off paused).
        let resume = match self.search.take() {
            Some(s) if s.scope != WIDE_SCOPE => Some(Box::new(s)),
            Some(s) => s.resume,
            None => None,
        };
        if let Some(r) = &resume {
            self.log(format!(
                "🚂 SDR whistle at {:.3} kHz; pausing the {} search to fine-lock on it",
                dial / 1e3,
                r.scope
            ));
        } else {
            self.log(format!(
                "🚂 SDR whistle at {:.3} kHz; tuning the radio there to fine-lock",
                dial / 1e3
            ));
        }
        self.search = Some(Search {
            channels: vec![dial],
            idx: 0,
            phase: SearchPhase::Verify {
                target: dial,
                from: 0,
                coarse: 0.0,
                again: false,
                hops: 0,
            },
            scope: WIDE_SCOPE.into(),
            resume,
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
        // Rest while transmitting, linked, calling or being called, and for a
        // while after links start and end: Mercury's connect and disconnect
        // bursts can look like a whistle.
        let busy = {
            let g = self.st.lock().unwrap();
            g.tx || g.calling.is_some() || g.incoming_call
        };
        let closing = self.leaving()
            || self
                .link_ended_at
                .is_some_and(|t| t.elapsed() < Duration::from_secs_f64(WATCH_AFTER_LINK_S));
        if busy || closing || self.peer.is_some() || Instant::now() < self.watch_quiet_until {
            self.watch_pending = None;
            return;
        }
        let n = (secs * FS as f64) as usize;
        let x = self.rx.lock().unwrap().last(n);
        if x.len() < n {
            return;
        }
        // Found by the scan detector (unambiguous to ±1100 Hz), measured
        // by the matched filter.
        let Some(h) = self
            .tracker
            .scan(&x)
            .filter(|h| h.z >= TrackDetector::found(secs))
        else {
            return;
        };
        let Some(mut d) = self.detector.detect(&x) else {
            self.watch_pending = None;
            return;
        };
        d.offset_hz = self.fine_offset(h.offset_hz, d.offset_hz);
        // Report only when two checks in a row (about a second apart) agree
        // on the offset: a single data burst isn't a whistle.
        let agreed = self
            .watch_pending
            .is_some_and(|(at, off)| at.elapsed() < Duration::from_secs(3) && (off - d.offset_hz).abs() <= 15.0);
        if !agreed {
            self.watch_pending = Some((Instant::now(), d.offset_hz));
            return;
        }
        self.watch_pending = None;
        {
            let dial = self.dial();
            let fresh = self
                .st
                .lock()
                .unwrap()
                .whistle
                .as_ref()
                .is_none_or(|w| w.at.elapsed() > Duration::from_secs(20));
            if fresh {
                self.log(format!(
                    "🚂 Whistle heard {:+.0} Hz from here (caller at {:.3} kHz)",
                    d.offset_hz,
                    (dial + d.offset_hz) / 1e3
                ));
            }
            self.set(|g| {
                g.whistle = Some(WhistleSeen {
                    at: Instant::now(),
                    dial_hz: dial,
                    offset_hz: d.offset_hz,
                    score: d.score,
                })
            });
        }
    }
}

/// Whistle watch stays off this long after a link (or connect attempt)
/// ends, while the other station's Mercury finishes its disconnect.
pub const WATCH_AFTER_LINK_S: f64 = 10.0;

/// Longest an auto-answer waits after the CQFRAME before CONNECT: the CQ
/// station's CALL packet (one DATAC3 frame, ~3.7 s) follows it on the air.
pub const ANSWER_AFTER_CQ_S: f64 = 4.5;
/// A fine correction at least this big right after retuning onto a whistle
/// is measured again before locking (the scan detector is good to ~10 Hz).
pub const RECHECK_HZ: f64 = 25.0;

/// How long a Roundtable join waits for Mercury to be free.
pub const JOIN_WAIT_S: f64 = 60.0;

/// May a link move from `home` to `hz`? (Settings → QSY requests.)
pub fn qsy_verdict(cfg: &Config, home: f64, hz: f64) -> Result<(), String> {
    if !cfg.accept_qsy {
        return Err("QSY requests are turned off here".into());
    }
    let here = bandplan::band_of(home);
    // The signal, not just the dial, must be inside the band: USB puts it
    // 300–2700 Hz above the dial.
    let Some(there) = bandplan::band_of(hz + 300.0)
        .filter(|b| bandplan::band_of(hz + 2700.0) == Some(*b))
    else {
        return Err("not an amateur band I can use (the whole signal must fit inside it)".into());
    };
    if cfg.qsy_same_band {
        if Some(there) != here {
            return Err(format!("I only QSY within {}", here.unwrap_or("this band")));
        }
    } else if !cfg.qsy_bands.iter().any(|b| b.trim().eq_ignore_ascii_case(there)) {
        return Err(format!("{there} isn't one of my QSY bands"));
    }
    if cfg.qsy_max_khz > 0.0 && (hz - home).abs() > cfg.qsy_max_khz * 1000.0 {
        return Err(format!("more than {:.0} kHz from here", cfg.qsy_max_khz));
    }
    if cfg.qsy_segments_only
        && !cfg
            .segments
            .iter()
            .any(|s| hz + 300.0 >= s.lo_hz && hz + 2700.0 <= s.hi_hz)
    {
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
    Link::ListAns {
        id,
        path: path.to_string(),
        entries,
        note,
    }
}

/// A library file to send, by the settings (or why not).
pub fn library_file(cfg: &Config, away: bool, path: &str) -> anyhow::Result<(String, Vec<u8>)> {
    let root = library_root(cfg, away).map_err(|e| anyhow::anyhow!(e))?;
    crate::library::read(
        &root,
        path,
        (cfg.library_max_mb.max(0.0) * 1_048_576.0) as u64,
    )
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
    assert_eq!(
        version_text("VARA version 4.9.0 registered"),
        "Mercury (VARA-compatible 4.9.0)"
    );
    assert_eq!(version_text("VERSION Mercury 1.9.23"), "Mercury 1.9.23");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mercury that opens its ports after the station started (the phone
    /// apps start the engine first): `ReconnectModem` connects to it
    /// without restarting the station, so the radio and its shared rigctld
    /// port (Mercury's PTT) stay up.
    #[test]
    fn reconnect_modem_reaches_a_late_mercury_without_a_restart() {
        use std::net::TcpListener as StdListener;
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Ports for Mercury: control and data consecutive, plus broadcast.
        let (ctl, bc) = loop {
            let a = StdListener::bind("127.0.0.1:0").unwrap();
            let p = a.local_addr().unwrap().port();
            // All three held until all are picked, so they're distinct.
            if let Ok(_d) = StdListener::bind(("127.0.0.1", p + 1)) {
                let b = StdListener::bind("127.0.0.1:0").unwrap();
                break (p, b.local_addr().unwrap().port());
            }
        };
        let h = spawn(|| {});
        let mut cfg = Config {
            callsign: "N0LATE".into(),
            demo: false,
            rigctld: "none".into(),
            mercury_port: ctl,
            mercury_bcast_port: bc,
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        cfg.sdr.wide_search = false;
        h.send(Cmd::Start(cfg));
        let log = |h: &Handle| h.state.lock().unwrap().log.iter().map(|l| l.1.clone()).collect::<Vec<_>>();
        let t = Instant::now();
        while !log(&h).iter().any(|l| l.contains("Radio: none")) && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(500));
        assert!(!h.state.lock().unwrap().mercury_ok, "nothing listens yet");
        // Mercury comes up: forward the reserved ports to a fake Mercury.
        rt.block_on(async {
            let f = crate::fake_mercury::FakeMercury::start(Default::default()).await.unwrap();
            for (from, to) in [(ctl, f.port), (ctl + 1, f.port + 1), (bc, f.bcast_port)] {
                let l = tokio::net::TcpListener::bind(("127.0.0.1", from))
                    .await
                    .unwrap_or_else(|e| panic!("port {from} ({ctl}/{bc}): {e}; fake {}/{}", f.port, f.bcast_port));
                tokio::spawn(async move {
                    while let Ok((mut a, _)) = l.accept().await {
                        let mut b = tokio::net::TcpStream::connect(("127.0.0.1", to)).await.unwrap();
                        tokio::spawn(async move {
                            let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                        });
                    }
                });
            }
        });
        h.send(Cmd::ReconnectModem);
        let t = Instant::now();
        while !h.state.lock().unwrap().mercury_ok && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(h.state.lock().unwrap().mercury_ok, "{:#?}", log(&h));
        h.send(Cmd::ReconnectModem);
        std::thread::sleep(Duration::from_millis(500));
        let l = log(&h);
        h.send(Cmd::Stop);
        // One station all along: started once, the radio set up once.
        assert_eq!(l.iter().filter(|x| x.contains("Radio: none")).count(), 1, "{l:#?}");
        // Asking again while connected does nothing.
        assert_eq!(l.iter().filter(|x| x.starts_with("Mercury: connected")).count(), 1, "{l:#?}");
    }

    /// Radio "none" (the phones' Radio → None, VOX keying) is no radio, not
    /// a rigctld host called "none".
    #[test]
    fn radio_none_is_no_cat() {
        let h = spawn(|| {});
        let mut cfg = Config {
            callsign: "N0NONE".into(),
            demo: false,
            rigctld: "none".into(),
            // Nothing listens here: the modem stays down, which is fine.
            mercury_port: 1,
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        cfg.sdr.wide_search = false;
        h.send(Cmd::Start(cfg));
        let t = Instant::now();
        let log = loop {
            let log: Vec<String> = h.state.lock().unwrap().log.iter().map(|l| l.1.clone()).collect();
            if log.iter().any(|l| l.starts_with("Radio: none")) || t.elapsed() > Duration::from_secs(10) {
                break log;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        h.send(Cmd::Stop);
        assert!(log.iter().any(|l| l.starts_with("Radio: none (no CAT)")), "{log:#?}");
        assert!(!log.iter().any(|l| l.contains("rigctld at none")), "{log:#?}");
        assert!(!h.state.lock().unwrap().rig_ok);
    }

    /// Demo on two real Mercury modems (skipped when Mercury isn't
    /// installed): search finds K7BOT's real whistle, Mercury decodes its
    /// real CQFRAME, and the stations link and chat over real ARQ.
    #[test]
    fn real_mercury_demo_search_link_chat() {
        if crate::realdemo::find_mercury("").is_none() {
            return;
        }
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            callsign: "N0REAL".into(),
            auto_answer: true,
            ..Default::default()
        };
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
        wait("real Mercury", 30, &|s| {
            s.mercury_ok && s.log.iter().any(|l| l.1.contains("two real Mercury"))
        });
        h.send(Cmd::StartSearch(SearchScope::CurrentBand));
        wait("link with K7BOT over real Mercury", 240, &|s| {
            s.link.as_ref().is_some_and(|l| l.peer == "K7BOT")
        });
        let f = h.state.lock().unwrap().rig.freq_hz;
        assert!((f - crate::realdemo::BOT_DIAL).abs() <= 55.0, "locked {f}");
        wait("K7BOT's greeting", 90, &|s| {
            s.chat.iter().any(|c| c.who == Who::Them)
        });
        h.send(Cmd::Chat("hello over real Mercury".into()));
        wait("reply and delivery", 120, &|s| {
            s.chat
                .iter()
                .any(|c| c.who == Who::Them && c.text.contains("hello over real Mercury"))
                && s.chat.iter().any(|c| c.who == Who::Me && c.delivered)
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
            let mut cfg = Config {
                demo: true,
                demo_real_mercury: false,
                callsign: "N0SWR".into(),
                whistle_s: 3.0,
                cq_cycles: 2,
                cq_listen_s: 2.0,
                ..Default::default()
            };
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
            assert!(
                wait(30, &|s| s
                    .log
                    .iter()
                    .any(|l| l.1.contains("High SWR: 2.8 on 40m"))),
                "auto={auto}: no SWR warning"
            );
            // Still calling: never tune in the middle of it.
            assert!(
                !h.state
                    .lock()
                    .unwrap()
                    .log
                    .iter()
                    .any(|l| l.1.contains("tune started"))
            );
            h.send(Cmd::StopCq);
            let tuned = wait(8, &|s| {
                s.log
                    .iter()
                    .any(|l| l.1.contains("Antenna tuner: tune started (high SWR)"))
            });
            assert_eq!(tuned, auto, "auto-tune {auto}");
            h.send(Cmd::Stop);
        }
    }

    /// Both methods at once: a stepping search is running (auto-tune off)
    /// when the SDR spots the demo station. The scan pauses, the radio goes
    /// straight to the hit and links. The search steps 40 m, where K7BOT
    /// can't be heard, while the simulated dongle stays on 20 m: only the
    /// SDR hit can bring the radio to him, so there is no race with
    /// stepping (on 20 m a slow runner let stepping lock on first).
    #[test]
    fn sdr_hit_interrupts_stepping_search() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0BOTH".into(),
            auto_answer: true,
            ..Default::default()
        };
        cfg.sdr.wide_search = true;
        cfg.sdr.wide_auto = false;
        cfg.download_dir = std::env::temp_dir().join(format!("ws-both-{}", std::process::id()));
        h.send(Cmd::Start(cfg));
        let t = Instant::now();
        while !h.state.lock().unwrap().mercury_ok {
            assert!(t.elapsed() < Duration::from_secs(20), "no Mercury");
            std::thread::sleep(Duration::from_millis(100));
        }
        h.send(Cmd::StartSearch(SearchScope::Band("40m".into())));
        let t = Instant::now();
        loop {
            {
                let g = h.state.lock().unwrap();
                assert!(
                    !g.log.iter().any(|l| l.1.contains("Whistle on ")),
                    "stepping 40 m heard a whistle"
                );
                if g.link.as_ref().is_some_and(|l| l.peer == "K7BOT") {
                    assert!(
                        (g.rig.freq_hz - 14_103_150.0).abs() <= 55.0,
                        "{}",
                        g.rig.freq_hz
                    );
                    assert!(
                        g.log
                            .iter()
                            .any(|l| l.1.contains("pausing the")
                                && l.1.contains("search to fine-lock")),
                        "the SDR hit didn't interrupt the scan"
                    );
                    break;
                }
            }
            if t.elapsed() > Duration::from_secs(60) {
                let g = h.state.lock().unwrap();
                let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
                panic!("no link\n{}", log.join("\n"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        h.send(Cmd::Stop);
        std::thread::sleep(Duration::from_millis(500));
    }

    /// Wide search: the (simulated) SDR spots the demo station anywhere in
    /// the segment, the radio tunes there and fine-locks on its own audio,
    /// hears the CQ and connects — without stepping through channels.
    #[test]
    fn demo_wide_search_finds_and_links() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0WIDE".into(),
            auto_answer: true,
            ..Default::default()
        };
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
                    assert!(
                        (g.rig.freq_hz - 14_103_150.0).abs() <= 55.0,
                        "{}",
                        g.rig.freq_hz
                    );
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
                let reply = g
                    .chat
                    .iter()
                    .any(|c| c.who == Who::Them && c.text.contains("wide-search link"));
                let delivered = g.chat.iter().any(|c| c.who == Who::Me && c.delivered);
                if reply && delivered {
                    break;
                }
                if t2.elapsed() > Duration::from_secs(40) {
                    let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
                    panic!(
                        "no chat over the link; chat {:?}\n{}",
                        g.chat
                            .iter()
                            .map(|c| (&c.who, &c.text, c.delivered))
                            .collect::<Vec<_>>(),
                        log.join("\n")
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let (when, dial) = saw_hit.expect("SDR hit");
        // The SDR's reading carries its crystal error (+240 Hz in the demo);
        // the lock above is within Mercury's window anyway.
        assert!((dial - 14_103_390.0).abs() < 5.0, "SDR saw {dial}");
        eprintln!(
            "SDR hit after {:.1} s at {dial:.1}; linked after {:.1} s",
            when.as_secs_f64(),
            t.elapsed().as_secs_f64()
        );
        h.send(Cmd::Stop);
    }

    /// The whole demo flow, headless: search the 20m segment, find the
    /// whistling demo station, lock on within Mercury's +/-55 Hz window,
    /// hear its CQFRAME, connect, chat, and send a zipped file.
    #[test]
    fn demo_search_connect_chat_file() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0TEST".into(),
            auto_answer: true,
            ..Default::default()
        };
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
        wait("link with K7BOT", 150, &|s| {
            s.link.as_ref().is_some_and(|l| l.peer == "K7BOT")
        });
        let f = h.state.lock().unwrap().rig.freq_hz;
        assert!((f - 14_103_150.0).abs() <= 55.0, "locked {f}");
        wait("bot greeting", 30, &|s| {
            s.chat.iter().any(|c| c.who == Who::Them)
        });
        h.send(Cmd::Chat("hello from the test".into()));
        wait("bot reply", 30, &|s| {
            s.chat
                .iter()
                .any(|c| c.who == Who::Them && c.text.contains("hello from the test"))
        });
        let dir = cfg.download_dir.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("log.txt"), "QSO log line\n".repeat(400)).unwrap();
        h.send(Cmd::SendFiles {
            paths: vec![dir.clone()],
            zip: true,
        });
        wait("file delivered", 60, &|s| {
            s.transfers
                .iter()
                .any(|t| t.dir == Dir::Out && t.state == XferState::Done(None))
        });
        h.send(Cmd::Stop);
        std::fs::remove_dir_all(&cfg.download_dir).ok();
    }

    /// Auto-answering a CQ that Search found is a call in progress, like
    /// Connect by hand: `calling` is set until the link comes up, so
    /// whistle watch, beacons, schedules and SDR auto-tune leave the radio
    /// alone while Mercury links up.
    #[test]
    fn demo_auto_answer_marks_calling() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0AUTO".into(),
            auto_answer: true,
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        h.send(Cmd::Start(cfg));
        let t = Instant::now();
        while !h.state.lock().unwrap().mercury_ok {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "Mercury didn't start"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        h.send(Cmd::StartSearch(SearchScope::CurrentBand));
        let mut seen_calling = false;
        while t.elapsed() < Duration::from_secs(150) {
            {
                let g = h.state.lock().unwrap();
                if g.log.iter().any(|l| l.1.contains("Auto-answering K7BOT")) && g.link.is_none() {
                    assert_eq!(
                        g.calling.as_deref(),
                        Some("K7BOT"),
                        "auto-answer must mark the call"
                    );
                    seen_calling = true;
                }
                if g.link.as_ref().is_some_and(|l| l.peer == "K7BOT") {
                    assert!(g.calling.is_none(), "calling cleared once linked");
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen_calling, "never saw the auto-answer call in progress");
        h.send(Cmd::Stop);
    }

    /// Join a Roundtable right after hanging up: Mercury refuses the mode
    /// change (BUSY) until its disconnect is done, so the join waits and
    /// goes through by itself instead of failing.
    #[test]
    fn demo_join_table_waits_for_hang_up() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0JOIN".into(),
            auto_answer: true,
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        h.send(Cmd::Start(cfg));
        let wait = |what: &str, secs: u64, f: &dyn Fn(&State) -> bool| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(secs) {
                if f(&h.state.lock().unwrap()) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let g = h.state.lock().unwrap();
            let log: Vec<_> = g.log.iter().map(|l| l.1.clone()).collect();
            panic!("timed out waiting for {what}; log:\n{}", log.join("\n"));
        };
        wait("Mercury", 10, &|s| s.mercury_ok);
        // Joining mid-link (not hanging up) is refused with a clear reason.
        h.send(Cmd::StartSearch(SearchScope::CurrentBand));
        wait("link with K7BOT", 150, &|s| {
            s.link.as_ref().is_some_and(|l| l.peer == "K7BOT")
        });
        h.send(Cmd::JoinTable("Evening Table".into()));
        wait("refusal", 5, &|s| {
            s.log
                .iter()
                .any(|l| l.1.contains("end the link first, then join"))
        });
        // Hang up, and join straight away.
        h.send(Cmd::Disconnect);
        h.send(Cmd::JoinTable("Evening Table".into()));
        wait("join queued", 5, &|s| {
            s.table_pending.as_deref() == Some("Evening Table")
        });
        wait("joined after the hang-up", 30, &|s| {
            s.link.is_none() && s.table.as_ref().is_some_and(|t| t.name == "Evening Table")
        });
        assert!(h.state.lock().unwrap().table_pending.is_none());
        h.send(Cmd::Stop);
    }

    /// Depot in the demo: mail posted for K7BOT waits in Departures, goes
    /// across when linked and moves to Delivered on K7BOT's MAILACK; while
    /// away, what K7BOT types is saved to Arrivals.
    #[test]
    fn demo_depot_mail_delivered() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0MAIL".into(),
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        cfg.whistle_watch = false;
        let _ = std::fs::remove_file(
            std::env::temp_dir().join(format!("ws-depot-{}-N0MAIL.json", std::process::id())),
        );
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
        h.send(Cmd::DepotPost {
            to: "k7bot".into(),
            via: None,
            text: "See you on 40 m".into(),
        });
        wait("mail in Departures", 5, &|s| {
            s.depot.departures.len() == 1 && s.depot.departures[0].to == "K7BOT"
        });
        h.send(Cmd::SetAway(true));
        wait("away", 5, &|s| s.away);
        h.send(Cmd::DepotConnect("K7BOT".into()));
        wait("link with K7BOT", 40, &|s| s.link.is_some());
        wait("mail delivered", 40, &|s| {
            s.depot.departures.is_empty()
                && s.depot
                    .delivered
                    .iter()
                    .any(|m| m.to == "K7BOT" && m.handed.as_deref() == Some("K7BOT"))
        });
        // While away, K7BOT's chat lands in Arrivals.
        wait("chat saved to Arrivals", 40, &|s| {
            s.depot.arrivals.iter().any(|m| m.from == "K7BOT")
        });
        h.send(Cmd::Stop);
        let _ = std::fs::remove_file(
            std::env::temp_dir().join(format!("ws-depot-{}-N0MAIL.json", std::process::id())),
        );
    }

    /// Direct CAT end to end: the engine finds a (simulated) IC-7300 over
    /// a network bridge, tunes it, and shares it on a rigctld port that a
    /// Hamlib-style client (as Mercury is) can key.
    #[test]
    fn direct_cat_controls_and_shares_the_radio() {
        let radio_port = crate::cat::sim::serve_tcp(crate::cat::sim::Kind::Icom(0x94));
        let share = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: false,
            callsign: "N0CAT".into(),
            mercury_port: 1,
            mercury_bcast_port: 1,
            ..Default::default()
        };
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

    #[allow(clippy::type_complexity)]
    fn demo_station(
        call: &str,
        f: impl FnOnce(&mut Config),
    ) -> (Handle, impl Fn(&Handle, &str, u64, &dyn Fn(&State) -> bool)) {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: call.into(),
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        cfg.whistle_watch = false;
        cfg.download_dir =
            std::env::temp_dir().join(format!("ws-dl-{}-{call}", std::process::id()));
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
        wait(&h, "refusal", 30, &|s| {
            s.chat
                .iter()
                .any(|c| c.text.contains("declined the QSY") && c.text.contains("stays on 20 m"))
        });
        h.send(Cmd::Qsy(14_106_500.0));
        wait(&h, "moved", 30, &|s| {
            s.rig.freq_hz == 14_106_500.0 && s.qsy.as_ref().is_some_and(|q| q.home_hz == home)
        });
        h.send(Cmd::Disconnect);
        wait(&h, "home again", 30, &|s| {
            s.link.is_none() && s.rig.freq_hz == home && s.qsy.is_none()
        });
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
        wait(&h, "listing", 30, &|s| {
            s.library
                .as_ref()
                .is_some_and(|l| l.entries.iter().any(|e| e.name == "k7bot-qsl.txt"))
        });
        h.send(Cmd::LibFetch("k7bot-qsl.txt".into()));
        wait(&h, "file saved", 60, &|s| {
            s.transfers.iter().any(|t| {
                t.dir == Dir::In
                    && t.name == "k7bot-qsl.txt"
                    && matches!(t.state, XferState::Done(_))
            })
        });
        h.send(Cmd::LibFetch("nope.bin".into()));
        wait(&h, "refusal", 30, &|s| {
            s.chat
                .iter()
                .any(|c| c.text.contains("couldn't send that: no such file"))
        });
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
        let mut cfg = Config {
            share_library: true,
            library_dir: dir.to_string_lossy().into(),
            library_max_mb: 1.0,
            ..Default::default()
        };
        match library_answer(&cfg, false, 1, "") {
            Link::ListAns { entries, note, .. } => {
                assert!(note.is_empty() && entries.len() == 2, "{note} {entries:?}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(library_file(&cfg, false, "small.txt").unwrap().1, b"hi");
        assert!(
            library_file(&cfg, false, "big.bin")
                .unwrap_err()
                .to_string()
                .contains("too big")
        );
        assert!(library_file(&cfg, false, "../etc/passwd").is_err());
        cfg.library_only_away = true;
        assert!(
            library_file(&cfg, false, "small.txt")
                .unwrap_err()
                .to_string()
                .contains("only while")
        );
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
        assert!(
            qsy_verdict(&c, home, 14_105_000.0)
                .unwrap_err()
                .contains("turned off")
        );
        c.accept_qsy = true;
        assert!(qsy_verdict(&c, home, 14_105_000.0).is_ok());
        assert!(
            qsy_verdict(&c, home, 7_102_000.0)
                .unwrap_err()
                .contains("only QSY within 20m")
        );
        assert!(
            qsy_verdict(&c, home, 14_200_000.0)
                .unwrap_err()
                .contains("kHz from here")
        );
        c.qsy_same_band = false;
        assert!(
            qsy_verdict(&c, home, 7_102_000.0)
                .unwrap_err()
                .contains("kHz from here")
        );
        c.qsy_max_khz = 0.0;
        assert!(
            qsy_verdict(&c, home, 7_102_000.0).is_ok(),
            "{:?}",
            qsy_verdict(&c, home, 7_102_000.0)
        );
        assert!(
            qsy_verdict(&c, home, 28_120_000.0)
                .unwrap_err()
                .contains("isn't one of my QSY bands")
        );
        assert!(
            qsy_verdict(&c, home, 14_090_000.0)
                .unwrap_err()
                .contains("outside my digital segments")
        );
    }

    /// The schedule tunes the radio to the slot that's on now.
    #[test]
    fn demo_schedule_tunes_the_radio() {
        let (h, wait) = demo_station("N0SKD", |c| {
            c.schedule_on = true;
            c.schedule_only_away = false;
            c.schedule = vec![crate::schedule::Slot {
                from: "00:00".into(),
                to: "00:00".into(),
                khz: 14_108.5,
                ..Default::default()
            }];
        });
        wait(&h, "scheduled frequency", 30, &|s| {
            s.rig.freq_hz == 14_108_500.0 && s.schedule_now.is_some()
        });
        h.send(Cmd::Stop);
    }

    /// Demo with encryption required: link to K7BOT, see the 🔒 and the
    /// code, chat and send a file sealed; after hanging up and calling
    /// again, the new link has new keys (a different code).
    #[test]
    fn demo_encrypted_chat_and_file() {
        let h = spawn(|| {});
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0SEC".into(),
            e2e: e2e::Policy::Require,
            ..Default::default()
        };
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
        wait("bot's code", 30, &|s| {
            s.chat
                .iter()
                .any(|c| c.who == Who::Them && c.text.contains(&first))
        });
        assert!(
            h.state
                .lock()
                .unwrap()
                .chat
                .iter()
                .any(|c| c.who == Who::System && c.text.contains("🔒"))
        );
        h.send(Cmd::Chat("hello sealed".into()));
        wait("bot reply", 30, &|s| {
            s.chat
                .iter()
                .any(|c| c.who == Who::Them && c.text.contains("hello sealed"))
        });
        let dir = cfg.download_dir.join("src");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("log.txt"), "QSO log line\n".repeat(400)).unwrap();
        h.send(Cmd::SendFiles {
            paths: vec![dir.clone()],
            zip: true,
        });
        wait("file delivered", 60, &|s| {
            s.transfers
                .iter()
                .any(|t| t.dir == Dir::Out && t.state == XferState::Done(None))
        });

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
