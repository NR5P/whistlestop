//! Where to search for whistles: the digital segments of the HF ham bands.
//!
//! Defaults are the US automatically-controlled digital segments of
//! 47 CFR 97.221(b), the part of each band where HF data stations already
//! gather (and away from FT8/FT4 watering holes and the NCDXF beacons).
//! They are only defaults: override or add segments in `whistlestop.toml`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub band: String,
    /// RF edges of the segment, Hz. Everything the whistle occupies must fit.
    pub lo_hz: f64,
    pub hi_hz: f64,
}

pub fn default_segments() -> Vec<Segment> {
    let s = |band: &str, lo: f64, hi: f64| Segment { band: band.into(), lo_hz: lo * 1e3, hi_hz: hi * 1e3 };
    vec![
        s("80m", 3585.0, 3600.0),
        s("40m", 7100.0, 7105.0),
        s("30m", 10140.0, 10150.0),
        s("20m", 14095.0, 14099.5),
        s("20m", 14100.5, 14112.0),
        s("17m", 18105.0, 18110.0),
        s("15m", 21090.0, 21100.0),
        s("12m", 24925.0, 24930.0),
        s("10m", 28120.0, 28189.0),
    ]
}

/// Band name for a dial frequency, if it is in an HF ham band.
pub fn band_of(hz: f64) -> Option<&'static str> {
    let k = hz / 1e3;
    let bands: [(&str, f64, f64); 10] = [
        ("160m", 1800.0, 2000.0),
        ("80m", 3500.0, 4000.0),
        ("60m", 5330.0, 5410.0),
        ("40m", 7000.0, 7300.0),
        ("30m", 10100.0, 10150.0),
        ("20m", 14000.0, 14350.0),
        ("17m", 18068.0, 18168.0),
        ("15m", 21000.0, 21450.0),
        ("12m", 24890.0, 24990.0),
        ("10m", 28000.0, 29700.0),
    ];
    bands.iter().find(|(_, lo, hi)| k >= *lo && k <= *hi).map(|b| b.0)
}

/// USB dial frequencies that cover `seg` with receive passbands
/// (`audio_lo..audio_hi` above the dial) stepped by `step_hz`.
/// Every RF frequency in the segment is heard on at least one channel.
pub fn channels(seg: &Segment, step_hz: f64, audio_lo: f64, audio_hi: f64) -> Vec<f64> {
    let first = seg.lo_hz - audio_lo;
    let last = seg.hi_hz - audio_hi;
    if last <= first {
        // Narrow segment: one channel centred on it.
        let mid_audio = 0.5 * (audio_lo + audio_hi);
        return vec![(0.5 * (seg.lo_hz + seg.hi_hz) - mid_audio).round()];
    }
    let n = ((last - first) / step_hz).ceil() as usize;
    (0..=n).map(|i| (first + (last - first) * i as f64 / n.max(1) as f64).round()).collect()
}

/// What a search covers.
#[derive(Clone, Debug, PartialEq)]
pub enum SearchScope {
    /// The segment(s) of the band the radio is on now.
    CurrentBand,
    /// One named band, e.g. "20m".
    Band(String),
    /// Every configured segment, band after band.
    AllBands,
}

pub fn segments_for(scope: &SearchScope, all: &[Segment], dial_hz: f64) -> Vec<Segment> {
    match scope {
        SearchScope::AllBands => all.to_vec(),
        SearchScope::Band(b) => all.iter().filter(|s| s.band.eq_ignore_ascii_case(b)).cloned().collect(),
        SearchScope::CurrentBand => match band_of(dial_hz) {
            Some(b) => all.iter().filter(|s| s.band == b).cloned().collect(),
            None => vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_cover_the_segment() {
        let seg = Segment { band: "20m".into(), lo_hz: 14_100_500.0, hi_hz: 14_112_000.0 };
        let ch = channels(&seg, 1800.0, 300.0, 2700.0);
        assert_eq!(*ch.first().unwrap(), 14_100_200.0);
        assert_eq!(*ch.last().unwrap(), 14_109_300.0);
        assert!(ch.windows(2).all(|w| w[1] - w[0] <= 1800.0));
    }

    #[test]
    fn narrow_segment_gets_one_channel() {
        let seg = Segment { band: "40m".into(), lo_hz: 7_100_000.0, hi_hz: 7_102_000.0 };
        assert_eq!(channels(&seg, 1800.0, 300.0, 2700.0), vec![7_099_500.0]);
    }

    #[test]
    fn current_band_scope() {
        let segs = default_segments();
        let s = segments_for(&SearchScope::CurrentBand, &segs, 14_074_000.0);
        assert_eq!(s.len(), 2);
        assert!(segments_for(&SearchScope::CurrentBand, &segs, 9_000_000.0).is_empty());
        assert_eq!(band_of(7_074_000.0), Some("40m"));
    }
}
