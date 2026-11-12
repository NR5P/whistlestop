//! Shared harness for the soak/stress tests (tests/soak_*.rs).
//!
//! Two ways to run a station through the public engine API:
//!
//! - `demo()`: the product's own Demo mode (simulated radio and band, the
//!   built-in fake Mercury with K7BOT, real-time delays: a connect takes
//!   about 6 s, the "air" runs at 1400 bit/s).
//! - `fast()`: a real (non-demo) station pointed at a `FakeMercury` with
//!   `fast: true` (every delay shrunk 100x, data at 1 Gbit/s), through a
//!   small proxy that (a) records every control line the engine sends and
//!   (b) reports `BUFFER n` / `BUFFER 0` after every read of the data port
//!   as real Mercury does. The fast fake itself never reports BUFFER (its
//!   uplink is always empty again by the time it would), and the engine's
//!   pump needs those reports to send more than two file pieces.
//!   Audio goes to a TCP "ether" stub (silence in, discard out); there is
//!   no radio (rigctld points at a closed port).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use whistlestop::config::Config;
use whistlestop::engine::{Cmd, Dir, Handle, State, Who, XferState, spawn};
use whistlestop::fake_mercury::{FakeMercury, FakeOptions};

pub fn st(h: &Handle) -> MutexGuard<'_, State> {
    h.state.lock().unwrap_or_else(|e| e.into_inner())
}

/// The engine thread is still running (its command receiver exists).
pub fn alive(h: &Handle) -> bool {
    !h.tx.is_closed()
}

pub fn logs(h: &Handle) -> Vec<String> {
    st(h).log.iter().map(|l| l.1.clone()).collect()
}

pub fn has_log(h: &Handle, needle: &str) -> bool {
    st(h).log.iter().any(|l| l.1.contains(needle))
}

pub fn dump(h: &Handle) -> String {
    let g = st(h);
    let log: Vec<_> = g.log.iter().rev().take(80).rev().map(|l| l.1.clone()).collect();
    let chat: Vec<_> = g
        .chat
        .iter()
        .rev()
        .take(20)
        .rev()
        .map(|c| format!("{:?} {} {:?} delivered={}", c.who, c.from, c.text, c.delivered))
        .collect();
    let xf: Vec<_> = g
        .transfers
        .iter()
        .rev()
        .take(10)
        .map(|t| format!("{:?} fid={} {} {}/{} {:?}", t.dir, t.fid, t.name, t.done, t.size, t.state))
        .collect();
    format!(
        "alive={} running={} link={:?} calling={:?} incoming_call={} tx={} cq={:?} search.active={}\n--- log (last 80) ---\n{}\n--- chat (last 20) ---\n{}\n--- transfers (last 10) ---\n{}",
        alive(h),
        g.running,
        g.link.as_ref().map(|l| (&l.peer, &l.e2e)),
        g.calling,
        g.incoming_call,
        g.tx,
        g.cq,
        g.search.active,
        log.join("\n"),
        chat.join("\n"),
        xf.join("\n")
    )
}

/// Wait until `f` holds; panic with the station's log if it doesn't.
/// Returns how long it took (s).
pub fn wait_for(h: &Handle, what: &str, secs: f64, f: impl Fn(&State) -> bool) -> f64 {
    let t = Instant::now();
    loop {
        if f(&st(h)) {
            return t.elapsed().as_secs_f64();
        }
        if !alive(h) {
            panic!("engine thread died while waiting for {what}\n{}", dump(h));
        }
        if t.elapsed().as_secs_f64() > secs {
            panic!("timed out ({secs} s) waiting for {what}\n{}", dump(h));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn try_wait(h: &Handle, secs: f64, f: impl Fn(&State) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < secs {
        if f(&st(h)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f(&st(h))
}

/// Holds for the whole time.
pub fn holds_for(h: &Handle, secs: f64, f: impl Fn(&State) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < secs {
        if !f(&st(h)) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

static PROBE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Liveness probe: a command that only logs an error ("… isn't a
/// callsign"). True if the engine is alive and processed it within 5 s.
pub fn responsive(h: &Handle) -> bool {
    let n = PROBE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    h.send(Cmd::Connect(format!("probe {n}")));
    let needle = format!("\"PROBE {n}\" isn't a callsign");
    alive(h) && try_wait(h, 5.0, |s| s.log.iter().any(|l| l.1.contains(&needle)))
}

pub fn temp_root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ws-soak-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Settings every soak station starts from.
pub fn base_cfg(call: &str, root: &Path) -> Config {
    let mut c = Config {
        callsign: call.into(),
        demo: true,
        demo_real_mercury: false,
        whistle_watch: false,
        ..Default::default()
    };
    c.sdr.enabled = false;
    c.sdr.wide_search = false;
    c.sdr.wide_auto = false;
    c.download_dir = root.join("dl");
    c.depot_file = root.join("depot.json").to_string_lossy().into_owned();
    c
}

pub struct Stn {
    pub h: Handle,
    pub root: PathBuf,
    pub cfg: Config,
    /// Control lines the engine sent to Mercury (fast stations only).
    pub sent: Arc<Mutex<Vec<String>>>,
    pub rt: Option<tokio::runtime::Runtime>,
}

impl Stn {
    pub fn send(&self, c: Cmd) {
        self.h.send(c)
    }
    pub fn st(&self) -> MutexGuard<'_, State> {
        st(&self.h)
    }
    pub fn wait(&self, what: &str, secs: f64, f: impl Fn(&State) -> bool) -> f64 {
        wait_for(&self.h, what, secs, f)
    }
    pub fn sent_lines(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
    pub fn dl(&self) -> PathBuf {
        self.cfg.download_dir.clone()
    }
}

impl Drop for Stn {
    fn drop(&mut self) {
        self.h.send(Cmd::Stop);
        std::thread::sleep(Duration::from_millis(300));
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// A station in the product's Demo mode (simulated Mercury with K7BOT).
pub fn demo(call: &str, f: impl FnOnce(&mut Config)) -> Stn {
    let root = temp_root(call);
    let mut cfg = base_cfg(call, &root);
    f(&mut cfg);
    let h = spawn(|| {});
    h.send(Cmd::Start(cfg.clone()));
    wait_for(&h, "demo Mercury", 20.0, |s| s.mercury_ok);
    Stn { h, root, cfg, sent: Default::default(), rt: None }
}

/// A real station on a fast fake Mercury (see the module comment).
pub fn fast(call: &str, opts: FakeOptions, f: impl FnOnce(&mut Config)) -> Stn {
    let root = temp_root(call);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let fake = rt
        .block_on(FakeMercury::start(FakeOptions { fast: true, ..opts }))
        .unwrap();
    let sent: Arc<Mutex<Vec<String>>> = Default::default();
    let port = rt.block_on(proxy(fake.port, sent.clone())).unwrap();
    let ether = ether_stub();
    let mut cfg = base_cfg(call, &root);
    cfg.demo = false;
    cfg.mercury_host = "127.0.0.1".into();
    cfg.mercury_port = port;
    cfg.mercury_bcast_port = fake.bcast_port;
    cfg.rigctld = "127.0.0.1:1".into();
    cfg.audio_in = format!("ether:127.0.0.1:{ether}");
    cfg.audio_out = cfg.audio_in.clone();
    cfg.whistle_s = 1.0;
    cfg.cq_listen_s = 1.0;
    f(&mut cfg);
    let h = spawn(|| {});
    h.send(Cmd::Start(cfg.clone()));
    wait_for(&h, "fast fake Mercury", 20.0, |s| s.mercury_ok);
    Stn { h, root, cfg, sent, rt: Some(rt) }
}

/// Audio "ether": accept connections, read and discard what arrives.
fn ether_stub() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            std::thread::spawn(move || {
                use std::io::Read;
                let mut b = [0u8; 8192];
                while matches!(s.read(&mut b), Ok(n) if n > 0) {}
            });
        }
    });
    port
}

/// Control+data proxy in front of the fake Mercury (consecutive ports, as
/// Mercury's). Records the engine's control lines; reports BUFFER after
/// every read of the data port.
async fn proxy(fake_port: u16, sent: Arc<Mutex<Vec<String>>>) -> std::io::Result<u16> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    let (cl, dl, port) = loop {
        let c = TcpListener::bind("127.0.0.1:0").await?;
        let p = c.local_addr()?.port();
        if p == u16::MAX {
            continue;
        }
        if let Ok(d) = TcpListener::bind(("127.0.0.1", p + 1)).await {
            break (c, d, p);
        }
    };
    tokio::spawn(async move {
        loop {
            let Ok((ec, _)) = cl.accept().await else { return };
            let Ok((ed, _)) = dl.accept().await else { return };
            let Ok(fc) = TcpStream::connect(("127.0.0.1", fake_port)).await else { return };
            let Ok(fd) = TcpStream::connect(("127.0.0.1", fake_port + 1)).await else { return };
            for s in [&ec, &ed, &fc, &fd] {
                s.set_nodelay(true).ok();
            }
            let (mut ecr, ecw) = ec.into_split();
            let (mut edr, mut edw) = ed.into_split();
            let (mut fcr, mut fcw) = fc.into_split();
            let (mut fdr, mut fdw) = fd.into_split();
            let ecw = Arc::new(tokio::sync::Mutex::new(ecw));
            let rec = sent.clone();
            let mut a = tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let mut line = vec![];
                loop {
                    let n = match ecr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    for &b in &buf[..n] {
                        if b == b'\r' {
                            rec.lock().unwrap().push(String::from_utf8_lossy(&line).into_owned());
                            line.clear();
                        } else {
                            line.push(b);
                        }
                    }
                    if fcw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            let w1 = ecw.clone();
            let mut b = tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                loop {
                    let n = match fcr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if w1.lock().await.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            let w2 = ecw.clone();
            let mut c = tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                loop {
                    let n = match edr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if w2.lock().await.write_all(format!("BUFFER {n}\r").as_bytes()).await.is_err() {
                        break;
                    }
                    if fdw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    if w2.lock().await.write_all(b"BUFFER 0\r").await.is_err() {
                        break;
                    }
                }
            });
            let mut d = tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                loop {
                    let n = match fdr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if edw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            tokio::select! {
                _ = &mut a => {}
                _ = &mut b => {}
                _ = &mut c => {}
                _ = &mut d => {}
            }
            a.abort();
            b.abort();
            c.abort();
            d.abort();
        }
    });
    Ok(port)
}

// ------------------------------------------------------------ link steps

pub fn linked_with(s: &State, call: &str) -> bool {
    s.link.as_ref().is_some_and(|l| l.peer == call)
}

pub fn them_count(s: &State) -> usize {
    s.chat.iter().filter(|c| c.who == Who::Them).count()
}

/// Did K7BOT say anything since the newest link came up? (Counting its
/// lines wouldn't do: the chat keeps only the newest 500.)
pub fn them_since_link(s: &State) -> bool {
    s.chat
        .iter()
        .rposition(|c| c.who == Who::System && c.text.starts_with("Linked with"))
        .is_some_and(|i| s.chat[i + 1..].iter().any(|c| c.who == Who::Them))
}

/// Connect to K7BOT and wait for its greeting.
pub fn link_up(stn: &Stn, secs: f64) -> f64 {
    stn.send(Cmd::Connect("K7BOT".into()));
    let t = stn.wait("link with K7BOT", secs, |s| linked_with(s, "K7BOT"));
    stn.wait("K7BOT's greeting", secs, them_since_link);
    t
}

/// Disconnect and wait for a clean idle state. Returns the time taken.
pub fn link_down(stn: &Stn, secs: f64) -> f64 {
    // Let K7BOT's last replies cross first: a packet still in flight when
    // the link ends is kept and replayed into the next link (a bug, see
    // soak_e2e.rs hangup_race_does_not_poison_the_next_link).
    std::thread::sleep(Duration::from_millis(300));
    stn.send(Cmd::Disconnect);
    let t = stn.wait("link down", secs, |s| s.link.is_none());
    // Let bytes still in flight to the fake land while no one is linked
    // (a real reconnect takes seconds; the fast fake's takes 60 ms, and it
    // would hand the old link's leftovers to the new one).
    std::thread::sleep(Duration::from_millis(300));
    t
}

/// Send a chat line and wait for K7BOT's echo and our delivery tick.
/// Returns the time to delivery.
pub fn chat_roundtrip(stn: &Stn, text: &str, secs: f64) -> f64 {
    stn.send(Cmd::Chat(text.into()));
    let t = Instant::now();
    stn.wait(&format!("delivery of {text:?}"), secs, |s| {
        s.chat.iter().any(|c| c.who == Who::Me && c.text == text && c.delivered)
    });
    let d = t.elapsed().as_secs_f64();
    stn.wait(&format!("K7BOT's echo of {text:?}"), secs, |s| {
        s.chat.iter().any(|c| c.who == Who::Them && c.text.contains(text))
    });
    d
}

/// The idle-state invariants that must hold between links.
pub fn assert_idle(stn: &Stn, ctx: &str) {
    assert_idle_opts(stn, ctx, true)
}

/// `check_tx: false` for Demo, where the transmit indicator can stay on
/// after a link (a known bug, see soak_links.rs demo_ptt_released_after_links).
pub fn assert_idle_opts(stn: &Stn, ctx: &str, check_tx: bool) {
    let g = stn.st();
    let bad: Vec<String> = g
        .transfers
        .iter()
        .filter(|t| {
            matches!(
                t.state,
                XferState::Sending | XferState::Receiving | XferState::WaitingForYou
            ) || (t.dir == Dir::In && t.state == XferState::Offered)
        })
        .map(|t| format!("{:?} fid={} {} {:?}", t.dir, t.fid, t.name, t.state))
        .collect();
    let problems = [
        (g.link.is_some(), "link still set"),
        (g.calling.is_some(), "calling stuck"),
        (g.incoming_call, "incoming_call stuck"),
        (check_tx && g.tx, "tx (PTT) stuck on"),
        (g.qsy.is_some(), "qsy stuck"),
        (g.library.is_some(), "library listing kept"),
        (!bad.is_empty(), "transfer rows in a live state"),
    ];
    let p: Vec<&str> = problems.iter().filter(|(b, _)| *b).map(|(_, w)| *w).collect();
    drop(g);
    assert!(p.is_empty(), "{ctx}: {p:?} {bad:?}\n{}", dump(&stn.h));
}

/// Files under the download folder's `.partial` directory: (name, bytes).
pub fn partials(dl: &Path) -> Vec<(String, u64)> {
    std::fs::read_dir(dl.join(".partial"))
        .map(|r| {
            r.flatten()
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        e.metadata().map(|m| m.len()).unwrap_or(0),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One line of sizes (locks the State once: never lock it twice in one
/// statement, std's Mutex deadlocks).
pub fn sizes(h: &Handle) -> String {
    let g = st(h);
    format!(
        "chat {} transfers {} log {} heard {} mail_alerts {} arrivals {} delivered {} audio_rows {} state JSON {} B",
        g.chat.len(),
        g.transfers.len(),
        g.log.len(),
        g.heard.len(),
        g.mail_alerts.len(),
        g.depot.arrivals.len(),
        g.depot.delivered.len(),
        g.audio_rows.len(),
        serde_json::to_vec(&*g).map(|v| v.len()).unwrap_or(0)
    )
}

/// Size of the State as the remote-operation server sends it 4x a second.
pub fn state_json_len(h: &Handle) -> usize {
    serde_json::to_vec(&*st(h)).map(|v| v.len()).unwrap_or(0)
}

/// Open file descriptors and threads of this process (Linux).
pub fn fds() -> usize {
    std::fs::read_dir("/proc/self/fd").map(|r| r.count()).unwrap_or(0)
}
pub fn threads() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Threads:"))
                .and_then(|l| l[8..].trim().parse().ok())
        })
        .unwrap_or(0)
}

pub fn write_file(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}

/// Pseudo-random, incompressible bytes.
pub fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

pub fn newest_xfer(s: &State, dir: Dir, name: &str) -> Option<whistlestop::engine::Transfer> {
    s.transfers.iter().rev().find(|t| t.dir == dir && t.name == name).cloned()
}
