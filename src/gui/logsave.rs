//! The Log, kept and sent back. Every line of this session is written to
//! `whistlestop.log` beside the settings as it comes (the session before is
//! kept as `whistlestop-previous.log`, so the log of a session that crashed
//! survives the restart). **Save log…** and **Copy log** on the Log tab give
//! a tester that, with the version, radio, Mercury and settings, to send.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use whistlestop::config::Config;
use whistlestop::engine::chrono_lite::Clock;

/// The version and the build (git commit) it was made from.
pub const VERSION: &str = concat!("Whistlestop ", env!("CARGO_PKG_VERSION"), " (", env!("WHISTLESTOP_COMMIT"), ")");

/// A session's file is started afresh (the full one kept as the previous
/// session's) past this size, so an unattended station can't fill the disk.
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;

pub struct SessionLog {
    path: PathBuf,
    file: Option<File>,
    written: u64,
    /// `State::log_seq` of the last line written.
    seen: Option<u64>,
}

impl SessionLog {
    /// `<settings file name>.log` beside the settings file (`whistlestop.log`).
    pub fn beside(settings: &Path) -> Self {
        let path = settings.with_extension("log");
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut s = Self { path, file: None, written: 0, seen: None };
        s.start();
        s
    }

    /// Keep the last session's file as the previous one and start a new one.
    fn start(&mut self) {
        self.file = None;
        let _ = std::fs::remove_file(self.previous_path());
        let _ = std::fs::rename(&self.path, self.previous_path());
        self.file = OpenOptions::new().create(true).write(true).truncate(true).open(&self.path).ok();
        self.written = 0;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn previous_path(&self) -> PathBuf {
        let stem = self.path.file_stem().unwrap_or_default().to_string_lossy();
        self.path.with_file_name(format!("{stem}-previous.log"))
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// Write the lines added since the last call. `seq` counts every line
    /// ever added (`State::log_seq`); `log` holds the newest of them.
    pub fn catch_up(&mut self, seq: u64, log: &VecDeque<(Clock, String)>) {
        let new = match self.seen {
            Some(s) => seq.saturating_sub(s),
            None => log.len() as u64,
        };
        self.seen = Some(seq);
        if new == 0 || self.file.is_none() {
            return;
        }
        let mut text = String::new();
        if new > log.len() as u64 {
            text += &format!("{}  ({} Log lines came and went before they could be written here)\n", stamp(Clock::now().0), new - log.len() as u64);
        }
        text += &lines(log.iter().skip(log.len().saturating_sub(new as usize)));
        let Some(f) = &mut self.file else { return };
        if f.write_all(text.as_bytes()).is_err() {
            self.file = None;
            return;
        }
        self.written += text.len() as u64;
        if self.written > MAX_BYTES {
            self.start();
        }
    }

    /// What this session's file holds.
    pub fn read(&self) -> Option<String> {
        std::fs::read_to_string(&self.path).ok().filter(|t| !t.is_empty())
    }
}

/// Log lines as text, one per line, each with its date and time (UTC).
pub fn lines<'a>(log: impl Iterator<Item = &'a (Clock, String)>) -> String {
    log.map(|(at, l)| format!("{}  {l}\n", stamp(at.0))).collect()
}

/// What Save log… writes and Copy log copies: who, what build, the radio and
/// Mercury, the settings (the remote-operation password left out), then the
/// Log itself.
pub fn report(cfg: &Config, rig: &str, mercury: Option<&str>, source: &str, log: &str) -> String {
    let mut c = cfg.clone();
    if !c.remote_password.is_empty() {
        c.remote_password = "(removed)".into();
    }
    let settings = toml::to_string_pretty(&c).unwrap_or_else(|e| format!("(couldn't write the settings out: {e})\n"));
    let rig = if rig.trim().is_empty() { "not connected" } else { rig.trim() };
    format!(
        "{VERSION} on {} {}\n\
         Saved {}\n\
         Station {}{}\n\
         Radio: {rig}\n\
         Mercury: {}\n\
         Log: {source}\n\
         Read this before sending it: it includes your settings (the remote-operation password is left out).\n\
         \n== Settings ==\n{settings}\n== Log (UTC) ==\n{log}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        stamp(Clock::now().0),
        if cfg.callsign.trim().is_empty() { "(no callsign set)" } else { cfg.callsign.trim() },
        if cfg.demo { ", Demo mode" } else { "" },
        mercury.unwrap_or("not connected"),
    )
}

/// `2026-11-16 14:03:22Z`
pub fn stamp(secs: u64) -> String {
    let (y, m, d) = civil((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

/// `20261116-1403Z`, for a file name.
pub fn file_stamp(secs: u64) -> String {
    let (y, m, d) = civil((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!("{y:04}{m:02}{d:02}-{:02}{:02}Z", s / 3600, s % 3600 / 60)
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00Z");
        assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(stamp(1_709_251_199), "2024-02-29 23:59:59Z");
        assert_eq!(stamp(1_735_689_599), "2024-12-31 23:59:59Z");
        assert_eq!(file_stamp(1_735_689_599), "20241231-2359Z");
    }

    #[test]
    fn session_file_keeps_every_line_and_the_last_session() {
        let dir = std::env::temp_dir().join(format!("ws-logsave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = dir.join("whistlestop.toml");
        let mut log: VecDeque<(Clock, String)> = VecDeque::new();
        let mut seq = 0;
        let mut add = |log: &mut VecDeque<(Clock, String)>, l: &str| {
            log.push_back((Clock(951_782_400), l.into()));
            seq += 1;
            while log.len() > 3 {
                log.pop_front();
            }
            seq
        };
        let mut s = SessionLog::beside(&settings);
        assert!(s.is_open());
        assert_eq!(s.path(), dir.join("whistlestop.log"));
        let mut n = add(&mut log, "one");
        n = add(&mut log, "two").max(n);
        s.catch_up(n, &log);
        n = add(&mut log, "three");
        s.catch_up(n, &log);
        s.catch_up(n, &log); // nothing new: nothing written twice
        // More lines than the Log keeps arrive between two looks.
        for l in ["four", "five", "six", "seven", "eight"] {
            n = add(&mut log, l);
        }
        s.catch_up(n, &log);
        let t = s.read().unwrap();
        assert!(t.starts_with("2000-02-29 00:00:00Z  one\n2000-02-29 00:00:00Z  two\n2000-02-29 00:00:00Z  three\n"), "{t}");
        assert!(t.contains("(2 Log lines came and went"), "{t}");
        assert!(t.ends_with("six\n2000-02-29 00:00:00Z  seven\n2000-02-29 00:00:00Z  eight\n"), "{t}");
        assert_eq!(t.matches("three").count(), 1);
        // The next session keeps this one as the previous.
        drop(s);
        let s2 = SessionLog::beside(&settings);
        assert_eq!(std::fs::read_to_string(s2.previous_path()).unwrap(), t);
        assert_eq!(s2.previous_path(), dir.join("whistlestop-previous.log"));
        assert!(s2.read().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_leaves_the_password_out() {
        let cfg = Config { callsign: "NR5P".into(), remote_password: "hunter22".into(), demo: false, ..Default::default() };
        let r = report(&cfg, "", Some("1.9.23"), "test", "2000-02-29 00:00:00Z  hello\n");
        assert!(r.starts_with(VERSION), "{r}");
        assert!(r.contains("Station NR5P\n"), "{r}");
        assert!(r.contains("Radio: not connected"), "{r}");
        assert!(r.contains("Mercury: 1.9.23"), "{r}");
        assert!(!r.contains("hunter22"), "{r}");
        assert!(r.contains("remote_password = \"(removed)\""), "{r}");
        assert!(r.ends_with("== Log (UTC) ==\n2000-02-29 00:00:00Z  hello\n"), "{r}");
    }
}
