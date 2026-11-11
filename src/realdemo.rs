//! Demo mode on real Mercury: when Mercury is installed, Demo starts two
//! real Mercury modems on the ether (a simulated HF band): one for you and
//! one for K7BOT, the demo station. What you see in the modem strip and hear
//! on the speakers is then real Mercury.

#[cfg(unix)]
use crate::demo_bot::{self, BotPorts};
#[cfg(unix)]
use crate::ether::{Channel, Ether, StationCfg};
use anyhow::Result;
#[cfg(unix)]
use anyhow::{Context, anyhow};
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::time::Duration;

/// Where the Mercury engine is: the configured path, the usual install
/// places for this system, or anywhere on PATH.
pub fn find_mercury(configured: &str) -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let exe = if cfg!(windows) {
        "mercury.exe"
    } else {
        "mercury"
    };
    let mut c: Vec<PathBuf> = vec![];
    if !configured.is_empty() {
        c.push(PathBuf::from(configured));
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(d) = std::env::var_os(var) {
                c.push(PathBuf::from(&d).join("Mercury").join(exe));
                c.push(PathBuf::from(&d).join("Mercury").join("bin").join(exe));
            }
        }
    } else {
        c.push(home.join(".local/bin/mercury"));
        c.push(PathBuf::from("/opt/homebrew/bin/mercury"));
        c.push(PathBuf::from("/usr/local/bin/mercury"));
        c.push(PathBuf::from("/usr/bin/mercury"));
        c.push(PathBuf::from(
            "/Applications/Mercury.app/Contents/MacOS/mercury",
        ));
    }
    c.push(
        home.join("Desktop")
            .join("mercury")
            .join("source")
            .join(exe),
    );
    c.into_iter()
        .find(|p| p.is_file())
        .or_else(|| crate::rigsetup::on_path("mercury"))
}

pub const YOUR_DIAL: f64 = 14_096_000.0;
pub const BOT_DIAL: f64 = 14_103_150.0;

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
/// Free ports for both Mercurys: two base ports (Mercury also uses
/// base + 1) and two broadcast ports, all six distinct.
fn pick_ports() -> Result<(u16, u16, u16, u16)> {
    let mut taken: Vec<u16> = vec![];
    let mut pick = |pair: bool| -> Result<u16> {
        for _ in 0..200 {
            let l = std::net::TcpListener::bind("127.0.0.1:0")?;
            let p = l.local_addr()?.port();
            let span: Vec<u16> = if pair {
                vec![p, p.saturating_add(1)]
            } else {
                vec![p]
            };
            if p < 65534
                && span.iter().all(|q| !taken.contains(q))
                && (!pair || std::net::TcpListener::bind(("127.0.0.1", p + 1)).is_ok())
            {
                taken.extend(span);
                return Ok(p);
            }
        }
        Err(anyhow!("no free ports"))
    };
    Ok((pick(true)?, pick(true)?, pick(false)?, pick(false)?))
}

#[cfg(unix)]
async fn wait_port(port: u16, secs: u64) -> bool {
    let t = std::time::Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

#[cfg(unix)]
pub async fn start(mercury: &std::path::Path) -> Result<RealDemo> {
    // Unix socket paths must stay under ~104 bytes; some temp folders are long.
    let base = std::env::temp_dir();
    let base = if base.as_os_str().len() > 60 {
        PathBuf::from("/tmp")
    } else {
        base
    };
    let dir = base.join(format!("whistlestop-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let (sa, sb) = (dir.join("you.sock"), dir.join("k7bot.sock"));
    let ether = Ether::start(
        vec![
            StationCfg {
                name: "you".into(),
                sock_path: sa.clone(),
                rig_port: 0,
                audio_port: 0,
                dial_hz: YOUR_DIAL,
                hear_own_tx: true,
            },
            StationCfg {
                name: demo_bot::CALL.into(),
                sock_path: sb.clone(),
                rig_port: 0,
                audio_port: 0,
                dial_hz: BOT_DIAL,
                hear_own_tx: false,
            },
        ],
        Channel {
            noise_dbfs: -42.0,
            path_loss_db: 0.0,
            fading: 0.15,
        },
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
    for (sock, port, bport, log) in [
        (&sa, pa, ba, "mercury-you.log"),
        (&sb, pb, bb, "mercury-k7bot.log"),
    ] {
        let out = std::fs::File::create(dir.join(log))?;
        demo.mercs.push(
            Command::new(mercury)
                .args([
                    "-x",
                    "sock",
                    "-p",
                    &port.to_string(),
                    "-b",
                    &bport.to_string(),
                ])
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
            return Err(anyhow!(
                "Mercury didn't open port {p} (logs in {})",
                dir.display()
            ));
        }
    }
    let ports = BotPorts {
        mercury: pb,
        bcast: bb,
        rig: demo.ether.ports[1].0,
        audio: demo.ether.ports[1].1,
    };
    tokio::spawn(async move {
        if let Err(e) = demo_bot::run(ports, stop).await {
            eprintln!("K7BOT: {e}");
        }
    });
    Ok(demo)
}

/// Mercury's simulated-audio link is a Unix socket, which Mercury doesn't
/// offer on Windows; Demo uses the simulated modem there.
#[cfg(not(unix))]
pub struct RealDemo {
    pub rig_addr: String,
    pub audio_addr: String,
    pub mercury_port: u16,
    pub bcast_port: u16,
}

#[cfg(not(unix))]
pub async fn start(_mercury: &std::path::Path) -> Result<RealDemo> {
    anyhow::bail!("Mercury's simulated-audio link isn't available on Windows")
}
