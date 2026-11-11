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
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc as smpsc};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 7373;
const K_JSON: u8 = 1;
const K_PCM: u8 = 2;
const K_STATE: u8 = 3;
const MAX_FRAME: usize = 64 << 20;

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

fn read_frame(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 5];
    r.read_exact(&mut h)?;
    let len = u32::from_le_bytes([h[1], h[2], h[3], h[4]]) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too big",
        ));
    }
    let mut d = vec![0u8; len];
    r.read_exact(&mut d)?;
    Ok((h[0], d))
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
/// remote operator may change; the remote-access settings stay local).
#[derive(Clone)]
pub struct Shared {
    pub handle: Handle,
    /// Settings in force (and a version number so the local window sees
    /// remote changes).
    pub cfg: Arc<Mutex<(u64, Config)>>,
    pub devices: (Vec<String>, Vec<String>),
}

/// Running server; stops when dropped.
pub struct Server {
    stop: Arc<AtomicBool>,
    pub port: u16,
    /// Who is connected now ("192.168.1.20").
    pub clients: Arc<Mutex<Vec<String>>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(
            &([127, 0, 0, 1], self.port).into(),
            Duration::from_millis(200),
        );
    }
}

type Listeners = Arc<Mutex<Vec<(u64, smpsc::Sender<Vec<i16>>)>>>;

pub fn serve(port: u16, password: &str, shared: Shared) -> Result<Server> {
    if password.trim().len() < 4 {
        bail!("set a password of at least 4 characters first");
    }
    let l = TcpListener::bind(("0.0.0.0", port))
        .map_err(|e| anyhow!("can't listen on port {port}: {e}"))?;
    let port = l.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let clients: Arc<Mutex<Vec<String>>> = Default::default();
    let listeners: Listeners = Default::default();
    let ids = Arc::new(AtomicU64::new(1));
    // One audio pump, fanned out to every listener.
    {
        let (stop, listeners, tap) = (stop.clone(), listeners.clone(), shared.handle.pcm.clone());
        std::thread::spawn(move || pump_audio(&stop, &listeners, &tap));
    }
    {
        let (stop, clients, password) = (stop.clone(), clients.clone(), password.to_string());
        std::thread::spawn(move || {
            for s in l.incoming() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(s) = s else { continue };
                let peer = s.peer_addr().map(|a| a.ip());
                if !peer.as_ref().is_ok_and(|ip| is_lan(*ip)) {
                    continue; // not from this home network: drop it silently
                }
                let (shared, clients, listeners, password, stop, id) = (
                    shared.clone(),
                    clients.clone(),
                    listeners.clone(),
                    password.clone(),
                    stop.clone(),
                    ids.fetch_add(1, Ordering::Relaxed),
                );
                std::thread::spawn(move || {
                    let who = peer.map(|p| p.to_string()).unwrap_or_default();
                    if let Err(e) =
                        serve_client(s, &who, id, &password, &shared, &clients, &listeners, &stop)
                    {
                        log(&shared.handle.state, format!("🛰 Remote {who}: {e}"));
                    }
                    clients.lock().unwrap().retain(|c| c != &who);
                    listeners.lock().unwrap().retain(|(i, _)| *i != id);
                });
            }
        });
    }
    Ok(Server {
        stop,
        port,
        clients,
    })
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
}

#[allow(clippy::too_many_arguments)]
fn serve_client(
    mut s: TcpStream,
    who: &str,
    id: u64,
    password: &str,
    shared: &Shared,
    clients: &Mutex<Vec<String>>,
    listeners: &Listeners,
    stop: &AtomicBool,
) -> Result<()> {
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let n = nonce();
    let station = shared.cfg.lock().unwrap().1.callsign.clone();
    send_json(
        &mut s,
        &ToClient::Challenge {
            nonce: n.clone(),
            station,
            version: env!("CARGO_PKG_VERSION").into(),
        },
    )?;
    let (k, d) = read_frame(&mut s)?;
    let ok = k == K_JSON
        && matches!(serde_json::from_slice::<ToServer>(&d), Ok(ToServer::Auth { mac }) if mac == hex::encode(hmac(password.as_bytes(), n.as_bytes())));
    if !ok {
        std::thread::sleep(Duration::from_secs(2)); // slow down guessing
        send_json(&mut s, &ToClient::Denied("wrong password".into()))?;
        bail!("refused (wrong password)");
    }
    s.set_read_timeout(None)?;
    let cfg = shared.cfg.lock().unwrap().1.clone();
    send_json(
        &mut s,
        &ToClient::Welcome {
            cfg: Box::new(cfg),
            devices: shared.devices.clone(),
        },
    )?;
    clients.lock().unwrap().push(who.to_string());
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
        let (st, sdr, alive) = (
            shared.handle.state.clone(),
            shared.handle.sdr.clone(),
            alive.clone(),
        );
        std::thread::spawn(move || {
            let mut last_state = Instant::now() - Duration::from_secs(1);
            let mut last_row: Option<(f64, usize)> = None;
            let mut seq = st.lock().unwrap().audio_seq;
            let res: std::io::Result<()> = (|| {
                while alive.load(Ordering::Relaxed) {
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
            res
        })
    };

    // Reader: commands and requests.
    let res: Result<()> = (|| {
        while !stop.load(Ordering::Relaxed) && alive.load(Ordering::Relaxed) {
            let (k, d) = read_frame(&mut s)?;
            if k != K_JSON {
                continue;
            }
            match serde_json::from_slice::<ToServer>(&d)? {
                ToServer::Auth { .. } => {}
                ToServer::Cmd(c) => match *c {
                    Cmd::Start(mut c) | Cmd::Update(mut c) => {
                        // Their settings, except how this computer is reached
                        // remotely, which only this computer may change.
                        let mut g = shared.cfg.lock().unwrap();
                        c.remote_serve = g.1.remote_serve;
                        c.remote_port = g.1.remote_port;
                        c.remote_password = g.1.remote_password.clone();
                        let _ = c.save();
                        g.0 += 1;
                        g.1 = c.clone();
                        shared.handle.send(Cmd::Start(c));
                    }
                    // Listening is per computer: this one's speakers are its own.
                    Cmd::Monitor(_) | Cmd::MonitorVolume(_) | Cmd::Stop => {}
                    c => shared.handle.send(c),
                },
                ToServer::Listen(on) => {
                    let mut l = listeners.lock().unwrap();
                    l.retain(|(i, _)| *i != id);
                    if on {
                        l.push((id, pcm_tx.clone()));
                    }
                }
                ToServer::Upload { name, hex } => {
                    let dir = std::env::temp_dir().join("whistlestop-remote-uploads");
                    std::fs::create_dir_all(&dir)?;
                    let safe: String = Path::new(&name)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or("file".into());
                    let p = dir.join(&safe);
                    std::fs::write(&p, hex::decode(hex)?)?;
                    shared.handle.send(Cmd::SendFiles {
                        paths: vec![p],
                        zip: false,
                    });
                }
                ToServer::Fetch { path } => {
                    let dl = shared.cfg.lock().unwrap().1.download_dir.clone();
                    let ok = Path::new(&path)
                        .canonicalize()
                        .ok()
                        .zip(dl.canonicalize().ok())
                        .is_some_and(|(p, d)| p.starts_with(d));
                    if ok {
                        let data = std::fs::read(&path)?;
                        let name = Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or("file".into());
                        let _ = reply_tx.send(ToClient::File {
                            path,
                            name,
                            hex: hex::encode(data),
                        });
                    } else {
                        let _ = reply_tx.send(ToClient::Note(
                            "that file isn't in the download folder".into(),
                        ));
                    }
                }
            }
        }
        Ok(())
    })();
    alive.store(false, Ordering::Relaxed);
    let _ = writer.join();
    log(
        &shared.handle.state,
        format!("🛰 Remote: {who} disconnected"),
    );
    match res {
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof) =>
        {
            Ok(())
        }
        r => r,
    }
}

// ------------------------------------------------------------------ client

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
        let (k, d) = read_frame(s)?;
        if k != K_JSON {
            bail!("unexpected reply");
        }
        Ok(serde_json::from_slice(&d)?)
    };
    let ToClient::Challenge { nonce, station, .. } = read_json(&mut s)? else {
        bail!("that isn't a Whistlestop")
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
        std::thread::spawn(move || {
            let mut fetched: HashSet<String> = HashSet::new();
            while alive.load(Ordering::Relaxed) {
                let Ok((k, d)) = read_frame(&mut r) else {
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
                                && fetched.insert(p.to_string_lossy().to_string())
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
                        Ok(ToClient::File { name, hex, .. }) => {
                            let saved = hex::decode(hex)
                                .map_err(anyhow::Error::from)
                                .and_then(|data| save_unique(&download_dir, &name, &data));
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
    if data.len() > 50 << 20 {
        bail!("{name} is over 50 MB");
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
