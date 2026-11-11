//! A small rigctld server in front of a direct-CAT radio, so programs that
//! speak Hamlib's network protocol (Mercury with Hamlib model 2 "NET
//! rigctl", and logging or digital-mode programs) share the radio with
//! Whistlestop, which owns its serial port.
//!
//! Handles what those programs use: frequency, mode, PTT, VFO, split,
//! levels and functions, `\chk_vfo` and `\dump_state` (the handshake Hamlib's
//! NET rigctl backend performs when it opens).

use crate::cat::CatHandle;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Stops the server when dropped.
pub struct RigServer {
    task: tokio::task::JoinHandle<()>,
    pub addr: String,
}

impl Drop for RigServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(radio: CatHandle, port: u16) -> Result<RigServer> {
    let l = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| anyhow::anyhow!("can't share the radio on port {port}: {e}"))?;
    let addr = l.local_addr()?.to_string();
    let task = tokio::spawn(async move {
        // A JoinSet aborts every client when the server stops, so their
        // handles on the radio go and its port is released.
        let mut clients = tokio::task::JoinSet::new();
        while let Ok((s, _)) = l.accept().await {
            while clients.try_join_next().is_some() {}
            clients.spawn(client(s, radio.clone()));
        }
    });
    Ok(RigServer { task, addr })
}

async fn client(s: TcpStream, radio: CatHandle) {
    let _ = s.set_nodelay(true);
    let (r, mut w) = s.into_split();
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(reply) = answer(&radio, line).await else { break };
        if w.write_all(reply.as_bytes()).await.is_err() {
            break;
        }
    }
}

const OK: &str = "RPRT 0\n";
const ENIMPL: &str = "RPRT -4\n";
const EIO: &str = "RPRT -6\n";

/// The reply to one command line, or None to close the connection.
async fn answer(radio: &CatHandle, line: &str) -> Option<String> {
    // Extended-response prefixes (+ ; | ,) aren't used by NET rigctl;
    // answer those requests in the plain format.
    let line = line.trim_start_matches(['+', ';', '|', ',']);
    let mut it = line.split_whitespace();
    let cmd = it.next().unwrap_or("");
    let args: Vec<String> = it.map(String::from).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let done = |r: Result<()>| Some(if r.is_ok() { OK.to_string() } else { EIO.to_string() });
    match cmd {
        "q" | "Q" | "\\quit" => None,
        "\\chk_vfo" | "chk_vfo" => Some("0\n".into()),
        "\\dump_state" | "dump_state" => Some(dump_state(radio)),
        "\\get_powerstat" | "get_powerstat" => Some("1\n".into()),
        "f" | "\\get_freq" => Some(match radio.call(|d| d.freq()).await {
            Ok(f) => format!("{f:.0}\n"),
            Err(_) => EIO.into(),
        }),
        "F" | "\\set_freq" => {
            let Ok(hz) = arg(0).parse::<f64>() else { return Some("RPRT -1\n".into()) };
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
            let on = arg(0) != "0";
            done(radio.call(move |d| d.set_ptt(on)).await)
        }
        "v" | "\\get_vfo" => Some(format!("{}\n", radio.call(|d| d.vfo()).await.unwrap_or("VFOA".into()))),
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
        "s" | "\\get_split_vfo" => Some(format!("{}\nVFOB\n", radio.call(|d| d.split()).await.unwrap_or(false) as u8)),
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
        "j" | "\\get_rit" => Some(format!("{}\n", radio.call(|d| d.offset(false)).await.unwrap_or(0))),
        "z" | "\\get_xit" => Some(format!("{}\n", radio.call(|d| d.offset(true)).await.unwrap_or(0))),
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
fn dump_state(radio: &CatHandle) -> String {
    let modes = "0x1fff"; // AM CW USB LSB RTTY FM WFM CWR RTTYR AMS PKTLSB PKTUSB PKTFM
    let mut s = String::new();
    s += "1\n2\n0\n";
    s += &format!("100000.000000 60000000.000000 {modes} -1 -1 0x3 0x0\n0 0 0 0 0 0 0\n");
    s += &format!("1800000.000000 54000000.000000 {modes} 5000 100000 0x3 0x0\n0 0 0 0 0 0 0\n");
    s += &format!("{modes} 1\n{modes} 0\n0 0\n");
    s += "0xc 2400\n0xc 1800\n0xc 3000\n0x800 3000\n0x400 3000\n0x2 500\n0x1 6000\n0x20 12000\n0 0\n";
    s += "9999\n9999\n0\n0\n";
    let list = |v: &[i32]| v.iter().map(|x| format!("{x} ")).collect::<String>();
    s += &format!("{}\n{}\n", list(&radio.caps.preamp_db), list(&radio.caps.att_db));
    // Functions and levels aren't advertised: clients ask, and get
    // "not implemented" for what this radio can't do.
    s += "0x0\n0x0\n0x0\n0x0\n0x0\n0x0\n";
    s += "vfo_ops=0x0\nptt_type=0x1\ntargetable_vfo=0x0\nhas_set_vfo=1\nhas_get_vfo=1\nhas_set_freq=1\nhas_get_freq=1\n";
    s += "has_set_conf=0\nhas_get_conf=0\nhas_power2mW=0\nhas_mW2power=0\ntimeout=0\nrig_model=2\n";
    s += &format!("rigctld_version=Whistlestop {} direct CAT\n", env!("CARGO_PKG_VERSION"));
    s += "done\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cat::sim::{Kind, SimRadio};

    async fn ask(s: &mut BufReader<TcpStream>, cmd: &str, lines: usize) -> Vec<String> {
        s.get_mut().write_all(format!("{cmd}\n").as_bytes()).await.unwrap();
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
        let d = crate::cat::kenwood::Kenwood::new(Box::new(SimRadio::new(Kind::Kenwood("023"))), "023", false, 100.0).unwrap();
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

    /// The real thing: Hamlib's own `rigctl` (model 2, NET rigctl, as
    /// Mercury uses) opens our server, reads the frequency and keys PTT.
    #[tokio::test(flavor = "multi_thread")]
    async fn hamlib_net_rigctl_can_use_it() {
        let Some(rigctl) = crate::rigsetup::hamlib_bin("rigctl") else {
            eprintln!("Hamlib not installed; skipping");
            return;
        };
        let d = crate::cat::icom::Icom::probe(Box::new(SimRadio::new(Kind::Icom(0x94))), 0).unwrap();
        let srv = serve(CatHandle::start(Box::new(d)), 0).await.unwrap();
        let addr = srv.addr.clone();
        let out = tokio::task::spawn_blocking(move || std::process::Command::new(rigctl).args(["-m", "2", "-r", &addr, "f", "F", "14100000", "f", "T", "1", "t", "T", "0", "m"]).output().unwrap())
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
}
