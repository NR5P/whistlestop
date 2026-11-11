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
    pub levels: std::collections::HashMap<String, f32>,
    pub funcs: std::collections::HashMap<String, bool>,
    pub vfo: String,
    pub split: bool,
    pub rit: i32,
    pub xit: i32,
    /// The band the antenna tuner last matched (demo: the antenna is
    /// resonant on 20 m and needs the tuner elsewhere).
    pub tuned_band: Option<&'static str>,
    /// Shared with the simulated band and fake Mercury, so they know where
    /// the "radio" is tuned.
    pub freq_link: std::sync::Arc<std::sync::Mutex<f64>>,
}

impl SimRig {
    /// Demo antenna: resonant on 20 m (SWR 1.3); elsewhere 2.8 until the
    /// tuner has matched that band.
    fn swr(&self) -> f32 {
        if !self.status.ptt {
            return 1.0;
        }
        let band = crate::bandplan::band_of(self.status.freq_hz);
        let tuned = *self.funcs.get("TUNER").unwrap_or(&false) && self.tuned_band.is_some() && self.tuned_band == band;
        if tuned { 1.15 } else if band == Some("20m") { 1.3 } else { 2.8 }
    }
}

/// What a radio supports, from Hamlib's capability report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RigCaps {
    pub model: String,
    pub get_levels: Vec<String>,
    pub set_levels: Vec<String>,
    pub get_funcs: Vec<String>,
    pub set_funcs: Vec<String>,
    pub vfo_ops: Vec<String>,
    pub preamp_db: Vec<i32>,
    pub att_db: Vec<i32>,
    pub modes: Vec<String>,
}

impl RigCaps {
    pub fn parse(text: &str) -> RigCaps {
        let mut c = RigCaps::default();
        let words = |v: &str| -> Vec<String> {
            v.split_whitespace().map(|w| w.split('(').next().unwrap_or(w).to_string()).filter(|w| !w.is_empty() && w != "None").collect()
        };
        let db = |v: &str| -> Vec<i32> { v.split_whitespace().filter_map(|w| w.trim_end_matches("dB").parse().ok()).filter(|d| *d != 0).collect() };
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            match k.trim() {
                "Model name" => c.model = v.trim().to_string(),
                "Mfg name" if c.model.is_empty() => c.model = v.trim().to_string(),
                "Get level" => c.get_levels = words(v),
                "Set level" => c.set_levels = words(v),
                "Get functions" => c.get_funcs = words(v),
                "Set functions" => c.set_funcs = words(v),
                "VFO Ops" => c.vfo_ops = words(v),
                "Preamp" => c.preamp_db = db(v),
                "Attenuator" => c.att_db = db(v),
                "Mode list" => c.modes = words(v),
                _ => {}
            }
        }
        c
    }

    /// The demo radio's abilities.
    pub fn demo() -> RigCaps {
        let v = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        RigCaps {
            model: "Demo transceiver".into(),
            get_levels: v("AF RF SQL MICGAIN COMP NR AGC PREAMP ATT RFPOWER SWR ALC RFPOWER_METER STRENGTH"),
            set_levels: v("AF RF SQL MICGAIN COMP NR AGC PREAMP ATT RFPOWER"),
            get_funcs: v("NB NR ANF COMP VOX MON TUNER"),
            set_funcs: v("NB NR ANF COMP VOX MON TUNER"),
            vfo_ops: v("CPY XCHG TUNE UP DOWN BAND_UP BAND_DOWN"),
            preamp_db: vec![10, 20],
            att_db: vec![6, 12, 18],
            modes: v("USB LSB PKTUSB CW AM FM"),
        }
    }

    pub fn can_set_level(&self, l: &str) -> bool {
        self.set_levels.iter().any(|x| x == l)
    }
    pub fn can_get_level(&self, l: &str) -> bool {
        self.get_levels.iter().any(|x| x == l)
    }
    pub fn can_func(&self, f: &str) -> bool {
        self.set_funcs.iter().any(|x| x == f)
    }
    pub fn can_op(&self, op: &str) -> bool {
        self.vfo_ops.iter().any(|x| x == op)
    }
    pub fn has_tuner(&self) -> bool {
        self.can_func("TUNER") || self.can_op("TUNE")
    }
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
            levels: [("AF", 0.5), ("RF", 1.0), ("SQL", 0.0), ("MICGAIN", 0.5), ("COMP", 0.0), ("NR", 0.3), ("AGC", 5.0), ("PREAMP", 0.0), ("ATT", 0.0)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            funcs: [("NB", false), ("NR", false), ("ANF", false), ("COMP", false), ("VOX", false), ("MON", false), ("TUNER", true)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            vfo: "VFOA".into(),
            split: false,
            rit: 0,
            xit: 0,
            tuned_band: Some("20m"),
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

    /// What the radio can do, from rigctld's capability report.
    pub async fn caps(&mut self) -> Result<RigCaps> {
        match self {
            Rig::Rigctld(c) => {
                c.w.write_all(b"\\dump_caps\n").await?;
                let mut text = String::new();
                loop {
                    let l = c.line().await?;
                    if l.starts_with("RPRT") {
                        break;
                    }
                    text.push_str(&l);
                    text.push('\n');
                }
                Ok(RigCaps::parse(&text))
            }
            Rig::Sim(_) => Ok(RigCaps::demo()),
        }
    }

    pub async fn get_level(&mut self, name: &str) -> Result<f32> {
        match self {
            Rig::Rigctld(c) => Ok(c.get(&format!("l {name}"), 1).await?[0].parse()?),
            Rig::Sim(s) => Ok(match name {
                "SWR" => s.swr(),
                "ALC" => if s.status.ptt { 0.25 } else { 0.0 },
                "RFPOWER_METER" => if s.status.ptt { s.status.rf_power.unwrap_or(0.5) } else { 0.0 },
                "STRENGTH" => s.status.strength_db.unwrap_or(-30) as f32,
                _ => *s.levels.get(name).unwrap_or(&0.0),
            }),
        }
    }

    pub async fn set_level(&mut self, name: &str, v: f32) -> Result<()> {
        match self {
            Rig::Rigctld(c) => {
                let val = if matches!(name, "AGC" | "PREAMP" | "ATT" | "KEYSPD" | "NOTCHF" | "IF" | "CWPITCH") { format!("{}", v.round() as i64) } else { format!("{v:.3}") };
                c.set(&format!("L {name} {val}")).await
            }
            Rig::Sim(s) => {
                s.levels.insert(name.into(), v);
                if name == "RFPOWER" {
                    s.status.rf_power = Some(v);
                }
                Ok(())
            }
        }
    }

    pub async fn get_func(&mut self, name: &str) -> Result<bool> {
        match self {
            Rig::Rigctld(c) => Ok(c.get(&format!("u {name}"), 1).await?[0] == "1"),
            Rig::Sim(s) => Ok(*s.funcs.get(name).unwrap_or(&false)),
        }
    }

    pub async fn set_func(&mut self, name: &str, on: bool) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("U {name} {}", on as u8)).await,
            Rig::Sim(s) => {
                s.funcs.insert(name.into(), on);
                Ok(())
            }
        }
    }

    /// A VFO operation: TUNE (start the antenna tuner), CPY (A=B),
    /// XCHG (swap A/B), UP, DOWN, BAND_UP, BAND_DOWN...
    pub async fn vfo_op(&mut self, op: &str) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("G {op}")).await,
            Rig::Sim(s) => {
                match op {
                    "TUNE" => {
                        s.funcs.insert("TUNER".into(), true);
                        s.tuned_band = crate::bandplan::band_of(s.status.freq_hz);
                    }
                    "XCHG" => s.vfo = if s.vfo == "VFOA" { "VFOB".into() } else { "VFOA".into() },
                    _ => {}
                }
                Ok(())
            }
        }
    }

    pub async fn vfo(&mut self) -> Result<String> {
        match self {
            Rig::Rigctld(c) => Ok(c.get("v", 1).await?[0].clone()),
            Rig::Sim(s) => Ok(s.vfo.clone()),
        }
    }

    pub async fn set_vfo(&mut self, vfo: &str) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("V {vfo}")).await,
            Rig::Sim(s) => {
                s.vfo = vfo.into();
                Ok(())
            }
        }
    }

    pub async fn split(&mut self) -> Result<bool> {
        match self {
            Rig::Rigctld(c) => Ok(c.get("s", 2).await?[0] == "1"),
            Rig::Sim(s) => Ok(s.split),
        }
    }

    pub async fn set_split(&mut self, on: bool) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("S {} VFOB", on as u8)).await,
            Rig::Sim(s) => {
                s.split = on;
                Ok(())
            }
        }
    }

    /// RIT (`xit = false`) or XIT offset in Hz.
    pub async fn offset(&mut self, xit: bool) -> Result<i32> {
        match self {
            Rig::Rigctld(c) => Ok(c.get(if xit { "z" } else { "j" }, 1).await?[0].parse()?),
            Rig::Sim(s) => Ok(if xit { s.xit } else { s.rit }),
        }
    }

    pub async fn set_offset(&mut self, xit: bool, hz: i32) -> Result<()> {
        match self {
            Rig::Rigctld(c) => c.set(&format!("{} {hz}", if xit { "Z" } else { "J" })).await,
            Rig::Sim(s) => {
                if xit {
                    s.xit = hz
                } else {
                    s.rit = hz
                }
                Ok(())
            }
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

    /// Against Hamlib's real rigctld with its Dummy radio (skipped when
    /// Hamlib isn't installed).
    #[tokio::test]
    async fn full_controls_against_real_rigctld() {
        let Ok(p) = crate::rigsetup::start_dummy(45_871) else { return };
        let mut rig = None;
        for _ in 0..50 {
            if let Ok(c) = RigctldClient::connect(&p.addr).await {
                rig = Some(Rig::Rigctld(c));
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let mut rig = rig.expect("rigctld dummy");
        let caps = rig.caps().await.unwrap();
        assert!(caps.can_set_level("AF") && caps.can_func("TUNER") && caps.can_op("TUNE"), "{caps:?}");
        assert!(!caps.att_db.is_empty());
        rig.set_level("AF", 0.4).await.unwrap();
        assert!((rig.get_level("AF").await.unwrap() - 0.4).abs() < 0.01);
        rig.set_func("TUNER", true).await.unwrap();
        assert!(rig.get_func("TUNER").await.unwrap());
        rig.vfo_op("TUNE").await.unwrap();
        rig.set_offset(false, 120).await.unwrap();
        assert_eq!(rig.offset(false).await.unwrap(), 120);
        rig.set_split(true).await.unwrap();
        assert!(rig.split().await.unwrap());
        assert_eq!(rig.vfo().await.unwrap(), "VFOA");
        let _ = rig.get_level("SWR").await.unwrap();
    }

    #[tokio::test]
    async fn demo_antenna_needs_the_tuner_off_20m() {
        let mut rig = Rig::sim(7_100_000.0, Default::default());
        rig.ptt(true).await.unwrap();
        assert!(rig.get_level("SWR").await.unwrap() > 2.0);
        rig.vfo_op("TUNE").await.unwrap();
        assert!(rig.get_level("SWR").await.unwrap() < 1.5);
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
