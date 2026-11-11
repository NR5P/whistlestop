//! The Depot: Whistlestop's mailbox for messages that wait for a station.
//!
//! * **Arrivals**: mail for you (left while you were away, or delivered by
//!   another station).
//! * **Departures**: mail you've written, waiting for its station.
//! * **In Transit**: mail you're carrying for other stations (relaying),
//!   forwarded when you next link with (or hear) its station.
//! * **Delivered**: departures and relayed mail the next station confirmed.
//!
//! Saved as JSON beside the settings, so mail survives restarts.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mail {
    /// Random 64-bit id, the same at both stations.
    pub id: u64,
    pub from: String,
    pub to: String,
    /// Unix seconds when written.
    pub created: u64,
    pub text: String,
    /// Arrivals: has the operator opened it?
    #[serde(default)]
    pub read: bool,
    /// Delivered: when the other station confirmed (unix seconds).
    #[serde(default)]
    pub delivered_at: Option<u64>,
    /// Hand this to a relay station rather than waiting for `to`.
    #[serde(default)]
    pub via: Option<String>,
    /// How many relays have carried it (a loop guard: at most MAX_HOPS).
    #[serde(default)]
    pub hops: u8,
    /// Who handed it to us (In Transit), or who took it (Delivered).
    #[serde(default)]
    pub handed: Option<String>,
}

pub const MAX_HOPS: u8 = 3;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Depot {
    pub arrivals: Vec<Mail>,
    pub departures: Vec<Mail>,
    #[serde(default)]
    pub transit: Vec<Mail>,
    pub delivered: Vec<Mail>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

pub fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn new_id() -> u64 {
    use std::hash::BuildHasher;
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::collections::hash_map::RandomState::new().hash_one(t) | 1
}

pub fn default_path() -> PathBuf {
    crate::config::path().with_file_name("depot.json")
}

impl Depot {
    pub fn load(path: PathBuf) -> Depot {
        let mut d: Depot = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        d.path = Some(path);
        d
    }

    pub fn save(&self) {
        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default()).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }

    /// Write a message to `to`; it waits in Departures. With `via`, it's
    /// handed to that relay station instead.
    pub fn post(&mut self, from: &str, to: &str, text: &str) -> &Mail {
        self.post_via(from, to, None, text)
    }

    pub fn post_via(&mut self, from: &str, to: &str, via: Option<&str>, text: &str) -> &Mail {
        let via = via
            .map(|v| v.trim().to_uppercase())
            .filter(|v| !v.is_empty());
        self.departures.push(Mail {
            id: new_id(),
            from: from.to_uppercase(),
            to: to.trim().to_uppercase(),
            created: now_s(),
            text: text.to_string(),
            read: true,
            delivered_at: None,
            via,
            hops: 0,
            handed: None,
        });
        self.save();
        self.departures.last().unwrap()
    }

    /// Everything we'd hand to `call` when linked: our own mail for them or
    /// routed via them, and relayed mail for them.
    pub fn waiting_for(&self, call: &str) -> Vec<&Mail> {
        let c = |x: &str| x.eq_ignore_ascii_case(call);
        self.departures
            .iter()
            .filter(|m| c(&m.to) || m.via.as_deref().is_some_and(c))
            .chain(self.transit.iter().filter(|m| c(&m.to)))
            .collect()
    }

    /// Callsigns we hold mail for, our own and relayed (each once).
    pub fn waiting_calls(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![];
        for m in self.departures.iter().chain(self.transit.iter()) {
            if !v.contains(&m.to) {
                v.push(m.to.clone());
            }
        }
        v
    }

    /// Carry mail for someone else (relaying). False if we already have it
    /// or it has been relayed too often.
    pub fn take_in_transit(&mut self, m: Mail, from_station: &str) -> bool {
        if m.hops >= MAX_HOPS
            || self
                .transit
                .iter()
                .chain(self.delivered.iter())
                .any(|x| x.id == m.id)
        {
            return false;
        }
        self.transit.push(Mail {
            handed: Some(from_station.to_uppercase()),
            via: None,
            ..m
        });
        self.save();
        true
    }

    /// Mail arrived (deduplicated: a repeat delivery is harmless).
    pub fn arrive(&mut self, m: Mail) -> bool {
        if self.arrivals.iter().any(|a| a.id == m.id) {
            return false;
        }
        self.arrivals.push(Mail { read: false, ..m });
        self.save();
        true
    }

    /// Station `by` confirmed `id`: move it to Delivered.
    pub fn confirm(&mut self, id: u64, by: &str) -> Option<&Mail> {
        let mut m = if let Some(i) = self.departures.iter().position(|m| m.id == id) {
            self.departures.remove(i)
        } else {
            let i = self.transit.iter().position(|m| m.id == id)?;
            self.transit.remove(i)
        };
        m.delivered_at = Some(now_s());
        m.handed = Some(by.to_uppercase());
        self.delivered.push(m);
        self.save();
        self.delivered.last()
    }

    pub fn unread(&self) -> usize {
        self.arrivals.iter().filter(|m| !m.read).count()
    }

    pub fn mark_read(&mut self, id: u64) {
        if let Some(m) = self.arrivals.iter_mut().find(|m| m.id == id) {
            m.read = true;
            self.save();
        }
    }

    pub fn delete(&mut self, id: u64) {
        self.arrivals.retain(|m| m.id != id);
        self.departures.retain(|m| m.id != id);
        self.transit.retain(|m| m.id != id);
        self.delivered.retain(|m| m.id != id);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_deliver_confirm_and_persist() {
        let p = std::env::temp_dir().join(format!("depot-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let mut d = Depot::load(p.clone());
        let id = d.post("n0call", "k7bot", "Meet on 40 m tonight").id;
        d.post("n0call", "K7BOT", "Second note");
        d.post("n0call", "W1AW", "Hello");
        assert_eq!(d.waiting_for("K7BOT").len(), 2);
        assert_eq!(
            d.waiting_calls(),
            vec!["K7BOT".to_string(), "W1AW".to_string()]
        );
        assert!(d.confirm(id, "K7BOT").is_some());
        assert_eq!(d.waiting_for("K7BOT").len(), 1);
        let m = Mail {
            id: 77,
            from: "K7BOT".into(),
            to: "N0CALL".into(),
            created: 1,
            text: "hi".into(),
            read: true,
            delivered_at: None,
            via: None,
            hops: 0,
            handed: None,
        };
        assert!(d.arrive(m.clone()));
        assert!(!d.arrive(m), "repeat delivery is ignored");
        assert_eq!(d.unread(), 1);
        let back = Depot::load(p.clone());
        assert_eq!(back.delivered.len(), 1);
        assert_eq!(back.departures.len(), 2);
        assert_eq!(back.unread(), 1);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn relaying_with_a_loop_guard() {
        let mut d = Depot::default();
        // Mail for W1AW routed via K7BOT is handed to K7BOT.
        d.post_via("N0CALL", "W1AW", Some("k7bot"), "via relay");
        assert_eq!(d.waiting_for("K7BOT").len(), 1);
        assert_eq!(d.waiting_calls(), vec!["W1AW".to_string()]);
        // Carrying mail for someone else, then forwarding it.
        let m = Mail {
            id: 9,
            from: "AA1AA".into(),
            to: "W1AW".into(),
            created: 1,
            text: "relay me".into(),
            read: true,
            delivered_at: None,
            via: None,
            hops: 1,
            handed: None,
        };
        assert!(d.take_in_transit(m.clone(), "AA1AA"));
        assert!(!d.take_in_transit(m.clone(), "AA1AA"), "no duplicates");
        assert_eq!(d.waiting_for("W1AW").len(), 2);
        assert!(d.confirm(9, "W1AW").is_some());
        assert!(d.transit.is_empty());
        assert!(
            !d.take_in_transit(
                Mail {
                    id: 10,
                    hops: MAX_HOPS,
                    ..m
                },
                "X"
            ),
            "too many hops"
        );
    }
}
