//! A small rigctld server in front of a direct-CAT radio, so programs that
//! speak Hamlib's network protocol (Mercury with Hamlib model 2 "NET
//! rigctl", and logging or digital-mode programs) share the radio with
//! Whistlestop, which owns its serial port.
//!
//! Handles what those programs use: frequency, mode, PTT, VFO, split,
//! levels and functions, `\chk_vfo` and `\dump_state` (the handshake Hamlib's
//! NET rigctl backend performs when it opens).
//!
//! The port stays open across a restart of the station (saving Settings):
//! Hamlib's NET rigctl connects once, when its program starts, and never
//! again, so a closed port left Mercury unable to key the radio until it
//! was restarted (found on the air, 2026-11-14). While no radio is behind
//! it (between stations, or the radio didn't open) every command gets an
//! I/O error, and the same connection works again once one is.

use crate::cat::CatHandle;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The radio behind a port, if any.
type Slot = Arc<Mutex<Option<CatHandle>>>;

/// Open ports, by number, that outlive the station that opened them.
struct Open {
    port: u16,
    slot: Slot,
    task: tokio::task::JoinHandle<()>,
    addr: String,
}

static OPEN: Mutex<Vec<Open>> = Mutex::new(Vec::new());

/// Takes the radio off its port when dropped (the port itself stays open
/// for the next station; port 0, a test's, closes).
pub struct RigServer {
    slot: Slot,
    own: Option<tokio::task::JoinHandle<()>>,
    pub addr: String,
    /// The port was already open: programs connected to it stay connected.
    pub kept: bool,
}

impl Drop for RigServer {
    fn drop(&mut self) {
        // Unkey through it as it goes (a client may have keyed), as when
        // its clients were dropped with it; then let the radio go, so its
        // serial port is released.
        if let Some(r) = self.slot.lock().unwrap().take() {
            r.send(|d| {
                let _ = d.set_ptt(false);
            });
        }
        if let Some(t) = self.own.take() {
            t.abort();
        }
    }
}

/// Close the open ports no station is using, except `keep` (the one about
/// to be shared again), so another program (rigctld) can have them.
pub fn close_idle(keep: Option<u16>) {
    OPEN.lock().unwrap().retain(|o| {
        let keep = Some(o.port) == keep || o.slot.lock().unwrap().is_some();
        if !keep {
            o.task.abort();
        }
        keep && !o.task.is_finished()
    });
}

pub async fn serve(radio: CatHandle, port: u16) -> Result<RigServer> {
    if port != 0 {
        let mut open = OPEN.lock().unwrap();
        open.retain(|o| !o.task.is_finished());
        if let Some(o) = open.iter().find(|o| o.port == port) {
            let mut slot = o.slot.lock().unwrap();
            if slot.is_some() {
                anyhow::bail!("can't share the radio on port {port}: it's already shared");
            }
            *slot = Some(radio);
            return Ok(RigServer {
                slot: o.slot.clone(),
                own: None,
                addr: o.addr.clone(),
                kept: true,
            });
        }
    }
    let l = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| anyhow::anyhow!("can't share the radio on port {port}: {e}"))?;
    let addr = l.local_addr()?.to_string();
    let slot: Slot = Arc::new(Mutex::new(Some(radio)));
    let s = slot.clone();
    let task = tokio::spawn(async move {
        // A JoinSet aborts every client when the port closes.
        let mut clients = tokio::task::JoinSet::new();
        while let Ok((c, _)) = l.accept().await {
            while clients.try_join_next().is_some() {}
            clients.spawn(client(c, s.clone()));
        }
    });
    if port == 0 {
        return Ok(RigServer { slot, own: Some(task), addr, kept: false });
    }
    OPEN.lock().unwrap().push(Open {
        port,
        slot: slot.clone(),
        task,
        addr: addr.clone(),
    });
    Ok(RigServer { slot, own: None, addr, kept: false })
}

/// Unkeys the radio when a client that keyed it goes away (closes the
/// connection, drops it, or the port closes), so a program that crashes
/// or loses its link mid-transmission doesn't leave the radio on the air.
struct KeyGuard {
    slot: Slot,
    keyed: bool,
}

impl Drop for KeyGuard {
    fn drop(&mut self) {
        if self.keyed
            && let Some(r) = self.slot.lock().unwrap().as_ref()
        {
            r.send(|d| {
                let _ = d.set_ptt(false);
            });
        }
    }
}

/// The PTT state a `T` command asks for: its last argument (after an
/// optional VFO, in rigctld's VFO mode: `T VFOA 0`), a number. None for
/// anything else (no argument, `T x`): refused, never taken as "key up".
fn ptt_arg(args: &[&str]) -> Option<bool> {
    args.last()?.parse::<u8>().ok().map(|n| n != 0)
}

/// `Some(on)` if this line asks to set PTT.
fn ptt_request(line: &str) -> Option<bool> {
    let line = line.trim_start_matches(['+', ';', '|', ',']);
    let mut it = line.split_whitespace();
    if !matches!(it.next(), Some("T" | "\\set_ptt")) {
        return None;
    }
    ptt_arg(&it.collect::<Vec<_>>())
}

async fn client(s: TcpStream, slot: Slot) {
    let _ = s.set_nodelay(true);
    let (r, mut w) = s.into_split();
    let mut lines = BufReader::new(r).lines();
    let mut guard = KeyGuard {
        slot: slot.clone(),
        keyed: false,
    };
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ptt = ptt_request(line);
        // Count a key-up as ours even if it reported an error (the radio
        // may have keyed anyway); only a confirmed unkey clears it.
        if ptt == Some(true) {
            guard.keyed = true;
        }
        // The radio behind the port now (none between stations).
        let radio = slot.lock().unwrap().clone();
        let reply = match &radio {
            Some(radio) => answer(radio, line).await,
            None => idle_answer(line),
        };
        drop(radio);
        let Some(reply) = reply else {
            break;
        };
        if ptt == Some(false) && reply == OK {
            guard.keyed = false;
        }
        if w.write_all(reply.as_bytes()).await.is_err() {
            break;
        }
    }
}

const OK: &str = "RPRT 0\n";
const ENIMPL: &str = "RPRT -4\n";
const EIO: &str = "RPRT -6\n";
/// Hamlib RIG_EINVAL: a malformed argument.
const EINVAL: &str = "RPRT -1\n";

/// The reply while no radio is behind the port: the handshake (so a
/// program can connect) and an I/O error for the rest.
fn idle_answer(line: &str) -> Option<String> {
    let line = line.trim_start_matches(['+', ';', '|', ',']);
    match line.split_whitespace().next().unwrap_or("") {
        "q" | "Q" | "\\quit" => None,
        "\\chk_vfo" | "chk_vfo" => Some("0\n".into()),
        "\\dump_state" | "dump_state" => Some(dump_state(&[], &[])),
        _ => Some(EIO.into()),
    }
}

/// The reply to one command line, or None to close the connection.
async fn answer(radio: &CatHandle, line: &str) -> Option<String> {
    // Extended-response prefixes (+ ; | ,) aren't used by NET rigctl;
    // answer those requests in the plain format.
    let line = line.trim_start_matches(['+', ';', '|', ',']);
    let mut it = line.split_whitespace();
    let cmd = it.next().unwrap_or("");
    let args: Vec<String> = it.map(String::from).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let done = |r: Result<()>| {
        Some(if r.is_ok() {
            OK.to_string()
        } else {
            EIO.to_string()
        })
    };
    match cmd {
        "q" | "Q" | "\\quit" => None,
        "\\chk_vfo" | "chk_vfo" => Some("0\n".into()),
        "\\dump_state" | "dump_state" => Some(dump_state(&radio.caps.preamp_db, &radio.caps.att_db)),
        "\\get_powerstat" | "get_powerstat" => Some("1\n".into()),
        "f" | "\\get_freq" => Some(match radio.call(|d| d.freq()).await {
            Ok(f) => format!("{f:.0}\n"),
            Err(_) => EIO.into(),
        }),
        "F" | "\\set_freq" => {
            let Ok(hz) = arg(0).parse::<f64>() else {
                return Some("RPRT -1\n".into());
            };
            done(radio.call(move |d| d.set_freq(hz)).await)
        }
        "m" | "\\get_mode" => Some(match radio.call(|d| d.mode()).await {
            Ok((m, pb)) => format!("{m}\n{pb}\n"),
            Err(_) => EIO.into(),
        }),
        "M" | "\\set_mode" => {
            let (m, pb) = (arg(0), arg(1).parse::<i64>().unwrap_or(0).max(0));
            if m == "?" {
                return Some(format!("{}\n", radio.caps.modes.join(" ")));
            }
            done(radio.call(move |d| d.set_mode(&m, pb)).await)
        }
        "t" | "\\get_ptt" => Some(match radio.call(|d| d.ptt()).await {
            Ok(p) => format!("{}\n", p as u8),
            Err(_) => EIO.into(),
        }),
        "T" | "\\set_ptt" => {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            match ptt_arg(&a) {
                Some(on) => done(radio.call(move |d| d.set_ptt(on)).await),
                None => Some(EINVAL.into()),
            }
        }
        "v" | "\\get_vfo" => Some(format!(
            "{}\n",
            radio.call(|d| d.vfo()).await.unwrap_or("VFOA".into())
        )),
        "V" | "\\set_vfo" => {
            let v = arg(0);
            if v == "?" {
                return Some("VFOA VFOB\n".into());
            }
            // Asking for the VFO that's already in use always succeeds.
            let cur = radio.call(|d| d.vfo()).await.unwrap_or("VFOA".into());
            if v == cur || v == "currVFO" || v == "Main" {
                return Some(OK.into());
            }
            done(radio.call(move |d| d.set_vfo(&v)).await)
        }
        "s" | "\\get_split_vfo" => Some(format!(
            "{}\nVFOB\n",
            radio.call(|d| d.split()).await.unwrap_or(false) as u8
        )),
        "S" | "\\set_split_vfo" => {
            let on = arg(0) == "1";
            done(radio.call(move |d| d.set_split(on)).await)
        }
        "l" | "\\get_level" => {
            let n = arg(0);
            if n == "?" {
                return Some(format!("{}\n", radio.caps.get_levels.join(" ")));
            }
            Some(match radio.call(move |d| d.get_level(&n)).await {
                Ok(v) => format!("{v}\n"),
                Err(_) => ENIMPL.into(),
            })
        }
        "L" | "\\set_level" => {
            let (n, v) = (arg(0), arg(1).parse::<f32>().unwrap_or(0.0));
            if n == "?" {
                return Some(format!("{}\n", radio.caps.set_levels.join(" ")));
            }
            Some(match radio.call(move |d| d.set_level(&n, v)).await {
                Ok(()) => OK.into(),
                Err(_) => ENIMPL.into(),
            })
        }
        "u" | "\\get_func" => {
            let n = arg(0);
            if n == "?" {
                return Some(format!("{}\n", radio.caps.get_funcs.join(" ")));
            }
            Some(match radio.call(move |d| d.get_func(&n)).await {
                Ok(v) => format!("{}\n", v as u8),
                Err(_) => ENIMPL.into(),
            })
        }
        "U" | "\\set_func" => {
            let (n, on) = (arg(0), arg(1) == "1");
            Some(match radio.call(move |d| d.set_func(&n, on)).await {
                Ok(()) => OK.into(),
                Err(_) => ENIMPL.into(),
            })
        }
        "G" | "\\vfo_op" => {
            let op = arg(0);
            Some(match radio.call(move |d| d.vfo_op(&op)).await {
                Ok(()) => OK.into(),
                Err(_) => ENIMPL.into(),
            })
        }
        "j" | "\\get_rit" => Some(format!(
            "{}\n",
            radio.call(|d| d.offset(false)).await.unwrap_or(0)
        )),
        "z" | "\\get_xit" => Some(format!(
            "{}\n",
            radio.call(|d| d.offset(true)).await.unwrap_or(0)
        )),
        "J" | "\\set_rit" | "Z" | "\\set_xit" => {
            let xit = cmd == "Z" || cmd == "\\set_xit";
            let hz = arg(0).parse::<i32>().unwrap_or(0);
            done(radio.call(move |d| d.set_offset(xit, hz)).await)
        }
        "\\get_info" | "_" => Some(format!("{}\n", radio.desc)),
        _ => Some(ENIMPL.into()),
    }
}

/// The capability report Hamlib's NET rigctl backend reads at open
/// (protocol 1, in the layout Hamlib's own rigctld uses).
fn dump_state(preamp_db: &[i32], att_db: &[i32]) -> String {
    let modes = "0x1fff"; // AM CW USB LSB RTTY FM WFM CWR RTTYR AMS PKTLSB PKTUSB PKTFM
    let mut s = String::new();
    s += "1\n2\n0\n";
    s += &format!("100000.000000 60000000.000000 {modes} -1 -1 0x3 0x0\n0 0 0 0 0 0 0\n");
    s += &format!("1800000.000000 54000000.000000 {modes} 5000 100000 0x3 0x0\n0 0 0 0 0 0 0\n");
    s += &format!("{modes} 1\n{modes} 0\n0 0\n");
    s += "0xc 2400\n0xc 1800\n0xc 3000\n0x800 3000\n0x400 3000\n0x2 500\n0x1 6000\n0x20 12000\n0 0\n";
    s += "9999\n9999\n0\n0\n";
    let list = |v: &[i32]| v.iter().map(|x| format!("{x} ")).collect::<String>();
    s += &format!(
        "{}\n{}\n",
        list(preamp_db),
        list(att_db)
    );
    // Functions and levels aren't advertised: clients ask, and get
    // "not implemented" for what this radio can't do.
    s += "0x0\n0x0\n0x0\n0x0\n0x0\n0x0\n";
    s += "vfo_ops=0x0\nptt_type=0x1\ntargetable_vfo=0x0\nhas_set_vfo=1\nhas_get_vfo=1\nhas_set_freq=1\nhas_get_freq=1\n";
    s += "has_set_conf=0\nhas_get_conf=0\nhas_power2mW=0\nhas_mW2power=0\ntimeout=0\nrig_model=2\n";
    s += &format!(
        "rigctld_version=Whistlestop {} direct CAT\n",
        env!("CARGO_PKG_VERSION")
    );
    s += "done\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cat::sim::{Kind, SimRadio};

    /// `T` takes its last argument as the state (after a VFO, in rigctld's
    /// VFO mode), and anything that isn't a number is refused, never taken
    /// as "key up".
    #[test]
    fn ptt_argument() {
        assert_eq!(ptt_request("T 1"), Some(true));
        assert_eq!(ptt_request("T 0"), Some(false));
        assert_eq!(ptt_request("T VFOA 0"), Some(false));
        assert_eq!(ptt_request("T VFOA 1"), Some(true));
        assert_eq!(ptt_request("\\set_ptt 0"), Some(false));
        assert_eq!(ptt_request("T"), None);
        assert_eq!(ptt_request("T x"), None);
        assert_eq!(ptt_request("t"), None);
    }

    async fn ask(s: &mut BufReader<TcpStream>, cmd: &str, lines: usize) -> Vec<String> {
        s.get_mut()
            .write_all(format!("{cmd}\n").as_bytes())
            .await
            .unwrap();
        let mut out = vec![];
        for _ in 0..lines {
            let mut l = String::new();
            s.read_line(&mut l).await.unwrap();
            out.push(l.trim().to_string());
        }
        out
    }

    #[tokio::test]
    async fn speaks_rigctld() {
        let d = crate::cat::kenwood::Kenwood::new(
            Box::new(SimRadio::new(Kind::Kenwood("023"))),
            "023",
            false,
            100.0,
        )
        .unwrap();
        let srv = serve(CatHandle::start(Box::new(d)), 0).await.unwrap();
        let mut s = BufReader::new(TcpStream::connect(&srv.addr).await.unwrap());
        assert_eq!(ask(&mut s, "f", 1).await, ["14074000"]);
        assert_eq!(ask(&mut s, "F 7074000", 1).await, ["RPRT 0"]);
        assert_eq!(ask(&mut s, "\\get_freq", 1).await, ["7074000"]);
        assert_eq!(ask(&mut s, "M PKTUSB 3000", 1).await, ["RPRT 0"]);
        assert_eq!(ask(&mut s, "m", 2).await[0], "PKTUSB");
        assert_eq!(ask(&mut s, "T 1", 1).await, ["RPRT 0"]);
        assert_eq!(ask(&mut s, "t", 1).await, ["1"]);
        assert_eq!(ask(&mut s, "T 0", 1).await, ["RPRT 0"]);
        assert_eq!(ask(&mut s, "\\chk_vfo", 1).await, ["0"]);
        assert_eq!(ask(&mut s, "w FA;", 1).await, ["RPRT -4"]);
    }

    /// A client that keyed the radio and then vanished leaves it unkeyed.
    #[tokio::test]
    async fn unkeys_when_a_keying_client_disconnects() {
        let d = crate::cat::kenwood::Kenwood::new(
            Box::new(SimRadio::new(Kind::Kenwood("023"))),
            "023",
            false,
            100.0,
        )
        .unwrap();
        let srv = serve(CatHandle::start(Box::new(d)), 0).await.unwrap();
        let mut watcher = BufReader::new(TcpStream::connect(&srv.addr).await.unwrap());
        // Keys, then drops the connection without unkeying.
        let mut s = BufReader::new(TcpStream::connect(&srv.addr).await.unwrap());
        assert_eq!(ask(&mut s, "T 1", 1).await, ["RPRT 0"]);
        assert_eq!(ask(&mut watcher, "t", 1).await, ["1"]);
        drop(s);
        let t = std::time::Instant::now();
        while ask(&mut watcher, "t", 1).await != ["0"] {
            assert!(t.elapsed().as_secs() < 5, "still transmitting");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        // One that unkeyed itself, or never keyed, changes nothing when it
        // goes: the watcher's own key-up stays.
        assert_eq!(ask(&mut watcher, "T 1", 1).await, ["RPRT 0"]);
        let mut s = BufReader::new(TcpStream::connect(&srv.addr).await.unwrap());
        assert_eq!(ask(&mut s, "f", 1).await, ["14074000"]);
        drop(s);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(ask(&mut watcher, "t", 1).await, ["1"]);
        assert_eq!(ask(&mut watcher, "q", 0).await, Vec::<String>::new());
        // ...but the watcher keyed, so its quitting unkeys.
        let mut s = BufReader::new(TcpStream::connect(&srv.addr).await.unwrap());
        let t = std::time::Instant::now();
        while ask(&mut s, "t", 1).await != ["0"] {
            assert!(t.elapsed().as_secs() < 5, "still transmitting after q");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    fn kenwood() -> CatHandle {
        let d = crate::cat::kenwood::Kenwood::new(
            Box::new(SimRadio::new(Kind::Kenwood("023"))),
            "023",
            false,
            100.0,
        )
        .unwrap();
        CatHandle::start(Box::new(d))
    }

    /// Saving Settings restarts the station; a program connected to the
    /// shared port (Mercury's Hamlib PTT, which connects only when Mercury
    /// starts) stays connected and works again with the next station.
    #[tokio::test]
    async fn the_port_outlives_a_station() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let srv = serve(kenwood(), port).await.unwrap();
        assert!(!srv.kept);
        let mut s = BufReader::new(TcpStream::connect(&srv.addr).await.unwrap());
        assert_eq!(ask(&mut s, "f", 1).await, ["14074000"]);
        assert_eq!(ask(&mut s, "T 1", 1).await, ["RPRT 0"]);
        // The station stops: no radio behind the port, but it's open.
        drop(srv);
        assert_eq!(ask(&mut s, "T 1", 1).await, ["RPRT -6"]);
        assert_eq!(ask(&mut s, "\\chk_vfo", 1).await, ["0"]);
        close_idle(Some(port));
        // The next station shares the radio on the same port: the same
        // connection works.
        let srv = serve(kenwood(), port).await.unwrap();
        assert!(srv.kept);
        assert_eq!(ask(&mut s, "f", 1).await, ["14074000"]);
        assert_eq!(ask(&mut s, "T 1", 1).await, ["RPRT 0"]);
        assert_eq!(ask(&mut s, "t", 1).await, ["1"]);
        // A second share of a port in use is refused.
        assert!(serve(kenwood(), port).await.is_err());
        // In use, it isn't closed as idle.
        close_idle(None);
        assert_eq!(ask(&mut s, "T 0", 1).await, ["RPRT 0"]);
        // A station that doesn't share it: closed, the connection with it.
        drop(srv);
        close_idle(None);
        let mut l = String::new();
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), s.read_line(&mut l))
            .await
            .expect("still open")
            .unwrap_or(0);
        assert_eq!(n, 0, "{l}");
        assert!(tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_ok());
    }

    #[test]
    fn spots_ptt_requests() {
        assert_eq!(ptt_request("T 1"), Some(true));
        assert_eq!(ptt_request("+\\set_ptt 2"), Some(true));
        assert_eq!(ptt_request("T 0"), Some(false));
        assert_eq!(ptt_request("t"), None);
        assert_eq!(ptt_request("F 7074000"), None);
    }

    /// The real thing: Hamlib's own `rigctl` (model 2, NET rigctl, as
    /// Mercury uses) opens our server, reads the frequency and keys PTT.
    #[tokio::test(flavor = "multi_thread")]
    async fn hamlib_net_rigctl_can_use_it() {
        let Some(rigctl) = crate::rigsetup::hamlib_bin("rigctl") else {
            eprintln!("Hamlib not installed; skipping");
            return;
        };
        let d =
            crate::cat::icom::Icom::probe(Box::new(SimRadio::new(Kind::Icom(0x94))), 0).unwrap();
        let srv = serve(CatHandle::start(Box::new(d)), 0).await.unwrap();
        let addr = srv.addr.clone();
        let out = tokio::task::spawn_blocking(move || {
            std::process::Command::new(rigctl)
                .args([
                    "-m", "2", "-r", &addr, "f", "F", "14100000", "f", "T", "1", "t", "T", "0", "m",
                ])
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(out.status.success(), "rigctl failed: {text}\n{err}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"14074000"), "{text}\n{err}");
        assert!(lines.contains(&"14100000"), "{text}\n{err}");
        assert!(lines.contains(&"1"), "PTT read back: {text}\n{err}");
    }

    /// Hamlib's own NET rigctl, connected once (as Mercury's PTT is), keeps
    /// working after the station restarts behind the port.
    #[tokio::test(flavor = "multi_thread")]
    async fn hamlib_stays_connected_across_a_restart() {
        use std::io::{BufRead, Write};
        let Some(rigctl) = crate::rigsetup::hamlib_bin("rigctl") else {
            eprintln!("Hamlib not installed; skipping");
            return;
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let srv = serve(kenwood(), port).await.unwrap();
        let mut child = std::process::Command::new(rigctl)
            .args(["-m", "2", "-r", &srv.addr, "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        });
        // Each command's reply (rigctl prints a value, or nothing for a set).
        let mut cmd = |c: &str| {
            writeln!(stdin, "{c}").unwrap();
            stdin.flush().unwrap();
        };
        // The value read by command `c`: Hamlib 4.7 prints it alone; 4.6
        // after the command ("f 14074000"), echoes set commands the same way
        // ("F 7074000") and adds blank lines.
        let next = |rx: &std::sync::mpsc::Receiver<String>, c: &str| loop {
            let Ok(l) = rx.recv_timeout(std::time::Duration::from_secs(5)) else {
                break String::new();
            };
            match l.split_whitespace().collect::<Vec<_>>()[..] {
                [v] => break v.to_string(),
                [k, v] if k == c => break v.to_string(),
                _ => {}
            }
        };
        cmd("f");
        assert_eq!(next(&rx, "f"), "14074000");
        drop(srv);
        let srv = serve(kenwood(), port).await.unwrap();
        assert!(srv.kept);
        cmd("F 7074000");
        cmd("f");
        assert_eq!(next(&rx, "f"), "7074000");
        cmd("T 1");
        cmd("t");
        assert_eq!(next(&rx, "t"), "1");
        cmd("T 0");
        cmd("q");
        drop(stdin);
        let _ = child.wait();
        drop(srv);
        close_idle(None);
    }
}
