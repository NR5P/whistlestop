//! Two-station test bench: two real Mercury modems and two Whistlestop
//! engines on one computer, joined by the ether (a simulated HF band).
//!
//!   cargo run --release --bin twotest -- <path/to/mercury> [workdir] [scenario...]
//!
//! Scenarios run in order; name some to run only those.

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    bench::main()
}

#[cfg(not(unix))]
fn main() {
    eprintln!("twotest needs Mercury's simulated-audio link (a Unix socket), which Mercury offers only on macOS and Linux.");
}

#[cfg(unix)]
mod bench {

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use whistlestop::bandplan::SearchScope;
use whistlestop::config::Config;
use whistlestop::engine::{self, Cmd, Dir, Handle, State, Who, XferState};
use whistlestop::ether::{Channel, Ether, StationCfg};
use whistlestop::modes::Bandwidth;

const A: &str = "ALPHA";
const B: &str = "BRAVO";
const A_DIAL: f64 = 14_103_150.0;

struct Bench {
    ether: Ether,
    a: Handle,
    b: Handle,
    mercs: Vec<Child>,
    work: PathBuf,
    results: Vec<(String, bool, String)>,
    started: Instant,
    /// Log lengths when the link came up, to count false whistles after.
    mark: (usize, usize),
}

impl Drop for Bench {
    fn drop(&mut self) {
        for c in self.mercs.iter_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.ether.stop();
    }
}

fn wait_port(port: u16, secs: u64) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn cfg_for(call: &str, port: u16, bport: u16, rig: u16, audio: u16, work: &Path) -> Config {
    let mut c = Config {
        callsign: call.into(),
        name: format!("{call} test station"),
        grid: if call == A { "EM12".into() } else { "FN31".into() },
        demo: false,
        mercury_host: "127.0.0.1".into(),
        mercury_port: port,
        mercury_bcast_port: bport,
        rigctld: format!("127.0.0.1:{rig}"),
        audio_in: format!("ether:127.0.0.1:{audio}"),
        audio_out: format!("ether:127.0.0.1:{audio}"),
        bandwidth: Bandwidth::Hz2300,
        whistle_s: 24.0,
        cq_cycles: 4,
        cq_listen_s: 20.0,
        auto_answer: call == B,
        download_dir: work.join(format!("{call}-downloads")),
        depot_file: work.join(format!("{call}-depot.json")).to_string_lossy().to_string(),
        ..Default::default()
    };
    c.sdr.enabled = false;
    c.sdr.wide_search = false;
    c
}

impl Bench {
    fn start(mercury: &Path, work: PathBuf) -> anyhow::Result<Bench> {
        std::fs::create_dir_all(&work)?;
        let work = std::fs::canonicalize(&work)?;
        let sa = work.join("a.sock");
        let sb = work.join("b.sock");
        let ether = Ether::start(
            vec![
                StationCfg { name: A.into(), sock_path: sa.clone(), rig_port: 4632, audio_port: 7300, dial_hz: A_DIAL, hear_own_tx: false },
                StationCfg { name: B.into(), sock_path: sb.clone(), rig_port: 4642, audio_port: 7310, dial_hz: 14_100_200.0, hear_own_tx: false },
            ],
            Channel::default(),
        )?;
        let mut mercs = vec![];
        for (sock, port, bport, log) in [(&sa, 8300u16, 8100u16, "mercury-A.log"), (&sb, 8310, 8110, "mercury-B.log")] {
            let out = std::fs::File::create(work.join(log))?;
            mercs.push(
                Command::new(mercury)
                    .args(["-x", "sock", "-p", &port.to_string(), "-b", &bport.to_string(), "-v"])
                    .env("MERCURY_AUDIO_SOCK", sock)
                    .current_dir(&work)
                    .stdout(Stdio::from(out.try_clone()?))
                    .stderr(Stdio::from(out))
                    .spawn()?,
            );
        }
        for p in [8300, 8310, 8100, 8110] {
            anyhow::ensure!(wait_port(p, 20), "Mercury did not open port {p} (see mercury-*.log)");
        }
        for c in [A, B] {
            let _ = std::fs::remove_file(work.join(format!("{c}-depot.json")));
        }
        let a = engine::spawn(|| {});
        let b = engine::spawn(|| {});
        a.send(Cmd::Start(cfg_for(A, 8300, 8100, 4632, 7300, &work)));
        b.send(Cmd::Start(cfg_for(B, 8310, 8110, 4642, 7310, &work)));
        Ok(Bench { ether, a, b, mercs, work, results: vec![], started: Instant::now(), mark: (0, 0) })
    }

    fn st<'a>(&self, h: &'a Handle) -> std::sync::MutexGuard<'a, State> {
        h.state.lock().unwrap()
    }

    /// Wait for a condition on both stations' state.
    fn wait(&self, secs: f64, f: impl Fn(&State, &State) -> bool) -> Option<Duration> {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs_f64(secs) {
            {
                let (a, b) = (self.a.state.lock().unwrap(), self.b.state.lock().unwrap());
                if f(&a, &b) {
                    return Some(t.elapsed());
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    fn record(&mut self, name: &str, ok: bool, detail: String) {
        println!("{} {name}: {detail}", if ok { "✅ PASS" } else { "❌ FAIL" });
        self.results.push((name.into(), ok, detail));
    }

    fn update(&self, h: &Handle, f: impl FnOnce(&mut Config)) {
        let mut c = if std::ptr::eq(h, &self.a) {
            cfg_for(A, 8300, 8100, 4632, 7300, &self.work)
        } else {
            cfg_for(B, 8310, 8110, 4642, 7310, &self.work)
        };
        f(&mut c);
        h.send(Cmd::Update(c));
    }

    fn disconnect_both(&self) {
        if self.st(&self.a).link.is_some() {
            self.a.send(Cmd::Disconnect);
        }
        self.wait(90.0, |a, b| a.link.is_none() && b.link.is_none());
        self.a.send(Cmd::StopCq);
        self.b.send(Cmd::StopSearch);
        std::thread::sleep(Duration::from_secs(2));
    }

    fn snr_note(&self) -> String {
        let s = self.ether.stats();
        let ch = self.ether.channel.lock().unwrap().clone();
        let noise = ch.noise_dbfs;
        let lvl = |r: f64| if r > 0.0 { 20.0 * r.log10() } else { -99.0 };
        format!(
            "TX level A {:.0} dBFS, B {:.0} dBFS, noise {noise:.0} dBFS → channel SNR ≈ A→B {:+.0} dB, B→A {:+.0} dB",
            lvl(s[0].tx_rms),
            lvl(s[1].tx_rms),
            lvl(s[0].tx_rms) - ch.path_loss_db - noise,
            lvl(s[1].tx_rms) - ch.path_loss_db - noise
        )
    }

    // ------------------------------------------------------------ scenarios

    fn s_ready(&mut self) {
        let ok = self.wait(30.0, |a, b| a.mercury_ok && b.mercury_ok && a.rig_ok && b.rig_ok && a.audio_ok && b.audio_ok);
        let stats = self.ether.stats();
        let detail = format!(
            "Mercury links {}/{}, ether sees Mercury {}/{} and apps {}/{}",
            self.st(&self.a).mercury_ok,
            self.st(&self.b).mercury_ok,
            stats[0].mercury_connected,
            stats[1].mercury_connected,
            stats[0].app_connected,
            stats[1].app_connected
        );
        self.record("ready", ok.is_some(), detail);
    }

    /// B parked 600 Hz below A hears A's whistle as +600 Hz.
    fn s_whistle_watch(&mut self) {
        self.b.send(Cmd::SetFreq(A_DIAL - 600.0));
        self.update(&self.a, |c| {
            c.whistle_s = 10.0;
            c.cq_cycles = 1;
        });
        std::thread::sleep(Duration::from_secs(4));
        self.a.send(Cmd::CallCq { table: None });
        let got = self.wait(30.0, |_, b| b.whistle.as_ref().is_some_and(|w| w.at.elapsed() < Duration::from_secs(5)));
        let off = self.st(&self.b).whistle.as_ref().map(|w| w.offset_hz);
        let ok = got.is_some() && off.is_some_and(|o| (o - 600.0).abs() < 5.0);
        let st = self.ether.stats();
        println!("   ether dials: A {:.0}  B {:.0}; A keyed {} tx {:.1}s", st[0].dial_hz, st[1].dial_hz, st[0].rig_ptt, st[0].tx_seconds);
        self.record(
            "whistle watch (B parked 600 Hz off)",
            ok,
            format!("B measured {} after {:.1} s", off.map(|o| format!("{o:+.1} Hz")).unwrap_or("nothing".into()), got.map(|d| d.as_secs_f64()).unwrap_or(-1.0)),
        );
        self.a.send(Cmd::StopCq);
        std::thread::sleep(Duration::from_secs(6));
    }

    /// The real thing: A calls CQ, B searches the 20 m segment, locks on,
    /// Mercury B decodes A's CQFRAME, B answers, Mercury links them.
    fn s_search_link(&mut self) {
        self.update(&self.a, |c| c.whistle_s = 26.0);
        self.update(&self.b, |c| c.auto_answer = true);
        self.b.send(Cmd::SetFreq(14_096_000.0));
        std::thread::sleep(Duration::from_secs(2));
        self.b.send(Cmd::StartSearch(SearchScope::CurrentBand));
        std::thread::sleep(Duration::from_secs(3));
        self.a.send(Cmd::CallCq { table: None });
        let t = self.wait(240.0, |a, b| {
            a.link.as_ref().is_some_and(|l| l.peer == B) && b.link.as_ref().is_some_and(|l| l.peer == A)
        });
        let (bf, heard) = {
            let b = self.st(&self.b);
            (b.rig.freq_hz, b.heard.iter().any(|h| h.call == A && h.what == "CQ"))
        };
        let lock_err = bf - A_DIAL;
        self.mark = (self.st(&self.a).log.len(), self.st(&self.b).log.len());
        self.record(
            "search → whistle → CQFRAME → link (real Mercury)",
            t.is_some() && lock_err.abs() <= 55.0 && heard,
            format!(
                "linked after {}; B tuned {:+.1} Hz from A; Mercury B decoded A's CQFRAME: {heard}",
                t.map(|d| format!("{:.0} s", d.as_secs_f64())).unwrap_or("— (no link)".into()),
                lock_err
            ),
        );
    }

    fn s_chat(&mut self) {
        if self.st(&self.a).link.is_none() {
            self.record("chat", false, "skipped: not linked".into());
            return;
        }
        let t0 = Instant::now();
        self.b.send(Cmd::Chat("Hello ALPHA, found you by your whistle!".into()));
        let ab = self.wait(120.0, |a, _| a.chat.iter().any(|c| c.who == Who::Them && c.text.contains("found you")));
        self.a.send(Cmd::Chat("Roger BRAVO, good copy. 73".into()));
        let ba = self.wait(120.0, |_, b| b.chat.iter().any(|c| c.who == Who::Them && c.text.contains("good copy")));
        let delivered = self.wait(60.0, |_, b| b.chat.iter().any(|c| c.who == Who::Me && c.delivered));
        self.record(
            "chat both ways + delivered receipt",
            ab.is_some() && ba.is_some() && delivered.is_some(),
            format!(
                "B→A {:.1} s, A→B {:.1} s, B saw '✓ delivered': {} (total {:.0} s)",
                ab.map(|d| d.as_secs_f64()).unwrap_or(-1.0),
                ba.map(|d| d.as_secs_f64()).unwrap_or(-1.0),
                delivered.is_some(),
                t0.elapsed().as_secs_f64()
            ),
        );
    }

    fn make_files(&self) -> (PathBuf, PathBuf) {
        let dir = self.work.join("to-send/Field Day");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..6 {
            let line = format!("QSO {i}: <call:5>K{i}ABC <band:3>20m <mode:7>MERCURY <rst:3>599 <eor>\n");
            std::fs::write(dir.join(format!("log-{i}.adi")), line.repeat(250)).unwrap();
        }
        // Incompressible data: what a photo would be.
        let mut seed = 0x1234_5678u64;
        let bin: Vec<u8> = (0..16_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                (seed >> 24) as u8
            })
            .collect();
        let binp = self.work.join("to-send/photo.bin");
        std::fs::write(&binp, bin).unwrap();
        (dir, binp)
    }

    fn transfer(&mut self, name: &str, from_a: bool, paths: Vec<PathBuf>, zip: bool, secs: f64) {
        let (tx, rx) = if from_a { (&self.a, &self.b) } else { (&self.b, &self.a) };
        if self.st(tx).link.is_none() {
            self.record(name, false, "skipped: not linked".into());
            return;
        }
        let before = self.st(rx).transfers.len();
        tx.send(Cmd::SendFiles { paths: paths.clone(), zip });
        // The receiver accepts when the offer arrives.
        let offered = self.wait(90.0, |a, b| {
            let r = if from_a { b } else { a };
            r.transfers.len() > before && r.transfers.last().is_some_and(|t| t.state == XferState::WaitingForYou)
        });
        let Some(_) = offered else {
            self.record(name, false, "offer never arrived".into());
            return;
        };
        let (fid, size, eta0) = {
            let r = self.st(rx);
            let t = r.transfers.last().unwrap();
            (t.fid, t.size, t.eta_s)
        };
        rx.send(Cmd::AnswerOffer { fid, accept: true });
        let t = Instant::now();
        let done = self.wait(secs, |a, b| {
            let (s, r) = if from_a { (a, b) } else { (b, a) };
            let sent_ok = s.transfers.iter().rev().find(|x| x.dir == Dir::Out).is_some_and(|x| x.state == XferState::Done(None));
            let got = r.transfers.iter().rev().find(|x| x.fid == fid && x.dir == Dir::In).is_some_and(|x| matches!(x.state, XferState::Done(Some(_))));
            sent_ok && got
        });
        let secs_taken = t.elapsed().as_secs_f64();
        let path = self.st(rx).transfers.iter().rev().find(|x| x.fid == fid).and_then(|x| match &x.state {
            XferState::Done(Some(p)) => Some(p.clone()),
            _ => None,
        });
        // Byte-exact check for an unzipped file.
        let exact = match (&path, zip, paths.first()) {
            (Some(p), false, Some(src)) => std::fs::read(p).ok() == std::fs::read(src).ok(),
            (Some(_), true, _) => true, // zip verified by SHA-256 end to end (DONE ok)
            _ => false,
        };
        let mode = self.st(tx).link.as_ref().and_then(|l| l.mode).map(|m| m.name).unwrap_or("?");
        self.record(
            name,
            done.is_some() && exact,
            format!(
                "{} bytes in {:.0} s = {:.0} bps goodput (predicted {:.0} s); SHA-256 good both ends: {}; byte-exact: {exact}; mode now {mode}",
                size,
                secs_taken,
                size as f64 * 8.0 / secs_taken,
                eta0,
                done.is_some()
            ),
        );
    }

    fn s_files(&mut self) {
        let (dir, bin) = self.make_files();
        self.transfer("zipped folder A→B", true, vec![dir], true, 420.0);
        self.transfer("binary file B→A (no zip)", false, vec![bin], false, 420.0);
    }

    fn s_false_whistles(&mut self) {
        let count = |h: &Handle, from: usize| h.state.lock().unwrap().log.iter().skip(from).filter(|l| l.1.contains("Whistle heard")).count();
        let (a, b) = (count(&self.a, self.mark.0), count(&self.b, self.mark.1));
        self.record("no false whistles during link traffic", a + b == 0, format!("whistle reports while linked: ALPHA {a}, BRAVO {b}"));
    }

    fn s_disconnect(&mut self) {
        self.a.send(Cmd::Disconnect);
        let t = self.wait(120.0, |a, b| a.link.is_none() && b.link.is_none());
        self.record("disconnect (73)", t.is_some(), format!("both sides idle after {:.1} s", t.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
    }

    fn s_roundtable(&mut self) {
        self.disconnect_both();
        self.b.send(Cmd::SetFreq(A_DIAL));
        self.update(&self.a, |c| {
            c.whistle_s = 6.0;
            c.cq_cycles = 1;
            c.cq_listen_s = 5.0;
        });
        std::thread::sleep(Duration::from_secs(2));
        self.a.send(Cmd::CallCq { table: Some("Bench Table".into()) });
        // B hears the CALL packet and joins.
        let heard = self.wait(60.0, |_, b| b.heard.iter().any(|h| h.table.as_deref() == Some("Bench Table")));
        self.b.send(Cmd::JoinTable("Bench Table".into()));
        let joined = self.wait(90.0, |a, _| a.table.as_ref().is_some_and(|t| t.members.contains_key(B)));
        std::thread::sleep(Duration::from_secs(8));
        let long = "This is a longer Roundtable message from the host, to check that a message split across several broadcast frames comes back together in the right order at every station. ".repeat(2);
        self.a.send(Cmd::TableSay(long.trim().into()));
        let got_long = self.wait(180.0, |_, b| b.table.as_ref().is_some_and(|t| t.lines.iter().any(|l| l.1 == A && l.2 == long.trim())));
        self.b.send(Cmd::TableSay("BRAVO checking in, solid copy.".into()));
        let got_short = self.wait(120.0, |a, _| a.table.as_ref().is_some_and(|t| t.lines.iter().any(|l| l.1 == B && l.2.contains("checking in"))));
        let (ma, mb) = (
            self.st(&self.a).table.as_ref().map(|t| t.mode.name).unwrap_or("-"),
            self.st(&self.b).table.as_ref().map(|t| t.mode.name).unwrap_or("-"),
        );
        self.record(
            "Roundtable: CALL heard, join, multi-frame message, reply",
            heard.is_some() && joined.is_some() && got_long.is_some() && got_short.is_some() && ma == mb,
            format!(
                "CALL heard {}, join seen by host {}, {}-char message {}, reply {}; table speed A {ma} / B {mb}",
                heard.is_some(),
                joined.is_some(),
                long.trim().len(),
                got_long.map(|d| format!("arrived in {:.0} s", d.as_secs_f64())).unwrap_or("LOST".into()),
                got_short.map(|d| format!("in {:.0} s", d.as_secs_f64())).unwrap_or("LOST".into()),
            ),
        );
        self.a.send(Cmd::LeaveTable);
        self.b.send(Cmd::LeaveTable);
        std::thread::sleep(Duration::from_secs(3));
    }

    fn direct_link(&mut self, label: &str) -> bool {
        self.b.send(Cmd::SetFreq(A_DIAL));
        std::thread::sleep(Duration::from_secs(2));
        self.b.send(Cmd::Connect(A.into()));
        let t = self.wait(180.0, |a, b| a.link.is_some() && b.link.is_some());
        let bw = self.st(&self.b).link.as_ref().map(|l| l.bw).unwrap_or(0);
        self.record(&format!("{label}: connect"), t.is_some(), format!("linked in {:.0} s at {bw} Hz", t.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        t.is_some()
    }

    fn s_narrow(&mut self) {
        self.disconnect_both();
        self.a.send(Cmd::SetBandwidth(Bandwidth::Hz500));
        self.b.send(Cmd::SetBandwidth(Bandwidth::Hz500));
        std::thread::sleep(Duration::from_secs(2));
        if self.direct_link("narrow 500 Hz") {
            let bw = self.st(&self.a).link.as_ref().map(|l| l.bw).unwrap_or(0);
            self.record("narrow 500 Hz: negotiated", bw == 500, format!("CONNECTED reports {bw}"));
            let p = self.work.join("to-send/note.txt");
            std::fs::write(&p, "Narrowband test note. ".repeat(120)).unwrap();
            self.transfer("narrow 500 Hz: file B→A", false, vec![p], false, 600.0);
        }
        self.disconnect_both();
        self.a.send(Cmd::SetBandwidth(Bandwidth::Hz2300));
        self.b.send(Cmd::SetBandwidth(Bandwidth::Hz2300));
    }

    fn s_weak(&mut self) {
        self.disconnect_both();
        let before = self.snr_note();
        self.ether.channel.lock().unwrap().noise_dbfs += 25.0;
        self.ether.channel.lock().unwrap().fading = 0.5;
        println!("   weak-signal channel: {}", self.snr_note());
        if self.direct_link("weak signal + fading") {
            self.b.send(Cmd::Chat("weak signal test".into()));
            let got = self.wait(240.0, |a, _| a.chat.iter().any(|c| c.text == "weak signal test"));
            let mode = self.st(&self.b).link.as_ref().and_then(|l| l.mode).map(|m| m.name).unwrap_or("?");
            let snr = self.st(&self.b).snr_db;
            self.record(
                "weak signal + fading: chat",
                got.is_some(),
                format!("delivered in {:.0} s; Mercury SNR {:?} dB, mode {mode}", got.map(|d| d.as_secs_f64()).unwrap_or(-1.0), snr),
            );
        }
        self.disconnect_both();
        self.ether.channel.lock().unwrap().noise_dbfs -= 25.0;
        self.ether.channel.lock().unwrap().fading = 0.0;
        let _ = before;
    }

    /// Encryption over real Mercury ARQ: both stations on Prefer.
    fn s_e2e(&mut self) {
        self.disconnect_both();
        for h in [&self.a, &self.b] {
            self.update(h, |c| c.e2e = whistlestop::e2e::Policy::Prefer);
        }
        std::thread::sleep(Duration::from_secs(1));
        if !self.direct_link("encrypted link") {
            return;
        }
        let sec = self.wait(120.0, |a, b| {
            let enc = |s: &State| s.link.as_ref().is_some_and(|l| matches!(l.e2e, engine::E2eView::Encrypted { .. }));
            enc(a) && enc(b)
        });
        let codes = {
            let code = |h: &Handle| match h.state.lock().unwrap().link.as_ref().map(|l| l.e2e.clone()) {
                Some(engine::E2eView::Encrypted { sas }) => sas,
                _ => String::new(),
            };
            (code(&self.a), code(&self.b))
        };
        self.record(
            "encryption: key exchange over real Mercury",
            sec.is_some() && !codes.0.is_empty() && codes.0 == codes.1,
            format!("encrypted after {:.0} s; check codes ALPHA '{}' / BRAVO '{}'", sec.map(|d| d.as_secs_f64()).unwrap_or(-1.0), codes.0, codes.1),
        );
        self.b.send(Cmd::Chat("secret test message".into()));
        let got = self.wait(120.0, |a, b| a.chat.iter().any(|c| c.text == "secret test message") && b.chat.iter().any(|c| c.who == Who::Me && c.delivered));
        self.record("encryption: chat delivered", got.is_some(), format!("{:.1} s", got.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        let p = self.work.join("to-send/sealed.bin");
        std::fs::write(&p, (0..6000u32).map(|i| (i * 31 % 251) as u8).collect::<Vec<u8>>()).unwrap();
        self.transfer("encryption: file A→B byte-exact", true, vec![p], false, 300.0);
        self.disconnect_both();
        for h in [&self.a, &self.b] {
            self.update(h, |c| c.e2e = whistlestop::e2e::Policy::Off);
        }
    }

    /// The Depot over real Mercury: mail handed over when linked, the away
    /// greeting and chat saved while away, then the Station ID beacon
    /// telling an away station there's mail, which it collects by itself.
    fn s_depot(&mut self) {
        self.disconnect_both();
        self.b.send(Cmd::DepotPost { to: A.into(), via: None, text: "Depot test: see you on 40 m".into() });
        self.a.send(Cmd::SetAway(true));
        std::thread::sleep(Duration::from_secs(1));
        if !self.direct_link("depot") {
            return;
        }
        let got = self.wait(180.0, |a, b| a.depot.arrivals.iter().any(|m| m.text.contains("see you on 40 m")) && b.depot.delivered.iter().any(|m| m.to == A));
        self.record("depot: mail handed over and confirmed", got.is_some(), format!("{:.0} s after linking", got.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        let greet = self.wait(120.0, |_, b| b.chat.iter().any(|c| c.who == Who::Them && c.text.contains("Away from the radio")));
        self.record("depot: away greeting reaches the caller", greet.is_some(), format!("{:.0} s", greet.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        self.b.send(Cmd::Chat("Left you a note while you were out".into()));
        let saved = self.wait(120.0, |a, _| a.depot.arrivals.iter().any(|m| m.from == B && m.text.contains("Left you a note")));
        self.record("depot: chat while away saved to Arrivals", saved.is_some(), format!("{:.0} s", saved.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        self.disconnect_both();

        // Station ID: B holds mail for A and says so; A (away, automatic
        // collection on) connects and collects it with no one at the keys.
        self.update(&self.a, |c| c.auto_collect = true);
        self.b.send(Cmd::DepotPost { to: A.into(), via: None, text: "Second note, collected automatically".into() });
        std::thread::sleep(Duration::from_secs(2));
        self.b.send(Cmd::BeaconNow);
        let heard = self.wait(90.0, |a, _| a.mail_alerts.iter().any(|m| m.call == B && !m.ours));
        self.record("depot: Station ID heard, mail waiting shown", heard.is_some(), format!("{:.0} s", heard.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        let col = self.wait(300.0, |a, b| a.depot.arrivals.iter().any(|m| m.text.contains("collected automatically")) && b.depot.departures.is_empty());
        self.record("depot: away station collects its mail by itself", col.is_some(), format!("{:.0} s", col.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        let hung = self.wait(120.0, |a, b| a.link.is_none() && b.link.is_none());
        self.record("depot: automatic link hangs up when done", hung.is_some(), format!("{:.0} s", hung.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
        self.disconnect_both();
        self.a.send(Cmd::SetAway(false));
        self.update(&self.a, |c| c.auto_collect = false);
    }

    fn report(&self) -> String {
        let mut s = String::from("# Whistlestop two-station bench (real Mercury 1.9.23 ×2)\n\n");
        s += &format!("Run time {:.0} s. {}\n\n| Result | Test | Details |\n|---|---|---|\n", self.started.elapsed().as_secs_f64(), self.snr_note());
        for (n, ok, d) in &self.results {
            s += &format!("| {} | {n} | {d} |\n", if *ok { "PASS" } else { "**FAIL**" });
        }
        let passed = self.results.iter().filter(|r| r.1).count();
        s += &format!("\n{passed}/{} passed.\n", self.results.len());
        s
    }

    fn dump_logs(&self) {
        for (h, n) in [(&self.a, A), (&self.b, B)] {
            let g = h.state.lock().unwrap();
            let lines: Vec<String> = g.log.iter().map(|(at, l)| format!("{} {l}", at.hhmm())).collect();
            let _ = std::fs::write(self.work.join(format!("whistlestop-{n}.log")), lines.join("\n"));
        }
    }
}

pub fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mercury = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "mercury".into()));
    let work = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "bench-run".into()));
    let only: Vec<String> = args.iter().skip(3).cloned().collect();
    let run = |n: &str| only.is_empty() || only.iter().any(|o| o == n);
    let mut b = Bench::start(&mercury, work)?;
    println!("Bench up: 2× Mercury on the ether. Work dir {}", b.work.display());
    b.s_ready();
    if run("watch") {
        b.s_whistle_watch();
    }
    if run("link") {
        b.s_search_link();
        println!("   {}", b.snr_note());
    }
    if run("chat") {
        b.s_chat();
    }
    if run("files") {
        b.s_files();
    }
    if run("link") || run("chat") || run("files") {
        b.s_false_whistles();
        b.s_disconnect();
    }
    if run("table") {
        b.s_roundtable();
    }
    if run("narrow") {
        b.s_narrow();
    }
    if run("weak") {
        b.s_weak();
    }
    if run("e2e") {
        b.s_e2e();
    }
    if run("depot") {
        b.s_depot();
    }
    b.dump_logs();
    let rep = b.report();
    std::fs::write(b.work.join("REPORT.md"), &rep)?;
    println!("\n{rep}");
    Ok(())
}

}
