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
    /// Which whistle these scan settings were made for (2 = whistle v2:
    /// 1000–2000 Hz, 0.2 s chirps). Older files get the new defaults once.
    #[serde(default = "whistle_v1")]
    pub whistle_version: u32,
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
    /// Direct CAT settings (rig_mode Direct).
    pub cat: crate::cat::CatConfig,
    /// Where Depot mail is kept; empty = depot.json beside the settings.
    pub depot_file: String,
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

    /// QSY: let a linked station move the link to another frequency (both
    /// stations go home when the link ends). Off by default.
    pub accept_qsy: bool,
    /// Only within the band you're on.
    pub qsy_same_band: bool,
    /// Bands a QSY may go to (when not same-band-only).
    pub qsy_bands: Vec<String>,
    /// At most this far from where you were, kHz (0 = anywhere allowed).
    pub qsy_max_khz: f64,
    /// Only inside the configured digital segments.
    pub qsy_segments_only: bool,
    /// If SWR stays high after a QSY (and tuning, when auto-tune is on),
    /// end the link so both go home.
    pub qsy_swr_home: bool,

    /// File library: a folder linked stations may browse and download from.
    pub share_library: bool,
    pub library_dir: String,
    /// Only while you're away.
    pub library_only_away: bool,
    /// Largest file sent from the library, MB.
    pub library_max_mb: f64,

    /// The schedule: where to listen (and beacon) at different times of
    /// day, UTC.
    pub schedule_on: bool,
    /// Follow it only while you're away.
    pub schedule_only_away: bool,
    pub schedule: Vec<crate::schedule::Slot>,

    /// Remote operation: let another Whistlestop on the LAN operate this
    /// one (off by default; needs a password).
    pub remote_serve: bool,
    pub remote_port: u16,
    pub remote_password: String,
    /// The radio computer this one last operated ("192.168.1.20").
    pub remote_addr: String,
    /// What was wrong with the settings file when it was loaded (never
    /// saved): the engine logs these when it starts with them.
    #[serde(skip)]
    pub load_problems: Vec<String>,
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
            whistle_s: 10.0,
            whistle_version: 2,
            cq_cycles: 5,
            cq_listen_s: 20.0,
            dwell_s: 0.5,
            step_hz: 2000.0,
            auto_answer: false,
            whistle_watch: true,
            e2e: Policy::Off,
            segments: default_segments(),
            download_dir: dirs::download_dir().unwrap_or_else(std::env::temp_dir).join("Whistlestop"),
            sdr: SdrConfig::default(),
            rig_mode: crate::rigsetup::RigMode::External,
            managed_rig: crate::rigsetup::ManagedRig::default(),
            cat: crate::cat::CatConfig::default(),
            depot_file: String::new(),
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
            accept_qsy: false,
            qsy_same_band: true,
            qsy_bands: vec!["40m".into(), "30m".into(), "20m".into()],
            qsy_max_khz: 50.0,
            qsy_segments_only: true,
            qsy_swr_home: true,
            share_library: false,
            library_dir: String::new(),
            library_only_away: false,
            library_max_mb: 5.0,
            schedule_on: false,
            schedule_only_away: true,
            schedule: vec![],
            remote_serve: false,
            remote_port: crate::remote::DEFAULT_PORT,
            remote_password: String::new(),
            remote_addr: String::new(),
            load_problems: vec![],
        }
    }
}

pub fn path() -> PathBuf {
    // Tests (and portable setups) can keep settings elsewhere.
    if let Some(p) = std::env::var_os("WHISTLESTOP_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Whistlestop")
        .join("whistlestop.toml")
}

impl Config {
    /// The settings file, or defaults. Problems are logged to stderr; use
    /// `load_with_report` to show them.
    pub fn load() -> Self {
        Self::load_with_report().0
    }

    /// The settings file, plus a line for every problem with it.
    ///
    /// A missing file is not a problem (defaults). In a readable file every
    /// readable value is kept and only the bad ones fall back to their
    /// defaults (see `from_toml_lenient`). A file that isn't TOML (or
    /// UTF-8) at all is copied to `whistlestop.toml.bad` before anything can
    /// save over it, and defaults are used.
    pub fn load_with_report() -> (Self, Vec<String>) {
        let p = path();
        let (mut c, problems) = match std::fs::read(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), vec![]),
            Err(e) => (
                Config::default(),
                vec![format!("can't read {} ({e}); using the default settings", p.display())],
            ),
            Ok(bytes) => match String::from_utf8(bytes) {
                Err(_) => (Config::default(), vec![keep_bad_copy(&p, "isn't a text (UTF-8) file")]),
                Ok(s) => match s.parse::<toml::Table>() {
                    Err(e) => {
                        let why = format!("isn't valid TOML ({})", e.message());
                        (Config::default(), vec![keep_bad_copy(&p, &why)])
                    }
                    Ok(t) => salvage(t),
                },
            },
        };
        c.upgrade();
        for m in &problems {
            eprintln!("whistlestop settings: {m}");
        }
        c.load_problems = problems.clone();
        (c, problems)
    }

    /// Parse settings text like `toml::from_str`, but never all-or-nothing:
    /// every readable value is kept, each bad one falls back to its default
    /// and is reported ("e2e = \"Prefer\": unknown variant ..."). For arrays
    /// of tables (segments, schedule) the readable elements are kept; if
    /// none is, the default list is used. Text that isn't TOML at all gives
    /// the defaults and one problem. `upgrade` is not applied.
    pub fn from_toml_lenient(s: &str) -> (Self, Vec<String>) {
        match s.parse::<toml::Table>() {
            Ok(t) => salvage(t),
            Err(e) => (
                Config::default(),
                vec![format!("not valid TOML ({}); using the default settings", e.message())],
            ),
        }
    }

    /// Settings saved for the first whistle: move scan settings still at
    /// the old defaults to whistle v2's (ones you changed are kept).
    pub fn upgrade(&mut self) {
        if self.whistle_version < 2 {
            if self.dwell_s == 2.2 {
                self.dwell_s = 0.5;
            }
            if self.step_hz == 1800.0 {
                self.step_hz = 2000.0;
            }
            if self.whistle_s == 24.0 {
                self.whistle_s = 10.0;
            }
            self.whistle_version = 2;
        }
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

fn whistle_v1() -> u32 {
    1
}

// ------------------------------------------------- lenient settings loading

/// Copy an unreadable settings file to "<name>.bad" (so a later save can't
/// lose it) and say so.
fn keep_bad_copy(p: &std::path::Path, why: &str) -> String {
    let mut bad = p.as_os_str().to_owned();
    bad.push(".bad");
    let bad = PathBuf::from(bad);
    match std::fs::copy(p, &bad) {
        Ok(_) => format!(
            "{} {why}; kept a copy as {}, using the default settings",
            p.display(),
            bad.display()
        ),
        Err(e) => format!(
            "{} {why}; using the default settings (couldn't keep a copy: {e})",
            p.display()
        ),
    }
}

fn try_config(t: &toml::Table) -> Result<Config, String> {
    toml::Value::Table(t.clone())
        .try_into::<Config>()
        .map_err(|e| e.message().trim().to_string())
}

/// The table at `path` (made by `salvage_into`, so always a table).
fn table_at<'a>(root: &'a mut toml::Table, path: &[String]) -> &'a mut toml::Table {
    let mut t = root;
    for k in path {
        t = match t.get_mut(k) {
            Some(toml::Value::Table(sub)) => sub,
            _ => unreachable!("salvage path {path:?} is not a table"),
        };
    }
    t
}

/// A value as written, shortened for a message.
fn shown(v: &toml::Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 60 {
        format!("{}…", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

/// Settings from a parsed file, keeping every value that reads.
fn salvage(file: toml::Table) -> (Config, Vec<String>) {
    if let Ok(c) = try_config(&file) {
        return (c, vec![]);
    }
    let mut good = toml::Table::new();
    let mut problems = vec![];
    salvage_into(&mut good, &[], &file, &mut problems);
    match try_config(&good) {
        Ok(c) => (c, problems),
        Err(e) => {
            // Can't happen (every step above was checked), but never fail.
            problems.push(format!("settings unreadable ({e}); using the defaults"));
            (Config::default(), problems)
        }
    }
}

/// Copy the keys of `src` (the file's table at `path`) into `root`'s table
/// at `path` one at a time, keeping each only if the whole Config still
/// reads. Tables are salvaged key by key, arrays of tables element by
/// element.
fn salvage_into(root: &mut toml::Table, path: &[String], src: &toml::Table, problems: &mut Vec<String>) {
    use toml::Value;
    for (k, v) in src {
        let label = path
            .iter()
            .chain(std::iter::once(k))
            .cloned()
            .collect::<Vec<_>>()
            .join(".");
        table_at(root, path).insert(k.clone(), v.clone());
        let err = match try_config(root) {
            Ok(_) => continue,
            Err(e) => e,
        };
        match v {
            Value::Table(sub) => {
                table_at(root, path).insert(k.clone(), Value::Table(toml::Table::new()));
                if try_config(root).is_ok() {
                    let mut p = path.to_vec();
                    p.push(k.clone());
                    salvage_into(root, &p, sub, problems);
                    continue;
                }
            }
            Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_table) => {
                table_at(root, path).insert(k.clone(), Value::Array(vec![]));
                if try_config(root).is_ok() {
                    let mut kept = 0;
                    for (i, item) in items.iter().enumerate() {
                        // Push, test, pop if it doesn't read.
                        if let Some(Value::Array(a)) = table_at(root, path).get_mut(k) {
                            a.push(item.clone());
                        }
                        match try_config(root) {
                            Ok(_) => kept += 1,
                            Err(e) => {
                                if let Some(Value::Array(a)) = table_at(root, path).get_mut(k) {
                                    a.pop();
                                }
                                problems.push(format!("{label} entry {}: {e}; left out", i + 1));
                            }
                        }
                    }
                    if kept == 0 {
                        table_at(root, path).remove(k);
                        problems.push(format!("{label}: no entry could be read; using the default"));
                    }
                    continue;
                }
            }
            _ => {}
        }
        table_at(root, path).remove(k);
        problems.push(format!("{label} = {}: {err}; using the default", shown(v)));
    }
}

#[cfg(test)]
mod upgrade_tests {
    #[test]
    fn old_scan_defaults_move_to_whistle_v2() {
        let mut c: super::Config = toml::from_str(
            "callsign = \"N0OLD\"\ndwell_s = 2.2\nstep_hz = 1800.0\nwhistle_s = 30.0\n",
        )
        .unwrap();
        assert_eq!(c.whistle_version, 1);
        c.upgrade();
        assert_eq!(
            (c.dwell_s, c.step_hz, c.whistle_s, c.whistle_version),
            (0.5, 2000.0, 30.0, 2),
            "custom whistle length kept"
        );
    }
}
