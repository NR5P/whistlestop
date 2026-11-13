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
    /// Listen before unattended transmissions (CQ, Station ID, automatic
    /// calls): Mercury's busy-channel detector, on while not linked. Turn
    /// off if the channel always reads busy (narrow receive filter).
    pub busy_detect: bool,

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
    /// While away: accept offered files by yourself (no one is there to
    /// press Accept), up to `auto_accept_max_mb` each.
    pub auto_accept_away: bool,
    pub auto_accept_max_mb: f64,
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
            busy_detect: true,
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
            auto_accept_away: false,
            auto_accept_max_mb: 5.0,
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
        let mut problems = problems;
        problems.extend(c.sanitize());
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

    /// `sanitize`, by value: the settings made safe, and a line for every
    /// value that had to change.
    pub fn sanitized(mut self) -> (Config, Vec<String>) {
        let notes = self.sanitize();
        (self, notes)
    }

    /// Make every value safe for the engine, whatever wrote it (a
    /// hand-edited file, an old version, a remote client's Update): each
    /// number is brought into a sane range (a NaN or infinity becomes the
    /// default), and the callsign is trimmed, uppercased and stripped of
    /// characters a callsign can't have. Returns one line per change
    /// ("cq_listen_s = -1 is below 0; using 0"); nothing for good values.
    ///
    /// The ranges are wider than the GUI's sliders (tests and the bench use
    /// short timings); they only keep out what would panic, hang or
    /// exhaust memory. Schedule slots outside 1800–30000 kHz or with bad
    /// times are left as they are (the schedule ignores them) and noted.
    pub fn sanitize(&mut self) -> Vec<String> {
        let d = Config::default();
        let mut n = vec![];

        // Callsign: what Mercury and the CQFRAME (the station's
        // identification) get. Letters, digits and '/' (portable) or '-'.
        let up = self.callsign.trim().to_uppercase();
        let clean: String = up
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '-')
            .collect();
        if clean != up {
            n.push(format!(
                "callsign {:?} has characters a callsign can't have; using {clean:?}",
                self.callsign
            ));
        }
        if clean.is_empty() {
            n.push("callsign is empty: set yours in Settings before transmitting".into());
        }
        self.callsign = clean;

        num(&mut n, "ramp_margin_db", &mut self.ramp_margin_db, 0.0, 20.0, d.ramp_margin_db);
        num(&mut n, "whistle_s", &mut self.whistle_s, 0.0, 120.0, d.whistle_s);
        int(&mut n, "cq_cycles", &mut self.cq_cycles, 0, 100, d.cq_cycles);
        num(&mut n, "cq_listen_s", &mut self.cq_listen_s, 0.0, 600.0, d.cq_listen_s);
        num(&mut n, "dwell_s", &mut self.dwell_s, 0.05, 10.0, d.dwell_s);
        num(&mut n, "step_hz", &mut self.step_hz, 100.0, 10_000.0, d.step_hz);
        num32(&mut n, "swr_limit", &mut self.swr_limit, 1.1, 10.0, d.swr_limit);
        num32(&mut n, "monitor_volume", &mut self.monitor_volume, 0.0, 1.5, d.monitor_volume);
        num(&mut n, "away_idle_s", &mut self.away_idle_s, 0.5, 86_400.0, d.away_idle_s);
        num(&mut n, "auto_accept_max_mb", &mut self.auto_accept_max_mb, 0.0, 4096.0, d.auto_accept_max_mb);
        num(&mut n, "beacon_minutes", &mut self.beacon_minutes, 10.0, 120.0, d.beacon_minutes);
        // Negative would mean "anywhere" (0): a typo mustn't widen it.
        if self.qsy_max_khz < 0.0 {
            n.push(format!(
                "qsy_max_khz = {} is negative; using the default {}",
                self.qsy_max_khz, d.qsy_max_khz
            ));
            self.qsy_max_khz = d.qsy_max_khz;
        }
        num(&mut n, "qsy_max_khz", &mut self.qsy_max_khz, 0.0, 30_000.0, d.qsy_max_khz);
        num(&mut n, "library_max_mb", &mut self.library_max_mb, 0.0, 4096.0, d.library_max_mb);

        // Ports: Mercury's data port is the control port + 1.
        int(&mut n, "mercury_port", &mut self.mercury_port, 1, 65534, d.mercury_port);
        int(&mut n, "mercury_bcast_port", &mut self.mercury_bcast_port, 1, 65535, d.mercury_bcast_port);
        port(&mut n, "remote_port", &mut self.remote_port, d.remote_port);
        port(&mut n, "managed_rig.listen_port", &mut self.managed_rig.listen_port, d.managed_rig.listen_port);
        int(&mut n, "managed_rig.baud", &mut self.managed_rig.baud, 0, 4_000_000, d.managed_rig.baud);
        // 0 = don't share.
        if self.cat.share_port != 0 {
            port(&mut n, "cat.share_port", &mut self.cat.share_port, d.cat.share_port);
        }
        int(&mut n, "cat.baud", &mut self.cat.baud, 0, 4_000_000, d.cat.baud);
        num32(&mut n, "cat.max_power_w", &mut self.cat.max_power_w, 1.0, 2000.0, d.cat.max_power_w);

        int(&mut n, "sdr.if_hz", &mut self.sdr.if_hz, 100_000, 500_000_000, d.sdr.if_hz);
        int(&mut n, "sdr.ppm", &mut self.sdr.ppm, -200, 200, d.sdr.ppm);
        num(&mut n, "sdr.span_hz", &mut self.sdr.span_hz, 5_000.0, 1_600_000.0, d.sdr.span_hz);

        // Segments: a bad edge leaves the segment out; edges the wrong way
        // round are swapped.
        let had = self.segments.len();
        let mut kept = Vec::with_capacity(had);
        for (i, mut s) in std::mem::take(&mut self.segments).into_iter().enumerate() {
            let ok = |f: f64| f.is_finite() && f > 0.0 && f <= 3e9;
            if !ok(s.lo_hz) || !ok(s.hi_hz) {
                n.push(format!(
                    "segments entry {} ({} {}–{} Hz) has a bad edge; left out",
                    i + 1,
                    s.band,
                    s.lo_hz,
                    s.hi_hz
                ));
                continue;
            }
            if s.lo_hz > s.hi_hz {
                n.push(format!(
                    "segments entry {} ({}): lo_hz is above hi_hz; swapped",
                    i + 1,
                    s.band
                ));
                std::mem::swap(&mut s.lo_hz, &mut s.hi_hz);
            }
            kept.push(s);
        }
        if had > 0 && kept.is_empty() {
            n.push("segments: none usable; using the default segments".into());
            kept = d.segments.clone();
        }
        self.segments = kept;

        for (i, sl) in self.schedule.iter_mut().enumerate() {
            if !sl.khz.is_finite() {
                n.push(format!("schedule entry {}: khz = {} is not a number; slot off", i + 1, sl.khz));
                sl.khz = 0.0;
            } else if !sl.valid() {
                n.push(format!(
                    "schedule entry {} ({}–{} UTC, {} kHz) isn't valid (times HH:MM, 1800–30000 kHz); it is ignored",
                    i + 1,
                    sl.from,
                    sl.to,
                    sl.khz
                ));
            }
        }
        n
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &p)?;
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

// ------------------------------------------------------ sanitizing helpers

/// Keep `v` in `lo..=hi`; a NaN or infinity becomes `def`.
fn num(notes: &mut Vec<String>, name: &str, v: &mut f64, lo: f64, hi: f64, def: f64) {
    if !v.is_finite() {
        notes.push(format!("{name} = {v} is not a usable number; using the default {def}"));
        *v = def;
    } else if *v < lo {
        notes.push(format!("{name} = {v} is below {lo}; using {lo}"));
        *v = lo;
    } else if *v > hi {
        notes.push(format!("{name} = {v} is above {hi}; using {hi}"));
        *v = hi;
    }
}

fn num32(notes: &mut Vec<String>, name: &str, v: &mut f32, lo: f32, hi: f32, def: f32) {
    let mut w = *v as f64;
    num(notes, name, &mut w, lo as f64, hi as f64, def as f64);
    *v = w as f32;
}

/// Keep an integer in `lo..=hi`; 0 (below a non-zero `lo`) means unset
/// and becomes `def`.
fn int<T>(notes: &mut Vec<String>, name: &str, v: &mut T, lo: T, hi: T, def: T)
where
    T: PartialOrd + Copy + std::fmt::Display + Default,
{
    if *v < lo {
        let to = if *v == T::default() { def } else { lo };
        notes.push(format!("{name} = {v} is below {lo}; using {to}"));
        *v = to;
    } else if *v > hi {
        notes.push(format!("{name} = {v} is above {hi}; using {hi}"));
        *v = hi;
    }
}

/// A TCP port to listen on: 1024–65535 (lower ones need root), else the
/// default.
fn port(notes: &mut Vec<String>, name: &str, v: &mut u16, def: u16) {
    if *v < 1024 {
        notes.push(format!("{name} = {v} is not a usable port (1024–65535); using the default {def}"));
        *v = def;
    }
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
mod sanitize_tests {
    use super::Config;

    #[test]
    fn defaults_are_untouched() {
        let (c, notes) = Config::default().sanitized();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn non_finite_numbers_get_defaults() {
        let d = Config::default();
        let mut c = Config::default();
        c.cq_listen_s = f64::NAN;
        c.beacon_minutes = f64::INFINITY;
        c.whistle_s = f64::NEG_INFINITY;
        c.dwell_s = f64::NAN;
        c.step_hz = f64::NAN;
        c.away_idle_s = f64::INFINITY;
        c.ramp_margin_db = f64::NAN;
        c.qsy_max_khz = f64::NAN;
        c.library_max_mb = f64::INFINITY;
        c.swr_limit = f32::NAN;
        c.monitor_volume = f32::INFINITY;
        c.cat.max_power_w = f32::NAN;
        c.sdr.span_hz = f64::NAN;
        let (c, notes) = c.sanitized();
        assert_eq!(notes.len(), 13, "{notes:?}");
        assert_eq!(
            (c.cq_listen_s, c.beacon_minutes, c.whistle_s, c.dwell_s, c.step_hz, c.away_idle_s),
            (d.cq_listen_s, d.beacon_minutes, d.whistle_s, d.dwell_s, d.step_hz, d.away_idle_s)
        );
        assert_eq!((c.ramp_margin_db, c.qsy_max_khz, c.library_max_mb), (d.ramp_margin_db, d.qsy_max_khz, d.library_max_mb));
        assert_eq!((c.swr_limit, c.monitor_volume, c.cat.max_power_w), (d.swr_limit, d.monitor_volume, d.cat.max_power_w));
        assert_eq!(c.sdr.span_hz, d.sdr.span_hz);
    }

    #[test]
    fn out_of_range_numbers_are_clamped() {
        let mut c = Config::default();
        c.cq_listen_s = -1.0;
        c.whistle_s = 1e6;
        c.dwell_s = -5.0;
        c.step_hz = 0.0;
        c.away_idle_s = -1.0;
        c.beacon_minutes = 0.0;
        c.cq_cycles = 1_000_000;
        c.qsy_max_khz = -3.0;
        c.library_max_mb = -1.0;
        c.monitor_volume = 9.0;
        c.sdr.ppm = -100_000;
        c.sdr.if_hz = 0;
        let (c, notes) = c.sanitized();
        assert_eq!(notes.len(), 12, "{notes:?}");
        assert_eq!(c.cq_listen_s, 0.0);
        assert_eq!(c.whistle_s, 120.0);
        assert_eq!(c.dwell_s, 0.05);
        assert_eq!(c.step_hz, 100.0);
        assert_eq!(c.away_idle_s, 0.5);
        assert_eq!(c.beacon_minutes, 10.0);
        assert_eq!(c.cq_cycles, 100);
        assert_eq!(c.qsy_max_khz, 50.0, "negative doesn't become 'anywhere'");
        assert_eq!(c.library_max_mb, 0.0);
        assert_eq!(c.monitor_volume, 1.5);
        assert_eq!(c.sdr.ppm, -200);
        assert_eq!(c.sdr.if_hz, Config::default().sdr.if_hz, "0 = unset");
        assert!(notes.iter().any(|m| m.starts_with("cq_listen_s = -1 ")), "{notes:?}");
        // Every value is usable as a Duration now.
        let _ = std::time::Duration::from_secs_f64(c.cq_listen_s);
        let _ = std::time::Duration::from_secs_f64(c.whistle_s);
        let _ = std::time::Duration::from_secs_f64(c.beacon_minutes * 60.0);
        let _ = std::time::Duration::from_secs_f64(c.away_idle_s);
        let _ = std::time::Duration::from_secs_f64(c.dwell_s);
    }

    #[test]
    fn short_test_timings_are_kept() {
        let mut c = Config::default();
        c.whistle_s = 0.0;
        c.cq_listen_s = 0.0;
        c.cq_cycles = 0;
        c.away_idle_s = 0.5;
        c.qsy_max_khz = 0.0;
        c.cat.share_port = 0;
        let (s, notes) = c.clone().sanitized();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(s, c);
    }

    #[test]
    fn ports() {
        let mut c = Config::default();
        c.mercury_port = 0;
        c.mercury_bcast_port = 0;
        c.remote_port = 80;
        c.managed_rig.listen_port = 0;
        c.cat.share_port = 22;
        let d = Config::default();
        let (c, notes) = c.sanitized();
        assert_eq!(notes.len(), 5, "{notes:?}");
        assert_eq!(
            (c.mercury_port, c.mercury_bcast_port, c.remote_port, c.managed_rig.listen_port, c.cat.share_port),
            (d.mercury_port, d.mercury_bcast_port, d.remote_port, d.managed_rig.listen_port, d.cat.share_port)
        );
        let mut c = Config::default();
        c.mercury_port = 65535; // its data port would be 65536
        assert_eq!(c.sanitized().0.mercury_port, 65534);
    }

    #[test]
    fn callsign_is_normalised() {
        let c = Config { callsign: "  n0abc/p \t".into(), ..Default::default() };
        let (c, notes) = c.sanitized();
        assert_eq!(c.callsign, "N0ABC/P");
        assert!(notes.is_empty(), "trim and case are silent: {notes:?}");

        let c = Config { callsign: "N0 SP".into(), ..Default::default() };
        let (c, notes) = c.sanitized();
        assert_eq!(c.callsign, "N0SP");
        assert!(notes.len() == 1 && notes[0].contains("callsign"), "{notes:?}");

        let c = Config { callsign: "   ".into(), ..Default::default() };
        let (c, notes) = c.sanitized();
        assert_eq!(c.callsign, "");
        assert!(notes.iter().any(|m| m.contains("callsign is empty")), "{notes:?}");
    }

    #[test]
    fn segments_and_schedule() {
        use crate::bandplan::Segment;
        let mut c = Config::default();
        let good = c.segments.clone();
        c.segments.push(Segment { band: "x".into(), lo_hz: f64::NAN, hi_hz: 1.0 });
        c.segments.push(Segment { band: "y".into(), lo_hz: 14_110_000.0, hi_hz: 14_100_000.0 });
        c.schedule.push(crate::schedule::Slot { khz: f64::NAN, ..Default::default() });
        c.schedule.push(crate::schedule::Slot { from: "25:00".into(), ..Default::default() });
        c.schedule.push(crate::schedule::Slot::default());
        let (c, notes) = c.sanitized();
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert_eq!(c.segments.len(), good.len() + 1);
        let y = c.segments.last().unwrap();
        assert_eq!((y.lo_hz, y.hi_hz), (14_100_000.0, 14_110_000.0));
        assert_eq!(c.schedule.len(), 3, "slots are kept (the schedule skips bad ones)");
        assert!(!c.schedule[0].valid() && c.schedule[0].khz.is_finite());
        assert!(c.schedule[2].valid());

        // Nothing usable: the default segments.
        let mut c = Config::default();
        c.segments = vec![Segment { band: "x".into(), lo_hz: -1.0, hi_hz: f64::INFINITY }];
        let (c, _) = c.sanitized();
        assert_eq!(c.segments, good);
        // Deliberately empty stays empty.
        let mut c = Config::default();
        c.segments.clear();
        let (c, notes) = c.sanitized();
        assert!(c.segments.is_empty() && notes.is_empty());
    }

    #[test]
    fn sanitizing_twice_changes_nothing_more() {
        let mut c = Config::default();
        c.callsign = " w1 aw ".into();
        c.whistle_s = -3.0;
        c.beacon_minutes = f64::INFINITY;
        c.sdr.span_hz = 1e12;
        let (c, first) = c.sanitized();
        assert!(!first.is_empty());
        let (c2, second) = c.clone().sanitized();
        assert!(second.is_empty(), "{second:?}");
        assert_eq!(c, c2);
    }

    #[test]
    fn busy_detect_defaults_on_for_old_files() {
        assert!(Config::default().busy_detect);
        let c: Config = toml::from_str("callsign = \"N0OLD\"").unwrap();
        assert!(c.busy_detect);
        let c: Config = toml::from_str("busy_detect = false").unwrap();
        assert!(!c.busy_detect);
    }

    #[test]
    fn a_hand_edited_file_is_reported() {
        let (c, _) = Config::from_toml_lenient("callsign = \"n0x\"\ncq_listen_s = -1.0\nbeacon_minutes = inf\nwhistle_s = nan\n");
        let (c, notes) = c.sanitized();
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert_eq!(c.callsign, "N0X");
        assert_eq!((c.cq_listen_s, c.beacon_minutes, c.whistle_s), (0.0, 30.0, 10.0));
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
