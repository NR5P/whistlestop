//! Remote operation over your LAN: run Whistlestop on the computer wired to
//! the radio, and operate it from another Whistlestop anywhere at home.
//!
//! * The radio computer serves (Remote → "Let other computers operate this
//!   one"; off by default). It accepts only LAN addresses (private,
//!   link-local and loopback) and a password, proven by HMAC-SHA256 over a
//!   fresh challenge (the password itself never crosses the network).
//! * The other computer connects and gets a `Handle` like a local engine's:
//!   the station's state arrives four times a second, every command goes
//!   to the radio computer, files you send are uploaded from this computer
//!   and files you receive are copied here, and Listen plays the radio's
//!   received audio (8 kHz PCM, 16 KB/s) on this computer's speakers.
//! * What belongs to the radio computer stays its own: its folders, the
//!   file library, Mercury, the radio connection, Demo mode and the remote
//!   settings (see `merge_remote_cfg`). Only files uploaded by the remote
//!   operator can be sent from it, and only received files copied back.
//! * Limits: 8 connections at once, 15 s and 4 KB to log in, and after 10
//!   wrong passwords within a minute all logins are refused for 5 minutes.
//!   Turning sharing off (or changing the password) disconnects everyone.
//!
//! The stream isn't encrypted: it's for your own LAN, not the internet.
//!
//! Wire: frames of `kind: u8, len: u32 LE, payload`. Kind 1 is JSON
//! (`ToServer` / `ToClient`), kind 2 a block of 16-bit PCM samples, kind 3
//! the station `State` as JSON.

use crate::config::Config;
use crate::engine::{Cmd, Dir, Handle, PcmTap, SharedState, State, XferState};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as smpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 7373;
const K_JSON: u8 = 1;
const K_PCM: u8 = 2;
const K_STATE: u8 = 3;
/// Largest frame once logged in.
const MAX_FRAME: usize = 64 << 20;
/// Largest frame before logging in (an `Auth` message is under 100 bytes).
const MAX_LOGIN_FRAME: usize = 4 << 10;
/// Largest file uploaded to, or copied from, the radio computer. Files
/// travel hex-encoded (twice the size), which must fit in a frame.
pub const MAX_FILE: usize = 25 << 20;
const _: () = assert!(MAX_FILE * 2 + (64 << 10) <= MAX_FRAME);
/// Connections at once (logged in or not).
const MAX_CONNS: usize = 8;
/// Logins in progress from one computer at once: a stranger can't fill
/// every slot with connections that never log in.
const MAX_PENDING_PER_IP: usize = 2;
/// Time to complete the login, from connecting.
const LOGIN_TIME: Duration = Duration::from_secs(15);
/// This many wrong passwords within `FAIL_WINDOW` from one computer
/// pauses that computer's logins for `LOCKOUT` (only its own: a stranger
/// guessing can't lock the owner out).
const FAIL_LIMIT: usize = 10;
const FAIL_WINDOW: Duration = Duration::from_secs(60);
const LOCKOUT: Duration = Duration::from_secs(5 * 60);
/// The backstop: this many wrong passwords within `ALL_FAIL_WINDOW` from
/// every computer together (someone changing address to get round the
/// per-computer limit) pauses, for `LOCKOUT`, logins from computers that
/// haven't logged in this session. Those that have keep working.
const ALL_FAIL_LIMIT: usize = 30;
const ALL_FAIL_WINDOW: Duration = Duration::from_secs(10 * 60);

#[derive(Serialize, Deserialize, Debug)]
enum ToServer {
    Auth {
        mac: String,
    },
    Cmd(Box<Cmd>),
    Listen(bool),
    /// A file to send from the radio computer (already zipped if asked).
    Upload {
        name: String,
        hex: String,
    },
    /// Copy a received file to the remote computer.
    Fetch {
        path: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
enum ToClient {
    Challenge {
        nonce: String,
        station: String,
        version: String,
    },
    Welcome {
        cfg: Box<Config>,
        devices: (Vec<String>, Vec<String>),
    },
    Denied(String),
    Sdr {
        running: bool,
        status: String,
        row: Option<crate::sdr::WideRow>,
    },
    File {
        path: String,
        name: String,
        hex: String,
    },
    Note(String),
    /// New radio-audio waterfall rows.
    Rows(Vec<Vec<f32>>),
}

fn write_frame(w: &mut impl Write, kind: u8, data: &[u8]) -> std::io::Result<()> {
    let mut f = Vec::with_capacity(data.len() + 5);
    f.push(kind);
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(data);
    w.write_all(&f)
}

/// One frame of at most `max` bytes (checked before allocating).
fn read_frame(r: &mut impl Read, max: usize) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 5];
    r.read_exact(&mut h)?;
    let len = u32::from_le_bytes([h[1], h[2], h[3], h[4]]) as usize;
    if len > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too big",
        ));
    }
    let mut d = vec![0u8; len];
    r.read_exact(&mut d)?;
    Ok((h[0], d))
}

/// Reads from a socket until a deadline, however the bytes trickle in.
struct Deadline<'a> {
    s: &'a TcpStream,
    until: Instant,
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "took too long to log in",
            ));
        }
        self.s.set_read_timeout(Some(left))?;
        let mut s: &TcpStream = self.s;
        s.read(buf)
    }
}

fn send_json<T: Serialize>(w: &mut impl Write, m: &T) -> std::io::Result<()> {
    write_frame(
        w,
        K_JSON,
        &serde_json::to_vec(m).map_err(std::io::Error::other)?,
    )
}

/// HMAC-SHA256 (RFC 2104).
pub fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let inner = Sha256::new()
        .chain_update(k.map(|b| b ^ 0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(k.map(|b| b ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// Does `mac_hex` prove the password for this nonce? (Constant time.)
fn mac_ok(password: &str, nonce: &str, mac_hex: &str) -> bool {
    let want = hmac(password.as_bytes(), nonce.as_bytes());
    let Ok(got) = hex::decode(mac_hex) else {
        return false;
    };
    got.len() == want.len()
        && std::hint::black_box(
            got.iter()
                .zip(want.iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b)),
        ) == 0
}

/// A home-network address: private, link-local, unique-local or loopback.
pub fn is_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local(),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_lan(IpAddr::V4(v4));
            }
            let s = v.segments()[0];
            v.is_loopback() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
        }
    }
}

/// This computer's address on the LAN (as others would reach it).
pub fn lan_address() -> Option<IpAddr> {
    // Connecting a UDP socket sends nothing; it only picks the route.
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.254.254.254:9").ok()?;
    s.local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_unspecified())
}

fn nonce() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("random");
    hex::encode(b)
}

// ------------------------------------------------------------------ server

/// What the radio computer shares: its engine, and its settings (which a
/// remote operator may change, except what belongs to this computer: see
/// `merge_remote_cfg`).
#[derive(Clone)]
pub struct Shared {
    pub handle: Handle,
    /// Settings in force (and a version number so the local window sees
    /// remote changes).
    pub cfg: Arc<Mutex<(u64, Config)>>,
    pub devices: (Vec<String>, Vec<String>),
}

/// Where files uploaded by remote operators are kept (in this user's own
/// cache folder, not the shared temp folder). Only files in here may be
/// sent on a remote operator's say-so.
pub fn uploads_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Whistlestop")
        .join("remote-uploads")
}

/// The settings a remote operator sent, with everything that belongs to
/// this computer kept as it is here: its folders and file library, Depot
/// file, Mercury (program, address), Demo mode, the radio connection (rig
/// control, CAT, rigctld), its speakers, the remote-access settings, and
/// sound devices other than ones this computer listed. The operator may
/// change how the station operates: callsign, CQ, search, bandwidth,
/// automation, QSY, schedule, SDR and so on.
///
/// Also returns what the operator tried to change but couldn't (for a
/// note), by name.
pub fn merge_remote_cfg(
    local: &Config,
    mut c: Config,
    devices: &(Vec<String>, Vec<String>),
) -> (Config, Vec<&'static str>) {
    let mut kept = Vec::new();
    macro_rules! keep {
        ($label:expr; $($f:ident),+) => {{
            let mut differs = false;
            $(
                if c.$f != local.$f {
                    differs = true;
                    c.$f = local.$f.clone();
                }
            )+
            if differs && !$label.is_empty() {
                kept.push($label);
            }
        }};
    }
    keep!("Download folder"; download_dir);
    keep!("File library"; share_library, library_dir, library_only_away, library_max_mb);
    keep!("Depot file"; depot_file);
    keep!("Mercury"; mercury_bin, mercury_host, mercury_port, mercury_bcast_port);
    keep!("Demo mode"; demo, demo_real_mercury);
    keep!("Radio connection"; rig_mode, rigctld, managed_rig, cat);
    // This computer's speakers and remote access: silently its own.
    keep!(""; monitor, monitor_volume, monitor_device);
    keep!(""; remote_serve, remote_port, remote_password, remote_addr);
    // Sound devices: one of those this computer offered, or the default.
    let listed = |v: &String, list: &[String]| v.is_empty() || list.contains(v);
    let mut bad_audio = false;
    if c.audio_in != local.audio_in && !listed(&c.audio_in, &devices.0) {
        c.audio_in = local.audio_in.clone();
        bad_audio = true;
    }
    if c.audio_out != local.audio_out && !listed(&c.audio_out, &devices.1) {
        c.audio_out = local.audio_out.clone();
        bad_audio = true;
    }
    if bad_audio {
        kept.push("Sound devices");
    }
    (c, kept)
}

/// Wrong passwords from one computer.
#[derive(Default)]
struct LoginGuard {
    fails: VecDeque<Instant>,
    until: Option<Instant>,
}

impl LoginGuard {
    /// While logins are paused: how much longer.
    fn locked(&self, now: Instant) -> Option<Duration> {
        self.until
            .map(|u| u.saturating_duration_since(now))
            .filter(|d| !d.is_zero())
    }

    /// A wrong password. True when it pauses logins.
    fn failed(&mut self, now: Instant) -> bool {
        while self
            .fails
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > FAIL_WINDOW)
        {
            self.fails.pop_front();
        }
        self.fails.push_back(now);
        if self.fails.len() >= FAIL_LIMIT && self.locked(now).is_none() {
            self.fails.clear();
            self.until = Some(now + LOCKOUT);
            return true;
        }
        false
    }

    /// Nothing to remember: no recent wrong password, not paused.
    fn idle(&self, now: Instant) -> bool {
        self.locked(now).is_none()
            && self
                .fails
                .back()
                .is_none_or(|t| now.saturating_duration_since(*t) > FAIL_WINDOW)
    }
}

/// Wrong passwords: per computer, and from all of them together.
#[derive(Default)]
struct Guards {
    per_ip: HashMap<IpAddr, LoginGuard>,
    all: VecDeque<Instant>,
    /// The backstop's pause, while it lasts.
    all_until: Option<Instant>,
    /// Computers that logged in this session: the backstop leaves them be.
    known: HashSet<IpAddr>,
}

/// What a wrong password set off.
#[derive(Debug, Default, PartialEq)]
struct Paused {
    /// That computer's logins.
    this_computer: bool,
    /// Logins from computers new to this session (the backstop).
    new_computers: bool,
}

impl Guards {
    /// Refused now, and for how much longer, and why.
    fn refused(&self, ip: IpAddr, now: Instant) -> Option<String> {
        if let Some(left) = self.per_ip.get(&ip).and_then(|g| g.locked(now)) {
            return Some(lockout_msg(left));
        }
        let left = self
            .all_until
            .map(|u| u.saturating_duration_since(now))
            .filter(|d| !d.is_zero())?;
        if self.known.contains(&ip) {
            return None;
        }
        let min = left.as_secs().div_ceil(60).max(1);
        Some(format!(
            "too many wrong passwords from several computers; logins from computers that haven't logged in yet are paused for {min} more minute{}",
            if min == 1 { "" } else { "s" }
        ))
    }

    /// A wrong password from `ip`. Computers with nothing left to remember
    /// are forgotten, so one trying from address after address doesn't
    /// grow the table for ever.
    fn failed(&mut self, ip: IpAddr, now: Instant) -> Paused {
        self.per_ip.retain(|_, g| !g.idle(now));
        let this_computer = self.per_ip.entry(ip).or_default().failed(now);
        while self
            .all
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > ALL_FAIL_WINDOW)
        {
            self.all.pop_front();
        }
        self.all.push_back(now);
        let paused = self.all_until.is_some_and(|u| u > now);
        let new_computers = self.all.len() >= ALL_FAIL_LIMIT && !paused;
        if new_computers {
            self.all.clear();
            self.all_until = Some(now + LOCKOUT);
        }
        Paused {
            this_computer,
            new_computers,
        }
    }

    fn logged_in(&mut self, ip: IpAddr) {
        self.known.insert(ip);
    }
}

fn lockout_msg(left: Duration) -> String {
    let min = left.as_secs().div_ceil(60).max(1);
    format!(
        "too many wrong passwords; remote logins are paused for {min} more minute{}",
        if min == 1 { "" } else { "s" }
    )
}

type Listeners = Arc<Mutex<Vec<(u64, smpsc::Sender<Vec<i16>>)>>>;
/// Every open connection (a clone of its socket), to cut them all off.
type Conns = Arc<Mutex<HashMap<u64, TcpStream>>>;

/// What every connection of one server shares.
struct Srv {
    shared: Shared,
    password: String,
    clients: Arc<Mutex<Vec<String>>>,
    listeners: Listeners,
    stop: Arc<AtomicBool>,
    guard: Mutex<Guards>,
}

/// Running server; stops when dropped (see `stop`).
pub struct Server {
    stop: Arc<AtomicBool>,
    pub port: u16,
    /// Who is connected now ("192.168.1.20").
    pub clients: Arc<Mutex<Vec<String>>>,
    conns: Conns,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    /// Stop sharing: disconnect everyone, and free the port before
    /// returning (so it can be listened on again at once).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop polls; when it ends, the listener is closed.
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        // No new connections now: cut off the open ones (their reader and
        // writer threads see the socket close and end).
        for s in self.conns.lock().unwrap().values() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Serve remote operation to the local network (the desktop's Settings →
/// Remote operation).
pub fn serve(port: u16, password: &str, shared: Shared) -> Result<Server> {
    serve_on(std::net::Ipv4Addr::UNSPECIFIED, port, password, shared)
}

/// Serve remote operation on one address only, e.g. loopback for the test
/// bench, which reaches a phone through `adb forward` or the Simulator's
/// shared network and must not open the station to the LAN.
pub fn serve_on(addr: std::net::Ipv4Addr, port: u16, password: &str, shared: Shared) -> Result<Server> {
    if password.trim().len() < 4 {
        bail!("set a password of at least 4 characters first");
    }
    let l = TcpListener::bind((addr, port))
        .map_err(|e| anyhow!("can't listen on port {port}: {e}"))?;
    let port = l.local_addr()?.port();
    // Polled, so stopping closes it promptly.
    l.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let conns: Conns = Default::default();
    let srv = Arc::new(Srv {
        shared,
        password: password.to_string(),
        clients: Default::default(),
        listeners: Default::default(),
        stop: stop.clone(),
        guard: Default::default(),
    });
    // One audio pump, fanned out to every listener.
    {
        let (stop, listeners, tap) = (
            stop.clone(),
            srv.listeners.clone(),
            srv.shared.handle.pcm.clone(),
        );
        std::thread::spawn(move || pump_audio(&stop, &listeners, &tap));
    }
    let accept = {
        let (srv, conns) = (srv.clone(), conns.clone());
        std::thread::spawn(move || accept_loop(l, &srv, &conns))
    };
    Ok(Server {
        stop,
        port,
        clients: srv.clients.clone(),
        conns,
        accept: Some(accept),
    })
}

fn accept_loop(l: TcpListener, srv: &Arc<Srv>, conns: &Conns) {
    let mut next_id = 1u64;
    // Logins in progress, per computer.
    let pending: Arc<Mutex<HashMap<std::net::IpAddr, usize>>> = Default::default();
    while !srv.stop.load(Ordering::SeqCst) {
        let (mut s, peer) = match l.accept() {
            Ok(x) => x,
            Err(_) => {
                // Nothing waiting (or out of sockets for now).
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if !is_lan(peer.ip()) {
            continue; // not from this home network: drop it silently
        }
        // Some systems hand out accepted sockets non-blocking like the listener.
        if s.set_nonblocking(false).is_err() {
            continue;
        }
        let id = next_id;
        next_id += 1;
        {
            let mut c = conns.lock().unwrap();
            if c.len() >= MAX_CONNS {
                drop(c);
                let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = send_json(
                    &mut s,
                    &ToClient::Denied("too many connections at once; try again shortly".into()),
                );
                continue;
            }
            let Ok(k) = s.try_clone() else { continue };
            c.insert(id, k);
        }
        {
            let mut p = pending.lock().unwrap();
            let n = p.entry(peer.ip()).or_insert(0);
            if *n >= MAX_PENDING_PER_IP {
                drop(p);
                conns.lock().unwrap().remove(&id);
                let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = send_json(
                    &mut s,
                    &ToClient::Denied("too many logins from this computer at once; try again shortly".into()),
                );
                continue;
            }
            *n += 1;
        }
        let (srv, conns, pending) = (srv.clone(), conns.clone(), pending.clone());
        std::thread::spawn(move || {
            let who = peer.ip().to_string();
            let logged_in = login(&mut s, &srv, peer.ip());
            {
                let mut p = pending.lock().unwrap();
                if let Some(n) = p.get_mut(&peer.ip()) {
                    *n -= 1;
                    if *n == 0 {
                        p.remove(&peer.ip());
                    }
                }
            }
            let res = logged_in.and_then(|()| {
                srv.clients.lock().unwrap().push(who.clone());
                let r = session(s, &who, id, &srv);
                let mut c = srv.clients.lock().unwrap();
                if let Some(i) = c.iter().position(|c| c == &who) {
                    c.remove(i);
                }
                r
            });
            if let Err(e) = res
                && !srv.stop.load(Ordering::SeqCst)
            {
                log(&srv.shared.handle.state, format!("🛰 Remote {who}: {e}"));
            }
            conns.lock().unwrap().remove(&id);
            srv.listeners.lock().unwrap().retain(|(i, _)| *i != id);
        });
    }
}

fn log(st: &SharedState, line: String) {
    let mut g = st.lock().unwrap();
    g.log
        .push_back((crate::engine::chrono_lite::Clock::now(), line));
}

fn pump_audio(stop: &AtomicBool, listeners: &Listeners, tap: &PcmTap) {
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
        let any = !listeners.lock().unwrap().is_empty();
        let block: Vec<i16> = {
            let mut t = tap.lock().unwrap();
            t.on = any;
            if !any {
                t.samples.clear();
                continue;
            }
            t.samples
                .drain(..)
                .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect()
        };
        if !block.is_empty() {
            listeners
                .lock()
                .unwrap()
                .retain(|(_, tx)| tx.send(block.clone()).is_ok());
        }
    }
    let mut t = tap.lock().unwrap();
    t.on = false;
    t.samples.clear();
}

/// The challenge and the password proof, within `LOGIN_TIME` and with
/// small frames only. Wrong passwords are slowed and counted.
fn login(s: &mut TcpStream, srv: &Srv, ip: std::net::IpAddr) -> Result<()> {
    s.set_nodelay(true)?;
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    let refused = srv.guard.lock().unwrap().refused(ip, Instant::now());
    if let Some(why) = refused {
        send_json(s, &ToClient::Denied(why))?;
        bail!("refused (logins paused after too many wrong passwords)");
    }
    let until = Instant::now() + LOGIN_TIME;
    let n = nonce();
    let station = srv.shared.cfg.lock().unwrap().1.callsign.clone();
    send_json(
        s,
        &ToClient::Challenge {
            nonce: n.clone(),
            station,
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    let (k, d) = read_frame(&mut Deadline { s: &*s, until }, MAX_LOGIN_FRAME)?;
    let mac = match (k == K_JSON).then(|| serde_json::from_slice::<ToServer>(&d)) {
        Some(Ok(ToServer::Auth { mac })) => mac,
        _ => String::new(),
    };
    // Paused meanwhile: even the right password waits.
    let refused = srv.guard.lock().unwrap().refused(ip, Instant::now());
    if let Some(why) = refused {
        send_json(s, &ToClient::Denied(why))?;
        bail!("refused (logins paused after too many wrong passwords)");
    }
    if !mac_ok(&srv.password, &n, &mac) {
        let paused = srv.guard.lock().unwrap().failed(ip, Instant::now());
        if paused.this_computer {
            log(
                &srv.shared.handle.state,
                format!(
                    "🛰 Remote: {FAIL_LIMIT} wrong passwords within a minute from {ip}; refusing its logins for {} minutes",
                    LOCKOUT.as_secs() / 60
                ),
            );
        }
        if paused.new_computers {
            log(
                &srv.shared.handle.state,
                format!(
                    "🛰 Remote: {ALL_FAIL_LIMIT} wrong passwords within {} minutes from several computers; refusing logins from computers that haven't logged in yet for {} minutes",
                    ALL_FAIL_WINDOW.as_secs() / 60,
                    LOCKOUT.as_secs() / 60
                ),
            );
        }
        std::thread::sleep(Duration::from_secs(2)); // slow down guessing
        send_json(s, &ToClient::Denied("wrong password".into()))?;
        bail!("refused (wrong password)");
    }
    srv.guard.lock().unwrap().logged_in(ip);
    s.set_read_timeout(None)?;
    Ok(())
}

/// A logged-in remote operator: state, audio and replies out; commands
/// and requests in.
fn session(mut s: TcpStream, who: &str, id: u64, srv: &Srv) -> Result<()> {
    let shared = &srv.shared;
    let cfg = shared.cfg.lock().unwrap().1.clone();
    send_json(
        &mut s,
        &ToClient::Welcome {
            cfg: Box::new(cfg),
            devices: shared.devices.clone(),
        },
    )?;
    log(
        &shared.handle.state,
        format!("🛰 Remote: {who} is operating this station"),
    );

    // Writer: state 4×/s, the newest SDR row, audio, replies.
    let (reply_tx, reply_rx) = smpsc::channel::<ToClient>();
    let (pcm_tx, pcm_rx) = smpsc::channel::<Vec<i16>>();
    let mut w = s.try_clone()?;
    let alive = Arc::new(AtomicBool::new(true));
    let writer = {
        let (st, sdr, alive, stop) = (
            shared.handle.state.clone(),
            shared.handle.sdr.clone(),
            alive.clone(),
            srv.stop.clone(),
        );
        std::thread::spawn(move || {
            let mut last_state = Instant::now() - Duration::from_secs(1);
            let mut last_row: Option<(f64, usize)> = None;
            let mut seq = st.lock().unwrap().audio_seq;
            let res: std::io::Result<()> = (|| {
                while alive.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                    while let Ok(m) = reply_rx.try_recv() {
                        send_json(&mut w, &m)?;
                    }
                    while let Ok(b) = pcm_rx.try_recv() {
                        let bytes: Vec<u8> = b.iter().flat_map(|v| v.to_le_bytes()).collect();
                        write_frame(&mut w, K_PCM, &bytes)?;
                    }
                    if last_state.elapsed() >= Duration::from_millis(250) {
                        last_state = Instant::now();
                        let (js, rows) = {
                            let g = st.lock().unwrap();
                            let fresh =
                                (g.audio_seq.saturating_sub(seq) as usize).min(g.audio_rows.len());
                            seq = g.audio_seq;
                            let rows: Vec<Vec<f32>> = g
                                .audio_rows
                                .iter()
                                .skip(g.audio_rows.len() - fresh)
                                .map(|r| r.iter().map(|v| (v * 10.0).round() / 10.0).collect())
                                .collect();
                            (
                                serde_json::to_vec(&*g).map_err(std::io::Error::other)?,
                                rows,
                            )
                        };
                        write_frame(&mut w, K_STATE, &js)?;
                        if !rows.is_empty() {
                            send_json(&mut w, &ToClient::Rows(rows))?;
                        }
                        let (running, status, row) = {
                            let g = sdr.lock().unwrap();
                            (g.running, g.status.clone(), g.rows.back().cloned())
                        };
                        let key = row.as_ref().map(|r| {
                            (
                                r.start_hz,
                                r.levels.len()
                                    ^ r.levels
                                        .iter()
                                        .take(8)
                                        .map(|v| v.to_bits() as usize)
                                        .fold(0, |a, b| a ^ b),
                            )
                        });
                        let row = if key != last_row {
                            last_row = key;
                            row
                        } else {
                            None
                        };
                        send_json(
                            &mut w,
                            &ToClient::Sdr {
                                running,
                                status,
                                row,
                            },
                        )?;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Ok(())
            })();
            alive.store(false, Ordering::Relaxed);
            // A client that stopped reading (an app that hung, Wi-Fi gone)
            // ends here, at the write timeout: close the connection so the
            // reader, blocked waiting for it, ends the session too.
            let _ = w.shutdown(std::net::Shutdown::Both);
            res
        })
    };

    // Reader: commands and requests. Only a broken connection ends it;
    // a bad request gets a note back.
    let mut noted = Vec::new();
    let res: Result<()> = (|| {
        while !srv.stop.load(Ordering::Relaxed) && alive.load(Ordering::Relaxed) {
            let (k, d) = read_frame(&mut s, MAX_FRAME)?;
            if k != K_JSON {
                continue;
            }
            let msg = match serde_json::from_slice::<ToServer>(&d) {
                Ok(m) => m,
                Err(e) => {
                    let _ = reply_tx.send(ToClient::Note(format!(
                        "The radio computer didn't understand a request ({e}). Are both computers running the same Whistlestop version?"
                    )));
                    continue;
                }
            };
            match msg {
                ToServer::Auth { .. } => {}
                ToServer::Cmd(c) => handle_cmd(*c, shared, &reply_tx, &mut noted),
                ToServer::Listen(on) => {
                    let mut l = srv.listeners.lock().unwrap();
                    l.retain(|(i, _)| *i != id);
                    if on {
                        l.push((id, pcm_tx.clone()));
                    }
                }
                ToServer::Upload { name, hex } => {
                    if let Err(e) = take_upload(&name, &hex, shared) {
                        let _ = reply_tx.send(ToClient::Note(format!(
                            "Couldn't send {name} from the radio computer: {e}"
                        )));
                    }
                }
                ToServer::Fetch { path } => {
                    let _ = reply_tx.send(fetch(path, shared));
                }
            }
        }
        Ok(())
    })();
    alive.store(false, Ordering::Relaxed);
    let _ = s.shutdown(std::net::Shutdown::Both);
    let _ = writer.join();
    log(
        &shared.handle.state,
        format!("🛰 Remote: {who} disconnected"),
    );
    match res {
        Err(e)
            if srv.stop.load(Ordering::SeqCst)
                || e.downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof) =>
        {
            Ok(())
        }
        r => r,
    }
}

/// A command from the remote operator, for this computer's engine.
fn handle_cmd(
    c: Cmd,
    shared: &Shared,
    reply: &smpsc::Sender<ToClient>,
    noted: &mut Vec<&'static str>,
) {
    match c {
        // Settings: theirs, except what belongs to this computer. Start
        // (Settings → Apply) restarts the station as it does here; Update
        // (CQ, search and automation settings) doesn't.
        Cmd::Start(new) => apply_cfg(new, true, shared, reply, noted),
        Cmd::Update(new) => apply_cfg(new, false, shared, reply, noted),
        // Only files the operator uploaded may be sent from here.
        Cmd::SendFiles { paths, zip } => match uploaded_files(&paths) {
            Some(paths) => shared.handle.send(Cmd::SendFiles { paths, zip }),
            None => {
                let _ = reply.send(ToClient::Note(
                    "Only files sent from your computer can be sent from the radio computer."
                        .into(),
                ));
            }
        },
        // Listening is per computer: this one's speakers are its own.
        Cmd::Monitor(_) | Cmd::MonitorVolume(_) | Cmd::Stop => {}
        c => shared.handle.send(c),
    }
}

fn apply_cfg(
    new: Config,
    restart: bool,
    shared: &Shared,
    reply: &smpsc::Sender<ToClient>,
    noted: &mut Vec<&'static str>,
) {
    let mut g = shared.cfg.lock().unwrap();
    let (c, kept) = merge_remote_cfg(&g.1, new, &shared.devices);
    // Say so once (the operator's window keeps its copy and sends it again).
    if !kept.is_empty() && kept != *noted {
        let _ = reply.send(ToClient::Note(format!(
            "These settings belong to the radio computer and weren't changed: {}",
            kept.join(", ")
        )));
    }
    *noted = kept;
    // (Unit tests don't touch this computer's settings file.)
    if !cfg!(test) {
        let _ = c.save();
    }
    g.0 += 1;
    g.1 = c.clone();
    drop(g);
    shared
        .handle
        .send(if restart { Cmd::Start(c) } else { Cmd::Update(c) });
}

/// These paths, if every one is a file a remote operator uploaded (as the
/// real paths, so links can't lead elsewhere).
fn uploaded_files(paths: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let dir = uploads_dir().canonicalize().ok()?;
    if paths.is_empty() {
        return None;
    }
    paths
        .iter()
        .map(|p| {
            p.canonicalize()
                .ok()
                .filter(|c| c.starts_with(&dir) && c != &dir && c.is_file())
        })
        .collect()
}

/// A file uploaded by the remote operator: keep it and send it.
fn take_upload(name: &str, hex: &str, shared: &Shared) -> Result<()> {
    if hex.len() > MAX_FILE * 2 {
        bail!("it's over {} MB", MAX_FILE >> 20);
    }
    let data = hex::decode(hex).map_err(|e| anyhow!("it arrived damaged ({e})"))?;
    let dir = uploads_dir();
    std::fs::create_dir_all(&dir)?;
    let safe: String = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or("file".into());
    let p = dir.join(&safe);
    std::fs::write(&p, data)?;
    shared.handle.send(Cmd::SendFiles {
        paths: vec![p],
        zip: false,
    });
    Ok(())
}

/// A received file (in this computer's download folder), for the remote
/// operator's computer.
fn fetch(path: String, shared: &Shared) -> ToClient {
    let dl = shared.cfg.lock().unwrap().1.download_dir.clone();
    let p = match (Path::new(&path).canonicalize(), dl.canonicalize()) {
        (Ok(p), Ok(d)) if p.starts_with(&d) && p.is_file() => p,
        _ => return ToClient::Note("that file isn't in the download folder".into()),
    };
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or("file".into());
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_FILE as u64) {
        return ToClient::Note(format!(
            "{name} is over {} MB: copy it from the radio computer's download folder yourself",
            MAX_FILE >> 20
        ));
    }
    match std::fs::read(&p) {
        Ok(data) => ToClient::File {
            path,
            name,
            hex: hex::encode(data),
        },
        Err(e) => ToClient::Note(format!("Couldn't copy {name} here: {e}")),
    }
}

// ------------------------------------------------------------------ client

/// The record (in the download folder) of received files already copied
/// from radio computers.
const COPIED_RECORD: &str = ".whistlestop-copied";

fn copied_key(station: &str, path: &str, size: u64) -> String {
    format!("{station}\t{path}\t{size}")
}

fn load_copied(dir: &Path) -> HashSet<String> {
    std::fs::read_to_string(dir.join(COPIED_RECORD))
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn note_copied(dir: &Path, key: &str) {
    use std::io::Write as _;
    let _ = std::fs::create_dir_all(dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(COPIED_RECORD))
    {
        let _ = writeln!(f, "{key}");
    }
}

/// Operating a radio computer from here.
pub struct Client {
    /// Behaves like a local engine's handle.
    pub handle: Handle,
    /// "N0CALL at 192.168.1.20:7373".
    pub desc: String,
    /// The radio computer's settings and sound devices.
    pub cfg: Config,
    pub devices: (Vec<String>, Vec<String>),
    pub alive: Arc<AtomicBool>,
    /// News for the window (files copied here, problems).
    pub notes: Arc<Mutex<Vec<String>>>,
    stream: TcpStream,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

pub fn connect(
    addr: &str,
    password: &str,
    download_dir: PathBuf,
    monitor_device: String,
    repaint: Arc<dyn Fn() + Send + Sync>,
) -> Result<Client> {
    let addr = if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:{DEFAULT_PORT}")
    };
    let sa = std::net::ToSocketAddrs::to_socket_addrs(&addr)?
        .next()
        .ok_or_else(|| anyhow!("can't find {addr}"))?;
    let mut s = TcpStream::connect_timeout(&sa, Duration::from_secs(5))
        .map_err(|e| anyhow!("can't reach {addr}: {e}"))?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let read_json = |s: &mut TcpStream| -> Result<ToClient> {
        let (k, d) = read_frame(s, MAX_FRAME)?;
        if k != K_JSON {
            bail!("unexpected reply");
        }
        serde_json::from_slice(&d).map_err(|_| anyhow!("unexpected reply"))
    };
    let mut version_note = None;
    let (nonce, station) = match read_json(&mut s) {
        Ok(ToClient::Challenge {
            nonce,
            station,
            version,
        }) => {
            // Different versions may still work together (unknown requests
            // and settings are skipped with a note), so warn, don't refuse.
            let ours = env!("CARGO_PKG_VERSION");
            if version != ours {
                version_note = Some(format!(
                    "{station} runs Whistlestop {version} and this computer runs {ours}: some things may not work until both run the same version"
                ));
            }
            (nonce, station)
        }
        Ok(ToClient::Denied(why)) => bail!("the radio computer refused: {why}"),
        Ok(_) => bail!("that isn't a Whistlestop"),
        Err(e) => bail!("{addr} didn't answer as a Whistlestop: {e}"),
    };
    send_json(
        &mut s,
        &ToServer::Auth {
            mac: hex::encode(hmac(password.as_bytes(), nonce.as_bytes())),
        },
    )?;
    let (cfg, devices) = match read_json(&mut s)? {
        ToClient::Welcome { cfg, devices } => (*cfg, devices),
        ToClient::Denied(why) => bail!("{station} refused: {why}"),
        _ => bail!("unexpected reply"),
    };
    s.set_read_timeout(None)?;
    let state: SharedState = Default::default();
    let sdr: crate::sdr::Shared = Default::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Cmd>();
    let handle = Handle {
        tx,
        state: state.clone(),
        sdr: sdr.clone(),
        pcm: Default::default(),
    };
    let alive = Arc::new(AtomicBool::new(true));
    let notes: Arc<Mutex<Vec<String>>> = Default::default();
    notes.lock().unwrap().extend(version_note);
    let listening = Arc::new(AtomicBool::new(false));
    let writer = Arc::new(Mutex::new(s.try_clone()?));

    // Speaker for remote Listen (its own thread: audio streams aren't Send
    // everywhere).
    let (spk_tx, spk_rx) = smpsc::channel::<SpeakerMsg>();
    {
        let notes = notes.clone();
        std::thread::spawn(move || speaker(spk_rx, &monitor_device, &notes));
    }

    // Reader: state, waterfall, audio, files.
    {
        let (alive, notes, listening, writer, spk_tx, repaint) = (
            alive.clone(),
            notes.clone(),
            listening.clone(),
            writer.clone(),
            spk_tx.clone(),
            repaint.clone(),
        );
        let mut r = s.try_clone()?;
        let station = station.clone();
        std::thread::spawn(move || {
            // What earlier connections already copied here (kept in the
            // download folder), so a reconnect doesn't copy it all again.
            let mut fetched: HashSet<String> = load_copied(&download_dir);
            while alive.load(Ordering::Relaxed) {
                let Ok((k, d)) = read_frame(&mut r, MAX_FRAME) else {
                    break;
                };
                match k {
                    K_STATE => {
                        let Ok(mut st) = serde_json::from_slice::<State>(&d) else {
                            continue;
                        };
                        st.monitor_on = listening.load(Ordering::Relaxed);
                        // Received files: copy each here once.
                        for t in st.transfers.iter().filter(|t| t.dir == Dir::In) {
                            if let XferState::Done(Some(p)) = &t.state
                                && fetched.insert(copied_key(&station, &p.to_string_lossy(), t.size))
                            {
                                let _ = send_json(
                                    &mut *writer.lock().unwrap(),
                                    &ToServer::Fetch {
                                        path: p.to_string_lossy().to_string(),
                                    },
                                );
                            }
                        }
                        let mut g = state.lock().unwrap();
                        st.audio_rows = std::mem::take(&mut g.audio_rows);
                        st.audio_seq = g.audio_seq;
                        *g = st;
                        drop(g);
                        repaint();
                    }
                    K_PCM => {
                        let v: Vec<f32> = d
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| i16::from_le_bytes(*b) as f32 / 32767.0)
                            .collect();
                        let _ = spk_tx.send(SpeakerMsg::Audio(v));
                    }
                    K_JSON => match serde_json::from_slice::<ToClient>(&d) {
                        Ok(ToClient::Sdr {
                            running,
                            status,
                            row,
                        }) => {
                            let mut g = sdr.lock().unwrap();
                            g.running = running;
                            g.status = status;
                            if let Some(r) = row {
                                g.rows.push_back(r);
                                while g.rows.len() > 64 {
                                    g.rows.pop_front();
                                }
                            }
                        }
                        Ok(ToClient::File { name, hex, path }) => {
                            let saved = hex::decode(hex)
                                .map_err(anyhow::Error::from)
                                .and_then(|data| {
                                    let p = save_unique(&download_dir, &name, &data)?;
                                    note_copied(&download_dir, &copied_key(&station, &path, data.len() as u64));
                                    Ok(p)
                                });
                            notes.lock().unwrap().push(match saved {
                                Ok(p) => {
                                    format!("Copied {name} to this computer ({})", p.display())
                                }
                                Err(e) => format!("Couldn't copy {name} here: {e}"),
                            });
                        }
                        Ok(ToClient::Note(n)) => notes.lock().unwrap().push(n),
                        Ok(ToClient::Rows(rows)) => {
                            let mut g = state.lock().unwrap();
                            g.audio_seq += rows.len() as u64;
                            g.audio_rows.extend(rows);
                            while g.audio_rows.len() > 64 {
                                g.audio_rows.pop_front();
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            alive.store(false, Ordering::Relaxed);
            repaint();
        });
    }

    // Forwarder: the GUI's commands, with the ones that belong here kept here.
    {
        let (alive, writer, notes, listening) = (
            alive.clone(),
            writer.clone(),
            notes.clone(),
            listening.clone(),
        );
        std::thread::spawn(move || {
            while let Some(c) = rx.blocking_recv() {
                if !alive.load(Ordering::Relaxed) {
                    break;
                }
                let msg = match c {
                    Cmd::Monitor(on) => {
                        listening.store(on, Ordering::Relaxed);
                        let _ = spk_tx.send(if on { SpeakerMsg::On } else { SpeakerMsg::Off });
                        ToServer::Listen(on)
                    }
                    Cmd::MonitorVolume(v) => {
                        let _ = spk_tx.send(SpeakerMsg::Volume(v));
                        continue;
                    }
                    Cmd::Stop => continue,
                    Cmd::SendFiles { paths, zip } => match upload(&paths, zip) {
                        Ok(m) => m,
                        Err(e) => {
                            notes.lock().unwrap().push(format!("Couldn't send: {e}"));
                            continue;
                        }
                    },
                    c => ToServer::Cmd(Box::new(c)),
                };
                if send_json(&mut *writer.lock().unwrap(), &msg).is_err() {
                    alive.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });
    }
    Ok(Client {
        handle,
        desc: format!("{station} at {addr}"),
        cfg,
        devices,
        alive,
        notes,
        stream: s,
    })
}

/// Files picked on this computer, ready to send from the radio computer.
fn upload(paths: &[PathBuf], zip: bool) -> Result<ToServer> {
    let single = paths.len() == 1 && paths[0].is_file();
    let (name, data) = if zip || !single {
        let z = crate::ziputil::zip_paths(paths, None)?;
        (z.name, z.data)
    } else {
        (
            paths[0]
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or("file".into()),
            std::fs::read(&paths[0])?,
        )
    };
    if data.len() > MAX_FILE {
        bail!(
            "{name} is {:.1} MB; files sent through the radio computer can be at most {} MB",
            data.len() as f64 / (1 << 20) as f64,
            MAX_FILE >> 20
        );
    }
    Ok(ToServer::Upload {
        name,
        hex: hex::encode(data),
    })
}

fn save_unique(dir: &Path, name: &str, data: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let safe = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or("file".into());
    let mut p = dir.join(&safe);
    let mut i = 2;
    while p.exists() {
        let stem = Path::new(&safe)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = Path::new(&safe)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        p = dir.join(format!("{stem}-{i}{ext}"));
        i += 1;
    }
    std::fs::write(&p, data)?;
    Ok(p)
}

enum SpeakerMsg {
    On,
    Off,
    Volume(f32),
    Audio(Vec<f32>),
}

fn speaker(rx: smpsc::Receiver<SpeakerMsg>, device: &str, notes: &Mutex<Vec<String>>) {
    let mut m: Option<crate::audio::Monitor> = None;
    let mut vol = 0.7;
    while let Ok(msg) = rx.recv() {
        match msg {
            SpeakerMsg::On => match crate::audio::Monitor::start(device, vol) {
                Ok(x) => m = Some(x),
                Err(e) => notes
                    .lock()
                    .unwrap()
                    .push(format!("Can't play the radio's audio here: {e}")),
            },
            SpeakerMsg::Off => m = None,
            SpeakerMsg::Volume(v) => {
                vol = v;
                if let Some(x) = &m {
                    x.set_volume(v);
                }
            }
            SpeakerMsg::Audio(a) => {
                if let Some(x) = m.as_mut() {
                    x.push_rx(&a);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole path: a demo station served, operated from a "remote"
    /// Whistlestop over loopback.
    #[test]
    fn operate_a_station_remotely() {
        let mut cfg = Config {
            demo: true,
            demo_real_mercury: false,
            callsign: "N0RIG".into(),
            ..Default::default()
        };
        cfg.sdr.enabled = false;
        cfg.whistle_watch = false;
        let local = crate::engine::spawn(|| {});
        local.send(Cmd::Start(cfg.clone()));
        let shared = Shared {
            handle: local.clone(),
            cfg: Arc::new(Mutex::new((0, cfg.clone()))),
            devices: (vec!["Radio USB".into()], vec![]),
        };
        let srv = serve(0, "hunter22", shared).unwrap();
        let addr = format!("127.0.0.1:{}", srv.port);
        let dl = std::env::temp_dir().join(format!("ws-remote-dl-{}", std::process::id()));
        // Wrong password: refused.
        let e = connect(&addr, "nope", dl.clone(), String::new(), Arc::new(|| {}))
            .err()
            .unwrap();
        assert!(e.to_string().contains("refused"), "{e}");
        // Through this computer's real LAN address too, as another computer would.
        if let Some(ip) = lan_address() {
            let lan = format!("{ip}:{}", srv.port);
            let c2 = connect(&lan, "hunter22", dl.clone(), String::new(), Arc::new(|| {}))
                .unwrap_or_else(|e| panic!("via {lan}: {e}"));
            drop(c2);
        }
        let c = connect(
            &addr,
            "hunter22",
            dl.clone(),
            String::new(),
            Arc::new(|| {}),
        )
        .unwrap();
        assert!(c.desc.starts_with("N0RIG at"), "{}", c.desc);
        assert_eq!(c.devices.0, ["Radio USB"]);
        let wait = |what: &str, secs: u64, f: &dyn Fn(&State) -> bool| {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(secs) {
                if f(&c.handle.state.lock().unwrap()) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let log: Vec<_> = local
                .state
                .lock()
                .unwrap()
                .log
                .iter()
                .map(|l| l.1.clone())
                .collect();
            panic!(
                "timed out waiting for {what}; radio computer's log:\n{}",
                log.join("\n")
            );
        };
        wait("mirrored state", 15, &|s| {
            s.mycall == "N0RIG" && s.mercury_ok
        });
        let t = Instant::now();
        while srv.clients.lock().unwrap().len() != 1 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(srv.clients.lock().unwrap().len(), 1);
        c.handle.send(Cmd::SetFreq(14_101_000.0));
        wait("remote tune", 10, &|s| s.rig.freq_hz == 14_101_000.0);
        c.handle.send(Cmd::Connect("K7BOT".into()));
        wait("link", 40, &|s| s.link.is_some());
        c.handle.send(Cmd::Chat("hello from the den".into()));
        wait("reply", 40, &|s| {
            s.chat
                .iter()
                .any(|l| l.text.contains("hello from the den") && l.who == crate::engine::Who::Them)
        });
        // Listen: the radio computer starts tapping its received audio.
        c.handle.send(Cmd::Monitor(true));
        let t = Instant::now();
        while !local.pcm.lock().unwrap().on && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            local.pcm.lock().unwrap().on,
            "audio tap on for the listener"
        );
        c.handle.send(Cmd::Monitor(false));
        // A file picked on the remote computer is sent from the radio's.
        let f = std::env::temp_dir().join(format!("ws-remote-up-{}.txt", std::process::id()));
        std::fs::write(&f, "QSL card text ".repeat(50)).unwrap();
        c.handle.send(Cmd::SendFiles {
            paths: vec![f.clone()],
            zip: false,
        });
        wait("upload offered", 30, &|s| {
            s.transfers
                .iter()
                .any(|t| t.dir == Dir::Out && t.name.starts_with("ws-remote-up"))
        });
        let _ = std::fs::remove_file(f);
        drop(c);
        local.send(Cmd::Stop);
    }

    /// A radio computer without an engine: commands land in the receiver.
    fn fake_shared() -> (Shared, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = Handle {
            tx,
            state: Default::default(),
            sdr: Default::default(),
            pcm: Default::default(),
        };
        let cfg = Config {
            callsign: "N0RIG".into(),
            download_dir: std::env::temp_dir().join(format!("ws-remote-rx-{}", std::process::id())),
            ..Default::default()
        };
        let shared = Shared {
            handle,
            cfg: Arc::new(Mutex::new((0, cfg))),
            devices: (vec!["Radio USB".into()], vec!["Radio USB".into()]),
        };
        (shared, rx)
    }

    fn wait_until(secs: f64, f: impl Fn() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs_f64(secs) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        f()
    }

    #[test]
    fn remote_settings_keep_this_computers_own() {
        let local = Config {
            callsign: "N0RIG".into(),
            download_dir: "/home/rig/Downloads/Whistlestop".into(),
            library_dir: "/home/rig/Library".into(),
            share_library: false,
            depot_file: "/home/rig/depot.json".into(),
            mercury_bin: "/opt/mercury/mercury".into(),
            demo: false,
            rigctld: "127.0.0.1:4532".into(),
            audio_in: "Radio USB".into(),
            remote_serve: true,
            remote_password: "hunter22".into(),
            ..Default::default()
        };
        let mut theirs = local.clone();
        theirs.callsign = "N0OP".into();
        theirs.cq_cycles = 9;
        theirs.auto_answer = true;
        theirs.beacon = true;
        theirs.download_dir = "/".into();
        theirs.library_dir = "/".into();
        theirs.share_library = true;
        theirs.library_max_mb = 1e6;
        theirs.depot_file = "/etc/passwd".into();
        theirs.mercury_bin = "/bin/sh".into();
        theirs.mercury_host = "10.9.9.9".into();
        theirs.demo = true;
        theirs.demo_real_mercury = false;
        theirs.rigctld = "10.9.9.9:4532".into();
        theirs.rig_mode = crate::rigsetup::RigMode::Managed;
        theirs.cat.port = "/dev/ttyS9".into();
        theirs.audio_in = "ether:10.9.9.9:9".into();
        theirs.audio_out = "Radio USB".into(); // offered by this computer: fine
        theirs.monitor = true;
        theirs.remote_serve = false;
        theirs.remote_password = "x".into();
        let devices = (vec!["Radio USB".into()], vec!["Radio USB".into()]);
        let (c, kept) = merge_remote_cfg(&local, theirs, &devices);
        // Theirs to change.
        assert_eq!(c.callsign, "N0OP");
        assert_eq!(c.cq_cycles, 9);
        assert!(c.auto_answer && c.beacon);
        assert_eq!(c.audio_out, "Radio USB");
        // This computer's own.
        assert_eq!(c.download_dir, local.download_dir);
        assert_eq!(c.library_dir, local.library_dir);
        assert!(!c.share_library);
        assert_eq!(c.library_max_mb, local.library_max_mb);
        assert_eq!(c.depot_file, local.depot_file);
        assert_eq!(c.mercury_bin, local.mercury_bin);
        assert_eq!(c.mercury_host, local.mercury_host);
        assert!(!c.demo);
        assert_eq!(c.demo_real_mercury, local.demo_real_mercury);
        assert_eq!(c.rigctld, local.rigctld);
        assert_eq!(c.rig_mode, local.rig_mode);
        assert_eq!(c.cat, local.cat);
        assert_eq!(c.audio_in, "Radio USB");
        assert!(!c.monitor);
        assert!(c.remote_serve);
        assert_eq!(c.remote_password, "hunter22");
        for k in [
            "Download folder",
            "File library",
            "Depot file",
            "Mercury",
            "Demo mode",
            "Radio connection",
            "Sound devices",
        ] {
            assert!(kept.contains(&k), "{k} in {kept:?}");
        }
        // Nothing of this computer's changed: nothing to say.
        let (_, kept) = merge_remote_cfg(&local, local.clone(), &devices);
        assert!(kept.is_empty(), "{kept:?}");
    }

    #[test]
    fn remote_update_stays_an_update() {
        let (shared, mut rx) = fake_shared();
        let (reply, notes) = smpsc::channel();
        let mut noted = vec![];
        let mut c = shared.cfg.lock().unwrap().1.clone();
        c.auto_answer = true;
        c.download_dir = "/".into();
        handle_cmd(Cmd::Update(c.clone()), &shared, &reply, &mut noted);
        match rx.try_recv() {
            Ok(Cmd::Update(got)) => {
                assert!(got.auto_answer);
                assert_ne!(got.download_dir, PathBuf::from("/"));
            }
            other => panic!("expected Update, got {other:?}"),
        }
        assert!(matches!(notes.try_recv(), Ok(ToClient::Note(n)) if n.contains("Download folder")));
        // Sent again: no second note.
        handle_cmd(Cmd::Update(c.clone()), &shared, &reply, &mut noted);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Update(_))));
        assert!(notes.try_recv().is_err());
        handle_cmd(Cmd::Start(c), &shared, &reply, &mut noted);
        assert!(matches!(rx.try_recv(), Ok(Cmd::Start(_))));
        let g = shared.cfg.lock().unwrap();
        assert_eq!(g.0, 3);
        assert!(g.1.auto_answer && g.1.download_dir != Path::new("/"));
    }

    #[test]
    fn only_uploaded_files_are_sent() {
        let (shared, mut rx) = fake_shared();
        let (reply, notes) = smpsc::channel();
        let mut noted = vec![];
        // A file elsewhere on this computer: refused.
        let outside = std::env::temp_dir().join(format!("ws-remote-secret-{}.txt", std::process::id()));
        std::fs::write(&outside, "secret").unwrap();
        let dir = uploads_dir();
        std::fs::create_dir_all(&dir).unwrap();
        for paths in [
            vec![outside.clone()],
            vec![dir.join("..").join("..").join("x")],
            vec![dir.clone()],
            vec![],
        ] {
            handle_cmd(
                Cmd::SendFiles { paths: paths.clone(), zip: false },
                &shared,
                &reply,
                &mut noted,
            );
            assert!(rx.try_recv().is_err(), "{paths:?} must not be sent");
            assert!(matches!(notes.try_recv(), Ok(ToClient::Note(_))));
        }
        // One uploaded file among others: still refused.
        let up = dir.join(format!("ws-remote-ok-{}.txt", std::process::id()));
        std::fs::write(&up, "73").unwrap();
        handle_cmd(
            Cmd::SendFiles { paths: vec![up.clone(), outside.clone()], zip: false },
            &shared,
            &reply,
            &mut noted,
        );
        assert!(rx.try_recv().is_err());
        // An uploaded file: sent.
        handle_cmd(
            Cmd::SendFiles { paths: vec![up.clone()], zip: true },
            &shared,
            &reply,
            &mut noted,
        );
        match rx.try_recv() {
            Ok(Cmd::SendFiles { paths, zip: true }) => {
                assert_eq!(paths, vec![up.canonicalize().unwrap()])
            }
            other => panic!("expected SendFiles, got {other:?}"),
        }
        // Fetch outside the download folder: refused too.
        assert!(matches!(
            fetch(outside.to_string_lossy().to_string(), &shared),
            ToClient::Note(_)
        ));
        let _ = std::fs::remove_file(up);
        let _ = std::fs::remove_file(outside);
    }

    /// The bench's server (phones) answers on loopback and not on the LAN.
    #[test]
    fn serve_on_loopback_is_not_on_the_lan() {
        let (shared, _rx) = fake_shared();
        let srv = serve_on(std::net::Ipv4Addr::LOCALHOST, 0, "hunter22", shared).unwrap();
        assert!(TcpStream::connect(("127.0.0.1", srv.port)).is_ok());
        if let Some(lan) = lan_address().filter(|ip| !ip.is_loopback()) {
            let r = TcpStream::connect_timeout(&std::net::SocketAddr::new(lan, srv.port), Duration::from_secs(2));
            assert!(r.is_err(), "reachable on the LAN at {lan}:{}", srv.port);
        }
    }

    #[test]
    fn oversized_frame_before_login_is_refused() {
        let (shared, _rx) = fake_shared();
        let state = shared.handle.state.clone();
        let srv = serve(0, "hunter22", shared).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (k, _) = read_frame(&mut s, MAX_FRAME).unwrap();
        assert_eq!(k, K_JSON);
        // Claims a 1 MB login message.
        let t = Instant::now();
        let mut h = vec![K_JSON];
        h.extend_from_slice(&(1u32 << 20).to_le_bytes());
        s.write_all(&h).unwrap();
        let mut b = [0u8; 16];
        let r = s.read(&mut b);
        assert!(
            matches!(r, Ok(0) | Err(_)),
            "the connection should be closed, got {r:?}"
        );
        assert!(t.elapsed() < Duration::from_secs(4), "closed at once");
        assert!(wait_until(2.0, || {
            state.lock().unwrap().log.iter().any(|l| l.1.contains("frame too big"))
        }));
    }

    /// One computer can hold only a couple of logins open at once, so a
    /// stranger can't fill every slot with connections that never log in.
    /// A received file is copied to the operator's computer once, not
    /// again on every reconnect.
    #[test]
    fn received_files_are_copied_once_across_reconnects() {
        let (shared, _rx) = fake_shared();
        let srv_dl = shared.cfg.lock().unwrap().1.download_dir.clone();
        std::fs::create_dir_all(&srv_dl).unwrap();
        let qsl = srv_dl.join(format!("qsl-{}.txt", std::process::id()));
        std::fs::write(&qsl, "73 de N0RIG").unwrap();
        shared.handle.state.lock().unwrap().transfers.push(crate::engine::Transfer {
            fid: 1,
            dir: Dir::In,
            name: "qsl.txt".into(),
            size: 11,
            done: 11,
            zipped: false,
            files: 1,
            original: 11,
            state: XferState::Done(Some(qsl.clone())),
            eta_s: 0.0,
            started: None,
        });
        let srv = serve(0, "hunter22", shared).unwrap();
        let here = std::env::temp_dir().join(format!("ws-remote-copies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&here);
        let copies = || {
            std::fs::read_dir(&here)
                .map(|r| {
                    r.flatten()
                        .filter(|e| e.file_name().to_string_lossy().starts_with("qsl"))
                        .count()
                })
                .unwrap_or(0)
        };
        for round in 0..3 {
            let c = connect(
                &format!("127.0.0.1:{}", srv.port),
                "hunter22",
                here.clone(),
                String::new(),
                Arc::new(|| {}),
            )
            .unwrap();
            if round == 0 {
                assert!(wait_until(5.0, || copies() == 1), "first connect copies it");
            } else {
                std::thread::sleep(Duration::from_millis(1500));
            }
            drop(c);
        }
        assert_eq!(copies(), 1, "copied again on a reconnect");
        std::fs::remove_dir_all(&here).ok();
        std::fs::remove_file(&qsl).ok();
    }

    #[test]
    fn connections_are_capped() {
        let (shared, _rx) = fake_shared();
        let srv = serve(0, "hunter22", shared).unwrap();
        let mut held = vec![];
        for _ in 0..MAX_PENDING_PER_IP {
            let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let (_, d) = read_frame(&mut s, MAX_FRAME).unwrap();
            assert!(matches!(serde_json::from_slice(&d), Ok(ToClient::Challenge { .. })));
            held.push(s);
        }
        let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (_, d) = read_frame(&mut s, MAX_FRAME).unwrap();
        assert!(
            matches!(serde_json::from_slice(&d), Ok(ToClient::Denied(m)) if m.contains("this computer"))
        );
        // One leaves: room again.
        drop(held.pop());
        assert!(wait_until(3.0, || {
            let Ok(mut s) = TcpStream::connect(("127.0.0.1", srv.port)) else {
                return false;
            };
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            read_frame(&mut s, MAX_FRAME).is_ok_and(|(_, d)| {
                matches!(serde_json::from_slice(&d), Ok(ToClient::Challenge { .. }))
            })
        }));
    }

    #[test]
    fn login_must_finish_in_time() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        // A client that trickles a byte every 100 ms.
        let t = std::thread::spawn(move || {
            let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
            for _ in 0..30 {
                if c.write_all(&[1]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let (s, _) = l.accept().unwrap();
        let start = Instant::now();
        let mut b = [0u8; 100];
        let r = Deadline {
            s: &s,
            until: Instant::now() + Duration::from_millis(400),
        }
        .read_exact(&mut b);
        assert!(r.is_err());
        assert!(start.elapsed() < Duration::from_millis(1500), "{:?}", start.elapsed());
        drop(s);
        let _ = t.join();
    }

    /// Computers whose wrong passwords are old news are forgotten.
    #[test]
    fn login_guards_forget_old_computers() {
        let mut m = Guards::default();
        let t0 = Instant::now();
        for i in 0..1000u32 {
            let ip = IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + i));
            m.failed(ip, t0);
        }
        assert_eq!(m.per_ip.len(), 1000);
        let later = t0 + LOCKOUT + Duration::from_secs(1);
        let one = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        m.failed(one, later);
        assert_eq!(m.per_ip.len(), 1);
        // A paused computer is kept until its pause is over.
        for _ in 0..FAIL_LIMIT {
            m.failed(one, later);
        }
        let other = IpAddr::V4(std::net::Ipv4Addr::BROADCAST);
        m.failed(other, later + FAIL_WINDOW * 2);
        assert!(m.per_ip.get(&one).is_some_and(|g| g.locked(later + FAIL_WINDOW * 2).is_some()));
    }

    /// Guessing from address after address: after 30 wrong passwords in
    /// 10 minutes, computers new to this session are refused for 5
    /// minutes; one that has logged in is not.
    #[test]
    fn many_computers_guessing_set_off_the_backstop() {
        let mut m = Guards::default();
        let t0 = Instant::now();
        let ip = |i: u32| IpAddr::V4(std::net::Ipv4Addr::from(0x0a00_0000 + i));
        let owner = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 20));
        m.logged_in(owner);
        for i in 0..ALL_FAIL_LIMIT as u32 - 1 {
            let p = m.failed(ip(i), t0 + Duration::from_secs(i as u64 * 10));
            assert!(!p.new_computers && !p.this_computer);
        }
        let t1 = t0 + Duration::from_secs(300);
        assert!(m.refused(ip(999), t1).is_none());
        assert!(m.failed(ip(500), t1).new_computers);
        let why = m.refused(ip(999), t1).expect("a new computer is refused");
        assert!(why.contains("several computers") && why.contains("5 more minutes"), "{why}");
        assert!(m.refused(owner, t1).is_none(), "the owner was locked out");
        // Once the pause is over, everyone may try again.
        assert!(m.refused(ip(999), t1 + LOCKOUT).is_none());
        // Spread over more than 10 minutes, the same number never sets it off.
        let mut m = Guards::default();
        for i in 0..ALL_FAIL_LIMIT as u32 * 2 {
            assert!(!m.failed(ip(i), t0 + Duration::from_secs(i as u64 * 30)).new_computers);
        }
    }

    #[test]
    fn wrong_passwords_pause_that_computers_logins() {
        let mut g = LoginGuard::default();
        let t0 = Instant::now();
        // Spread out: never paused.
        for i in 0..30 {
            assert!(!g.failed(t0 + FAIL_WINDOW * i / 5));
        }
        assert!(g.locked(t0 + FAIL_WINDOW * 6).is_none());
        let t1 = t0 + FAIL_WINDOW * 10;
        for i in 0..FAIL_LIMIT - 1 {
            assert!(!g.failed(t1 + Duration::from_secs(i as u64)));
        }
        assert!(g.failed(t1 + Duration::from_secs(20)));
        assert!(g.locked(t1 + Duration::from_secs(21)).is_some());
        assert!(lockout_msg(g.locked(t1 + Duration::from_secs(21)).unwrap()).contains("5 more minutes"));
        assert!(g.locked(t1 + Duration::from_secs(20) + LOCKOUT).is_none());
        // MAC check.
        let mac = hex::encode(hmac(b"hunter22", b"abc"));
        assert!(mac_ok("hunter22", "abc", &mac));
        assert!(!mac_ok("hunter23", "abc", &mac));
        assert!(!mac_ok("hunter22", "abc", &mac[..62]));
        assert!(!mac_ok("hunter22", "abc", "zz"));
        assert!(!mac_ok("hunter22", "abc", ""));
    }

    /// Turning sharing off (or a new password) cuts off a connected
    /// operator, and the port can be listened on again at once.
    #[test]
    fn stopping_the_server_drops_clients_and_frees_the_port() {
        let (shared, _rx) = fake_shared();
        let srv = serve(0, "hunter22", shared.clone()).unwrap();
        let port = srv.port;
        let dl = std::env::temp_dir().join(format!("ws-remote-dl2-{}", std::process::id()));
        let c = connect(
            &format!("127.0.0.1:{port}"),
            "hunter22",
            dl,
            String::new(),
            Arc::new(|| {}),
        )
        .unwrap();
        assert!(wait_until(3.0, || srv.clients.lock().unwrap().len() == 1));
        assert!(c.alive.load(Ordering::Relaxed));
        drop(srv);
        assert!(
            wait_until(3.0, || !c.alive.load(Ordering::Relaxed)),
            "the operator was cut off"
        );
        // Restart on the same port right away, as the Remote window does.
        let srv2 = serve(port, "newpass1", shared).expect("port free again");
        let e = connect(
            &format!("127.0.0.1:{port}"),
            "hunter22",
            std::env::temp_dir(),
            String::new(),
            Arc::new(|| {}),
        )
        .err()
        .expect("old password refused");
        assert!(e.to_string().contains("refused"), "{e}");
        drop(srv2);
    }

    #[test]
    fn hmac_matches_rfc4231() {
        // RFC 4231 test case 2.
        assert_eq!(
            hex::encode(hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn lan_only() {
        for ok in [
            "192.168.1.20",
            "10.0.0.5",
            "172.16.3.1",
            "127.0.0.1",
            "169.254.1.1",
            "::1",
            "fd12::1",
            "fe80::1",
            "::ffff:192.168.1.2",
        ] {
            assert!(is_lan(ok.parse().unwrap()), "{ok}");
        }
        for bad in ["8.8.8.8", "203.0.113.9", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!is_lan(bad.parse().unwrap()), "{bad}");
        }
    }
}
