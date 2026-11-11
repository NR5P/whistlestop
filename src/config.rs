//! Settings, saved as TOML in the platform config folder
//! (macOS: ~/Library/Application Support/Whistlestop/whistlestop.toml).

use crate::bandplan::{Segment, default_segments};
use crate::modes::Bandwidth;
use crate::sdr::SdrConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub callsign: String,
    pub name: String,
    pub grid: String,

    /// Demo mode: simulated radio, band and Mercury. No radio needed.
    pub demo: bool,

    pub mercury_host: String,
    pub mercury_port: u16,
    pub mercury_bcast_port: u16,
    /// Hamlib rigctld or RigHelm's rigctld server.
    pub rigctld: String,
    pub audio_in: String,
    pub audio_out: String,

    pub bandwidth: Bandwidth,
    /// Leave headroom (dB) when picking a Roundtable speed.
    pub ramp_margin_db: f64,
    pub auto_ramp: bool,

    /// Length of the CQ whistle, seconds. Should cover searchers' cycle.
    pub whistle_s: f64,
    pub cq_cycles: u32,
    pub cq_listen_s: f64,
    pub dwell_s: f64,
    pub step_hz: f64,
    /// Answer a CQ automatically when search locks onto one.
    pub auto_answer: bool,
    /// Report whistles heard while parked on a frequency.
    pub whistle_watch: bool,

    pub segments: Vec<Segment>,
    pub download_dir: PathBuf,
    pub sdr: SdrConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            callsign: "N0CALL".into(),
            name: String::new(),
            grid: String::new(),
            demo: true,
            mercury_host: "127.0.0.1".into(),
            mercury_port: 8300,
            mercury_bcast_port: 8100,
            rigctld: "127.0.0.1:4532".into(),
            audio_in: String::new(),
            audio_out: String::new(),
            bandwidth: Bandwidth::Hz2300,
            ramp_margin_db: 3.0,
            auto_ramp: true,
            whistle_s: 24.0,
            cq_cycles: 5,
            cq_listen_s: 20.0,
            dwell_s: 2.2,
            step_hz: 1800.0,
            auto_answer: false,
            whistle_watch: true,
            segments: default_segments(),
            download_dir: dirs::download_dir().unwrap_or_else(std::env::temp_dir).join("Whistlestop"),
            sdr: SdrConfig::default(),
        }
    }
}

pub fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("Whistlestop").join("whistlestop.toml")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrips_toml() {
        let c = super::Config::default();
        let s = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<super::Config>(&s).unwrap(), c);
    }
}
