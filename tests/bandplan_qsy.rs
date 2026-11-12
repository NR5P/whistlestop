//! Band plan and QSY rules: band edges, passband fitting (dial +300..+2700 Hz
//! must be inside the band), qsy_max_khz, segments-only, search channels.
//! USER-GUIDE "QSY (moving a link)" and "Search".

use whistlestop::bandplan::*;
use whistlestop::config::Config;
use whistlestop::engine::qsy_verdict;

const BANDS: [(&str, f64, f64); 10] = [
    ("160m", 1_800_000.0, 2_000_000.0),
    ("80m", 3_500_000.0, 4_000_000.0),
    ("60m", 5_330_000.0, 5_410_000.0),
    ("40m", 7_000_000.0, 7_300_000.0),
    ("30m", 10_100_000.0, 10_150_000.0),
    ("20m", 14_000_000.0, 14_350_000.0),
    ("17m", 18_068_000.0, 18_168_000.0),
    ("15m", 21_000_000.0, 21_450_000.0),
    ("12m", 24_890_000.0, 24_990_000.0),
    ("10m", 28_000_000.0, 29_700_000.0),
];

fn open_cfg() -> Config {
    Config {
        accept_qsy: true,
        qsy_same_band: false,
        qsy_bands: BANDS.iter().map(|b| b.0.to_string()).collect(),
        qsy_max_khz: 0.0,
        qsy_segments_only: false,
        ..Default::default()
    }
}

#[test]
fn band_of_edges_are_inclusive() {
    for (name, lo, hi) in BANDS {
        assert_eq!(band_of(lo), Some(name));
        assert_eq!(band_of(hi), Some(name));
        assert_eq!(band_of((lo + hi) / 2.0), Some(name));
        assert_eq!(band_of(lo - 1.0), None, "{name} lo-1");
        assert_eq!(band_of(hi + 1.0), None, "{name} hi+1");
    }
    for x in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e30] {
        assert_eq!(band_of(x), None);
    }
}

#[test]
fn whole_passband_must_fit_inside_the_band() {
    let c = open_cfg();
    for (name, lo, hi) in BANDS {
        let home = (lo + hi) / 2.0;
        // Top edge: dial + 2700 Hz may touch the edge, not cross it.
        assert!(qsy_verdict(&c, home, hi - 2700.0).is_ok(), "{name} top fits");
        assert!(qsy_verdict(&c, home, hi - 2699.0).is_err(), "{name} top 1 Hz over");
        assert!(qsy_verdict(&c, home, hi).is_err(), "{name} dial on the top edge");
        // Bottom edge: dial + 300 Hz may touch it; the dial itself may be
        // below the band (USB).
        assert!(qsy_verdict(&c, home, lo - 300.0).is_ok(), "{name} bottom fits");
        assert!(qsy_verdict(&c, home, lo - 301.0).is_err(), "{name} bottom 1 Hz under");
    }
}

#[test]
fn odd_frequencies_are_refused_without_panic() {
    let c = open_cfg();
    for hz in [0.0, -14_100_000.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, u64::MAX as f64] {
        assert!(qsy_verdict(&c, 14_100_000.0, hz).is_err(), "{hz}");
    }
    for home in [f64::NAN, 0.0, f64::INFINITY] {
        let _ = qsy_verdict(&c, home, 14_100_000.0);
        let mut s = c.clone();
        s.qsy_same_band = true;
        assert!(qsy_verdict(&s, home, 14_100_000.0).is_err(), "same band with home {home}");
    }
}

#[test]
fn qsy_off_by_default() {
    let c = Config::default();
    assert!(qsy_verdict(&c, 14_103_000.0, 14_105_000.0).unwrap_err().contains("turned off"));
}

#[test]
fn max_distance_boundary() {
    let mut c = open_cfg();
    c.qsy_max_khz = 50.0;
    let home = 14_103_000.0;
    assert!(qsy_verdict(&c, home, home + 50_000.0).is_ok(), "exactly 50 kHz up");
    assert!(qsy_verdict(&c, home, home - 50_000.0).is_ok(), "exactly 50 kHz down");
    assert!(qsy_verdict(&c, home, home + 50_001.0).unwrap_err().contains("kHz from here"));
    assert!(qsy_verdict(&c, home, home - 50_001.0).unwrap_err().contains("kHz from here"));
    c.qsy_max_khz = 0.0;
    assert!(qsy_verdict(&c, home, 28_500_000.0).is_ok(), "0 = anywhere");
    // Odd values never panic.
    for k in [-1.0, f64::NAN, f64::INFINITY] {
        c.qsy_max_khz = k;
        let _ = qsy_verdict(&c, home, 28_500_000.0);
    }
}

#[test]
fn segments_only_needs_the_whole_signal_inside_one_segment() {
    let mut c = open_cfg();
    c.qsy_segments_only = true;
    let home = 14_103_000.0;
    // 20 m upper segment 14100.5–14112 kHz.
    assert!(qsy_verdict(&c, home, 14_100_200.0).is_ok());
    assert!(qsy_verdict(&c, home, 14_100_199.0).unwrap_err().contains("segments"));
    assert!(qsy_verdict(&c, home, 14_109_300.0).is_ok());
    assert!(qsy_verdict(&c, home, 14_109_301.0).unwrap_err().contains("segments"));
    // Straddling the beacon gap (14099.5–14100.5) fits neither segment.
    assert!(qsy_verdict(&c, home, 14_097_500.0).is_err());
    // Every default segment wide enough for the signal admits a QSY to it.
    for s in default_segments() {
        if s.hi_hz - s.lo_hz >= 2400.0 {
            assert!(qsy_verdict(&c, home, s.lo_hz - 300.0).is_ok(), "{s:?}");
        }
    }
    // No segments configured: nothing passes.
    c.segments.clear();
    assert!(qsy_verdict(&c, home, 14_105_000.0).is_err());
}

#[test]
fn same_band_rule() {
    let mut c = open_cfg();
    c.qsy_same_band = true;
    assert!(qsy_verdict(&c, 14_103_000.0, 14_200_000.0).is_ok());
    let e = qsy_verdict(&c, 14_103_000.0, 7_102_000.0).unwrap_err();
    assert!(e.contains("only QSY within 20m"), "{e}");
    // From outside any band, nothing is "the same band".
    assert!(qsy_verdict(&c, 5_000_000.0, 14_200_000.0).is_err());
}

#[test]
fn qsy_band_list() {
    let mut c = open_cfg();
    c.qsy_bands = vec!["40m".into()];
    assert!(qsy_verdict(&c, 14_103_000.0, 7_150_000.0).is_ok());
    let e = qsy_verdict(&c, 14_103_000.0, 14_150_000.0).unwrap_err();
    assert!(e.contains("isn't one of my QSY bands"), "{e}");
    c.qsy_bands.clear();
    assert!(qsy_verdict(&c, 14_103_000.0, 7_150_000.0).is_err());
}

/// `segments_for(Band(..))` matches band names case-insensitively, but the
/// QSY band list (and `segments_for(CurrentBand)`) compare exactly, so a
/// hand-edited `qsy_bands = ["20M"]` or a segment with `band = "20M"`
/// silently never matches.
#[test]
fn band_names_are_case_insensitive_everywhere() {
    let mut c = open_cfg();
    c.qsy_bands = vec!["20M".into()];
    assert!(qsy_verdict(&c, 14_103_000.0, 14_150_000.0).is_ok());
    let segs = vec![Segment { band: "20M".into(), lo_hz: 14_100_000.0, hi_hz: 14_110_000.0 }];
    assert_eq!(segments_for(&SearchScope::Band("20m".into()), &segs, 0.0).len(), 1);
    assert_eq!(segments_for(&SearchScope::CurrentBand, &segs, 14_105_000.0).len(), 1);
}

#[test]
fn default_segments_lie_inside_their_bands() {
    for s in default_segments() {
        assert!(s.lo_hz < s.hi_hz, "{s:?}");
        assert_eq!(band_of(s.lo_hz), Some(s.band.as_str()), "{s:?}");
        assert_eq!(band_of(s.hi_hz), Some(s.band.as_str()), "{s:?}");
    }
}

#[test]
fn scopes() {
    let segs = default_segments();
    assert_eq!(segments_for(&SearchScope::AllBands, &segs, 0.0), segs);
    assert_eq!(segments_for(&SearchScope::Band("20M".into()), &segs, 0.0).len(), 2);
    assert!(segments_for(&SearchScope::Band("6m".into()), &segs, 0.0).is_empty());
    assert!(segments_for(&SearchScope::CurrentBand, &segs, f64::NAN).is_empty());
    assert_eq!(segments_for(&SearchScope::CurrentBand, &segs, 28_500_000.0).len(), 1);
}

/// Deterministic xorshift.
fn rng(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

#[test]
fn channels_cover_every_frequency_of_the_segment() {
    let mut s = 0x5EED_u64;
    for _ in 0..3000 {
        let lo = 3_500_000.0 + rng(&mut s) * 25_000_000.0;
        let width = rng(&mut s) * 80_000.0;
        let seg = Segment { band: "x".into(), lo_hz: lo, hi_hz: lo + width };
        let alo = 200.0 + rng(&mut s) * 600.0;
        let ahi = alo + 500.0 + rng(&mut s) * 2500.0;
        let step = 100.0 + rng(&mut s) * (ahi - alo - 100.0);
        let ch = channels(&seg, step, alo, ahi);
        assert!(!ch.is_empty());
        assert!(ch.windows(2).all(|w| w[1] >= w[0]), "sorted");
        if seg.hi_hz - seg.lo_hz > ahi - alo {
            // Wide segment: neighbours no further apart than the step, and
            // every passband stays inside the segment (±1 Hz rounding).
            assert!(ch.windows(2).all(|w| w[1] - w[0] <= step + 1.0), "{seg:?} step {step}");
            for &c in &ch {
                assert!(c + alo >= seg.lo_hz - 1.0 && c + ahi <= seg.hi_hz + 1.0, "{c} {seg:?}");
            }
        }
        // Every RF frequency in the segment is heard on some channel.
        for k in 0..=50 {
            let f = seg.lo_hz + (seg.hi_hz - seg.lo_hz) * k as f64 / 50.0;
            assert!(
                ch.iter().any(|&c| f >= c + alo - 1.0 && f <= c + ahi + 1.0),
                "{f} not covered: {seg:?} step {step} {alo}..{ahi} -> {ch:?}"
            );
        }
    }
}

/// `step_hz` comes from whistlestop.toml; the engine only caps it from
/// above (`cfg.step_hz.min(...)`). With step 0 `channels` computes
/// `(span / 0).ceil() as usize` = usize::MAX and panics ("capacity
/// overflow") inside the engine; a negative step silently gives a single
/// channel that covers almost nothing.
#[test]
fn bad_step_never_panics_and_still_covers() {
    let seg = Segment { band: "20m".into(), lo_hz: 14_100_500.0, hi_hz: 14_112_000.0 };
    for step in [0.0, -1.0, -2000.0, f64::NAN] {
        let r = std::panic::catch_unwind(|| channels(&seg, step, 300.0, 2700.0));
        let ch = r.unwrap_or_else(|_| panic!("panicked with step {step}"));
        for k in 0..=20 {
            let f = seg.lo_hz + (seg.hi_hz - seg.lo_hz) * k as f64 / 20.0;
            assert!(ch.iter().any(|&c| f >= c + 299.0 && f <= c + 2701.0), "step {step}: {f} not covered by {ch:?}");
        }
    }
}

#[test]
fn reversed_or_empty_segment_does_not_panic() {
    let seg = Segment { band: "x".into(), lo_hz: 14_110_000.0, hi_hz: 14_100_000.0 };
    assert_eq!(channels(&seg, 2000.0, 300.0, 2700.0).len(), 1);
    let seg = Segment { band: "x".into(), lo_hz: 14_100_000.0, hi_hz: 14_100_000.0 };
    assert_eq!(channels(&seg, 2000.0, 300.0, 2700.0), vec![14_098_500.0]);
}
