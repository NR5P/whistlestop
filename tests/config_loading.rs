//! Settings file: old, partial, corrupt and future whistlestop.toml files,
//! and the whistle-v2 settings upgrade.

use whistlestop::config::Config;
use whistlestop::e2e::Policy;
use whistlestop::schedule::{Days, Slot};

fn parse(s: &str) -> Config {
    let mut c: Config = toml::from_str(s).unwrap_or_else(|e| panic!("{e}\n{s}"));
    c.upgrade();
    c
}

/// The only test here that touches WHISTLESTOP_CONFIG (tests in one file
/// share a process), so it runs every file-based case in sequence.
#[test]
fn load_and_save_through_the_settings_file() {
    let dir = std::env::temp_dir().join(format!("ws-itest-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = dir.join("sub/whistlestop.toml");
    // SAFETY: no other test in this binary reads the environment.
    unsafe { std::env::set_var("WHISTLESTOP_CONFIG", &p) };
    assert_eq!(whistlestop::config::path(), p);

    // Missing file: defaults (and already at whistle v2).
    let c = Config::load();
    assert_eq!(c, Config::default());
    assert_eq!(c.whistle_version, 2);

    // Save creates the folder; everything round-trips.
    let mut c = Config {
        callsign: "N0TEST".into(),
        name: "Tëst \"quote\" \\ back".into(),
        demo: false,
        e2e: Policy::Prefer,
        qsy_bands: vec![],
        schedule_on: true,
        schedule: vec![Slot {
            from: "22:00".into(),
            to: "02:00".into(),
            days: Days::Weekends,
            khz: 7_103.5,
            beacon: true,
        }],
        library_max_mb: 0.25,
        ..Default::default()
    };
    c.segments.push(whistlestop::bandplan::Segment { band: "6m".into(), lo_hz: 50_300_000.0, hi_hz: 50_310_000.0 });
    c.save().unwrap();
    assert_eq!(Config::load(), c);

    // Empty file.
    std::fs::write(&p, "").unwrap();
    let e = Config::load();
    assert_eq!(e.callsign, "N0CALL");
    assert_eq!(e.whistle_version, 2);

    // Corrupt file: falls back to defaults rather than failing to start.
    for junk in ["callsign = ", "[[[", "\u{0}\u{1}\u{2}", "callsign = \"A\"\ncallsign = \"B\""] {
        std::fs::write(&p, junk).unwrap();
        assert_eq!(Config::load().callsign, "N0CALL", "{junk:?}");
    }
    // ...and the unreadable file is kept as whistlestop.toml.bad.
    let bad = dir.join("sub/whistlestop.toml.bad");
    assert_eq!(std::fs::read_to_string(&bad).unwrap(), "callsign = \"A\"\ncallsign = \"B\"");
    std::fs::write(&p, "[[[").unwrap();
    let (c, problems) = Config::load_with_report();
    assert_eq!(c.callsign, "N0CALL");
    assert!(problems.len() == 1 && problems[0].contains(".bad"), "{problems:?}");
    // One bad value: the rest is kept, the bad one reported, nothing copied.
    let _ = std::fs::remove_file(&bad);
    std::fs::write(&p, "callsign = \"N0KEEP\"\ndemo = false\ne2e = \"Prefer\"\n").unwrap();
    let (c, problems) = Config::load_with_report();
    assert_eq!((c.callsign.as_str(), c.demo, c.e2e, c.whistle_version), ("N0KEEP", false, Policy::Off, 2));
    assert!(problems.len() == 1 && problems[0].starts_with("e2e"), "{problems:?}");
    assert!(!bad.exists());
    // Not UTF-8 at all.
    std::fs::write(&p, [0xFFu8, 0xFE, 0x00, 0x41]).unwrap();
    assert_eq!(Config::load().callsign, "N0CALL");
    assert_eq!(std::fs::read(&bad).unwrap(), [0xFFu8, 0xFE, 0x00, 0x41]);
    // The path is a folder.
    std::fs::remove_file(&p).unwrap();
    std::fs::create_dir_all(&p).unwrap();
    assert_eq!(Config::load().callsign, "N0CALL");
    assert!(Config::default().save().is_err(), "can't save over a folder");

    unsafe { std::env::remove_var("WHISTLESTOP_CONFIG") };
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn partial_file_fills_in_defaults() {
    let c = parse("callsign = \"N0OLD\"\n");
    let d = Config::default();
    assert_eq!(c.callsign, "N0OLD");
    assert_eq!((c.dwell_s, c.step_hz, c.whistle_s), (d.dwell_s, d.step_hz, d.whistle_s));
    assert_eq!(c.whistle_version, 2);
    assert_eq!(c.e2e, Policy::Off);
    assert!(!c.accept_qsy && !c.share_library && !c.schedule_on && !c.remote_serve);
    assert_eq!(c.segments, d.segments);
    // Partial nested tables too.
    let c = parse("callsign = \"X\"\n[sdr]\n[managed_rig]\n[cat]\n");
    assert_eq!(c.sdr, d.sdr);
    assert_eq!(c.managed_rig, d.managed_rig);
    assert_eq!(c.cat, d.cat);
}

#[test]
fn unknown_keys_and_tables_from_a_newer_version_are_ignored() {
    let c = parse(
        "callsign = \"N0NEW\"\nfuture_flag = true\nwhistle_version = 3\n\
         [future_table]\nx = 1\n[sdr]\nfuture_sdr_key = \"y\"\n",
    );
    assert_eq!(c.callsign, "N0NEW");
    assert_eq!(c.whistle_version, 3, "a newer version number is left alone");
}

#[test]
fn whistle_v1_upgrade_moves_only_old_defaults() {
    // Saved by the first whistle with its defaults.
    let c = parse("callsign = \"A\"\ndwell_s = 2.2\nstep_hz = 1800.0\nwhistle_s = 24.0\n");
    assert_eq!((c.dwell_s, c.step_hz, c.whistle_s, c.whistle_version), (0.5, 2000.0, 10.0, 2));
    // Customised values are kept.
    let c = parse("callsign = \"A\"\ndwell_s = 1.0\nstep_hz = 1500.0\nwhistle_s = 30.0\n");
    assert_eq!((c.dwell_s, c.step_hz, c.whistle_s, c.whistle_version), (1.0, 1500.0, 30.0, 2));
    // Explicit version 0 or 1 is upgraded too.
    for v in [0, 1] {
        let c = parse(&format!("whistle_version = {v}\ndwell_s = 2.2\n"));
        assert_eq!((c.dwell_s, c.whistle_version), (0.5, 2));
    }
    // A v2 file with the old numbers set on purpose is left alone.
    let c = parse("whistle_version = 2\ndwell_s = 2.2\nstep_hz = 1800.0\nwhistle_s = 24.0\n");
    assert_eq!((c.dwell_s, c.step_hz, c.whistle_s), (2.2, 1800.0, 24.0));
    // Idempotent.
    let mut c = parse("dwell_s = 2.2\n");
    let once = c.clone();
    c.upgrade();
    assert_eq!(c, once);
}

#[test]
fn e2e_policy_names() {
    assert_eq!(parse("e2e = \"off\"").e2e, Policy::Off);
    assert_eq!(parse("e2e = \"prefer\"").e2e, Policy::Prefer);
    assert_eq!(parse("e2e = \"require\"").e2e, Policy::Require);
    assert!(toml::from_str::<Config>("e2e = \"on\"").is_err());
}

#[test]
fn default_round_trips_and_is_safe() {
    let d = Config::default();
    let s = toml::to_string_pretty(&d).unwrap();
    assert_eq!(toml::from_str::<Config>(&s).unwrap(), d);
    // Everything that transmits on its own or opens the station is off.
    assert!(d.demo);
    assert_eq!(d.e2e, Policy::Off);
    assert!(!d.auto_answer && !d.beacon && !d.auto_deliver && !d.auto_collect && !d.relay);
    assert!(!d.accept_qsy && !d.share_library && !d.schedule_on && !d.remote_serve && !d.auto_tune);
    assert!(d.auto_in_segments_only && d.qsy_segments_only && d.schedule_only_away);
}

/// A single unreadable value (a typo'd enum, a number written as text, a
/// segment missing one key: "override or add segments in whistlestop.toml")
/// used to make `Config::load` throw away the *whole* file (callsign,
/// Mercury address, rig settings, Demo back on) with no message. The
/// owner's decision: keep every readable value, default only the bad ones,
/// and report which (`Config::load_with_report` / `from_toml_lenient`).
#[test]
fn one_bad_value_keeps_the_other_settings() {
    let d = Config::default();
    for (bad, key) in [
        ("e2e = \"Prefer\"", "e2e"),
        ("mercury_port = \"8300\"", "mercury_port"),
        ("mercury_port = 70000", "mercury_port"),
        ("bandwidth = \"Hz9999\"", "bandwidth"),
        ("[[segments]]\nlo_hz = 14100000.0\nhi_hz = 14110000.0", "segments"),
        ("[sdr]\nspan_hz = \"wide\"", "sdr.span_hz"),
    ] {
        let s = format!("callsign = \"N0KEEP\"\ndemo = false\nmercury_host = \"10.0.0.9\"\n{bad}\n");
        let (c, problems) = Config::from_toml_lenient(&s);
        assert_eq!(
            (c.callsign.as_str(), c.demo, c.mercury_host.as_str()),
            ("N0KEEP", false, "10.0.0.9"),
            "lost everything over: {bad}"
        );
        assert!(
            problems.iter().any(|p| p.starts_with(key)),
            "{bad}: problem not reported: {problems:?}"
        );
        // The bad value itself is the default.
        assert_eq!((c.e2e, c.mercury_port, c.bandwidth), (d.e2e, d.mercury_port, d.bandwidth));
        assert_eq!(c.segments, d.segments);
        assert_eq!(c.sdr, d.sdr);
    }
    // A good file reports nothing.
    let (c, problems) = Config::from_toml_lenient("callsign = \"N0OK\"\n");
    assert!(problems.is_empty() && c.callsign == "N0OK");
    // In a list, the readable entries are kept and the bad one left out.
    let (c, problems) = Config::from_toml_lenient(
        "callsign = \"N0SEG\"\n\
         [[segments]]\nband = \"20m\"\nlo_hz = 14100000.0\nhi_hz = 14110000.0\n\
         [[segments]]\nlo_hz = 7100000.0\nhi_hz = 7105000.0\n\
         [[segments]]\nband = \"40m\"\nlo_hz = 7100000.0\nhi_hz = 7105000.0\n",
    );
    assert_eq!(c.callsign, "N0SEG");
    assert_eq!(c.segments.iter().map(|s| s.band.as_str()).collect::<Vec<_>>(), ["20m", "40m"]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("segments entry 2"), "{problems:?}");
    // Missing whistle_version still means "old file": the upgrade runs.
    let (mut c, _) = Config::from_toml_lenient("e2e = \"Prefer\"\ndwell_s = 2.2\n");
    assert_eq!(c.whistle_version, 1);
    c.upgrade();
    assert_eq!((c.dwell_s, c.whistle_version), (0.5, 2));
}
