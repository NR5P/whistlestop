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

        /// Wait until neither modem has keyed the radio for `quiet_s`
        /// seconds (at most `max_s`).
        fn wait_air_quiet(&self, quiet_s: f64, max_s: f64) {
            let start = Instant::now();
            let mut quiet_since = Instant::now();
            while start.elapsed().as_secs_f64() < max_s {
                if self.ether.stats().iter().any(|s| s.mercury_ptt) {
                    quiet_since = Instant::now();
                } else if quiet_since.elapsed().as_secs_f64() >= quiet_s {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            println!("   air quiet after {:.0} s", start.elapsed().as_secs_f64());
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
            let mut c = if std::ptr::eq(h, &self.a) { cfg_for(A, 8300, 8100, 4632, 7300, &self.work) } else { cfg_for(B, 8310, 8110, 4642, 7310, &self.work) };
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
                format!(
                    "B measured {} after {:.1} s",
                    off.map(|o| format!("{o:+.1} Hz")).unwrap_or("nothing".into()),
                    got.map(|d| d.as_secs_f64()).unwrap_or(-1.0)
                ),
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
            let t = self.wait(240.0, |a, b| a.link.as_ref().is_some_and(|l| l.peer == B) && b.link.as_ref().is_some_and(|l| l.peer == A));
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
            let (ma, mb) =
                (self.st(&self.a).table.as_ref().map(|t| t.mode.name).unwrap_or("-"), self.st(&self.b).table.as_ref().map(|t| t.mode.name).unwrap_or("-"));
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
            let got =
                self.wait(180.0, |a, b| a.depot.arrivals.iter().any(|m| m.text.contains("see you on 40 m")) && b.depot.delivered.iter().any(|m| m.to == A));
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

        /// Scan speed: BRAVO scans the 20 m segment (from a different start
        /// each time); ALPHA calls CQ once. How long until BRAVO is locked on
        /// within Mercury's ±55 Hz, and does it ever pass ALPHA over?
        fn s_scan(&mut self) {
            self.disconnect_both();
            // Let the last link's teardown finish first: a modem still sending
            // DISCONNECT bursts mutes the scanning station's receiver (as a
            // real radio does on TX), and the first trial would measure that.
            self.wait_air_quiet(25.0, 240.0);
            let w = Config::default().whistle_s;
            let base = self.ether.channel.lock().unwrap().noise_dbfs;
            for (label, extra_db) in [("normal", 0.0), ("weak", 22.0), ("very weak", 28.0)] {
                self.ether.channel.lock().unwrap().noise_dbfs = base + extra_db;
                println!("   scan test ({label}): {}", self.snr_note());
                let mut times: Vec<f64> = vec![];
                let trials = 4;
                for trial in 0..trials {
                    self.a.send(Cmd::StopCq);
                    self.b.send(Cmd::StopSearch);
                    self.update(&self.a, |c| {
                        c.whistle_s = w;
                        c.cq_cycles = 1;
                    });
                    self.update(&self.b, |c| c.auto_answer = false);
                    self.b.send(Cmd::SetFreq(14_096_000.0 + 2300.0 * trial as f64));
                    std::thread::sleep(Duration::from_secs(2));
                    self.b.send(Cmd::StartSearch(SearchScope::CurrentBand));
                    std::thread::sleep(Duration::from_millis(800 + 1700 * trial as u64));
                    let mark = self.st(&self.b).log.len();
                    self.a.send(Cmd::CallCq { table: None });
                    let t = self.wait(w + 25.0, |_, b| {
                        b.log.iter().skip(mark).any(|l| l.1.starts_with("Locked on") || l.1.starts_with("Whistle ended; staying"))
                            && (b.rig.freq_hz - A_DIAL).abs() <= 55.0
                    });
                    if let Some(t) = t {
                        times.push(t.as_secs_f64());
                    }
                    println!(
                        "     trial {trial}: {}",
                        t.map(|d| format!("locked {:.1} s after the whistle began", d.as_secs_f64())).unwrap_or("missed".into())
                    );
                    self.a.send(Cmd::StopCq);
                    self.b.send(Cmd::StopSearch);
                    std::thread::sleep(Duration::from_secs(3));
                }
                times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let med = times.get(times.len() / 2).copied();
                self.record(
                    &format!("scan speed, {label} signal"),
                    times.len() == trials,
                    format!(
                        "found {}/{trials} within one {w:.0} s whistle; median {} (times {:?})",
                        times.len(),
                        med.map(|m| format!("{m:.1} s")).unwrap_or("—".into()),
                        times.iter().map(|t| (t * 10.0).round() / 10.0).collect::<Vec<_>>()
                    ),
                );
            }
            self.ether.channel.lock().unwrap().noise_dbfs = base;
        }

        /// QSY and the library over real Mercury, BRAVO away and serving:
        /// ALPHA browses and downloads, asks BRAVO to move (and to another
        /// band, refused), and both come home when the link ends.
        fn s_qsylib(&mut self) {
            self.disconnect_both();
            let lib = self.work.join("BRAVO-library");
            let _ = std::fs::remove_dir_all(&lib);
            std::fs::create_dir_all(lib.join("logs")).unwrap();
            let notes: Vec<u8> = (0..3000u32).map(|i| b"QSL via the Whistlestop library. "[i as usize % 33]).collect();
            std::fs::write(lib.join("notes.txt"), &notes).unwrap();
            std::fs::write(lib.join("logs/today.adi"), "<CALL:5>ALPHA<EOR>").unwrap();
            let libdir = lib.to_string_lossy().to_string();
            self.update(&self.b, |c| {
                c.accept_qsy = true;
                c.share_library = true;
                c.library_dir = libdir.clone();
            });
            self.b.send(Cmd::SetAway(true));
            std::thread::sleep(Duration::from_secs(1));
            if !self.direct_link("qsy/library") {
                return;
            }
            let home = A_DIAL;
            self.a.send(Cmd::LibList(String::new()));
            let t = self.wait(120.0, |a, _| {
                a.library.as_ref().is_some_and(|l| l.entries.iter().any(|e| e.name == "notes.txt") && l.entries.iter().any(|e| e.name == "logs" && e.dir))
            });
            self.record("library: browse over real Mercury", t.is_some(), format!("listing in {:.0} s", t.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
            self.a.send(Cmd::LibFetch("notes.txt".into()));
            let got = self.wait(300.0, |a, _| a.transfers.iter().any(|t| t.dir == Dir::In && t.name == "notes.txt" && matches!(t.state, XferState::Done(_))));
            let same = std::fs::read(self.work.join("ALPHA-downloads/notes.txt")).map(|d| d == notes).unwrap_or(false);
            self.record(
                "library: download byte-exact",
                got.is_some() && same,
                format!("{:.0} s, byte-exact: {same}", got.map(|d| d.as_secs_f64()).unwrap_or(-1.0)),
            );
            self.a.send(Cmd::Qsy(7_102_000.0));
            let refused = self.wait(120.0, |a, _| a.chat.iter().any(|c| c.text.contains("declined the QSY") && c.text.contains("only QSY within 20m")));
            self.record("qsy: other band refused by BRAVO's rules", refused.is_some(), format!("{:.0} s", refused.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
            let there = 14_106_000.0;
            self.a.send(Cmd::Qsy(there));
            let moved = self.wait(120.0, |a, b| a.rig.freq_hz == there && b.rig.freq_hz == there);
            self.a.send(Cmd::Chat("Hello from the new frequency".into()));
            let talk = self.wait(180.0, |_, b| {
                b.depot.arrivals.iter().any(|m| m.text.contains("new frequency")) || b.chat.iter().any(|c| c.text.contains("new frequency"))
            });
            self.record(
                "qsy: both move, link carries on",
                moved.is_some() && talk.is_some(),
                format!(
                    "moved in {:.0} s; chat across in {:.0} s",
                    moved.map(|d| d.as_secs_f64()).unwrap_or(-1.0),
                    talk.map(|d| d.as_secs_f64()).unwrap_or(-1.0)
                ),
            );
            self.a.send(Cmd::Disconnect);
            let back = self.wait(120.0, |a, b| a.link.is_none() && b.link.is_none() && a.rig.freq_hz == home && b.rig.freq_hz == home);
            self.record("qsy: both back home when the link ends", back.is_some(), format!("{:.0} s", back.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
            self.b.send(Cmd::SetAway(false));
            self.update(&self.b, |c| {
                c.accept_qsy = false;
                c.share_library = false;
            });
        }

        // ------------------------------------------- transfers, cancel, resume

        /// Incompressible bytes (what a photo would be), different per seed.
        fn noise_bytes(n: usize, seed: u64) -> Vec<u8> {
            let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
            (0..n)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    (s >> 24) as u8
                })
                .collect()
        }

        fn row(s: &State, fid: u64, dir: Dir) -> Option<&engine::Transfer> {
            s.transfers.iter().rev().find(|t| t.fid == fid && t.dir == dir)
        }

        fn rows(s: &State) -> Vec<(u64, Dir, String, XferState)> {
            s.transfers.iter().map(|t| (t.fid, t.dir, t.name.clone(), t.state.clone())).collect()
        }

        fn state_of(s: &State, fid: u64, dir: Dir) -> String {
            Self::row(s, fid, dir).map(|t| format!("{:?} ({}/{} B)", t.state, t.done, t.size)).unwrap_or("no row".into())
        }

        fn received(s: &State, fid: u64) -> Option<PathBuf> {
            match &Self::row(s, fid, Dir::In)?.state {
                XferState::Done(Some(p)) => Some(p.clone()),
                _ => None,
            }
        }

        fn sent_ok(s: &State, fid: u64) -> bool {
            Self::row(s, fid, Dir::Out).is_some_and(|t| t.state == XferState::Done(None))
        }

        fn cancelled(s: &State, fid: u64, dir: Dir) -> bool {
            Self::row(s, fid, dir).is_some_and(|t| matches!(&t.state, XferState::Failed(w) if w.starts_with("cancelled")))
        }

        /// Wait for an offer of `name` at the receiver, accept it, and return
        /// (fid, bytes the receiver already had).
        fn accept_named(&self, rx_is_b: bool, name: &str, secs: f64) -> Option<(u64, u64)> {
            let rx = if rx_is_b { &self.b } else { &self.a };
            self.wait(secs, |a, b| {
                let r = if rx_is_b { b } else { a };
                r.transfers.iter().any(|t| t.dir == Dir::In && t.name == name && t.state == XferState::WaitingForYou)
            })?;
            let (fid, have) = {
                let r = self.st(rx);
                let t = r.transfers.iter().rev().find(|t| t.dir == Dir::In && t.name == name && t.state == XferState::WaitingForYou)?;
                (t.fid, t.done)
            };
            rx.send(Cmd::AnswerOffer { fid, accept: true });
            Some((fid, have))
        }

        fn out_fid(&self, h: &Handle, name: &str) -> Option<u64> {
            self.st(h).transfers.iter().rev().find(|t| t.dir == Dir::Out && t.name == name).map(|t| t.fid)
        }

        /// A log line containing `pat` written after line `from`.
        fn log_line(&self, h: &Handle, from: usize, pat: &str) -> Option<String> {
            self.st(h).log.iter().skip(from).find(|l| l.1.contains(pat)).map(|l| l.1.clone())
        }

        /// A calls B (B on A's frequency).
        fn link_a_calls_b(&mut self, label: &str) -> bool {
            self.b.send(Cmd::SetFreq(A_DIAL));
            std::thread::sleep(Duration::from_secs(2));
            self.a.send(Cmd::Connect(B.into()));
            let t = self.wait(180.0, |a, b| a.link.is_some() && b.link.is_some());
            let bw = self.st(&self.a).link.as_ref().map(|l| l.bw).unwrap_or(0);
            self.record(&format!("{label}: connect"), t.is_some(), format!("linked in {:.0} s at {bw} Hz", t.map(|d| d.as_secs_f64()).unwrap_or(-1.0)));
            t.is_some()
        }

        /// Files both ways at once, then one cancelled mid-transfer: first by
        /// its receiver, then by its sender. The other must finish byte-exact,
        /// the cancelled one must read "cancelled…" at both ends, and no
        /// other transfer row may change.
        fn s_cancel(&mut self) {
            self.disconnect_both();
            self.wait_air_quiet(10.0, 120.0);
            if !self.direct_link("cancel") {
                return;
            }
            let dir = self.work.join("to-send");
            std::fs::create_dir_all(&dir).unwrap();
            for (round, by_receiver) in [(1u64, true), (2, false)] {
                let who = if by_receiver { "receiver (BRAVO)" } else { "sender (ALPHA)" };
                let name = format!("cancel {round}, by the {who}");
                let (na, nb) = (format!("cancel-a{round}.bin"), format!("cancel-b{round}.bin"));
                let (pa, pb) = (dir.join(&na), dir.join(&nb));
                std::fs::write(&pa, Self::noise_bytes(12_000, 10 * round)).unwrap();
                std::fs::write(&pb, Self::noise_bytes(12_000, 10 * round + 1)).unwrap();
                let (a0, b0) = (Self::rows(&self.st(&self.a)), Self::rows(&self.st(&self.b)));
                self.a.send(Cmd::SendFiles { paths: vec![pa.clone()], zip: false });
                self.b.send(Cmd::SendFiles { paths: vec![pb.clone()], zip: false });
                let ab = self.accept_named(true, &na, 120.0);
                let ba = self.accept_named(false, &nb, 120.0);
                let (Some((ab, _)), Some((ba, _))) = (ab, ba) else {
                    self.record(&name, false, format!("offers: A→B {:?}, B→A {:?}", ab, ba));
                    continue;
                };
                let (a_out_ab, b_out_ba) = (self.out_fid(&self.a, &na), self.out_fid(&self.b, &nb));
                println!("   round {round}: A→B fid {ab} (A's out row {a_out_ab:?}), B→A fid {ba} (B's out row {b_out_ba:?})");
                // Mid-transfer: B holds a couple of pieces of A's file.
                let mid = self.wait(400.0, |_, b| Self::row(b, ab, Dir::In).is_some_and(|t| t.done >= 2048 && t.done < t.size));
                let at_cancel = {
                    let (a, b) = (self.st(&self.a), self.st(&self.b));
                    format!("A→B at B {}; B→A at A {}", Self::state_of(&b, ab, Dir::In), Self::state_of(&a, ba, Dir::In))
                };
                if by_receiver {
                    self.b.send(Cmd::Cancel { fid: ab, dir: Dir::In });
                } else {
                    self.a.send(Cmd::Cancel { fid: ab, dir: Dir::Out });
                }
                let both_cancelled = self.wait(240.0, |a, b| Self::cancelled(a, ab, Dir::Out) && Self::cancelled(b, ab, Dir::In));
                let other_done = self.wait(600.0, |a, b| Self::sent_ok(b, ba) && Self::received(a, ba).is_some());
                // Give stray pieces time to show up.
                std::thread::sleep(Duration::from_secs(5));
                let (a, b) = (self.st(&self.a), self.st(&self.b));
                let exact = Self::received(&a, ba).and_then(|p| std::fs::read(p).ok()) == std::fs::read(&pb).ok();
                // Rows from before this round are untouched; this round
                // added one out and one in row at each station.
                let (a1, b1) = (Self::rows(&a), Self::rows(&b));
                let untouched = a1.len() == a0.len() + 2 && b1.len() == b0.len() + 2 && a1[..a0.len()] == a0[..] && b1[..b0.len()] == b0[..];
                let cancelled_file_absent = Self::received(&b, ab).is_none();
                let detail = format!(
                    "at cancel: {at_cancel}. After: A→B at A {} / at B {}; B→A at B {} / at A {}; B→A byte-exact {exact}; \
                     cancelled both ends {} ; other rows untouched {untouched} (A {}→{} rows, B {}→{}); mid-transfer reached {}",
                    Self::state_of(&a, ab, Dir::Out),
                    Self::state_of(&b, ab, Dir::In),
                    Self::state_of(&b, ba, Dir::Out),
                    Self::state_of(&a, ba, Dir::In),
                    both_cancelled.map(|d| format!("in {:.0} s", d.as_secs_f64())).unwrap_or("NO".into()),
                    a0.len(),
                    a1.len(),
                    b0.len(),
                    b1.len(),
                    mid.is_some()
                );
                drop((a, b));
                self.record(
                    &name,
                    mid.is_some() && both_cancelled.is_some() && other_done.is_some() && exact && untouched && cancelled_file_absent,
                    detail,
                );
            }
            self.disconnect_both();
        }

        /// A chat typed during a file transfer should wait for at most two
        /// file pieces (pump() keeps no more than 2 × CHUNK in Mercury's
        /// buffer), not for the rest of the file.
        fn s_chat_during(&mut self) {
            self.disconnect_both();
            self.wait_air_quiet(10.0, 120.0);
            if !self.direct_link("chat during a transfer") {
                return;
            }
            let p = self.work.join("to-send/chat-during.bin");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, Self::noise_bytes(16_000, 33)).unwrap();
            self.a.send(Cmd::SendFiles { paths: vec![p], zip: false });
            let Some((fid, _)) = self.accept_named(true, "chat-during.bin", 120.0) else {
                self.record("chat during a transfer", false, "offer never arrived".into());
                return;
            };
            self.wait(300.0, |_, b| Self::row(b, fid, Dir::In).is_some_and(|t| t.done >= 1024));
            let at_send = Self::row(&self.st(&self.b), fid, Dir::In).map(|t| t.done).unwrap_or(0);
            self.a.send(Cmd::Chat("typed during the transfer".into()));
            let got = self.wait(400.0, |_, b| b.chat.iter().any(|c| c.text == "typed during the transfer"));
            let at_got = Self::row(&self.st(&self.b), fid, Dir::In).map(|t| t.done).unwrap_or(0);
            let behind = at_got.saturating_sub(at_send);
            self.record(
                "chat during a transfer waits ≤ 2 file pieces",
                got.is_some() && behind <= 3 * 1024,
                format!(
                    "chat arrived after {:.0} s; B had {at_send} B of the file when it was typed and {at_got} B when it arrived ({behind} B of file went first)",
                    got.map(|d| d.as_secs_f64()).unwrap_or(-1.0)
                ),
            );
            self.wait(300.0, |a, b| Self::sent_ok(a, fid) && Self::received(b, fid).is_some());
            self.disconnect_both();
        }

        /// A file A→B, the link dropped mid-transfer (BRAVO hangs up), then a
        /// new link: ALPHA offers it again and it carries on from where it
        /// stopped, byte-exact.
        fn s_resume(&mut self) {
            self.disconnect_both();
            self.wait_air_quiet(10.0, 120.0);
            if !self.direct_link("resume") {
                return;
            }
            let p = self.work.join("to-send/resume.bin");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let data = Self::noise_bytes(12_000, 77);
            std::fs::write(&p, &data).unwrap();
            let (ma, mb) = (self.st(&self.a).log.len(), self.st(&self.b).log.len());
            self.a.send(Cmd::SendFiles { paths: vec![p.clone()], zip: false });
            let Some((fid, _)) = self.accept_named(true, "resume.bin", 120.0) else {
                self.record("resume: offer", false, "offer never arrived".into());
                return;
            };
            let mid = self.wait(300.0, |_, b| Self::row(b, fid, Dir::In).is_some_and(|t| t.done >= 2048));
            let had = Self::row(&self.st(&self.b), fid, Dir::In).map(|t| t.done).unwrap_or(0);
            self.b.send(Cmd::Disconnect);
            let down = self.wait(150.0, |a, b| a.link.is_none() && b.link.is_none());
            let (sa, sb) = (Self::state_of(&self.st(&self.a), fid, Dir::Out), Self::state_of(&self.st(&self.b), fid, Dir::In));
            let finished_anyway = Self::received(&self.st(&self.b), fid).is_some();
            let partial: u64 = std::fs::read_dir(self.work.join("BRAVO-downloads/.partial"))
                .map(|d| d.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).filter(|&l| l > 0 && l < data.len() as u64).max().unwrap_or(0))
                .unwrap_or(0);
            self.record(
                "resume: link dropped mid-transfer",
                mid.is_some() && down.is_some() && !finished_anyway && partial >= 1024,
                format!(
                    "B had {had} B when it hung up; both idle after {:.0} s; A {sa}; B {sb}; partial on B's disk {partial} B; finished before the link went: {finished_anyway}",
                    down.map(|d| d.as_secs_f64()).unwrap_or(-1.0)
                ),
            );
            if finished_anyway || down.is_none() {
                self.disconnect_both();
                return;
            }
            self.wait_air_quiet(10.0, 120.0);
            if !self.direct_link("resume: link again") {
                return;
            }
            let Some((fid2, have)) = self.accept_named(true, "resume.bin", 120.0) else {
                self.record("resume: offered again", false, "no new offer on the new link".into());
                self.disconnect_both();
                return;
            };
            let t = Instant::now();
            let done = self.wait(400.0, |a, b| Self::sent_ok(a, fid2) && Self::received(b, fid2).is_some());
            let exact = Self::received(&self.st(&self.b), fid2).and_then(|p| std::fs::read(p).ok()).is_some_and(|d| d == data);
            let a_says = self.log_line(&self.a, ma, "already has");
            let b_says = self.log_line(&self.b, mb, "will resume");
            let a_rows = self.st(&self.a).transfers.iter().filter(|t| t.dir == Dir::Out && t.name == "resume.bin").count();
            self.record(
                "resume: carries on after relinking, byte-exact",
                done.is_some() && exact && have > 0 && a_says.is_some() && fid2 == fid,
                format!(
                    "re-offered with fid {fid2} (was {fid}), B already had {have} B; rest took {:.0} s; byte-exact {exact}; \
                     ALPHA: {:?}; BRAVO: {:?}; ALPHA shows {a_rows} row(s) for it, now {}",
                    t.elapsed().as_secs_f64(),
                    a_says,
                    b_says,
                    Self::state_of(&self.st(&self.a), fid2, Dir::Out)
                ),
            );
            self.disconnect_both();
        }

        /// ALPHA requires encryption, BRAVO has it off. ALPHA must end the
        /// link with a clear reason, and nothing ALPHA queued (a chat and a
        /// file) may cross in the clear.
        fn s_e2e_require(&mut self) {
            self.disconnect_both();
            self.wait_air_quiet(10.0, 120.0);
            self.update(&self.a, |c| c.e2e = whistlestop::e2e::Policy::Require);
            self.update(&self.b, |c| c.e2e = whistlestop::e2e::Policy::Off);
            std::thread::sleep(Duration::from_secs(1));
            let (ma, mb) = (self.st(&self.a).log.len(), self.st(&self.b).log.len());
            let (ca, cb) = (self.st(&self.a).chat.len(), self.st(&self.b).chat.len());
            let p = self.work.join("to-send/require.bin");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, Self::noise_bytes(2000, 5)).unwrap();
            self.b.send(Cmd::SetFreq(A_DIAL));
            std::thread::sleep(Duration::from_secs(2));
            self.b.send(Cmd::Connect(A.into()));
            // Queue the chat and the file the moment ALPHA sees the link,
            // before BRAVO's HELLO can arrive.
            let linked = self.wait(180.0, |a, _| a.link.is_some());
            if linked.is_some() {
                self.a.send(Cmd::Chat("plaintext must not leak".into()));
                self.a.send(Cmd::SendFiles { paths: vec![p.clone()], zip: false });
            }
            let t = Instant::now();
            let ended = self.wait(240.0, |a, b| a.link.is_none() && b.link.is_none());
            // Anything still on its way would have arrived by now.
            std::thread::sleep(Duration::from_secs(20));
            let (a, b) = (self.st(&self.a), self.st(&self.b));
            let queued = a.chat.iter().skip(ca).any(|c| c.who == Who::Me && c.text.contains("must not leak"));
            let leaked_chat = b.chat.iter().any(|c| c.text.contains("must not leak"));
            let leaked_file = b.transfers.iter().any(|t| t.dir == Dir::In && t.name == "require.bin");
            let leaked_mail = b.depot.arrivals.iter().any(|m| m.text.contains("must not leak"));
            let reason = a.log.iter().skip(ma).find(|l| l.1.contains("Encryption")).map(|l| l.1.clone());
            let b_saw: Vec<String> = b.chat.iter().skip(cb).filter(|c| c.who != Who::Me).map(|c| c.text.clone()).collect();
            let b_log: Vec<String> = b.log.iter().skip(mb).filter(|l| l.1.contains("ncrypt")).map(|l| l.1.clone()).collect();
            let fid = a.transfers.iter().rev().find(|t| t.dir == Dir::Out && t.name == "require.bin").map(|t| t.fid);
            drop((a, b));
            self.record(
                "encryption Require vs Off: link refused with a reason",
                linked.is_some() && ended.is_some_and(|d| d.as_secs_f64() <= 150.0) && reason.is_some(),
                format!(
                    "linked {}; ended {:.0} s after linking; ALPHA says {:?}; BRAVO sees {:?} / log {:?}",
                    linked.is_some(),
                    ended.map(|d| d.as_secs_f64()).unwrap_or(-1.0),
                    reason,
                    b_saw,
                    b_log
                ),
            );
            self.record(
                "encryption Require vs Off: nothing sent in the clear",
                linked.is_some() && queued && !leaked_chat && !leaked_file && !leaked_mail,
                format!("chat queued at ALPHA {queued}; reached BRAVO: chat {leaked_chat}, file offer {leaked_file}, depot {leaked_mail} ({:.0} s watched)", t.elapsed().as_secs_f64()),
            );
            // Don't let the queued file go out on a later (plain) link.
            if let Some(fid) = fid {
                self.a.send(Cmd::Cancel { fid, dir: Dir::Out });
            }
            self.update(&self.a, |c| c.e2e = whistlestop::e2e::Policy::Off);
            self.disconnect_both();
        }

        /// BRAVO away with a short idle timeout receives a file that takes
        /// much longer than that (500 Hz). It must not hang up mid-transfer,
        /// and must hang up about `away_idle_s` after it ends (the signal
        /// reports every 30 s don't count as activity).
        fn s_away_long(&mut self, label: &str, idle: f64, extra_noise_db: f64, size: usize) {
            #[allow(non_snake_case)]
            let IDLE = idle;
            self.disconnect_both();
            self.wait_air_quiet(10.0, 120.0);
            let base = self.ether.channel.lock().unwrap().noise_dbfs;
            self.ether.channel.lock().unwrap().noise_dbfs = base + extra_noise_db;
            println!("   {label}: {}", self.snr_note());
            self.a.send(Cmd::SetBandwidth(Bandwidth::Hz500));
            self.b.send(Cmd::SetBandwidth(Bandwidth::Hz500));
            self.update(&self.b, |c| c.away_idle_s = IDLE);
            self.b.send(Cmd::SetAway(true));
            std::thread::sleep(Duration::from_secs(2));
            if self.link_a_calls_b(label) {
                let p = self.work.join("to-send/away-long.bin");
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                let data = Self::noise_bytes(size, 9);
                std::fs::write(&p, &data).unwrap();
                let mb = self.st(&self.b).log.len();
                self.a.send(Cmd::SendFiles { paths: vec![p.clone()], zip: false });
                if let Some((fid, _)) = self.accept_named(true, "away-long.bin", 180.0) {
                    let t = Instant::now();
                    let end = self.wait(900.0, |a, b| (Self::sent_ok(a, fid) && Self::received(b, fid).is_some()) || b.link.is_none());
                    let took = t.elapsed().as_secs_f64();
                    let (ok, sb, sa) = {
                        let (a, b) = (self.st(&self.a), self.st(&self.b));
                        let ok = Self::received(&b, fid).and_then(|p| std::fs::read(p).ok()).is_some_and(|d| d == data) && b.link.is_some();
                        (ok, Self::state_of(&b, fid, Dir::In), Self::state_of(&a, fid, Dir::Out))
                    };
                    let hung = self.log_line(&self.b, mb, "Hanging up");
                    self.record(
                        &format!("{label}: no hang-up mid-transfer"),
                        end.is_some() && ok && took > 2.0 * IDLE,
                        format!(
                            "transfer {:.0} s with away_idle_s {IDLE:.0}; byte-exact with link still up {ok}; B {sb}; A {sa}; B's hang-up: {:?}",
                            took, hung
                        ),
                    );
                    if ok {
                        let mb2 = self.st(&self.b).log.len();
                        let h = self.wait(IDLE + 120.0, |_, b| b.log.iter().skip(mb2).any(|l| l.1.contains("Hanging up")));
                        let gone = self.wait(120.0, |a, b| a.link.is_none() && b.link.is_none());
                        let line = self.log_line(&self.b, mb2, "Hanging up");
                        let hs = h.map(|d| d.as_secs_f64()).unwrap_or(-1.0);
                        // BRAVO counts from the last piece it got, a few
                        // seconds before the bench saw ALPHA's Done: judge
                        // by the idle time BRAVO reports.
                        let idle_said: f64 = line
                            .as_deref()
                            .and_then(|l| l.split("idle ").nth(1))
                            .and_then(|r| r.split(' ').next())
                            .and_then(|n| n.parse().ok())
                            .unwrap_or(-1.0);
                        self.record(
                            &format!("{label}: hangs up when idle afterwards"),
                            h.is_some() && idle_said >= IDLE && idle_said <= IDLE + 15.0 && hs <= IDLE + 20.0 && gone.is_some(),
                            format!("hang-up {hs:.0} s after the file arrived ({line:?}); link gone {:.0} s later", gone.map(|d| d.as_secs_f64()).unwrap_or(-1.0)),
                        );
                    }
                } else {
                    self.record(&format!("{label}: offer"), false, "offer never arrived".into());
                }
            }
            self.disconnect_both();
            self.ether.channel.lock().unwrap().noise_dbfs = base;
            self.b.send(Cmd::SetAway(false));
            self.update(&self.b, |_| {});
            self.a.send(Cmd::SetBandwidth(Bandwidth::Hz2300));
            self.b.send(Cmd::SetBandwidth(Bandwidth::Hz2300));
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
        if run("qsylib") {
            b.s_qsylib();
        }
        if run("scan") {
            b.s_scan();
        }
        if run("e2e") {
            b.s_e2e();
        }
        if run("depot") {
            b.s_depot();
        }
        // Slow ones, run only when named.
        let named = |n: &str| only.iter().any(|o| o == n);
        if named("cancel") {
            b.s_cancel();
        }
        if named("chat_during") {
            b.s_chat_during();
        }
        if named("resume") {
            b.s_resume();
        }
        if named("e2e_require") {
            b.s_e2e_require();
        }
        // 60 s is the shortest away timeout Settings offers.
        if named("away_long") {
            b.s_away_long("away, long transfer", 60.0, 0.0, 4000);
        }
        if named("away_weak") {
            b.s_away_long("away, long transfer, weak signal", 60.0, 27.0, 3000);
        }
        b.dump_logs();
        let rep = b.report();
        std::fs::write(b.work.join("REPORT.md"), &rep)?;
        println!("\n{rep}");
        Ok(())
    }
}
