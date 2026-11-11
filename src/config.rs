//! Settings, saved as TOML in the platform config folder
//! (macOS: ~/Library/Application Support/Whistlestop/whistlestop.toml).

use crate::bandplan::{Segment, default_segments};
use crate::e2e::Policy;
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

    /// End-to-end encryption of 1:1 links. Off by default: NOT legal on
    /// amateur bands (47 CFR 97.113(a)(4) in the US, and most countries).
    pub e2e: Policy,

    pub segments: Vec<Segment>,
    pub download_dir: PathBuf,
    pub sdr: SdrConfig,

    /// How to reach the radio: an existing rigctld (External) or one
    /// Whistlestop runs (Managed, with auto-detect).
    pub rig_mode: crate::rigsetup::RigMode,
    pub managed_rig: crate::rigsetup::ManagedRig,
    /// Ask the antenna tuner to tune when a transmission's SWR is above
    /// `swr_limit` (off by default; tuning transmits a carrier).
    pub auto_tune: bool,
    pub swr_limit: f32,
    /// Play the radio's audio through the computer's speakers (and your own
    /// CQ in Demo mode). Off by default.
    pub monitor: bool,
    pub monitor_volume: f32,
    /// Speaker device; empty = the system's default output.
    pub monitor_device: String,
    /// Demo mode on two real Mercury modems when Mercury is installed.
    pub demo_real_mercury: bool,
    /// Path to the Mercury engine; empty = look in the usual places.
    pub mercury_bin: String,

    // ---- Automation (all off by default) ----
    /// Shown to stations that connect while you're away.
    pub away_message: String,
    /// While away, hang up a link that has been quiet this long (s).
    pub away_idle_s: f64,
    /// Send a Station ID beacon every `beacon_minutes` (no whistle).
    pub beacon: bool,
    pub beacon_minutes: f64,
    /// Tell stations you hear that you have mail for them.
    pub notify_mail: bool,
    /// While away: connect to stations you hear that have mail waiting,
    /// and deliver it.
    pub auto_deliver: bool,
    /// While away: when a station says it has mail for you, connect and
    /// collect it.
    pub auto_collect: bool,
    /// Automatic transmissions only inside the automatic-control digital
    /// segments (US 47 CFR 97.221(b)).
    pub auto_in_segments_only: bool,
    /// Carry Depot mail for other stations (store and forward).
    pub relay: bool,
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
            e2e: Policy::Off,
            segments: default_segments(),
            download_dir: dirs::download_dir().unwrap_or_else(std::env::temp_dir).join("Whistlestop"),
            sdr: SdrConfig::default(),
            rig_mode: crate::rigsetup::RigMode::External,
            managed_rig: crate::rigsetup::ManagedRig::default(),
            auto_tune: false,
            swr_limit: 2.0,
            monitor: false,
            monitor_volume: 0.7,
            monitor_device: String::new(),
            demo_real_mercury: true,
            mercury_bin: String::new(),
            away_message: "Away from the radio right now. Type a message and it will wait for me in my Depot. 73".into(),
            away_idle_s: 180.0,
            beacon: false,
            beacon_minutes: 30.0,
            notify_mail: false,
            auto_deliver: false,
            auto_collect: false,
            auto_in_segments_only: true,
            relay: false,
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

    #[test]
    fn encryption_is_off_by_default() {
        assert_eq!(super::Config::default().e2e, crate::e2e::Policy::Off);
        // Old config files without the key load as Off too.
        let c: super::Config = toml::from_str("callsign = \"N0CALL\"").unwrap();
        assert_eq!(c.e2e, crate::e2e::Policy::Off);
    }
}
