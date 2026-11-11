//! Radio control over the Hamlib `rigctld` protocol (TCP, default :4532).
//!
//! Works with Hamlib's own `rigctld` and with RigHelm's built-in rigctld
//! server. The rigctld owns the radio's CAT port; Mercury (Hamlib model 2,
//! NET rigctl) and Whistlestop both talk to it, so nobody fights over it.
//!
//! `Rig::Sim` is an in-memory radio for Demo mode and tests.

use anyhow::{Context, Result, anyhow, bail};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RigStatus {
    pub freq_hz: f64,
    pub mode: String,
    pub passband_hz: i64,
    pub ptt: bool,
    /// S-meter, dB relative to S9 (rigctld `STRENGTH`).
    pub strength_db: Option<i32>,
    /// 0.0..1.0
    pub rf_power: Option<f32>,
}

pub struct RigctldClient {
    r: BufReader<OwnedReadHalf>,
    w: OwnedWriteHalf,
}

impl RigctldClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        let s = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(addr))
            .await
            .map_err(|_| anyhow!("rigctld at {addr} did not answer"))?
            .with_context(|| format!("connecting to rigctld at {addr}"))?;
        s.set_nodelay(true).ok();
        let (r, w) = s.into_split();
        Ok(Self { r: BufReader::new(r), w })
    }

    async fn line(&mut self) -> Result<String> {
        let mut l = String::new();
        let n = tokio::time::timeout(Duration::from_secs(3), self.r.read_line(&mut l)).await??;
        if n == 0 {
            bail!("rigctld closed the connection");
        }
        Ok(l.trim().to_string())
    }

    /// A set command: one `RPRT n` line back.
    async fn set(&mut self, cmd: &str) -> Result<()> {
        self.w.write_all(format!("{cmd}\n").as_bytes()).await?;
        let l = self.line().await?;
        if l != "RPRT 0" {
            bail!("rigctld: {cmd:?} -> {l}");
        }
        Ok(())
    }

    /// A get command returning `n` lines (or an `RPRT` error).
    async fn get(&mut self, cmd: &str, n: usize) -> Result<Vec<String>> {
        self.w.write_all(format!("{cmd}\n").as_bytes()).await?;
        let mut out = vec![];
        for _ in 0..n {
            let l = self.line().await?;
            if l.starts_with("RPRT") {
                bail!("rigctld: {cmd:?} -> {l}");
            }
            out.push(l);
        }
        Ok(out)
    }
}

pub struct SimRig {
    pub status: RigStatus,
    /// Shared with the simulated band and fake Mercury, so they know where
    /// the "radio" is tuned.
    pub freq_link: std::sync::Arc<std::sync::Mutex<f64>>,
}

pub enum Rig {
    Rigctld(RigctldClient),
    Sim(SimRig),
}

impl Rig {
    pub fn sim(freq_hz: f64, freq_link: std::sync::Arc<std::sync::Mutex<f64>>) -> Self {
        *freq_link.lock().unwrap() = freq_hz;
        Rig::Sim(SimRig {
            freq_link,
            status: RigStatus {
                freq_hz,
                mode: "PKTUSB".into(),
                passband_hz: 3000,
                ptt: false,
                strength_db: Some(-30),
                rf_power: Some(0.5),
            },
        })
    }

    pub async fn set_freq(&mut self, hz: f64) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("F {}", hz.round() as i64)).await,
            Rig::Sim(s) => {
                s.status.freq_hz = hz.round();
                *s.freq_link.lock().unwrap() = s.status.freq_hz;
                Ok(())
            }
        }
    }

    pub async fn freq(&mut self) -> Result<f64> {
        match self {
            Rig::Rigctld(c) => Ok(c.get("f", 1).await?[0].parse()?),
            Rig::Sim(s) => Ok(s.status.freq_hz),
        }
    }

    /// Mode is a Hamlib name: USB, LSB, PKTUSB, CW, AM, FM...
    pub async fn set_mode(&mut self, mode: &str, passband_hz: i64) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("M {mode} {passband_hz}")).await,
            Rig::Sim(s) => {
                s.status.mode = mode.into();
                s.status.passband_hz = passband_hz;
                Ok(())
            }
        }
    }

    pub async fn ptt(&mut self, on: bool) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("T {}", on as u8)).await,
            Rig::Sim(s) => {
                s.status.ptt = on;
                Ok(())
            }
        }
    }

    pub async fn set_power(&mut self, frac: f32) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("L RFPOWER {:.2}", frac.clamp(0.0, 1.0))).await,
            Rig::Sim(s) => {
                s.status.rf_power = Some(frac.clamp(0.0, 1.0));
                Ok(())
            }
        }
    }

    /// Everything the GUI shows. Optional readings that a radio doesn't
    /// support come back as `None` instead of failing the whole poll.
    pub async fn poll(&mut self) -> Result<RigStatus> {
        match self {
            Rig::Rigctld(c) => {
                let freq_hz = c.get("f", 1).await?[0].parse()?;
                let m = c.get("m", 2).await?;
                let ptt = c.get("t", 1).await.map(|v| v[0] == "1").unwrap_or(false);
                let strength_db = c.get("l STRENGTH", 1).await.ok().and_then(|v| v[0].parse().ok());
                let rf_power = c.get("l RFPOWER", 1).await.ok().and_then(|v| v[0].parse().ok());
                Ok(RigStatus {
                    freq_hz,
                    mode: m[0].clone(),
                    passband_hz: m.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                    ptt,
                    strength_db,
                    rf_power,
                })
            }
            Rig::Sim(s) => Ok(s.status.clone()),
        }
    }

    pub fn is_sim(&self) -> bool {
        matches!(self, Rig::Sim(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    /// A tiny rigctld that answers the commands we send.
    async fn fake_rigctld() -> String {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut freq = 14_074_000i64;
            let mut buf = vec![0u8; 1024];
            let mut pending = String::new();
            loop {
                let n = s.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                pending.push_str(std::str::from_utf8(&buf[..n]).unwrap());
                while let Some(i) = pending.find('\n') {
                    let cmd: String = pending.drain(..=i).collect();
                    let cmd = cmd.trim();
                    let reply = if let Some(f) = cmd.strip_prefix("F ") {
                        freq = f.parse().unwrap();
                        "RPRT 0\n".to_string()
                    } else if cmd == "f" {
                        format!("{freq}\n")
                    } else if cmd == "m" {
                        "PKTUSB\n3000\n".into()
                    } else if cmd == "t" {
                        "0\n".into()
                    } else if cmd == "l STRENGTH" {
                        "-12\n".into()
                    } else if cmd.starts_with('T') || cmd.starts_with('M') {
                        "RPRT 0\n".into()
                    } else {
                        "RPRT -4\n".into()
                    };
                    s.write_all(reply.as_bytes()).await.unwrap();
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn talks_rigctld() {
        let addr = fake_rigctld().await;
        let mut rig = Rig::Rigctld(RigctldClient::connect(&addr).await.unwrap());
        rig.set_freq(14_101_234.0).await.unwrap();
        assert_eq!(rig.freq().await.unwrap(), 14_101_234.0);
        let st = rig.poll().await.unwrap();
        assert_eq!(st.mode, "PKTUSB");
        assert_eq!(st.strength_db, Some(-12));
        assert_eq!(st.rf_power, None); // unsupported -> None, not an error
        rig.ptt(true).await.unwrap();
    }
}
