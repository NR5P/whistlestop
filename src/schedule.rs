//! The station schedule: where to listen (and whether to beacon) at
//! different times of day (Settings → Schedule; off by default). Times are
//! UTC, as is usual on the air. A slot may cross midnight (22:00–02:00).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Days {
    #[default]
    Every,
    Weekdays,
    Weekends,
}

impl Days {
    pub const ALL: [Days; 3] = [Days::Every, Days::Weekdays, Days::Weekends];
    pub fn label(&self) -> &'static str {
        match self {
            Days::Every => "every day",
            Days::Weekdays => "Mon–Fri",
            Days::Weekends => "Sat–Sun",
        }
    }
    /// Does this include the UTC weekday `wd` (0 = Monday)?
    fn has(&self, wd: u64) -> bool {
        match self {
            Days::Every => true,
            Days::Weekdays => wd < 5,
            Days::Weekends => wd >= 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Slot {
    /// "HH:MM" UTC.
    pub from: String,
    pub to: String,
    pub days: Days,
    /// Dial frequency, kHz.
    pub khz: f64,
    /// Send the Station ID beacon while this slot is on.
    pub beacon: bool,
}

impl Default for Slot {
    fn default() -> Self {
        Slot { from: "13:00".into(), to: "17:00".into(), days: Days::Every, khz: 14_103.0, beacon: false }
    }
}

/// "HH:MM" → minutes after midnight.
pub fn minutes(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

impl Slot {
    pub fn valid(&self) -> bool {
        minutes(&self.from).is_some() && minutes(&self.to).is_some() && self.khz > 1_800.0 && self.khz < 30_000.0
    }

    /// Is the slot on at unix time `t` (seconds, UTC)?
    pub fn on_at(&self, t: u64) -> bool {
        let (Some(a), Some(b)) = (minutes(&self.from), minutes(&self.to)) else { return false };
        let day = t / 86_400;
        let now = ((t % 86_400) / 60) as u32;
        let wd = (day + 3) % 7; // 1970-01-01 was a Thursday (Mon = 0)
        if a == b {
            return self.days.has(wd); // all day
        }
        if a < b {
            self.days.has(wd) && now >= a && now < b
        } else {
            // Crosses midnight: the part before midnight belongs to today,
            // the part after to the day it started.
            (now >= a && self.days.has(wd)) || (now < b && self.days.has((wd + 6) % 7))
        }
    }
}

/// The slot in charge at `t`: the first listed that is on.
pub fn active(slots: &[Slot], t: u64) -> Option<(usize, &Slot)> {
    slots.iter().enumerate().find(|(_, s)| s.valid() && s.on_at(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-11 is a Wednesday.
    const WED: u64 = 1_794_355_200;

    fn at(day: u64, h: u64, m: u64) -> u64 {
        day + h * 3600 + m * 60
    }

    #[test]
    fn slots_follow_the_clock() {
        let day = Slot { from: "13:00".into(), to: "17:00".into(), khz: 14_103.0, ..Default::default() };
        assert!(!day.on_at(at(WED, 12, 59)));
        assert!(day.on_at(at(WED, 13, 0)) && day.on_at(at(WED, 16, 59)));
        assert!(!day.on_at(at(WED, 17, 0)));
        let night = Slot { from: "22:00".into(), to: "02:00".into(), khz: 7_103.0, days: Days::Weekdays, ..Default::default() };
        assert!(night.on_at(at(WED, 23, 0)) && night.on_at(at(WED + 86_400, 1, 0)));
        // Friday night runs into Saturday morning; Saturday night doesn't run.
        let fri = WED + 2 * 86_400;
        assert!(night.on_at(at(fri, 23, 0)) && night.on_at(at(fri + 86_400, 1, 30)));
        assert!(!night.on_at(at(fri + 86_400, 23, 0)));
        let both = [night.clone(), day.clone()];
        assert_eq!(active(&both, at(WED, 14, 0)).map(|a| a.0), Some(1));
        assert_eq!(active(&both, at(WED, 8, 0)).map(|a| a.0), None);
        assert!(minutes("24:00").is_none() && minutes("7:05") == Some(425));
    }
}
