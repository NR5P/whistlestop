//! Over-the-air test: this computer and an Android phone on USB, talking
//! through real sound (the computer's speakers and microphone, the phone's
//! speaker and microphone), each with its own real Mercury. No radio, no
//! simulated band: what one plays, the other hears across the room.
//!
//!   cargo run --release --bin airtest -- <path/to/mercury> [workdir] [scenario...]
//!
//! ALPHA is an engine in this process, with Mercury on this computer's
//! default sound devices (`-x coreaudio` on macOS, `alsa` on Linux). BRAVO
//! is the Android app (debug build, installed) on the phone named by
//! `ANDROID_SERIAL`, operated remotely through `adb forward`, its built-in
//! Mercury on the phone's own audio (`bench.json` with an empty
//! `mercury_audio`, see `mobile/core/src/bench.rs`).
//!
//! Scenarios: `whistle` (BRAVO's CQ whistle, heard by ALPHA parked on
//! whistle watch), `link` (BRAVO calls ALPHA), `chat` (both ways, delivery
//! confirmed), `e2e` (encryption Prefer on both: same check code, sealed
//! chat), `table` (ALPHA hosts a Roundtable by CQ, BRAVO hears the CALL and
//! joins, both talk). Results go to `<workdir>/AIR-REPORT.md`.
//!
//! Keep the phone a little way from the computer with its volume up. The
//! computer hears its own transmissions too; Mercury is half duplex, so
//! that's what a radio without RX muting would do.

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    air::main()
}

#[cfg(not(unix))]
fn main() {
    eprintln!("airtest runs on macOS and Linux.");
}

#[cfg(unix)]
mod air {
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use whistlestop::config::Config;
    use whistlestop::e2e::Policy;
    use whistlestop::engine::{self, Cmd, E2eView, Handle, State, Who};
    use whistlestop::modes::Bandwidth;

    const A: &str = "N0MAC";
    const B: &str = "N0PHN";
    const APP_ID: &str = "com.nr5p.whistlestop";
    const ANDROID_DIR: &str = "/data/user/0/com.nr5p.whistlestop/files";
    const REMOTE_PW: &str = "bench";
    /// ALPHA's Mercury, away from a desktop Mercury's usual 8300/8100.
    const A_PORT: u16 = 8350;
    const A_BCAST: u16 = 8150;
    const REMOTE_PORT: u16 = 7383;

    fn adb(args: &[&str]) -> anyhow::Result<Vec<u8>> {
        let sdk = std::env::var_os("ANDROID_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap().join("Library/Android/sdk"));
        let o = Command::new(sdk.join("platform-tools/adb")).args(args).output()?;
        anyhow::ensure!(o.status.success(), "adb {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr));
        Ok(o.stdout)
    }

    fn adb_put(path: &str, data: &[u8]) -> anyhow::Result<()> {
        use std::io::Write;
        let sdk = std::env::var_os("ANDROID_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap().join("Library/Android/sdk"));
        let parent = Path::new(path).parent().unwrap().to_string_lossy().to_string();
        adb(&["shell", "run-as", APP_ID, "mkdir", "-p", &parent])?;
        // exec-in: the redirect runs inside run-as (as the app), not in adb's shell.
        let mut c = Command::new(sdk.join("platform-tools/adb"))
            .args(["exec-in", "run-as", APP_ID, "sh", "-c", &format!("cat > '{path}'")])
            .stdin(Stdio::piped())
            .spawn()?;
        c.stdin.take().unwrap().write_all(data)?;
        anyhow::ensure!(c.wait()?.success(), "adb: couldn't write {path}");
        Ok(())
    }

    fn cfg_for(call: &str, work: &Path) -> Config {
        let mut c = Config {
            callsign: call.into(),
            name: format!("{call} air test"),
            grid: if call == A { "EM12".into() } else { "FN31".into() },
            demo: false,
            mercury_host: "127.0.0.1".into(),
            // No radio: nothing to tune or key (the speaker is the transmitter).
            rigctld: "none".into(),
            bandwidth: Bandwidth::Hz2300,
            whistle_s: 8.0,
            cq_cycles: 2,
            cq_listen_s: 20.0,
            auto_answer: false,
            download_dir: work.join(format!("{call}-downloads")),
            depot_file: work.join(format!("{call}-depot.json")).to_string_lossy().to_string(),
            ..Default::default()
        };
        if call == A {
            c.mercury_port = A_PORT;
            c.mercury_bcast_port = A_BCAST;
        }
        c.sdr.enabled = false;
        c.sdr.wide_search = false;
        c
    }

    struct Air {
        a: Handle,
        b: Handle,
        cfg: [Config; 2],
        _client: whistlestop::remote::Client,
        mercury: Child,
        /// The phone's own settings file before the test (None: it had none).
        phone_toml: Option<Vec<u8>>,
        work: PathBuf,
        results: Vec<(String, bool, String)>,
    }

    impl Air {
        fn start(mercury: &Path, work: PathBuf) -> anyhow::Result<Air> {
            std::fs::create_dir_all(&work)?;
            let work = std::fs::canonicalize(&work)?;
            anyhow::ensure!(
                std::net::TcpListener::bind(("127.0.0.1", A_PORT)).is_ok(),
                "port {A_PORT} is taken (a Mercury left from an earlier run?)"
            );
            let sound = if cfg!(target_os = "macos") { "coreaudio" } else { "alsa" };
            let log = std::fs::File::create(work.join(format!("mercury-{A}.log")))?;
            let merc = Command::new(mercury)
                .args(["-x", sound, "-p", &A_PORT.to_string(), "-b", &A_BCAST.to_string(), "-v"])
                .current_dir(&work)
                .stdout(Stdio::from(log.try_clone()?))
                .stderr(Stdio::from(log))
                .spawn()?;
            // The test's settings replace the phone's own (remote operation on,
            // a test callsign); keep them to put back afterwards.
            let toml_path = format!("{ANDROID_DIR}/whistlestop.toml");
            let phone_toml = adb(&["exec-out", "run-as", APP_ID, "sh", "-c", &format!("[ -f '{toml_path}' ] && cat '{toml_path}' || echo -n NONE")])
                .ok()
                .filter(|b| b.as_slice() != b"NONE");
            match Self::stations(&work) {
                Ok((a, b, cfg, client)) => Ok(Air { a, b, cfg, _client: client, mercury: merc, phone_toml, work, results: vec![] }),
                Err(e) => {
                    restore_phone(phone_toml.as_deref());
                    // Don't leave this Mercury holding its ports for the next run.
                    let mut merc = merc;
                    let _ = merc.kill();
                    let _ = merc.wait();
                    Err(e)
                }
            }
        }

        fn stations(work: &Path) -> anyhow::Result<(Handle, Handle, [Config; 2], whistlestop::remote::Client)> {
            let work = work.to_path_buf();
            let ca = cfg_for(A, &work);
            let a = engine::spawn(|| {});
            a.send(Cmd::Start(ca.clone()));

            // BRAVO: the phone, its own sound, remote operation over USB.
            let mut cb = cfg_for(B, &work);
            cb.download_dir = PathBuf::from(format!("{ANDROID_DIR}/Downloads"));
            cb.depot_file = format!("{ANDROID_DIR}/{B}-depot.json");
            let mut remote = cb.clone();
            remote.remote_serve = true;
            remote.remote_port = REMOTE_PORT;
            remote.remote_password = REMOTE_PW.into();
            adb(&["shell", "am", "force-stop", APP_ID])?;
            let _ = adb(&["shell", "pm", "grant", APP_ID, "android.permission.RECORD_AUDIO"]);
            let _ = adb(&["shell", "run-as", APP_ID, "rm", "-f", &format!("{ANDROID_DIR}/mercury.log"), &cb.depot_file]);
            adb_put(&format!("{ANDROID_DIR}/whistlestop.toml"), toml::to_string(&remote)?.as_bytes())?;
            let bench = serde_json::json!({"mercury_audio": "", "remote_port": REMOTE_PORT, "remote_password": REMOTE_PW});
            adb_put(&format!("{ANDROID_DIR}/bench.json"), bench.to_string().as_bytes())?;
            let fwd = format!("tcp:{REMOTE_PORT}");
            adb(&["forward", &fwd, &fwd])?;
            adb(&["shell", "am", "start", "-n", &format!("{APP_ID}/.MainActivity")])?;
            let t = Instant::now();
            let client = loop {
                match whistlestop::remote::connect(&format!("127.0.0.1:{REMOTE_PORT}"), REMOTE_PW, work.join("fetched"), String::new(), Arc::new(|| {})) {
                    Ok(c) => break c,
                    Err(e) if t.elapsed() > Duration::from_secs(90) => anyhow::bail!("the phone: {e:#}"),
                    Err(_) => std::thread::sleep(Duration::from_millis(500)),
                }
            };
            println!("   {B}: the phone ({})", client.desc);
            let b = client.handle.clone();
            Ok((a, b, [ca, cb], client))
        }

        fn wait(&self, secs: f64, f: impl Fn(&State, &State) -> bool) -> Option<Duration> {
            let t = Instant::now();
            while t.elapsed().as_secs_f64() < secs {
                {
                    let (a, b) = (self.a.state.lock().unwrap(), self.b.state.lock().unwrap());
                    if f(&a, &b) {
                        return Some(t.elapsed());
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            None
        }

        fn record(&mut self, name: &str, ok: bool, note: String) {
            println!("{} {name}: {note}", if ok { "PASS" } else { "FAIL" });
            self.results.push((name.into(), ok, note));
        }

        fn secs(d: Option<Duration>) -> String {
            d.map(|d| format!("{:.0} s", d.as_secs_f64())).unwrap_or("timed out".into())
        }

        fn update(&mut self, i: usize, f: impl FnOnce(&mut Config)) {
            f(&mut self.cfg[i]);
            [&self.a, &self.b][i].send(Cmd::Update(self.cfg[i].clone()));
        }

        fn disconnect(&self) {
            for h in [&self.a, &self.b] {
                if h.state.lock().unwrap().link.is_some() {
                    h.send(Cmd::Disconnect);
                }
            }
            self.wait(90.0, |a, b| a.link.is_none() && b.link.is_none());
            std::thread::sleep(Duration::from_secs(3));
        }

        fn ready(&mut self) -> bool {
            let t = self.wait(60.0, |a, b| a.mercury_ok && b.mercury_ok);
            let (a, b) = (self.a.state.lock().unwrap().modem_version.clone(), self.b.state.lock().unwrap().modem_version.clone());
            self.record("both Mercury modems up", t.is_some(), format!("{} · {A} {a:?} · {B} {b:?}", Self::secs(t)));
            t.is_some()
        }

        fn link(&mut self, label: &str) -> bool {
            self.b.send(Cmd::Connect(A.into()));
            let t = self.wait(240.0, |a, b| a.link.is_some() && b.link.is_some());
            let note = {
                let b = self.b.state.lock().unwrap();
                match &b.link {
                    Some(l) => format!(
                        "linked in {} · {} Hz · {} · {:.0} bps · {B} hears {A} at {}",
                        Self::secs(t),
                        l.bw,
                        l.mode.map(|m| m.name).unwrap_or("-"),
                        l.bps,
                        b.snr_db.map(|s| format!("{s:.0} dB")).unwrap_or("-".into())
                    ),
                    None => format!("no link after {}", Self::secs(t)),
                }
            };
            self.record(label, t.is_some(), note);
            t.is_some()
        }

        fn chat(&mut self, label: &str, from_a: bool, text: &str) {
            let (h, name) = if from_a { (&self.a, "A→B") } else { (&self.b, "B→A") };
            h.send(Cmd::Chat(text.into()));
            let t = self.wait(180.0, |a, b| {
                let (tx, rx) = if from_a { (a, b) } else { (b, a) };
                rx.chat.iter().any(|c| c.who == Who::Them && c.text == text) && tx.chat.iter().any(|c| c.who == Who::Me && c.text == text && c.delivered)
            });
            self.record(&format!("{label} {name}"), t.is_some(), format!("arrived and confirmed in {}", Self::secs(t)));
        }

        /// The phone whistles (its CQ); the computer, parked, hears it.
        fn s_whistle(&mut self) {
            let warn = self.b.state.lock().unwrap().log.iter().find(|l| l.1.contains("Audio output")).map(|l| l.1.clone());
            self.record("phone: audio output started", warn.is_none(), warn.unwrap_or_else(|| "no warning in its log".into()));
            self.update(1, |c| {
                c.whistle_s = 12.0;
                c.cq_cycles = 1;
            });
            // Control: nothing whistles for 15 s, so nothing may be heard.
            let quiet = self.wait(15.0, |a, _| a.whistle.is_some());
            self.record("computer hears no whistle in 15 s of quiet", quiet.is_none(), match quiet {
                None => "none".into(),
                Some(d) => format!("false alarm after {:.0} s", d.as_secs_f64()),
            });
            let before = self.a.state.lock().unwrap().whistle.as_ref().map(|w| w.at);
            let start = Instant::now();
            self.b.send(Cmd::CallCq { table: None });
            let t = self.wait(60.0, |a, _| a.whistle.as_ref().is_some_and(|w| Some(w.at) != before && w.at > start));
            let note = match &self.a.state.lock().unwrap().whistle {
                Some(w) => format!("heard after {} · score {:.1} · offset {:+.0} Hz", Self::secs(t), w.score, w.offset_hz),
                None => "not heard".into(),
            };
            self.record("computer hears the phone's whistle", t.is_some(), note);
            self.b.send(Cmd::StopCq);
            std::thread::sleep(Duration::from_secs(5));
        }

        fn s_plain(&mut self) {
            if self.link("link: phone calls the computer") {
                self.chat("chat", true, "Hello from the Mac, over the air.");
                self.chat("chat", false, "Phone here, copy you fine.");
            }
            self.disconnect();
        }

        fn s_e2e(&mut self) {
            for i in 0..2 {
                self.update(i, |c| c.e2e = Policy::Prefer);
            }
            std::thread::sleep(Duration::from_secs(2));
            if self.link("encrypted link") {
                let t = self.wait(180.0, |a, b| {
                    let enc = |s: &State| s.link.as_ref().is_some_and(|l| matches!(l.e2e, E2eView::Encrypted { .. }));
                    enc(a) && enc(b)
                });
                let code = |h: &Handle| match h.state.lock().unwrap().link.as_ref().map(|l| l.e2e.clone()) {
                    Some(E2eView::Encrypted { sas }) => sas,
                    other => format!("{other:?}"),
                };
                let (ca, cb) = (code(&self.a), code(&self.b));
                self.record("encryption: same check code", t.is_some() && ca == cb, format!("after {} · {A} '{ca}' / {B} '{cb}'", Self::secs(t)));
                self.chat("encrypted chat", false, "sealed message from the phone");
            }
            self.disconnect();
            for i in 0..2 {
                self.update(i, |c| c.e2e = Policy::Off);
            }
        }

        fn s_table(&mut self) {
            let name = "Air Table";
            self.a.send(Cmd::CallCq { table: Some(name.into()) });
            let heard = self.wait(150.0, |_, b| b.heard.iter().any(|h| h.table.as_deref() == Some(name)));
            let whistle = self.b.state.lock().unwrap().whistle.as_ref().map(|w| format!("score {:.1}", w.score));
            self.record(
                "Roundtable: phone hears the CALL",
                heard.is_some(),
                format!("{} · phone heard the whistle: {}", Self::secs(heard), whistle.unwrap_or("no".into())),
            );
            if heard.is_none() {
                self.a.send(Cmd::StopCq);
                return;
            }
            self.b.send(Cmd::JoinTable(name.into()));
            let joined = self.wait(150.0, |a, _| a.table.as_ref().is_some_and(|t| t.members.contains_key(B)));
            self.record("Roundtable: host sees the phone join", joined.is_some(), Self::secs(joined));
            self.a.send(Cmd::TableSay("Welcome to the air table, phone.".into()));
            let got = self.wait(180.0, |_, b| b.table.as_ref().is_some_and(|t| t.lines.iter().any(|l| l.1 == A && l.2.contains("air table"))));
            self.record("Roundtable: host → phone", got.is_some(), Self::secs(got));
            self.b.send(Cmd::TableSay("Phone checking in at the table.".into()));
            let got = self.wait(180.0, |a, _| a.table.as_ref().is_some_and(|t| t.lines.iter().any(|l| l.1 == B && l.2.contains("checking in"))));
            self.record("Roundtable: phone → host", got.is_some(), Self::secs(got));
            self.a.send(Cmd::StopCq);
            self.a.send(Cmd::LeaveTable);
            self.b.send(Cmd::LeaveTable);
            std::thread::sleep(Duration::from_secs(5));
        }

        fn report(&mut self) -> anyhow::Result<()> {
            let mut r = String::from("# Over-the-air test (computer speakers/mic ↔ Android phone)\n\n");
            let pass = self.results.iter().filter(|r| r.1).count();
            r += &format!("{pass}/{} passed\n\n| | Check | Result |\n|---|---|---|\n", self.results.len());
            for (n, ok, note) in &self.results {
                r += &format!("| {} | {n} | {note} |\n", if *ok { "✅" } else { "❌" });
            }
            r += "\n## Log, computer (ALPHA)\n\n```\n";
            for l in self.a.state.lock().unwrap().log.iter() {
                r += &format!("{} {}\n", l.0.hhmm(), l.1);
            }
            r += "```\n\n## Log, phone (BRAVO)\n\n```\n";
            for l in self.b.state.lock().unwrap().log.iter() {
                r += &format!("{} {}\n", l.0.hhmm(), l.1);
            }
            r += "```\n";
            std::fs::write(self.work.join("AIR-REPORT.md"), &r)?;
            if let Ok(m) = adb(&["exec-out", "run-as", APP_ID, "cat", &format!("{ANDROID_DIR}/mercury.log")]) {
                std::fs::write(self.work.join(format!("mercury-{B}.log")), m)?;
            }
            println!("\n{pass}/{} passed · {}", self.results.len(), self.work.join("AIR-REPORT.md").display());
            Ok(())
        }
    }

    impl Drop for Air {
        fn drop(&mut self) {
            self.a.send(Cmd::Stop);
            let _ = self.mercury.kill();
            let _ = self.mercury.wait();
            restore_phone(self.phone_toml.as_deref());
            let _ = adb(&["forward", "--remove", &format!("tcp:{REMOTE_PORT}")]);
        }
    }

    /// Stop the app and put the phone's own settings back (or remove the
    /// test's, if it had none), so remote operation isn't left switched on.
    fn restore_phone(toml: Option<&[u8]>) {
        let _ = adb(&["shell", "am", "force-stop", APP_ID]);
        let _ = adb(&["shell", "run-as", APP_ID, "rm", "-f", &format!("{ANDROID_DIR}/bench.json"), &format!("{ANDROID_DIR}/{B}-depot.json")]);
        let path = format!("{ANDROID_DIR}/whistlestop.toml");
        let _ = match toml {
            Some(t) => adb_put(&path, t),
            None => adb(&["shell", "run-as", APP_ID, "rm", "-f", &path]).map(|_| ()),
        };
    }

    pub fn main() -> anyhow::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let mercury = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "mercury".into()));
        let work = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "air-run".into()));
        let only: Vec<String> = args.iter().skip(3).cloned().collect();
        let run = |n: &str| only.is_empty() || only.iter().any(|o| o == n);
        let mut air = Air::start(&mercury, work)?;
        if air.ready() {
            if run("whistle") {
                air.s_whistle();
            }
            if run("link") || run("chat") {
                air.s_plain();
            }
            if run("e2e") {
                air.s_e2e();
            }
            if run("table") {
                air.s_table();
            }
        }
        air.report()
    }
}
