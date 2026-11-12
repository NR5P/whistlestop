//! The two-station bench (`docs/BENCH.md`) with this app as one station.
//!
//! Only when the app's folder holds `bench.json` (the bench puts it there;
//! nothing else does):
//!
//! ```json
//! {"mercury_audio": "/path/a.sock" | "tcp:10.0.2.2:7401",
//!  "remote_port": 7373, "remote_password": "bench"}
//! ```
//!
//! * With `"mercury_audio": ""` (`airtest`, real sound between a computer
//!   and the phone) Mercury keeps the phone's own audio; the rest is the same.
//! * The embedded Mercury's audio is the bench's simulated band, through
//!   Mercury's framed-socket backend (`-x sock`). A `tcp:` address is
//!   reached through a socket in the app's folder, bridged over TCP (the
//!   Android emulator can't reach the computer's Unix sockets).
//! * The bench operates the station by remote operation (`remote.rs`), as a
//!   desktop operates another: the same engine, Mercury and app, driven
//!   from outside.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use whistlestop::config::Config;
use whistlestop::engine::Handle;

pub struct Bench {
    pub mercury_audio: String,
    pub remote_port: u16,
    pub remote_password: String,
}

pub fn load(dir: &Path) -> Option<Bench> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("bench.json")).ok()?).ok()?;
    Some(Bench {
        mercury_audio: v["mercury_audio"].as_str().unwrap_or_default().to_string(),
        remote_port: v["remote_port"].as_u64().unwrap_or(7373) as u16,
        remote_password: v["remote_password"].as_str().unwrap_or("bench").to_string(),
    })
}

/// Mercury's input, "sock:<path>" (see `mercury::start`).
pub fn mercury_audio(dir: &Path, b: &Bench) -> anyhow::Result<String> {
    let Some(addr) = b.mercury_audio.strip_prefix("tcp:") else {
        return Ok(format!("sock:{}", b.mercury_audio));
    };
    let path = dir.join("bench-audio.sock");
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path)?;
    let addr = addr.to_string();
    std::thread::Builder::new().name("bench-audio".into()).spawn(move || {
        for s in l.incoming().flatten() {
            let Ok(t) = TcpStream::connect(&addr) else {
                eprintln!("whistlestop: bench: can't reach {addr}");
                continue;
            };
            t.set_nodelay(true).ok();
            let ends = || {
                let (s, t) = (s.try_clone().expect("clone"), t.try_clone().expect("clone"));
                move || {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                    let _ = t.shutdown(std::net::Shutdown::Both);
                }
            };
            pipe(s.try_clone().expect("clone"), t.try_clone().expect("clone"), ends());
            pipe(t.try_clone().expect("clone"), s.try_clone().expect("clone"), ends());
        }
    })?;
    Ok(format!("sock:{}", path.display()))
}

/// Copy one way; when it ends, `ends` closes both sides (the bench dropped
/// this Mercury, or Mercury went), so Mercury sees it and reconnects.
fn pipe(mut from: impl Read + Send + 'static, mut to: impl Write + Send + 'static, ends: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 16384];
        while let Ok(n) = from.read(&mut buf) {
            if n == 0 || to.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        ends();
    });
}

pub fn serve(b: &Bench, h: &Handle, cfg: &Config) -> Option<whistlestop::remote::Server> {
    let shared = whistlestop::remote::Shared {
        handle: h.clone(),
        cfg: Arc::new(Mutex::new((0, cfg.clone()))),
        devices: Default::default(),
    };
    // Loopback only: the bench reaches it through `adb forward` (Android)
    // or the Simulator's shared network. A real phone on Wi-Fi must not
    // serve the bench's well-known password to its LAN.
    whistlestop::remote::serve_on(std::net::Ipv4Addr::LOCALHOST, b.remote_port, &b.remote_password, shared)
        .map_err(|e| eprintln!("whistlestop: bench: {e:#}"))
        .ok()
}
