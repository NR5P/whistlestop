//! `whistlestop serve`: the station with no window, operated over the LAN.
//! Runs the real binary (in Demo mode, the default settings) and operates
//! it with the remote client, as Whistlestop on another computer does.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn settings_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("whistlestop-headless-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn serve_without_a_password_says_how_to_set_one() {
    let d = settings_dir("nopw");
    let out = Command::new(env!("CARGO_BIN_EXE_whistlestop"))
        .args(["serve", "--port", &free_port().to_string()])
        .env("WHISTLESTOP_CONFIG", d.join("whistlestop.toml"))
        .env_remove("WHISTLESTOP_REMOTE_PASSWORD")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--set-password"), "{err}");
    // Nothing written: no settings file appears from a failed start.
    assert!(!d.join("whistlestop.toml").exists());
}

#[test]
fn serve_is_operated_remotely_and_stops_cleanly() {
    let d = settings_dir("run");
    let port = free_port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_whistlestop"))
        .args(["serve", "--port", &port.to_string()])
        .env("WHISTLESTOP_CONFIG", d.join("whistlestop.toml"))
        .env("WHISTLESTOP_REMOTE_PASSWORD", "bench-pass")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    let mut seen = Vec::new();
    let wait_for = |seen: &mut Vec<String>, what: &str, secs: u64| {
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            if let Ok(l) = rx.recv_timeout(Duration::from_millis(100)) {
                seen.push(l);
                if seen.last().unwrap().contains(what) {
                    return true;
                }
            }
        }
        false
    };
    assert!(wait_for(&mut seen, "running with no window", 20), "{seen:#?}");

    // A wrong password is refused; the right one operates the station.
    let repaint = std::sync::Arc::new(|| {}) as std::sync::Arc<dyn Fn() + Send + Sync>;
    let addr = format!("127.0.0.1:{port}");
    assert!(whistlestop::remote::connect(&addr, "wrong", d.clone(), String::new(), repaint.clone()).is_err());
    let c = whistlestop::remote::connect(&addr, "bench-pass", d.clone(), String::new(), repaint).expect("connect");
    assert!(c.cfg.demo, "default settings run Demo mode");
    let until = Instant::now() + Duration::from_secs(20);
    while !c.handle.state.lock().unwrap().running && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(c.handle.state.lock().unwrap().running, "the station runs");
    drop(c);
    // Its Log reaches standard output.
    // (Log lines start with their date: "2026-11-18 …".)
    let is_log = |l: &String| l.len() > 4 && l[..4].chars().all(|c| c.is_ascii_digit());
    let until = Instant::now() + Duration::from_secs(10);
    while !seen.iter().any(is_log) && Instant::now() < until {
        if let Ok(l) = rx.recv_timeout(Duration::from_millis(100)) {
            seen.push(l);
        }
    }
    assert!(seen.iter().any(is_log), "{seen:#?}");

    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
        let until = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            assert!(Instant::now() < until, "serve didn't stop on SIGTERM");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(status.success(), "{status}");
        while let Ok(l) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push(l);
        }
        assert!(seen.iter().any(|l| l.contains("Stopping the station")), "{seen:#?}");
        // The session's Log was written beside the settings.
        let log = std::fs::read_to_string(d.join("whistlestop.log")).unwrap_or_default();
        assert!(!log.is_empty(), "whistlestop.log is empty");
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&d);
}
