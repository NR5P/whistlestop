//! Demo mode on real Mercury: when Mercury is installed, Demo starts two
//! real Mercury modems on the ether (a simulated HF band): one for you and
//! one for K7BOT, the demo station. What you see in the modem strip and hear
//! on the speakers is then real Mercury.

use crate::demo_bot::{self, BotPorts};
use crate::ether::{Channel, Ether, StationCfg};
use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Where the Mercury engine is: the configured path, or the usual places.
pub fn find_mercury(configured: &str) -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut c: Vec<PathBuf> = vec![];
    if !configured.is_empty() {
        c.push(PathBuf::from(configured));
    }
    c.push(home.join(".local/bin/mercury"));
    c.push(PathBuf::from("/opt/homebrew/bin/mercury"));
    c.push(PathBuf::from("/usr/local/bin/mercury"));
    c.push(PathBuf::from("/usr/bin/mercury"));
    c.push(home.join("Desktop/mercury/source/mercury"));
    c.into_iter().find(|p| p.is_file())
}

pub const YOUR_DIAL: f64 = 14_096_000.0;
pub const BOT_DIAL: f64 = 14_103_150.0;

pub struct RealDemo {
    pub ether: Ether,
    mercs: Vec<Child>,
    stop: Arc<AtomicBool>,
    pub rig_addr: String,
    pub audio_addr: String,
    pub mercury_port: u16,
    pub bcast_port: u16,
    pub dir: PathBuf,
}

impl Drop for RealDemo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for c in self.mercs.iter_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.ether.stop();
    }
}

/// Free ports for both Mercurys: two base ports (Mercury also uses
/// base + 1) and two broadcast ports, all six distinct.
fn pick_ports() -> Result<(u16, u16, u16, u16)> {
    let mut taken: Vec<u16> = vec![];
    let mut pick = |pair: bool| -> Result<u16> {
        for _ in 0..200 {
            let l = std::net::TcpListener::bind("127.0.0.1:0")?;
            let p = l.local_addr()?.port();
            let span: Vec<u16> = if pair { vec![p, p.saturating_add(1)] } else { vec![p] };
            if p < 65534 && span.iter().all(|q| !taken.contains(q)) && (!pair || std::net::TcpListener::bind(("127.0.0.1", p + 1)).is_ok()) {
                taken.extend(span);
                return Ok(p);
            }
        }
        Err(anyhow!("no free ports"))
    };
    Ok((pick(true)?, pick(true)?, pick(false)?, pick(false)?))
}

async fn wait_port(port: u16, secs: u64) -> bool {
    let t = std::time::Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

pub async fn start(mercury: &std::path::Path) -> Result<RealDemo> {
    let dir = std::env::temp_dir().join(format!("whistlestop-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let (sa, sb) = (dir.join("you.sock"), dir.join("k7bot.sock"));
    let ether = Ether::start(
        vec![
            StationCfg { name: "you".into(), sock_path: sa.clone(), rig_port: 0, audio_port: 0, dial_hz: YOUR_DIAL, hear_own_tx: true },
            StationCfg { name: demo_bot::CALL.into(), sock_path: sb.clone(), rig_port: 0, audio_port: 0, dial_hz: BOT_DIAL, hear_own_tx: false },
        ],
        Channel { noise_dbfs: -42.0, path_loss_db: 0.0, fading: 0.15 },
    )?;
    let (pa, pb, ba, bb) = pick_ports()?;
    let stop = Arc::new(AtomicBool::new(false));
    let mut demo = RealDemo {
        rig_addr: format!("127.0.0.1:{}", ether.ports[0].0),
        audio_addr: format!("127.0.0.1:{}", ether.ports[0].1),
        mercury_port: pa,
        bcast_port: ba,
        ether,
        mercs: vec![],
        stop: stop.clone(),
        dir: dir.clone(),
    };
    for (sock, port, bport, log) in [(&sa, pa, ba, "mercury-you.log"), (&sb, pb, bb, "mercury-k7bot.log")] {
        let out = std::fs::File::create(dir.join(log))?;
        demo.mercs.push(
            Command::new(mercury)
                .args(["-x", "sock", "-p", &port.to_string(), "-b", &bport.to_string()])
                .env("MERCURY_AUDIO_SOCK", sock)
                .current_dir(&dir)
                .stdout(Stdio::from(out.try_clone()?))
                .stderr(Stdio::from(out))
                .spawn()
                .with_context(|| format!("starting {}", mercury.display()))?,
        );
    }
    for p in [pa, pb, ba, bb] {
        if !wait_port(p, 20).await {
            return Err(anyhow!("Mercury didn't open port {p} (logs in {})", dir.display()));
        }
    }
    let ports = BotPorts { mercury: pb, bcast: bb, rig: demo.ether.ports[1].0, audio: demo.ether.ports[1].1 };
    tokio::spawn(async move {
        if let Err(e) = demo_bot::run(ports, stop).await {
            eprintln!("K7BOT: {e}");
        }
    });
    Ok(demo)
}
