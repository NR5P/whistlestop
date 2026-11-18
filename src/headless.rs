//! `whistlestop serve`: the station with no window, for a computer with no
//! screen (a Raspberry Pi beside the radio, a server in the shack). It runs
//! the same engine the app does, with the settings file the app uses, and
//! shares it on the LAN exactly as Remote → "Let other computers operate
//! this one" does: you operate it from Whistlestop on another computer.
//!
//! * Sharing is always on here (it's the only way in), so a remote-access
//!   password of at least 4 characters is required: from the settings,
//!   `WHISTLESTOP_REMOTE_PASSWORD`, or `--set-password` (asks once, saves).
//! * The Log goes to standard output (for journald or a terminal) and to
//!   `whistlestop.log` beside the settings, as in the app.
//! * Ctrl-C or SIGTERM stops the station the way quitting the app does:
//!   it waits up to 3 s for the engine to unkey the radio.
//! * Settings a remote operator changes are saved by the server, as in the
//!   app; this mode never writes the settings file on its own (except a
//!   password you set with `--set-password`).

use crate::gui::logsave::{SessionLog, lines};
use anyhow::{Context, bail};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use whistlestop::config::Config;
use whistlestop::engine::{self, Cmd};

/// Where a password given in the environment comes from.
pub const PASSWORD_ENV: &str = "WHISTLESTOP_REMOTE_PASSWORD";

pub struct Opts {
    pub port: Option<u16>,
    pub set_password: bool,
    pub quiet: bool,
}

pub fn run(o: Opts) -> anyhow::Result<()> {
    let mut cfg = Config::load();
    let settings = whistlestop::config::path();
    if o.set_password {
        let pw = ask_password()?;
        cfg.remote_password = pw;
        cfg.save().with_context(|| format!("can't save the password to {}", settings.display()))?;
        println!("Saved the remote-access password to {}", settings.display());
    }
    let password = match std::env::var(PASSWORD_ENV) {
        Ok(p) if !p.trim().is_empty() => p,
        _ => cfg.remote_password.clone(),
    };
    if password.trim().len() < 4 {
        bail!(
            "no remote-access password: this station can only be operated over the LAN, so it needs one of at least 4 characters.\n\
             Set one with `whistlestop serve --set-password`, or in the environment as {PASSWORD_ENV}."
        );
    }
    let port = o.port.unwrap_or(cfg.remote_port);

    let h = engine::spawn(|| {});
    let problems = std::mem::take(&mut cfg.load_problems);
    h.send(Cmd::Start(cfg.clone()));
    let devices = whistlestop::audio::list_devices();
    let shared_cfg = Arc::new(Mutex::new((0u64, cfg)));
    let shared = whistlestop::remote::Shared { handle: h.clone(), cfg: shared_cfg, devices };
    let server = match whistlestop::remote::serve(port, &password, shared) {
        Ok(s) => s,
        Err(e) => {
            stop_engine(&h);
            return Err(e.context("can't share this station on the LAN"));
        }
    };

    let mut log = SessionLog::beside(&settings);
    let addr = whistlestop::remote::lan_address().map(|a| a.to_string()).unwrap_or_else(|| "this computer's address".into());
    println!("Whistlestop {} running with no window.", env!("CARGO_PKG_VERSION"));
    println!("  Operate it from Whistlestop on another computer: Remote → {addr}:{}", server.port);
    println!("  Settings: {}", settings.display());
    println!("  Log file: {}", log.path().display());
    if !problems.is_empty() {
        println!("  {} setting(s) couldn't be read and use their defaults: see the Log", problems.len());
    }
    println!("  Ctrl-C (or SIGTERM) stops the station.");

    let stop = Arc::new(AtomicBool::new(false));
    watch_signals(stop.clone())?;
    let mut printed: Option<u64> = None;
    while !stop.load(Ordering::SeqCst) {
        if let Ok(g) = h.state.lock() {
            log.catch_up(g.log_seq, &g.log);
            if !o.quiet {
                print!("{}", new_lines(&mut printed, g.log_seq, &g.log));
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    println!("Stopping the station…");
    drop(server);
    stop_engine(&h);
    if let Ok(g) = h.state.lock() {
        log.catch_up(g.log_seq, &g.log);
        if !o.quiet {
            print!("{}", new_lines(&mut printed, g.log_seq, &g.log));
        }
    }
    Ok(())
}

/// The Log lines added since the last call, as text (the first call gives
/// what's there). Lines that came and went between calls are counted.
fn new_lines(seen: &mut Option<u64>, seq: u64, log: &std::collections::VecDeque<(engine::chrono_lite::Clock, String)>) -> String {
    let new = match *seen {
        Some(s) => seq.saturating_sub(s),
        None => log.len() as u64,
    };
    *seen = Some(seq);
    let mut text = String::new();
    if new > log.len() as u64 {
        text += &format!("({} Log lines came and went before they could be printed)\n", new - log.len() as u64);
    }
    text += &lines(log.iter().skip(log.len().saturating_sub(new as usize)));
    text
}

/// Stop the engine and wait (briefly) for it to unkey the radio, as the app
/// does when it quits.
fn stop_engine(h: &engine::Handle) {
    h.send(Cmd::Stop);
    let until = Instant::now() + Duration::from_secs(3);
    while h.state.lock().map(|g| g.running).unwrap_or(false) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ctrl-C, and on Unix SIGTERM (systemd's stop) and SIGHUP, set `stop`.
fn watch_signals(stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    std::thread::Builder::new().name("whistlestop-signals".into()).spawn(move || {
        rt.block_on(async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                };
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                    _ = hup.recv() => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        });
        stop.store(true, Ordering::SeqCst);
    })?;
    Ok(())
}

/// Ask for the password on the terminal, twice.
fn ask_password() -> anyhow::Result<String> {
    let read = |prompt: &str| -> anyhow::Result<String> {
        use std::io::Write;
        print!("{prompt}");
        std::io::stdout().flush()?;
        let mut s = String::new();
        if std::io::stdin().read_line(&mut s)? == 0 {
            bail!("no password given");
        }
        Ok(s.trim_end_matches(['\r', '\n']).to_string())
    };
    let a = read("Remote-access password (at least 4 characters; it shows as you type): ")?;
    if a.trim().len() < 4 {
        bail!("that password is shorter than 4 characters");
    }
    if read("Again: ")? != a {
        bail!("the two passwords don't match");
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::chrono_lite::Clock;
    use std::collections::VecDeque;

    #[test]
    fn new_lines_prints_each_line_once_and_counts_missed_ones() {
        let mut log: VecDeque<(Clock, String)> = VecDeque::new();
        log.push_back((Clock(0), "one".into()));
        let mut seen = None;
        assert!(new_lines(&mut seen, 1, &log).ends_with("one\n"));
        assert_eq!(new_lines(&mut seen, 1, &log), "");
        log.push_back((Clock(0), "two".into()));
        let t = new_lines(&mut seen, 2, &log);
        assert!(t.ends_with("two\n") && !t.contains("one"));
        // Five lines added, only the last two still held.
        log.clear();
        log.push_back((Clock(0), "six".into()));
        log.push_back((Clock(0), "seven".into()));
        let t = new_lines(&mut seen, 7, &log);
        assert!(t.starts_with("(3 Log lines came and went"), "{t}");
        assert!(t.contains("six") && t.contains("seven"));
    }
}
