//! Two-station test bench: two real Mercury modems and two Whistlestop
//! engines on one computer, joined by the ether (a simulated HF band).
//! Naming a three-station scenario (`table3 relay3 cq_pileup busy3`) adds a
//! third station, CHARLIE, with its own Mercury.
//!
//!   cargo run --release --bin twotest -- <path/to/mercury> [workdir] [--a=KIND] [--b=KIND] [scenario...]
//!
//! Scenarios run in order; name some to run only those. KIND picks what runs
//! each station: `engine` (default, in this process), `desktop` (the desktop
//! app), `ios` (the iPhone app in the booted Simulator) or `android` (the
//! Android app in the emulator); the apps run their own real Mercury. See docs/BENCH.md.

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
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use whistlestop::bandplan::SearchScope;
    use whistlestop::config::Config;
    use whistlestop::engine::{self, Cmd, Dir, Handle, State, Who, XferState};
    use whistlestop::ether::{Channel, Ether, StationCfg};
    use whistlestop::modes::{self, Bandwidth};

    const A: &str = "ALPHA";
    const B: &str = "BRAVO";
    const A_DIAL: f64 = 14_103_150.0;
    /// The third station, started only when a three-station scenario is
    /// named. It waits 5 kHz below ALPHA, out of earshot, until one tunes it.
    const C: &str = "CHARLIE";
    const C_DIAL: f64 = 14_098_000.0;
    /// CHARLIE's Mercury control and broadcast ports, rigctld, app audio.
    /// Scenarios that need CHARLIE.
    const THREE: &[&str] = &["table3", "relay3", "cq_pileup", "busy3"];
    /// table3: how much weaker CHARLIE hears ALPHA (and ALPHA CHARLIE) than
    /// BRAVO does, dB.
    const TABLE3_WEAK_DB: f64 = 25.0;

    struct Bench {
        ether: Ether,
        a: Handle,
        b: Handle,
        /// CHARLIE, when a three-station scenario runs.
        c: Option<Handle>,
        mercs: Vec<Child>,
        stations: Vec<Station>,
        work: PathBuf,
        results: Vec<(String, bool, String)>,
        started: Instant,
        /// Log lengths when the link came up, to count false whistles after.
        mark: (usize, usize),
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            for s in self.stations.iter_mut() {
                s.stop();
            }
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
            grid: if call == A {
                "EM12".into()
            } else if call == C {
                "EN61".into()
            } else {
                "FN31".into()
            },
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
        fn start(mercury: &Path, work: PathBuf, kinds: [Kind; 2], three: bool) -> anyhow::Result<Bench> {
            std::fs::create_dir_all(&work)?;
            let work = std::fs::canonicalize(&work)?;
            // Files the scenarios send; any of them may run without `files`.
            std::fs::create_dir_all(work.join("to-send"))?;
            let sa = work.join("a.sock");
            let sb = work.join("b.sock");
            let sc = work.join("c.sock");
            let mut ecfg = vec![
                StationCfg { name: A.into(), sock_path: sa.clone(), rig_port: 4632 + off(), audio_port: 7300 + off(), dial_hz: A_DIAL, hear_own_tx: false },
                StationCfg { name: B.into(), sock_path: sb.clone(), rig_port: 4642 + off(), audio_port: 7310 + off(), dial_hz: 14_100_200.0, hear_own_tx: false },
            ];
            if three {
                ecfg.push(StationCfg { name: C.into(), sock_path: sc.clone(), rig_port: 4652 + off(), audio_port: 7320 + off(), dial_hz: C_DIAL, hear_own_tx: false });
            }
            let ether = Ether::start(ecfg, Channel::default())?;
            let mut mercs = vec![];
            let (st_a, a) = Station::start(kinds[0], 0, mercury, &work, &sa, &mut mercs)?;
            let (st_b, b) = Station::start(kinds[1], 1, mercury, &work, &sb, &mut mercs)?;
            let mut stations = vec![st_a, st_b];
            let c = if three {
                let (st_c, c) = Station::start(Kind::Engine, 2, mercury, &work, &sc, &mut mercs)?;
                stations.push(st_c);
                Some(c)
            } else {
                None
            };
            Ok(Bench { ether, a, b, c, mercs, stations, work, results: vec![], started: Instant::now(), mark: (0, 0) })
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
            let i = if std::ptr::eq(h, &self.a) {
                0
            } else if self.c.as_ref().is_some_and(|c| std::ptr::eq(h, c)) {
                2
            } else {
                1
            };
            let mut c = self.stations[i].cfg.clone();
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
            // An app's own Mercury starts with the app; give it time.
            let mut ok = self.wait(120.0, |a, b| a.mercury_ok && b.mercury_ok && a.rig_ok && b.rig_ok && a.audio_ok && b.audio_ok);
            if ok.is_some() && self.c.is_some() {
                ok = self.wait3(30.0, |_, _, c| c.mercury_ok && c.rig_ok && c.audio_ok);
            }
            let stats = self.ether.stats();
            let mut detail = format!(
                "ready after {:.0} s; Mercury links {}/{}, ether sees Mercury {}/{} and apps {}/{}",
                ok.map(|d| d.as_secs_f64()).unwrap_or(-1.0),
                self.st(&self.a).mercury_ok,
                self.st(&self.b).mercury_ok,
                stats[0].mercury_connected,
                stats[1].mercury_connected,
                stats[0].app_connected,
                stats[1].app_connected
            );
            if let Some(c) = &self.c {
                detail += &format!(
                    "; CHARLIE: Mercury link {}, ether sees Mercury {} and app {}",
                    self.st(c).mercury_ok,
                    stats[2].mercury_connected,
                    stats[2].app_connected
                );
            }
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
                (Some(p), false, Some(src)) => self.stations[if from_a { 1 } else { 0 }].read(p) == std::fs::read(src).ok(),
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
            // Not about listen-before-transmit: Mercury's busy detector can
            // still be latched from the last scenario's noise step, and the
            // CQ would wait out its 60 s.
            self.update(&self.a, |c| {
                c.whistle_s = 6.0;
                c.cq_cycles = 1;
                c.cq_listen_s = 5.0;
                c.busy_detect = false;
            });
            self.update(&self.b, |c| c.busy_detect = false);
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
            // The host closes the table: the member, knowing who hosts it
            // from the CALL it joined on, leaves by itself.
            self.wait_air_quiet(5.0, 60.0);
            self.a.send(Cmd::LeaveTable);
            let left = self.wait(90.0, |_, b| b.table.is_none());
            let said = self.st(&self.b).chat.iter().any(|c| c.text.contains("closed the Roundtable"));
            self.record(
                "Roundtable: the host leaving closes it for the member",
                left.is_some() && said,
                format!("BRAVO left {}; said why {said}", Self::took(left)),
            );
            if left.is_none() {
                self.b.send(Cmd::LeaveTable);
            }
            self.update(&self.a, |_| {});
            self.update(&self.b, |_| {});
            std::thread::sleep(Duration::from_secs(3));
        }

        /// A table slowed right down to DATAC15 (27-byte frames), with a
        /// name too long for the member's report to fit with it: the reports
        /// still reach the host (without the name), a message queued for the
        /// old speed is split again and arrives, and a long one goes out as
        /// several messages, all of which arrive.
        fn s_table_slow(&mut self) {
            self.disconnect_both();
            self.b.send(Cmd::SetFreq(A_DIAL));
            let name = "Sunday Morning Net";
            let base = |c: &mut Config| {
                c.whistle_s = 6.0;
                c.cq_cycles = 1;
                c.cq_listen_s = 5.0;
                c.busy_detect = false;
            };
            self.update(&self.a, base);
            self.update(&self.b, |c| c.busy_detect = false);
            // The bench channel is clean (reports of +20 dB and more), and the
            // margin goes to 20 dB at most: some noise brings the report
            // down to where a margin sets DATAC15.
            let mut noise = 12.0;
            self.ether.channel.lock().unwrap().noise_dbfs += noise;
            println!("   slow table channel: {}", self.snr_note());
            std::thread::sleep(Duration::from_secs(2));
            self.a.send(Cmd::CallCq { table: Some(name.into()) });
            self.wait(60.0, |_, b| b.heard.iter().any(|h| h.table.as_deref() == Some(name)));
            self.b.send(Cmd::JoinTable(name.into()));
            let report_of_b = |a: &State| a.table.as_ref().and_then(|t| t.members.get(B)).and_then(|m| m.snr_of_host);
            let joined = self.wait(90.0, |a, _| report_of_b(a).is_some());
            let mut snr = report_of_b(&self.st(&self.a)).unwrap_or(0) as f64;
            // Still too strong for a margin of 20 dB at most (the first
            // report can be the CQFRAME's, and channels differ from set to
            // set): more noise, and BRAVO's next report (every 2 min).
            for _ in 0..2 {
                if joined.is_none() || snr + 5.5 <= 18.0 {
                    break;
                }
                let more = snr + 5.5 - 14.0;
                noise += more;
                self.ether.channel.lock().unwrap().noise_dbfs += more;
                println!("   slow table: report {snr:+} dB, {more:.0} dB more noise");
                let before = self.st(&self.a).table.as_ref().and_then(|t| t.members.get(B)).and_then(|m| m.last);
                self.wait(150.0, |a, _| {
                    a.table.as_ref().and_then(|t| t.members.get(B)).and_then(|m| m.last) != before
                });
                snr = report_of_b(&self.st(&self.a)).unwrap_or(0) as f64;
            }
            // DATAC15 is the fastest mode the margin allows when the report
            // less the margin is in [-7, -4) dB: aim for the middle.
            let margin = snr + 5.5;
            let cleanup = |s: &mut Self| {
                s.wait_air_quiet(5.0, 120.0);
                s.a.send(Cmd::LeaveTable);
                if s.wait(90.0, |_, b| b.table.is_none()).is_none() {
                    s.b.send(Cmd::LeaveTable);
                }
                s.update(&s.a, |_| {});
                s.update(&s.b, |_| {});
                s.ether.channel.lock().unwrap().noise_dbfs -= noise;
                std::thread::sleep(Duration::from_secs(3));
            };
            if joined.is_none() || !(0.0..=20.0).contains(&margin) {
                self.record(
                    "slow table: set up at DATAC15",
                    false,
                    format!("BRAVO joined {}; its report {snr:+} dB → margin {margin:.1} dB (0-20 allowed)", Self::took(joined)),
                );
                cleanup(self);
                return;
            }
            self.wait_air_quiet(5.0, 60.0);
            self.update(&self.a, |c| {
                base(c);
                c.ramp_margin_db = margin;
            });
            // From whatever it runs at (DATAC1 on the bench).
            let announced = self.wait(90.0, |a, _| {
                a.log.iter().any(|l| l.1.contains("announcing speed change") && l.1.contains("→ DATAC15"))
            });
            // Queued while the change is announced: cut for the old frames.
            let queued = "Queued at the old speed just as the table slowed down: split again for DATAC15, it still arrives whole.";
            self.a.send(Cmd::TableSay(queued.into()));
            let slow = self.wait(120.0, |a, b| Self::table_mode(a) == "DATAC15" && Self::table_mode(b) == "DATAC15");
            let switched = Instant::now();
            let got = self.wait(420.0, |_, b| Self::has_line(b, A, queued));
            let resplit = self.st(&self.a).log.iter().any(|l| l.1.contains("split again into"));
            self.record(
                "slow table: a message queued for the old speed is split again and arrives",
                announced.is_some() && slow.is_some() && got.is_some() && resplit,
                format!(
                    "BRAVO reports {snr:+} dB, host margin {margin:.1} dB; change announced {}; both at DATAC15 {}; message {}; split again {resplit}",
                    Self::took(announced),
                    Self::took(slow),
                    Self::took(got)
                ),
            );
            // BRAVO's report with the name is 31 bytes, 27 fit: it goes
            // without the name. (BRAVO says nothing meanwhile, so the host
            // hearing from it means a report.)
            let report = self.wait(200.0, |a, _| {
                a.table.as_ref().and_then(|t| t.members.get(B)).and_then(|m| m.last).is_some_and(|l| l > switched)
            });
            let dropped = self.st(&self.b).log.iter().any(|l| l.1.contains("doesn't fit"));
            self.record(
                "slow table: the member's reports reach the host at DATAC15",
                report.is_some() && !dropped,
                format!(
                    "a report from BRAVO {}; host at {}; BRAVO dropped a frame as too big: {dropped}",
                    Self::took(report),
                    Self::table_mode(&self.st(&self.a))
                ),
            );
            // A long message: more parts than a resend request can name at
            // this speed, so it goes out as several.
            let long = "BRAVO at the slow speed: seventy two degrees and sunny here, the wind has dropped, and the new dipole is up at forty feet and working well. ".repeat(4);
            let long = long.trim().to_string();
            self.b.send(Cmd::TableSay(long.clone()));
            let joined_from_b = |s: &State| -> String {
                s.table
                    .as_ref()
                    .map(|t| t.lines.iter().filter(|l| l.1 == B).map(|l| l.2.as_str()).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default()
            };
            let all = self.wait(900.0, |a, _| joined_from_b(a).ends_with(&long));
            let pieces = self.st(&self.b).table.as_ref().map(|t| t.lines.iter().filter(|l| l.1 == B).count()).unwrap_or(0);
            let at_a: Vec<usize> = self
                .st(&self.a)
                .table
                .as_ref()
                .map(|t| t.lines.iter().filter(|l| l.1 == B).map(|l| l.2.len()).collect())
                .unwrap_or_default();
            self.record(
                "slow table: a long message goes as several and all of it arrives, in order",
                all.is_some() && pieces > 1,
                format!(
                    "{} chars as {pieces} messages; all at ALPHA in order {} (ALPHA's lines from BRAVO, chars: {at_a:?}); table speed A {} / B {}",
                    long.len(),
                    Self::took(all),
                    Self::table_mode(&self.st(&self.a)),
                    Self::table_mode(&self.st(&self.b))
                ),
            );
            cleanup(self);
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
            // This measures the scan, not listen-before-transmit: the bench
            // steps the noise up 22-28 dB at once, which Mercury's busy
            // detector reads as a signal for minutes (and a real CQ would
            // then wait up to a minute for a clear channel).
            self.update(&self.a, |c| c.busy_detect = false);
            let base = self.ether.channel.lock().unwrap().noise_dbfs;
            for (label, extra_db) in [("normal", 0.0), ("weak", 22.0), ("very weak", 28.0)] {
                self.ether.channel.lock().unwrap().noise_dbfs = base + extra_db;
                println!("   scan test ({label}): {}", self.snr_note());
                let mut times: Vec<f64> = vec![];
                let trials = 4;
                for trial in 0..trials {
                    self.a.send(Cmd::StopCq);
                    self.b.send(Cmd::StopSearch);
                    // (`update` starts from the station's own settings each
                    // time, so busy_detect is turned off here too.)
                    self.update(&self.a, |c| {
                        c.whistle_s = w;
                        c.cq_cycles = 1;
                        c.busy_detect = false;
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
            self.update(&self.a, |c| c.busy_detect = true);
        }

        /// QSY and the library over real Mercury, BRAVO away and serving:
        /// ALPHA browses and downloads, asks BRAVO to move (and to another
        /// band, refused), and both come home when the link ends.
        fn s_qsylib(&mut self) {
            self.disconnect_both();
            let notes: Vec<u8> = (0..3000u32).map(|i| b"QSL via the Whistlestop library. "[i as usize % 33]).collect();
            let sb = &self.stations[1];
            match sb.kind {
                Kind::Engine | Kind::Desktop => {
                    let lib = PathBuf::from(&sb.cfg.library_dir);
                    let _ = std::fs::remove_dir_all(&lib);
                    std::fs::create_dir_all(lib.join("logs")).unwrap();
                    std::fs::write(lib.join("notes.txt"), &notes).unwrap();
                    std::fs::write(lib.join("logs/today.adi"), "<CALL:5>ALPHA<EOR>").unwrap();
                }
                _ => {
                    sb.put("bench-lib/notes.txt", &notes).unwrap();
                    sb.put("bench-lib/logs/today.adi", b"<CALL:5>ALPHA<EOR>").unwrap();
                }
            }
            self.update(&self.b, |c| c.accept_qsy = true);
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
            let same = {
                let sa = &self.stations[0];
                sa.read(&sa.cfg.download_dir.join("notes.txt")).is_some_and(|d| d == notes)
            };
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
            self.update(&self.b, |c| c.accept_qsy = false);
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

        // ------------------------------------------------------ three stations

        fn c(&self) -> &Handle {
            self.c.as_ref().expect("CHARLIE runs only when a three-station scenario is named")
        }

        fn all(&self) -> [&Handle; 3] {
            [&self.a, &self.b, self.c()]
        }

        fn handle_of(&self, call: &str) -> Option<&Handle> {
            match call {
                A => Some(&self.a),
                B => Some(&self.b),
                C => self.c.as_ref(),
                _ => None,
            }
        }

        /// Wait for a condition on all three stations' state.
        fn wait3(&self, secs: f64, f: impl Fn(&State, &State, &State) -> bool) -> Option<Duration> {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs_f64(secs) {
                if self.peek3(|a, b, c| f(a, b, c)) {
                    return Some(t.elapsed());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            None
        }

        /// Read all three stations' state at once.
        fn peek3<T>(&self, f: impl FnOnce(&State, &State, &State) -> T) -> T {
            let (a, b, c) = (self.a.state.lock().unwrap(), self.b.state.lock().unwrap(), self.c().state.lock().unwrap());
            f(&a, &b, &c)
        }

        fn pick<'s>(call: &str, a: &'s State, b: &'s State, c: &'s State) -> &'s State {
            match call {
                A => a,
                B => b,
                _ => c,
            }
        }

        fn linked_with(s: &State, peer: &str) -> bool {
            s.link.as_ref().is_some_and(|l| l.peer == peer)
        }

        fn took(d: Option<Duration>) -> String {
            d.map(|d| format!("in {:.0} s", d.as_secs_f64())).unwrap_or("NO".into())
        }

        /// Everyone leaves any table and link, and stops calling and searching.
        fn disconnect_all(&self) {
            for h in self.all() {
                if self.st(h).table.is_some() {
                    h.send(Cmd::LeaveTable);
                }
            }
            for _ in 0..3 {
                let Some(h) = self.all().into_iter().find(|h| self.st(h).link.is_some()) else { break };
                let peer = self.st(h).link.as_ref().map(|l| l.peer.clone()).unwrap_or_default();
                h.send(Cmd::Disconnect);
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(90) {
                    if self.st(h).link.is_none() && self.handle_of(&peer).is_none_or(|p| self.st(p).link.is_none()) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            for h in self.all() {
                h.send(Cmd::StopCq);
                h.send(Cmd::StopSearch);
            }
            std::thread::sleep(Duration::from_secs(2));
        }

        /// `from` calls `to` (both on ALPHA's frequency); records the connect.
        fn link3(&mut self, from: &str, to: &str, label: &str) -> bool {
            self.handle_of(from).unwrap().send(Cmd::Connect(to.into()));
            let t = self.wait3(180.0, |a, b, c| Self::linked_with(Self::pick(from, a, b, c), to) && Self::linked_with(Self::pick(to, a, b, c), from));
            self.record(label, t.is_some(), format!("{from} → {to} linked {}", Self::took(t)));
            t.is_some()
        }

        /// Where a station's log ends now (it keeps only its last 500 lines,
        /// so a plain index would drift).
        fn log_mark(&self, h: &Handle) -> (usize, Option<String>) {
            let g = self.st(h);
            (g.log.len(), g.log.back().map(|l| l.1.clone()))
        }

        fn log_after(&self, h: &Handle, mark: &(usize, Option<String>)) -> Vec<String> {
            let g = self.st(h);
            let start = match &mark.1 {
                None => 0,
                Some(t) => (0..mark.0.min(g.log.len())).rev().find(|&i| &g.log[i].1 == t).map(|i| i + 1).unwrap_or(0),
            };
            g.log.iter().skip(start).map(|l| l.1.clone()).collect()
        }

        fn count_after(&self, h: &Handle, mark: &(usize, Option<String>), pat: &str) -> usize {
            self.log_after(h, mark).iter().filter(|l| l.contains(pat)).count()
        }

        fn merc_log(&self, tag: &str) -> String {
            let call = match tag {
                "A" => A,
                "B" => B,
                "C" => C,
                t => t,
            };
            std::fs::read_to_string(self.work.join(format!("mercury-{call}.log"))).unwrap_or_default()
        }

        /// Mercury's ARQ state after its last change ("conn: X -> Y" in its
        /// log), and how many times it went to `to` after byte `from`.
        fn merc_state(&self, tag: &str) -> String {
            self.merc_log(tag)
                .lines()
                .rev()
                .find_map(|l| l.split("conn: ").nth(1).and_then(|s| s.split("-> ").nth(1)).map(|s| s.trim().to_string()))
                .unwrap_or("?".into())
        }

        fn merc_count(&self, tag: &str, from: usize, pat: &str) -> usize {
            let log = self.merc_log(tag);
            log.get(from.min(log.len())..).unwrap_or("").lines().filter(|l| l.contains(pat)).count()
        }

        fn merc_len(&self, tag: &str) -> usize {
            self.merc_log(tag).len()
        }

        /// Seconds of the day of the first line after byte `from` with `pat`
        /// ("2026-11-11 19:05:13.257 …").
        fn merc_time(&self, tag: &str, from: usize, pat: &str) -> Option<f64> {
            let log = self.merc_log(tag);
            let l = log.get(from.min(log.len())..)?.lines().find(|l| l.contains(pat))?;
            let hms = l.split_whitespace().nth(1)?;
            let mut p = hms.split(':');
            let (h, m, s): (f64, f64, f64) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
            Some(h * 3600.0 + m * 60.0 + s)
        }

        /// The channel SNR of every path (from the TX levels seen so far).
        fn snr3_note(&self) -> String {
            let s = self.ether.stats();
            let ch = self.ether.channel.lock().unwrap().clone();
            let nominal = s.iter().map(|x| x.tx_rms).fold(0.0, f64::max);
            let lvl = |r: f64| if r > 0.0 { 20.0 * r.log10() } else { -99.0 };
            let names = ["A", "B", "C"];
            let mut v = vec![];
            for t in 0..3 {
                for r in 0..3 {
                    if r != t {
                        let tx = if s[t].tx_rms > 0.0 { s[t].tx_rms } else { nominal };
                        v.push(format!("{}→{} {:+.0} dB", names[t], names[r], lvl(tx) - ch.path_loss_db - self.ether.pair_loss(r, t) - ch.noise_dbfs));
                    }
                }
            }
            format!("channel SNR {}", v.join(", "))
        }

        fn accept_on(&self, rx: &Handle, name: &str, secs: f64) -> Option<(u64, u64)> {
            let t = Instant::now();
            loop {
                let found = self
                    .st(rx)
                    .transfers
                    .iter()
                    .rev()
                    .find(|t| t.dir == Dir::In && t.name == name && t.state == XferState::WaitingForYou)
                    .map(|t| (t.fid, t.done));
                if let Some((fid, have)) = found {
                    rx.send(Cmd::AnswerOffer { fid, accept: true });
                    return Some((fid, have));
                }
                if t.elapsed().as_secs_f64() > secs {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        fn member(s: &State, who: &str) -> bool {
            s.table.as_ref().is_some_and(|t| t.members.contains_key(who))
        }

        fn has_line(s: &State, from: &str, text: &str) -> bool {
            s.table.as_ref().is_some_and(|t| t.lines.iter().any(|l| l.1 == from && l.2 == text))
        }

        fn table_mode(s: &State) -> &'static str {
            s.table.as_ref().map(|t| t.mode.name).unwrap_or("-")
        }

        fn table_bps(s: &State) -> f64 {
            s.table.as_ref().map(|t| t.mode.payload_bps).unwrap_or(0.0)
        }

        fn listen(s: &State) -> &'static str {
            s.listen_mode.map(|m| m.name).unwrap_or("?")
        }

        /// The host's members and what each reported.
        fn reports(s: &State) -> String {
            let mut v: Vec<String> = s
                .table
                .as_ref()
                .map(|t| t.members.iter().map(|(k, m)| format!("{k} {}", m.snr_of_host.map(|v| format!("{v:+} dB")).unwrap_or("?".into()))).collect())
                .unwrap_or_default();
            v.sort();
            if v.is_empty() { "none".into() } else { v.join(", ") }
        }

        fn now_s() -> u64 {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
        }

        fn heard_since(s: &State, call: &str, what: &str, since: u64) -> bool {
            s.heard.iter().any(|h| h.call == call && h.what == what && h.at.0 >= since)
        }

        /// A Roundtable of three on unequal paths. CHARLIE is far away: it
        /// hears ALPHA (the host) `TABLE3_WEAK_DB` weaker than BRAVO does,
        /// and ALPHA hears it as weakly; BRAVO and CHARLIE hear each other
        /// well. The weak member joins first: a JOIN goes out in DATAC3, so a
        /// host that has already sped up for a strong member couldn't hear it
        /// (that case is checked at the end, as a late join).
        fn s_table3(&mut self) {
            const NAME: &str = "Three Table";
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            self.ether.set_pair_loss(0, 2, TABLE3_WEAK_DB);
            self.b.send(Cmd::SetFreq(A_DIAL));
            self.c().send(Cmd::SetFreq(A_DIAL));
            self.update(&self.a, |c| {
                c.whistle_s = 6.0;
                c.cq_cycles = 1;
                c.cq_listen_s = 5.0;
            });
            std::thread::sleep(Duration::from_secs(3));
            self.a.send(Cmd::CallCq { table: Some(NAME.into()) });
            let called = |s: &State| s.heard.iter().any(|h| h.call == A && h.table.as_deref() == Some(NAME));
            let heard = self.wait3(60.0, |_, b, c| called(b) && called(c));
            println!("   table3 paths: {}", self.snr3_note());
            self.c().send(Cmd::JoinTable(NAME.into()));
            let cj = self.wait3(90.0, |a, _, _| Self::member(a, C));
            std::thread::sleep(Duration::from_secs(5));
            self.b.send(Cmd::JoinTable(NAME.into()));
            let bj = self.wait3(90.0, |a, _, _| Self::member(a, B));
            let (reports, hb, hc) = self.peek3(|a, b, c| (Self::reports(a), called(b), called(c)));
            // Joining needs only the table's name ("Join here"), so the CALL
            // is a check of its own.
            self.record(
                "table3: the host's CALL is heard (Roundtable under Heard)",
                heard.is_some(),
                format!("BRAVO {hb}, CHARLIE {hc} ({} after the CQ began)", Self::took(heard)),
            );
            self.record(
                "table3: host sees both joins",
                cj.is_some() && bj.is_some(),
                format!(
                    "CHARLIE's join seen by the host {}, BRAVO's {}; host's members (as they hear the host): {reports}; table speed now host {}",
                    Self::took(cj),
                    Self::took(bj),
                    Self::table_mode(&self.st(&self.a))
                ),
            );
            if cj.is_none() && bj.is_none() {
                self.disconnect_all();
                self.ether.set_pair_loss(0, 2, 0.0);
                return;
            }

            // A multi-frame message from each station, one after another.
            let mut all_ok = true;
            let mut notes = vec![];
            let mut c_spoke = Instant::now();
            for who in [A, B, C] {
                let text = format!(
                    "{who} here with a longer Roundtable message, split over several broadcast frames, to check that every station puts it back together in the right order. "
                )
                .repeat(2)
                    + "73";
                if who == C {
                    c_spoke = Instant::now();
                }
                self.handle_of(who).unwrap().send(Cmd::TableSay(text.clone()));
                let t = self.wait3(300.0, |a, b, c| [(A, a), (B, b), (C, c)].iter().all(|(n, s)| *n == who || Self::has_line(s, who, &text)));
                let got: Vec<&str> =
                    self.peek3(|a, b, c| [(A, a), (B, b), (C, c)].iter().filter(|(n, s)| *n != who && Self::has_line(s, who, &text)).map(|(n, _)| *n).collect());
                all_ok &= t.is_some();
                notes.push(format!("{who}'s {} chars reached {} {}", text.len(), if got.is_empty() { "nobody".into() } else { got.join(" and ") }, Self::took(t)));
            }
            let modes_now = self.peek3(|a, b, c| format!("table speed A {} / B {} / C {}", Self::table_mode(a), Self::table_mode(b), Self::table_mode(c)));
            self.record("table3: a multi-frame message from each member reaches the other two", all_ok, format!("{}; {modes_now}", notes.join("; ")));

            // The host re-checks the speed every 30 s: give it two goes.
            std::thread::sleep(Duration::from_secs(65));
            let (ma, mb, mc, worst, reports) = self.peek3(|a, b, c| {
                (
                    Self::table_mode(a),
                    Self::table_mode(b),
                    Self::table_mode(c),
                    a.table.as_ref().and_then(|t| t.members.values().filter_map(|m| m.snr_of_host).min()),
                    Self::reports(a),
                )
            });
            let want = worst.map(|w| modes::best_mode(Bandwidth::Hz2300, w as f64, 3.0).name).unwrap_or("-");
            let probe = "ALPHA at the table's speed: CHARLIE, do you copy?";
            self.a.send(Cmd::TableSay(probe.into()));
            let copied = self.wait3(240.0, |_, _, c| Self::has_line(c, A, probe));
            self.record(
                "table3: host runs at the speed the weakest member can copy",
                ma == want && mb == ma && mc == ma && copied.is_some(),
                format!(
                    "host {ma}, BRAVO {mb}, CHARLIE {mc}; reports {reports} → weakest {worst:?} dB → {want} expected (3 dB margin); CHARLIE copied the host at that speed {}",
                    Self::took(copied)
                ),
            );

            // CHARLIE now only listens while BRAVO keeps talking. The host
            // counts only reports from members heard in the last 5 minutes.
            let mut said = 0;
            while c_spoke.elapsed() < Duration::from_secs(330) {
                said += 1;
                self.b.send(Cmd::TableSay(format!("BRAVO again ({said})")));
                std::thread::sleep(Duration::from_secs(60).min(Duration::from_secs(331).saturating_sub(c_spoke.elapsed())));
            }
            // Two ramp checks, and a switch announced in DATAC3 (~10 s).
            std::thread::sleep(Duration::from_secs(70));
            let (ma, mc) = self.peek3(|a, _, c| (Self::table_mode(a), Self::table_mode(c)));
            let probe = "ALPHA again, after CHARLIE has listened for five minutes";
            self.a.send(Cmd::TableSay(probe.into()));
            let copied = self.wait3(150.0, |_, _, c| Self::has_line(c, A, probe));
            self.record(
                "table3: host stays at the weakest member's speed while that member only listens",
                ma == want && copied.is_some(),
                format!(
                    "CHARLIE silent {:.0} s, BRAVO spoke {said}×; host {ma} (CHARLIE listening in {mc}), {want} expected; CHARLIE copied the host {}",
                    c_spoke.elapsed().as_secs_f64(),
                    Self::took(copied)
                ),
            );

            // The weak member leaves.
            let before_leave = Self::table_mode(&self.st(&self.a));
            self.c().send(Cmd::LeaveTable);
            let left_line = |s: &State| s.table.as_ref().is_some_and(|t| t.lines.iter().any(|l| l.1.is_empty() && l.2 == format!("{C} left")));
            let left = self.wait3(120.0, |a, b, c| !Self::member(a, C) && left_line(a) && left_line(b) && c.table.is_none());
            let back = self.wait3(30.0, |_, _, c| Self::listen(c) == "DATAC3");
            let (lc, ra) = self.peek3(|a, _, c| (Self::listen(c), Self::reports(a)));
            self.record(
                "table3: LEAVE handled (host and the other member)",
                left.is_some() && back.is_some(),
                format!("CHARLIE gone from the host's members, \"{C} left\" at ALPHA and BRAVO {}; CHARLIE listens in {lc}; host's members now {ra}", Self::took(left)),
            );

            // Only BRAVO (strong) is left: the host may speed up.
            let dc3 = modes::mode_by_name("DATAC3").unwrap().payload_bps;
            let faster = self.wait3(180.0, |a, b, _| Self::table_bps(a) > dc3 && Self::table_mode(b) == Self::table_mode(a));
            let talk = if faster.is_some() {
                let t = "BRAVO at the new speed";
                self.b.send(Cmd::TableSay(t.into()));
                self.wait3(120.0, |a, _, _| Self::has_line(a, B, t))
            } else {
                None
            };
            let (ma, mb) = self.peek3(|a, b, _| (Self::table_mode(a), Self::table_mode(b)));
            self.record(
                "table3: host speeds up once the weak member has left",
                faster.is_some() && talk.is_some(),
                format!(
                    "host {before_leave} when CHARLIE left, now {ma}, BRAVO {mb} (faster than DATAC3 {}); BRAVO→ALPHA at that speed {}",
                    Self::took(faster),
                    Self::took(talk)
                ),
            );

            // A station joining now: its JOIN goes out in DATAC3, which the
            // host hears at its next join window (every 3 minutes, 20 s).
            if faster.is_some() {
                self.c().send(Cmd::JoinTable(NAME.into()));
                let lj = self.wait3(240.0, |a, _, _| Self::member(a, C));
                let slowed = if lj.is_some() { self.wait3(90.0, |a, _, c| Self::table_bps(a) <= dc3 && Self::table_mode(c) == Self::table_mode(a)) } else { None };
                let (ma, mc, ra) = self.peek3(|a, _, c| (Self::table_mode(a), Self::table_mode(c), Self::reports(a)));
                self.record(
                    "table3: a station joining after the speed-up is heard (and the table slows for it)",
                    lj.is_some() && slowed.is_some(),
                    format!("CHARLIE's JOIN (sent in DATAC3) seen by the host {}; slowed {}; host {ma}, CHARLIE {mc}; host's members {ra}", Self::took(lj), Self::took(slowed)),
                );
            }

            // Everyone leaves: back to DATAC3, the calling mode.
            self.b.send(Cmd::LeaveTable);
            std::thread::sleep(Duration::from_secs(1));
            if self.st(self.c()).table.is_some() {
                self.c().send(Cmd::LeaveTable);
            }
            std::thread::sleep(Duration::from_secs(1));
            self.a.send(Cmd::LeaveTable);
            let calling = self.wait3(120.0, |a, b, c| [a, b, c].iter().all(|s| s.table.is_none() && Self::listen(s) == "DATAC3"));
            let lm = self.peek3(|a, b, c| format!("A {} / B {} / C {}", Self::listen(a), Self::listen(b), Self::listen(c)));
            // And they hear calls again: ALPHA calls CQ, whose CALL packet goes
            // out in DATAC3.
            self.wait_air_quiet(5.0, 60.0);
            self.update(&self.a, |c| {
                c.whistle_s = 4.0;
                c.cq_cycles = 1;
                c.cq_listen_s = 5.0;
            });
            std::thread::sleep(Duration::from_secs(1));
            let t0 = Self::now_s();
            self.a.send(Cmd::CallCq { table: None });
            let cq = self.wait3(60.0, |_, b, c| Self::heard_since(b, A, "CQ (chat)", t0) && Self::heard_since(c, A, "CQ (chat)", t0));
            self.a.send(Cmd::StopCq);
            self.record(
                "table3: after everyone leaves, all listen in DATAC3 (calling mode)",
                calling.is_some() && cq.is_some(),
                format!("listen modes {lm} {}; ALPHA's next CQ CALL heard by BRAVO and CHARLIE {}", Self::took(calling), Self::took(cq)),
            );
            self.ether.set_pair_loss(0, 2, 0.0);
            self.update(&self.a, |_| {});
            std::thread::sleep(Duration::from_secs(3));
        }

        /// ALPHA posts mail for CHARLIE via BRAVO; returns its id.
        fn post_via_b(&self, text: &str) -> Option<u64> {
            self.a.send(Cmd::DepotPost { to: C.into(), via: Some(B.into()), text: text.into() });
            self.wait3(10.0, |a, _, _| a.depot.departures.iter().any(|m| m.text == text))?;
            self.st(&self.a).depot.departures.iter().find(|m| m.text == text).map(|m| m.id)
        }

        fn in_transit(s: &State, id: u64) -> bool {
            s.depot.transit.iter().any(|m| m.id == id)
        }

        fn delivered_to(s: &State, id: u64, by: &str) -> bool {
            s.depot.delivered.iter().any(|m| m.id == id && m.handed.as_deref() == Some(by))
        }

        fn arrivals(s: &State, id: u64) -> usize {
            s.depot.arrivals.iter().filter(|m| m.id == id).count()
        }

        /// ALPHA hands mail `id` to BRAVO over a link; true when BRAVO has it
        /// In Transit and ALPHA shows it delivered to BRAVO.
        fn hand_to_relay(&mut self, id: u64, label: &str) -> Option<Duration> {
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            if !self.link3(A, B, label) {
                return None;
            }
            let t = self.wait3(180.0, |a, b, _| Self::in_transit(b, id) && Self::delivered_to(a, id, B));
            self.disconnect_all();
            t
        }

        /// Store and forward: ALPHA's mail for CHARLIE goes via BRAVO (relay
        /// on). Hand-over to the relay, then BRAVO links CHARLIE by hand;
        /// then CHARLIE (away, automatic collection) hears BRAVO's Station
        /// ID and collects by itself; then BRAVO (away, automatic delivery)
        /// hears CHARLIE's Station ID and delivers by itself.
        fn s_relay3(&mut self) {
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            self.b.send(Cmd::SetFreq(A_DIAL));
            self.c().send(Cmd::SetFreq(A_DIAL));
            self.update(&self.b, |c| c.relay = true);
            std::thread::sleep(Duration::from_secs(2));

            // 1. ALPHA → BRAVO (the relay).
            let Some(id1) = self.post_via_b("Relay test 1: ALPHA to CHARLIE via BRAVO") else {
                self.record("relay3: post", false, "ALPHA's mail never showed in Departures".into());
                return;
            };
            if !self.link3(A, B, "relay3: ALPHA links BRAVO") {
                self.disconnect_all();
                return;
            }
            let handed = self.wait3(180.0, |a, b, _| Self::in_transit(b, id1) && Self::delivered_to(a, id1, B));
            std::thread::sleep(Duration::from_secs(5));
            let (ok, detail) = self.peek3(|a, b, c| {
                let tr = b.depot.transit.iter().find(|m| m.id == id1);
                let dl = a.depot.delivered.iter().find(|m| m.id == id1);
                let still = a.depot.departures.iter().any(|m| m.id == id1);
                let at_c = Self::arrivals(c, id1);
                (
                    tr.is_some_and(|m| m.hops == 1 && m.handed.as_deref() == Some(A)) && dl.is_some_and(|m| m.handed.as_deref() == Some(B)) && !still && at_c == 0,
                    format!(
                        "BRAVO In Transit: {}; ALPHA Delivered: {}; still in ALPHA's Departures {still}; at CHARLIE already {at_c}",
                        tr.map(|m| format!("for {} hops {} handed by {:?}", m.to, m.hops, m.handed)).unwrap_or("no".into()),
                        dl.map(|m| format!("taken by {:?}", m.handed)).unwrap_or("no".into()),
                    ),
                )
            });
            self.record("relay3: mail In Transit at BRAVO, ALPHA shows it delivered (MAILACK)", handed.is_some() && ok, format!("{} {detail}", Self::took(handed)));
            self.disconnect_all();

            // 2. BRAVO links CHARLIE by hand.
            self.wait_air_quiet(10.0, 120.0);
            let mc = self.log_mark(self.c());
            if self.link3(B, C, "relay3: BRAVO links CHARLIE") {
                let got = self.wait3(180.0, |_, b, c| Self::arrivals(c, id1) > 0 && !Self::in_transit(b, id1));
                // Time for a repeat to show up.
                std::thread::sleep(Duration::from_secs(15));
                let said = self.count_after(self.c(), &mc, "Mail from ALPHA arrived");
                let (ok, detail) = self.peek3(|_, b, c| {
                    let m = c.depot.arrivals.iter().find(|m| m.id == id1);
                    let n = Self::arrivals(c, id1);
                    let by = Self::delivered_to(b, id1, C);
                    (
                        n == 1 && m.is_some_and(|m| m.hops == 1 && m.from == A) && by && !Self::in_transit(b, id1),
                        format!(
                            "CHARLIE's Arrivals hold it {n}×{}; BRAVO's In Transit {} left, Delivered to CHARLIE {by}",
                            m.map(|m| format!(" (from {}, hops {} on the wire)", m.from, m.hops)).unwrap_or_default(),
                            b.depot.transit.len()
                        ),
                    )
                });
                self.record(
                    "relay3: BRAVO links CHARLIE: mail arrives once, hops 1, transit empties",
                    got.is_some() && ok && said == 1,
                    format!("{} {detail}; CHARLIE logged the arrival {said}×", Self::took(got)),
                );
            }

            // 3. CHARLIE away, collecting by itself, hears BRAVO's Station ID.
            let id2 = self.post_via_b("Relay test 2: collected by CHARLIE on its own");
            let carried2 = id2.and_then(|id| self.hand_to_relay(id, "relay3: ALPHA hands BRAVO a second message"));
            if let (Some(id2), Some(_)) = (id2, carried2) {
                self.wait_air_quiet(10.0, 120.0);
                self.update(self.c(), |c| c.auto_collect = true);
                self.c().send(Cmd::SetAway(true));
                std::thread::sleep(Duration::from_secs(2));
                let mc = self.log_mark(self.c());
                self.b.send(Cmd::BeaconNow);
                let alert = self.wait3(90.0, |_, _, c| c.mail_alerts.iter().any(|m| m.call == B && !m.ours));
                let col = self.wait3(300.0, |_, b, c| Self::arrivals(c, id2) > 0 && !Self::in_transit(b, id2));
                let hung = self.wait3(150.0, |_, b, c| b.link.is_none() && c.link.is_none());
                let went = self.count_after(self.c(), &mc, "connecting to BRAVO to collect");
                let n = self.peek3(|_, _, c| Self::arrivals(c, id2));
                self.record(
                    "relay3: away CHARLIE hears BRAVO's Station ID and collects by itself",
                    alert.is_some() && col.is_some() && hung.is_some() && went >= 1 && n == 1,
                    format!(
                        "\"BRAVO has mail\" shown {}; CHARLIE connected by itself {went}×; mail arrived {} ({n}×), BRAVO's transit empty; link hung up {}",
                        Self::took(alert),
                        Self::took(col),
                        Self::took(hung)
                    ),
                );
                self.disconnect_all();
                self.c().send(Cmd::SetAway(false));
                self.update(self.c(), |_| {});
            } else {
                self.record("relay3: second message handed to BRAVO", false, format!("posted {id2:?}, handed {}", Self::took(carried2)));
            }

            // 4. BRAVO away, delivering by itself, hears CHARLIE's Station ID.
            let id3 = self.post_via_b("Relay test 3: delivered by BRAVO on its own");
            let carried3 = id3.and_then(|id| self.hand_to_relay(id, "relay3: ALPHA hands BRAVO a third message"));
            if let (Some(id3), Some(_)) = (id3, carried3) {
                self.wait_air_quiet(10.0, 120.0);
                self.update(&self.b, |c| {
                    c.relay = true;
                    c.auto_deliver = true;
                });
                self.b.send(Cmd::SetAway(true));
                std::thread::sleep(Duration::from_secs(2));
                let mb = self.log_mark(&self.b);
                self.c().send(Cmd::BeaconNow);
                let del = self.wait3(300.0, |_, b, c| Self::arrivals(c, id3) > 0 && !Self::in_transit(b, id3) && Self::delivered_to(b, id3, C));
                let hung = self.wait3(150.0, |_, b, c| b.link.is_none() && c.link.is_none());
                let went = self.count_after(&self.b, &mb, "connecting to CHARLIE to deliver");
                let n = self.peek3(|_, _, c| Self::arrivals(c, id3));
                self.record(
                    "relay3: away BRAVO hears CHARLIE's Station ID and delivers by itself",
                    del.is_some() && hung.is_some() && went >= 1 && n == 1,
                    format!("BRAVO connected by itself {went}×; delivered and confirmed {} ({n}×); link hung up {}", Self::took(del), Self::took(hung)),
                );
                self.disconnect_all();
                self.b.send(Cmd::SetAway(false));
            } else {
                self.record("relay3: third message handed to BRAVO", false, format!("posted {id3:?}, handed {}", Self::took(carried3)));
            }
            self.update(&self.b, |_| {});
        }

        /// ALPHA calls CQ; BRAVO and CHARLIE both search with automatic
        /// answer on, find it and call it at about the same moment.
        fn s_cq_pileup(&mut self) {
            self.disconnect_all();
            self.wait_air_quiet(25.0, 240.0);
            self.update(&self.a, |c| {
                c.whistle_s = 20.0;
                c.cq_cycles = 2;
                c.cq_listen_s = 20.0;
            });
            self.update(&self.b, |c| c.auto_answer = true);
            self.update(self.c(), |c| c.auto_answer = true);
            self.b.send(Cmd::SetFreq(14_096_000.0));
            self.c().send(Cmd::SetFreq(14_100_600.0));
            std::thread::sleep(Duration::from_secs(2));
            self.b.send(Cmd::StartSearch(SearchScope::CurrentBand));
            self.c().send(Cmd::StartSearch(SearchScope::CurrentBand));
            std::thread::sleep(Duration::from_secs(2));
            let (mb, mc) = (self.log_mark(&self.b), self.log_mark(self.c()));
            let mlen: Vec<usize> = ["A", "B", "C"].iter().map(|t| self.merc_len(t)).collect();
            self.a.send(Cmd::CallCq { table: None });

            // Watch: who ALPHA links with, and never both at once.
            let t0 = Instant::now();
            let mut a_peers: Vec<String> = vec![];
            let mut both = false;
            let mut linked_at: Option<Instant> = None;
            let mut marks = None;
            while t0.elapsed() < Duration::from_secs(300) {
                let (ap, bl, cl) = self.peek3(|a, b, c| (a.link.as_ref().map(|l| l.peer.clone()), Self::linked_with(b, A), Self::linked_with(c, A)));
                if let Some(p) = ap {
                    if !a_peers.contains(&p) {
                        a_peers.push(p);
                    }
                    if linked_at.is_none() {
                        linked_at = Some(Instant::now());
                        marks = Some([self.log_mark(&self.a), self.log_mark(&self.b), self.log_mark(self.c())]);
                    }
                }
                both |= bl && cl;
                // Mercury gives up a call after about 45 s: watch 90 s more.
                if linked_at.is_some_and(|t| t.elapsed() > Duration::from_secs(90)) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let (ans_b, ans_c) = (self.count_after(&self.b, &mb, "Auto-answering ALPHA"), self.count_after(self.c(), &mc, "Auto-answering ALPHA"));
            let (tb, tc) = (self.merc_time("B", mlen[1], "CONNECT BRAVO ALPHA"), self.merc_time("C", mlen[2], "CONNECT CHARLIE ALPHA"));
            let gap = tb.zip(tc).map(|(b, c)| format!("{:.1} s apart", (b - c).abs())).unwrap_or("—".into());
            self.record(
                "cq_pileup: both callers answer ALPHA's CQ",
                ans_b >= 1 && ans_c >= 1,
                format!("BRAVO auto-answered {ans_b}×, CHARLIE {ans_c}×; their CONNECTs reached Mercury {gap}"),
            );
            let winner = a_peers.first().cloned();
            self.record(
                "cq_pileup: ALPHA links with exactly one",
                a_peers.len() == 1 && !both,
                format!(
                    "ALPHA's link peers {a_peers:?} (first after {}); both linked to ALPHA at once: {both}",
                    linked_at.map(|t| format!("{:.0} s", (t - t0).as_secs_f64())).unwrap_or("—".into())
                ),
            );
            let Some(winner) = winner else {
                self.disconnect_all();
                self.update(&self.a, |_| {});
                self.update(self.c(), |_| {});
                return;
            };
            let loser = if winner == B { C } else { B };
            let (ltag, wtag) = if loser == B { ("B", "C") } else { ("C", "B") };
            let lidx = if loser == B { 1 } else { 2 };

            // The winner's link carries traffic.
            let wh = self.handle_of(&winner).unwrap();
            wh.send(Cmd::Chat("Pileup winner checking in".into()));
            let chat = self.wait3(150.0, |a, b, c| {
                a.chat.iter().any(|l| l.who == Who::Them && l.text == "Pileup winner checking in")
                    && Self::pick(&winner, a, b, c).chat.iter().any(|l| l.who == Who::Me && l.text == "Pileup winner checking in" && l.delivered)
            });
            // The loser: no link, no call hanging, not searching, Mercury idle.
            let clean = self.wait3(90.0, |a, b, c| {
                let l = Self::pick(loser, a, b, c);
                l.link.is_none() && l.calling.is_none() && !l.incoming_call && !l.search.active && !a.incoming_call
            });
            let lstate = self.merc_state(ltag);
            let lcalls = self.merc_count(ltag, mlen[lidx], "-> CALLING");
            let (lv, ai) = self.peek3(|a, b, c| {
                let l = Self::pick(loser, a, b, c);
                (format!("link {:?}, calling {:?}, incoming {}, search {}", l.link.as_ref().map(|l| l.peer.clone()), l.calling, l.incoming_call, l.search.active), a.incoming_call)
            });
            self.record(
                "cq_pileup: the winner's link works",
                chat.is_some(),
                format!("{winner} → ALPHA chat delivered {}", Self::took(chat)),
            );
            self.record(
                "cq_pileup: the other caller gives up cleanly",
                clean.is_some() && lstate == "LISTENING" && lcalls >= 1,
                format!("{loser}: {lv} {}; its Mercury went CALLING {lcalls}× and is now {lstate}; ALPHA incoming_call {ai}", Self::took(clean)),
            );
            // ALPHA's CQ ended when the link came up: any whistle reported
            // after that is someone's data heard as one.
            let reports: Vec<String> = marks
                .iter()
                .flat_map(|m| {
                    [(A, &self.a), (B, &self.b), (C, self.c())]
                        .into_iter()
                        .zip(m.iter())
                        .flat_map(|((n, h), mk)| self.log_after(h, mk).into_iter().filter(|l| l.contains("Whistle heard")).map(move |l| format!("{n}: {l}")))
                })
                .collect();
            self.record(
                "cq_pileup: no false whistles once ALPHA is linked",
                marks.is_some() && reports.is_empty(),
                format!("whistle reports after the link came up: {}", if reports.is_empty() { "none".into() } else { reports.join("; ") }),
            );

            // The first link ends; the other caller now gets through.
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            let later = self.link3(loser, A, &format!("cq_pileup: {loser} links ALPHA after the first link ends"));
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            let states: Vec<String> = ["A", "B", "C"].iter().map(|t| format!("{t} {}", self.merc_state(t))).collect();
            self.record(
                "cq_pileup: no modem left stuck",
                later && states.iter().all(|s| s.ends_with("LISTENING")),
                format!("Mercury states at the end: {}; winner {wtag}", states.join(", ")),
            );
            self.update(&self.a, |_| {});
            self.update(&self.b, |_| {});
            self.update(self.c(), |_| {});
        }

        /// A busy channel: BRAVO and CHARLIE in a 1:1 link on ALPHA's
        /// frequency, ALPHA parked there. ALPHA's whistle watch must not
        /// mistake their traffic for whistles; ALPHA calls BRAVO (busy) and
        /// then calls CQ on top of them: what happens to their link, and is
        /// everyone left in a clean state?
        fn s_busy3(&mut self) {
            self.disconnect_all();
            self.wait_air_quiet(25.0, 240.0);
            self.b.send(Cmd::SetFreq(A_DIAL));
            self.c().send(Cmd::SetFreq(A_DIAL));
            self.update(&self.a, |c| {
                c.whistle_s = 10.0;
                c.cq_cycles = 1;
                c.cq_listen_s = 10.0;
            });
            std::thread::sleep(Duration::from_secs(2));
            let ma = self.log_mark(&self.a);
            if !self.link3(B, C, "busy3: BRAVO links CHARLIE") {
                self.disconnect_all();
                return;
            }
            let p = self.work.join("to-send/busy.bin");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let data = Self::noise_bytes(8000, 41);
            std::fs::write(&p, &data).unwrap();
            self.b.send(Cmd::SendFiles { paths: vec![p], zip: false });
            let Some((fid, _)) = self.accept_on(self.c(), "busy.bin", 120.0) else {
                self.record("busy3: file offer", false, "offer never arrived".into());
                self.disconnect_all();
                return;
            };
            self.wait3(300.0, |_, _, c| Self::row(c, fid, Dir::In).is_some_and(|t| t.done >= 1024));

            // ALPHA calls BRAVO, who is busy with CHARLIE.
            let la = self.merc_len("A");
            self.a.send(Cmd::Connect(B.into()));
            std::thread::sleep(Duration::from_secs(3));
            let calling0 = self.st(&self.a).calling.clone();
            let mut a_linked = false;
            let t = Instant::now();
            let mut gave_up = None;
            while t.elapsed() < Duration::from_secs(180) {
                let (al, ac) = self.peek3(|a, _, _| (a.link.is_some(), a.calling.is_some()));
                a_linked |= al;
                if !al && !ac {
                    gave_up = Some(t.elapsed());
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let a_merc = self.merc_state("A");
            let a_calls = self.merc_count("A", la, "-> CALLING");
            let (bc, flags) = self.peek3(|_, b, c| {
                (Self::linked_with(b, C) && Self::linked_with(c, B), format!("BRAVO incoming_call {}, CHARLIE incoming_call {}", b.incoming_call, c.incoming_call))
            });
            self.record(
                "busy3: calling a station that is in another link ends cleanly",
                calling0.is_some() && gave_up.is_some() && !a_linked && bc,
                format!(
                    "ALPHA showed calling {calling0:?}; gave up {} later; ALPHA ever linked {a_linked}; Mercury A went CALLING {a_calls}× and is now {a_merc}; BRAVO–CHARLIE still linked {bc}; {flags}",
                    gave_up.map(|d| format!("{:.0} s", d.as_secs_f64())).unwrap_or("NOT".into())
                ),
            );

            let done = self.wait3(600.0, |_, b, c| Self::sent_ok(b, fid) && Self::received(c, fid).is_some());
            let exact = Self::received(&self.st(self.c()), fid).and_then(|p| std::fs::read(p).ok()).is_some_and(|d| d == data);
            let st = self.peek3(|_, b, c| format!("B {}, C {}", Self::state_of(b, fid, Dir::Out), Self::state_of(c, fid, Dir::In)));
            self.record("busy3: their 8 KB file gets through ALPHA's call, byte-exact", done.is_some() && exact, format!("{} — {st}", Self::took(done)));
            let fw = self.count_after(&self.a, &ma, "Whistle heard");
            self.record(
                "busy3: no false whistles at the bystander during their link",
                fw == 0,
                format!("ALPHA (parked, not linked) reported {fw} whistle(s) while BRAVO and CHARLIE linked and sent a file"),
            );

            // ALPHA calls CQ on the busy frequency.
            if self.st(&self.b).link.is_some() {
                let t0 = Self::now_s();
                self.a.send(Cmd::CallCq { table: None });
                let start = Instant::now();
                let (mut overlap, mut a_tx) = (0.0, 0.0);
                let mut seen_cq = false;
                while start.elapsed() < Duration::from_secs(90) {
                    let s = self.ether.stats();
                    let a_on = s[0].rig_ptt || s[0].mercury_ptt;
                    if a_on {
                        a_tx += 0.1;
                        if s[1].mercury_ptt || s[2].mercury_ptt {
                            overlap += 0.1;
                        }
                    }
                    let active = self.st(&self.a).cq.active;
                    seen_cq |= active;
                    if seen_cq && !active {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                self.b.send(Cmd::Chat("Still here after ALPHA's CQ".into()));
                let chat = self.wait3(180.0, |_, b, c| {
                    c.chat.iter().any(|l| l.who == Who::Them && l.text.contains("after ALPHA's CQ")) && b.chat.iter().any(|l| l.who == Who::Me && l.text.contains("after ALPHA's CQ") && l.delivered)
                });
                let (up, heard) = self.peek3(|_, b, c| {
                    (
                        Self::linked_with(b, C) && Self::linked_with(c, B),
                        format!("CQ seen while linked: BRAVO {}, CHARLIE {}", Self::heard_since(b, A, "CQ", t0), Self::heard_since(c, A, "CQ", t0)),
                    )
                });
                self.record(
                    "busy3: ALPHA's CQ on a busy channel: their link survives",
                    chat.is_some() && up,
                    format!(
                        "ALPHA keyed {a_tx:.1} s for the CQ, {overlap:.1} s of it while BRAVO or CHARLIE was transmitting (nothing held it back); link still up {up}; chat across afterwards {}; {heard}",
                        Self::took(chat)
                    ),
                );
            }

            // Once they're done, ALPHA gets through.
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            let later = self.link3(A, B, "busy3: ALPHA links BRAVO once the channel is free");
            self.disconnect_all();
            self.wait_air_quiet(10.0, 120.0);
            let states: Vec<String> = ["A", "B", "C"].iter().map(|t| format!("{t} {}", self.merc_state(t))).collect();
            self.record(
                "busy3: no modem left stuck",
                later && states.iter().all(|s| s.ends_with("LISTENING")),
                format!("Mercury states at the end: {}", states.join(", ")),
            );
            self.update(&self.a, |_| {});
        }

        fn report(&self) -> String {
            let mut s = if self.c.is_some() {
                format!(
                    "# Whistlestop three-station bench (real Mercury 1.9.23 ×3)\n\nALPHA: {}. BRAVO: {}. CHARLIE: engine.\n\n",
                    self.stations[0].kind.label(),
                    self.stations[1].kind.label()
                )
            } else {
                format!(
                    "# Whistlestop two-station bench (real Mercury 1.9.23 ×2)\n\nALPHA: {}. BRAVO: {}.\n\n",
                    self.stations[0].kind.label(),
                    self.stations[1].kind.label()
                )
            };
            s += &format!("Run time {:.0} s. {}\n\n| Result | Test | Details |\n|---|---|---|\n", self.started.elapsed().as_secs_f64(), self.snr_note());
            for (n, ok, d) in &self.results {
                s += &format!("| {} | {n} | {d} |\n", if *ok { "PASS" } else { "**FAIL**" });
            }
            let passed = self.results.iter().filter(|r| r.1).count();
            s += &format!("\n{passed}/{} passed.\n", self.results.len());
            s
        }

        fn dump_logs(&self) {
            for (h, n) in [(&self.a, A), (&self.b, B)].into_iter().chain(self.c.as_ref().map(|c| (c, C))) {
                let g = h.state.lock().unwrap();
                let lines: Vec<String> = g.log.iter().map(|(at, l)| format!("{} {l}", at.hhmm())).collect();
                let _ = std::fs::write(self.work.join(format!("whistlestop-{n}.log")), lines.join("\n"));
            }
            for st in &self.stations {
                if matches!(st.kind, Kind::Ios | Kind::Android)
                    && let Some(l) = st.mercury_log(&self.work)
                {
                    let _ = std::fs::write(self.work.join(format!("mercury-{}-{}.log", st.call, if st.kind == Kind::Ios { "ios" } else { "android" })), l);
                }
            }
        }
    }

    // ------------------------------------------------------------ stations

    /// What runs a station: this process's engine (the original bench), or
    /// a real app operated by remote operation: the desktop app, the iPhone
    /// app in the Simulator, the Android app in the emulator, each with its
    /// own real Mercury on the ether.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum Kind {
        Engine,
        Desktop,
        Ios,
        Android,
    }

    impl Kind {
        fn parse(s: &str) -> anyhow::Result<Kind> {
            Ok(match s {
                "engine" => Kind::Engine,
                "desktop" => Kind::Desktop,
                "ios" => Kind::Ios,
                "android" => Kind::Android,
                _ => anyhow::bail!("station kind {s}: engine, desktop, ios or android"),
            })
        }
        fn label(self) -> &'static str {
            match self {
                Kind::Engine => "engine",
                Kind::Desktop => "desktop app",
                Kind::Ios => "iPhone app",
                Kind::Android => "Android app",
            }
        }
    }

    /// `TWOTEST_PORT_OFFSET` moves every port this bench opens on the
    /// computer, so two benches can run side by side.
    fn off() -> u16 {
        std::env::var("TWOTEST_PORT_OFFSET").ok().and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    const APP_ID: &str = "com.nr5p.whistlestop";
    const ANDROID_DIR: &str = "/data/user/0/com.nr5p.whistlestop/files";
    const REMOTE_PW: &str = "bench";

    pub struct Station {
        pub kind: Kind,
        pub call: &'static str,
        /// The station's settings, in its own paths.
        pub cfg: Config,
        /// Its folder (the app's, for a phone; on the phone for Android).
        pub dir: PathBuf,
        client: Option<whistlestop::remote::Client>,
        child: Option<Child>,
        forwards: Vec<String>,
        /// `adb reverse` ports (a real phone).
        reverses: Vec<String>,
        /// The phone app's own settings file (real phone, emulator or
        /// Simulator), put back when the bench ends (`Some(None)`: it had
        /// none), so the app isn't left set up as a bench station.
        phone_toml: Option<Option<Vec<u8>>>,
    }

    /// A station that started before a later one failed still gets its app
    /// stopped and its settings put back. (`stop` may run twice: it's
    /// harmless.)
    impl Drop for Station {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn adb(args: &[&str]) -> anyhow::Result<Vec<u8>> {
        let o = Command::new(adb_path()).args(args).output()?;
        anyhow::ensure!(o.status.success(), "adb {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr));
        Ok(o.stdout)
    }

    /// How the Android app reaches this computer: the emulator's alias for
    /// it, or (a real phone on USB, `ANDROID_SERIAL` not `emulator-…`)
    /// localhost, carried over the cable by `adb reverse`.
    fn android_host() -> &'static str {
        match std::env::var("ANDROID_SERIAL") {
            Ok(s) if !s.starts_with("emulator-") => "127.0.0.1",
            _ => "10.0.2.2",
        }
    }

    fn adb_path() -> PathBuf {
        let sdk = std::env::var_os("ANDROID_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Android/sdk"));
        sdk.join("platform-tools/adb")
    }

    /// Write a file in the Android app's folder (run-as: a debug build).
    fn adb_put(path: &str, data: &[u8]) -> anyhow::Result<()> {
        use std::io::Write;
        let parent = Path::new(path).parent().unwrap().to_string_lossy().to_string();
        adb(&["shell", "run-as", APP_ID, "mkdir", "-p", &parent])?;
        let mut c = Command::new(adb_path())
            .args(["exec-in", "run-as", APP_ID, "sh", "-c", &format!("cat > '{path}'")])
            .stdin(Stdio::piped())
            .spawn()?;
        c.stdin.take().unwrap().write_all(data)?;
        anyhow::ensure!(c.wait()?.success(), "couldn't write {path} on the phone");
        Ok(())
    }

    fn ios_dir() -> anyhow::Result<PathBuf> {
        let o = Command::new("xcrun").args(["simctl", "get_app_container", "booted", APP_ID, "data"]).output()?;
        anyhow::ensure!(o.status.success(), "the iPhone app isn't installed in the booted Simulator");
        Ok(PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()).join("Documents"))
    }

    /// Bridge TCP connections on `port` to the ether's socket for a
    /// station's Mercury (the Android emulator can't reach Unix sockets).
    fn bridge(port: u16, sock: PathBuf) -> anyhow::Result<()> {
        let l = std::net::TcpListener::bind(("127.0.0.1", port))?;
        std::thread::spawn(move || {
            for t in l.incoming().flatten() {
                let Ok(u) = std::os::unix::net::UnixStream::connect(&sock) else { continue };
                t.set_nodelay(true).ok();
                // When either side ends (the ether drops a Mercury that
                // stalls), close both, so Mercury sees it and reconnects.
                let (mut t2, mut u2) = (t.try_clone().unwrap(), u.try_clone().unwrap());
                let (mut t, mut u) = (t, u);
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut t, &mut u);
                    let _ = u.shutdown(std::net::Shutdown::Both);
                    let _ = t.shutdown(std::net::Shutdown::Both);
                });
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut u2, &mut t2);
                    let _ = u2.shutdown(std::net::Shutdown::Both);
                    let _ = t2.shutdown(std::net::Shutdown::Both);
                });
            }
        });
        Ok(())
    }

    impl Station {
        /// Start a station: `i` 0 for ALPHA, 1 for BRAVO, 2 for CHARLIE.
        fn start(kind: Kind, i: usize, mercury: &Path, work: &Path, sock: &Path, mercs: &mut Vec<Child>) -> anyhow::Result<(Station, Handle)> {
            let call = [A, B, C][i];
            let (rig, audio) = ([4632u16, 4642, 4652][i] + off(), [7300u16, 7310, 7320][i] + off());
            // Host Mercury ports per station; the iPhone's Mercury shares the
            // computer's network too, so it gets its own.
            let (port, bport) = match kind {
                Kind::Ios => ([8320u16, 8330, 8350][i] + off(), [8120u16, 8130, 8150][i] + off()),
                Kind::Android => (8300, 8100),
                _ => ([8300u16, 8310, 8340][i] + off(), [8100u16, 8110, 8140][i] + off()),
            };
            let remote_port = [7381u16, 7382, 7383][i] + off();
            let mut cfg = cfg_for(call, port, bport, rig, audio, work);
            let dir = match kind {
                Kind::Engine | Kind::Desktop => work.to_path_buf(),
                Kind::Ios => ios_dir()?,
                Kind::Android => PathBuf::from(ANDROID_DIR),
            };
            // The file library is the station's own (remote operation can't
            // switch it on or move it), so it's set here, at the station.
            // s_qsylib fills it.
            cfg.share_library = true;
            cfg.library_dir = match kind {
                Kind::Engine | Kind::Desktop => work.join(format!("{call}-library")),
                _ => dir.join("bench-lib"),
            }
            .to_string_lossy()
            .to_string();
            if matches!(kind, Kind::Ios | Kind::Android) {
                cfg.download_dir = dir.join("Downloads");
                cfg.depot_file = dir.join(format!("{call}-depot.json")).to_string_lossy().to_string();
            }
            // Ports a real phone dials back to this computer (over USB, `adb reverse`).
            let mut reverses = vec![];
            if kind == Kind::Android {
                let host = android_host();
                if host == "127.0.0.1" {
                    for p in [rig, audio] {
                        let r = format!("tcp:{p}");
                        adb(&["reverse", &r, &r])?;
                        reverses.push(r);
                    }
                }
                cfg.rigctld = format!("{host}:{rig}");
                cfg.audio_in = format!("ether:{host}:{audio}");
                cfg.audio_out = cfg.audio_in.clone();
            }
            if matches!(kind, Kind::Engine | Kind::Desktop) {
                let out = std::fs::File::create(work.join(format!("mercury-{call}.log")))?;
                mercs.push(
                    Command::new(mercury)
                        .args(["-x", "sock", "-p", &port.to_string(), "-b", &bport.to_string(), "-v"])
                        .env("MERCURY_AUDIO_SOCK", sock)
                        .current_dir(work)
                        .stdout(Stdio::from(out.try_clone()?))
                        .stderr(Stdio::from(out))
                        .spawn()?,
                );
                for p in [port, bport] {
                    anyhow::ensure!(wait_port(p, 20), "Mercury did not open port {p} (see mercury-*.log)");
                }
            }
            let mut st = Station { kind, call, cfg: cfg.clone(), dir: dir.clone(), client: None, child: None, forwards: vec![], reverses, phone_toml: None };
            let mut remote_cfg = cfg.clone();
            remote_cfg.remote_serve = true;
            remote_cfg.remote_port = remote_port;
            remote_cfg.remote_password = REMOTE_PW.into();
            let toml = toml::to_string(&remote_cfg)?;
            let bench_json = |audio: String| serde_json::json!({"mercury_audio": audio, "remote_port": remote_port, "remote_password": REMOTE_PW}).to_string();
            let addr = match kind {
                Kind::Engine => {
                    let _ = std::fs::remove_file(&cfg.depot_file);
                    let h = engine::spawn(|| {});
                    h.send(Cmd::Start(cfg));
                    return Ok((st, h));
                }
                Kind::Desktop => {
                    let _ = std::fs::remove_file(&cfg.depot_file);
                    let p = work.join(format!("{call}-desktop.toml"));
                    std::fs::write(&p, toml)?;
                    let bin = std::env::var_os("WHISTLESTOP_BIN").map(PathBuf::from).unwrap_or_else(|| std::env::current_exe().unwrap().with_file_name("whistlestop"));
                    let out = std::fs::File::create(work.join(format!("desktop-{call}.log")))?;
                    st.child = Some(
                        Command::new(bin).env("WHISTLESTOP_CONFIG", &p).stdout(Stdio::from(out.try_clone()?)).stderr(Stdio::from(out)).spawn()?,
                    );
                    format!("127.0.0.1:{remote_port}")
                }
                Kind::Ios => {
                    let _ = Command::new("xcrun").args(["simctl", "terminate", "booted", APP_ID]).output();
                    std::thread::sleep(Duration::from_secs(1));
                    for f in ["Downloads", "bench-lib"] {
                        let _ = std::fs::remove_dir_all(dir.join(f));
                    }
                    let _ = std::fs::remove_file(&cfg.depot_file);
                    let _ = std::fs::remove_file(dir.join("mercury.log"));
                    // The app's own settings, to put back at the end.
                    st.phone_toml = Some(std::fs::read(dir.join("whistlestop.toml")).ok());
                    std::fs::write(dir.join("whistlestop.toml"), toml)?;
                    std::fs::write(dir.join("bench.json"), bench_json(sock.to_string_lossy().to_string()))?;
                    let o = Command::new("xcrun").args(["simctl", "launch", "booted", APP_ID]).output()?;
                    anyhow::ensure!(o.status.success(), "couldn't launch the iPhone app: {}", String::from_utf8_lossy(&o.stderr));
                    format!("127.0.0.1:{remote_port}")
                }
                Kind::Android => {
                    adb(&["shell", "am", "force-stop", APP_ID])?;
                    let _ = adb(&["shell", "pm", "grant", APP_ID, "android.permission.RECORD_AUDIO"]);
                    let _ = adb(&["shell", "run-as", APP_ID, "rm", "-rf", &format!("{ANDROID_DIR}/Downloads"), &format!("{ANDROID_DIR}/bench-lib"), &cfg.depot_file, &format!("{ANDROID_DIR}/mercury.log")]);
                    let tcp = [7401u16, 7402][i] + off();
                    bridge(tcp, sock.to_path_buf())?;
                    let path = format!("{ANDROID_DIR}/whistlestop.toml");
                    {
                        // The app's own settings, to put back at the end.
                        st.phone_toml = Some(
                            adb(&["exec-out", "run-as", APP_ID, "sh", "-c", &format!("[ -f '{path}' ] && cat '{path}' || echo -n NONE")])
                                .ok()
                                .filter(|b| b.as_slice() != b"NONE"),
                        );
                    }
                    adb_put(&path, toml.as_bytes())?;
                    let host = android_host();
                    if host == "127.0.0.1" {
                        let r = format!("tcp:{tcp}");
                        adb(&["reverse", &r, &r])?;
                        st.reverses.push(r);
                    }
                    adb_put(&format!("{ANDROID_DIR}/bench.json"), bench_json(format!("tcp:{host}:{tcp}")).as_bytes())?;
                    let local = format!("tcp:{remote_port}");
                    adb(&["forward", &local, &format!("tcp:{remote_port}")])?;
                    st.forwards.push(local);
                    adb(&["shell", "am", "start", "-n", &format!("{APP_ID}/.MainActivity")])?;
                    format!("127.0.0.1:{remote_port}")
                }
            };
            // Operate it remotely, as one desktop operates another.
            let t = Instant::now();
            let client = loop {
                match whistlestop::remote::connect(&addr, REMOTE_PW, work.join(format!("{call}-fetched")), String::new(), Arc::new(|| {})) {
                    Ok(c) => break c,
                    Err(e) if t.elapsed() > Duration::from_secs(90) => anyhow::bail!("{} {call}: {e:#}", kind.label()),
                    Err(_) => std::thread::sleep(Duration::from_millis(500)),
                }
            };
            println!("   {call}: {} at {addr} ({})", kind.label(), client.desc);
            let h = client.handle.clone();
            st.client = Some(client);
            Ok((st, h))
        }

        /// Read a file on the station.
        pub fn read(&self, p: &Path) -> Option<Vec<u8>> {
            match self.kind {
                Kind::Android => adb(&["exec-out", "run-as", APP_ID, "cat", &p.to_string_lossy()]).ok(),
                _ => std::fs::read(p).ok(),
            }
        }

        /// Put a file on the station, at `rel` under its folder; returns
        /// the folder `rel`'s first component names, as the station sees it.
        pub fn put(&self, rel: &str, data: &[u8]) -> anyhow::Result<PathBuf> {
            let p = self.dir.join(rel);
            match self.kind {
                Kind::Android => adb_put(&p.to_string_lossy(), data)?,
                _ => {
                    std::fs::create_dir_all(p.parent().unwrap())?;
                    std::fs::write(&p, data)?;
                }
            }
            Ok(self.dir.join(rel.split('/').next().unwrap()))
        }

        pub fn mercury_log(&self, work: &Path) -> Option<Vec<u8>> {
            match self.kind {
                Kind::Engine | Kind::Desktop => std::fs::read(work.join(format!("mercury-{}.log", self.call))).ok(),
                _ => self.read(&self.dir.join("mercury.log")),
            }
        }

        fn stop(&mut self) {
            self.client = None;
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            match self.kind {
                Kind::Ios => {
                    let _ = Command::new("xcrun").args(["simctl", "terminate", "booted", APP_ID]).output();
                    let _ = std::fs::remove_file(self.dir.join("bench.json"));
                    if let Some(t) = self.phone_toml.take() {
                        let p = self.dir.join("whistlestop.toml");
                        let _ = match t {
                            Some(t) => std::fs::write(&p, t),
                            None => std::fs::remove_file(&p),
                        };
                    }
                }
                Kind::Android => {
                    let _ = adb(&["shell", "am", "force-stop", APP_ID]);
                    let _ = adb(&["shell", "run-as", APP_ID, "rm", "-f", &format!("{ANDROID_DIR}/bench.json")]);
                    for f in &self.forwards {
                        let _ = adb(&["forward", "--remove", f]);
                    }
                    for r in &self.reverses {
                        let _ = adb(&["reverse", "--remove", r]);
                    }
                    if let Some(t) = self.phone_toml.take() {
                        let path = format!("{ANDROID_DIR}/whistlestop.toml");
                        let _ = match t {
                            Some(t) => adb_put(&path, &t),
                            None => adb(&["shell", "run-as", APP_ID, "rm", "-f", &path]).map(|_| ()),
                        };
                    }
                }
                _ => {}
            }
        }
    }

    pub fn main() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let mercury = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "mercury".into()));
        let work = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "bench-run".into()));
        let mut kinds = [Kind::Engine, Kind::Engine];
        let mut only: Vec<String> = vec![];
        for a in args.iter().skip(3) {
            if let Some(k) = a.strip_prefix("--a=") {
                kinds[0] = Kind::parse(k)?;
            } else if let Some(k) = a.strip_prefix("--b=") {
                kinds[1] = Kind::parse(k)?;
            } else {
                only.push(a.clone());
            }
        }
        let run = |n: &str| only.is_empty() || only.iter().any(|o| o == n);
        let three = only.iter().any(|o| THREE.contains(&o.as_str()));
        let mut b = Bench::start(&mercury, work, kinds, three)?;
        println!("Bench up: {}× Mercury on the ether. Work dir {}", if three { 3 } else { 2 }, b.work.display());
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
        if named("table_slow") {
            b.s_table_slow();
        }
        // Three stations (CHARLIE is on the band only when one is named).
        if named("table3") {
            b.s_table3();
        }
        if named("relay3") {
            b.s_relay3();
        }
        if named("cq_pileup") {
            b.s_cq_pileup();
        }
        if named("busy3") {
            b.s_busy3();
        }
        b.dump_logs();
        let rep = b.report();
        std::fs::write(b.work.join("REPORT.md"), &rep)?;
        println!("\n{rep}");
        Ok(())
    }
}
