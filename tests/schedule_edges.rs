//! Schedule slots: UTC, weekday masks, midnight crossing, overlaps, slot
//! edges, leap days and year ends. USER-GUIDE "Schedule".

use whistlestop::schedule::*;

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm),
/// an independent oracle for the tests.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn t(y: i64, mo: u32, d: u32, h: u64, mi: u64, s: u64) -> u64 {
    days_from_civil(y, mo, d) as u64 * 86_400 + h * 3600 + mi * 60 + s
}

fn slot(from: &str, to: &str, days: Days) -> Slot {
    Slot { from: from.into(), to: to.into(), days, khz: 7_103.0, beacon: false }
}

#[test]
fn oracle_sanity() {
    assert_eq!(t(1970, 1, 1, 0, 0, 0), 0);
    assert_eq!(t(2026, 11, 11, 0, 0, 0), 1_794_355_200); // the unit test's WED
}

#[test]
fn weekday_masks_on_known_dates() {
    // (date, is weekend)
    let dates = [
        ((2024, 2, 29), false), // Thursday, leap day
        ((2028, 2, 29), false), // Tuesday, leap day
        ((2000, 2, 29), false), // Tuesday (2000 is a leap year)
        ((2100, 3, 1), false),  // Monday (2100 is not a leap year)
        ((2026, 12, 31), false), // Thursday
        ((2027, 1, 1), false),  // Friday
        ((2027, 1, 2), true),   // Saturday
        ((2027, 1, 3), true),   // Sunday
        ((2027, 1, 4), false),  // Monday
        ((1970, 1, 1), false),  // Thursday
        ((1970, 1, 3), true),   // Saturday
        ((2038, 1, 19), false), // Tuesday (32-bit time_t rollover)
        ((2106, 2, 7), true),   // Sunday (u32 seconds rollover)
    ];
    let wd = slot("00:00", "00:00", Days::Weekdays);
    let we = slot("00:00", "00:00", Days::Weekends);
    let ev = slot("00:00", "00:00", Days::Every);
    for ((y, m, d), weekend) in dates {
        for (h, mi) in [(0, 0), (12, 0), (23, 59)] {
            let at = t(y, m, d, h, mi, 30);
            assert_eq!(wd.on_at(at), !weekend, "{y}-{m}-{d} {h}:{mi} weekdays");
            assert_eq!(we.on_at(at), weekend, "{y}-{m}-{d} {h}:{mi} weekends");
            assert!(ev.on_at(at));
        }
    }
}

#[test]
fn slot_edges_to_the_second() {
    let s = slot("13:00", "17:00", Days::Every);
    let day = t(2026, 11, 11, 0, 0, 0);
    let at = |h: u64, m: u64, sec: u64| day + h * 3600 + m * 60 + sec;
    assert!(!s.on_at(at(12, 59, 59)));
    assert!(s.on_at(at(13, 0, 0)), "starts exactly at from");
    assert!(s.on_at(at(16, 59, 59)));
    assert!(!s.on_at(at(17, 0, 0)), "ends exactly at to");
}

#[test]
fn midnight_crossing_belongs_to_the_day_it_started() {
    // Mon–Fri 22:00–02:00: Fri night runs into Sat morning; Sun night
    // (into Mon morning) does not run; Mon 01:00 is off.
    let s = slot("22:00", "02:00", Days::Weekdays);
    let fri = (2027, 1, 1);
    let sat = (2027, 1, 2);
    let sun = (2027, 1, 3);
    let mon = (2027, 1, 4);
    let at = |(y, m, d): (i64, u32, u32), h, mi, sec| t(y, m, d, h, mi, sec);
    assert!(s.on_at(at(fri, 22, 0, 0)));
    assert!(s.on_at(at(fri, 23, 59, 59)));
    assert!(s.on_at(at(sat, 0, 0, 0)), "Friday's slot carries past midnight");
    assert!(s.on_at(at(sat, 1, 59, 59)));
    assert!(!s.on_at(at(sat, 2, 0, 0)));
    assert!(!s.on_at(at(sat, 22, 0, 0)), "Saturday's slot doesn't run");
    assert!(!s.on_at(at(mon, 1, 0, 0)), "started on Sunday");
    assert!(s.on_at(at(mon, 22, 30, 0)));
    assert!(!s.on_at(at(sun, 23, 0, 0)));
    // Weekends 22:00–02:00: Sunday night runs into Monday morning.
    let w = slot("22:00", "02:00", Days::Weekends);
    assert!(w.on_at(at(mon, 1, 0, 0)));
    assert!(!w.on_at(at(sat, 1, 0, 0)), "started Friday");
    assert!(w.on_at(at(sun, 1, 0, 0)), "started Saturday");
}

#[test]
fn crossing_new_year_and_leap_day() {
    let s = slot("23:00", "01:00", Days::Every);
    assert!(s.on_at(t(2026, 12, 31, 23, 30, 0)));
    assert!(s.on_at(t(2027, 1, 1, 0, 30, 0)));
    assert!(!s.on_at(t(2027, 1, 1, 1, 0, 0)));
    assert!(s.on_at(t(2028, 2, 28, 23, 30, 0)));
    assert!(s.on_at(t(2028, 2, 29, 0, 30, 0)));
    assert!(s.on_at(t(2028, 2, 29, 23, 30, 0)));
    assert!(s.on_at(t(2028, 3, 1, 0, 30, 0)));
    // Weekdays across a leap day that is a Tuesday into Wednesday.
    let wd = slot("23:00", "01:00", Days::Weekdays);
    assert!(wd.on_at(t(2028, 3, 1, 0, 30, 0)));
    // ...and across New Year 2027 (Thu→Fri→Sat).
    assert!(wd.on_at(t(2027, 1, 2, 0, 30, 0)), "Friday night into Saturday");
    assert!(!wd.on_at(t(2027, 1, 3, 0, 30, 0)), "Saturday night into Sunday");
}

#[test]
fn ending_at_midnight_and_starting_at_midnight() {
    let to_midnight = slot("20:00", "00:00", Days::Every);
    assert!(to_midnight.on_at(t(2027, 1, 1, 23, 59, 59)));
    assert!(!to_midnight.on_at(t(2027, 1, 2, 0, 0, 0)));
    assert!(!to_midnight.on_at(t(2027, 1, 1, 19, 59, 59)));
    let from_midnight = slot("00:00", "06:00", Days::Weekends);
    assert!(from_midnight.on_at(t(2027, 1, 2, 0, 0, 0)), "Saturday 00:00");
    assert!(!from_midnight.on_at(t(2027, 1, 1, 23, 59, 59)), "Friday");
    assert!(!from_midnight.on_at(t(2027, 1, 4, 0, 0, 0)), "Monday 00:00");
}

#[test]
fn equal_from_and_to_is_all_day() {
    let s = slot("08:00", "08:00", Days::Weekends);
    for h in 0..24 {
        assert!(s.on_at(t(2027, 1, 2, h, 0, 0)));
        assert!(!s.on_at(t(2027, 1, 4, h, 0, 0)));
    }
}

#[test]
fn first_listed_slot_wins_on_overlap() {
    let a = Slot { khz: 14_103.0, ..slot("12:00", "18:00", Days::Every) };
    let b = Slot { khz: 7_103.0, ..slot("16:00", "20:00", Days::Every) };
    let bad = Slot { khz: 0.0, ..slot("00:00", "00:00", Days::Every) };
    let list = [bad.clone(), a.clone(), b.clone()];
    let at = |h| t(2027, 1, 5, h, 0, 0);
    assert_eq!(active(&list, at(13)).map(|x| x.0), Some(1));
    assert_eq!(active(&list, at(17)).map(|x| x.0), Some(1), "a is listed first");
    assert_eq!(active(&list, at(19)).map(|x| x.0), Some(2));
    assert_eq!(active(&list, at(21)).map(|x| x.0), None, "invalid slots never win");
    assert_eq!(active(&[b, a], at(17)).map(|x| x.0), Some(0));
    assert!(active(&[], at(17)).is_none());
}

#[test]
fn invalid_slots_never_match() {
    for (from, to, khz) in [
        ("24:00", "02:00", 7_103.0),
        ("12:60", "13:00", 7_103.0),
        ("12:00", "", 7_103.0),
        ("noon", "13:00", 7_103.0),
        ("12:00:00", "13:00", 7_103.0),
        ("-1:00", "13:00", 7_103.0),
        ("12:00", "13:00", f64::NAN),
        ("12:00", "13:00", 1_800.0),
        ("12:00", "13:00", 30_000.0),
        ("12:00", "13:00", -14_000.0),
        ("12:00", "13:00", f64::INFINITY),
    ] {
        let s = Slot { from: from.into(), to: to.into(), days: Days::Every, khz, beacon: true };
        assert!(!s.valid(), "{from}-{to} {khz}");
        assert!(active(std::slice::from_ref(&s), t(2027, 1, 5, 12, 30, 0)).is_none(), "{from}-{to} {khz}");
    }
}

#[test]
fn minutes_parsing() {
    assert_eq!(minutes("00:00"), Some(0));
    assert_eq!(minutes("23:59"), Some(23 * 60 + 59));
    assert_eq!(minutes(" 7:05 "), Some(425));
    assert_eq!(minutes("07 : 05"), Some(425));
    assert_eq!(minutes("24:00"), None);
    assert_eq!(minutes("23:60"), None);
    assert_eq!(minutes("4294967296:00"), None);
    assert_eq!(minutes(""), None);
    assert_eq!(minutes(":"), None);
    assert_eq!(minutes("1300"), None);
}

#[test]
fn far_future_times_do_not_panic() {
    let s = slot("22:00", "02:00", Days::Weekdays);
    for at in [0, 1, u64::MAX, u64::MAX - 86_400, u64::MAX / 2] {
        let _ = s.on_at(at);
    }
}

#[test]
fn every_minute_of_a_week_matches_a_simple_model() {
    // Brute force against a direct model of the rules over a whole week,
    // for a set of slots.
    let mon = days_from_civil(2027, 1, 4) as u64; // a Monday
    let specs = [
        ("13:00", "17:00"),
        ("22:00", "02:00"),
        ("00:00", "00:01"),
        ("23:59", "00:00"),
        ("06:30", "06:30"),
    ];
    for (from, to) in specs {
        for days in Days::ALL {
            let s = slot(from, to, days);
            let (a, b) = (minutes(from).unwrap(), minutes(to).unwrap());
            for minute in 0..(7 * 24 * 60) {
                let wd = (minute / (24 * 60)) as u64; // 0 = Monday
                let m = (minute % (24 * 60)) as u32;
                let has = |w: u64| match days {
                    Days::Every => true,
                    Days::Weekdays => w < 5,
                    Days::Weekends => w >= 5,
                };
                let want = if a == b {
                    has(wd)
                } else if a < b {
                    has(wd) && m >= a && m < b
                } else {
                    (m >= a && has(wd)) || (m < b && has((wd + 6) % 7))
                };
                let at = mon * 86_400 + minute as u64 * 60;
                assert_eq!(s.on_at(at), want, "{from}-{to} {days:?} minute {minute}");
            }
        }
    }
}

#[test]
fn slots_round_trip_through_toml() {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct W {
        schedule: Vec<Slot>,
    }
    let w = W {
        schedule: vec![
            slot("22:00", "02:00", Days::Weekdays),
            Slot { beacon: true, ..slot("13:00", "17:00", Days::Weekends) },
        ],
    };
    let s = toml::to_string(&w).unwrap();
    let back: W = toml::from_str(&s).unwrap();
    assert_eq!(back.schedule, w.schedule);
    // A partial slot fills in defaults.
    let back: W = toml::from_str("[[schedule]]\nfrom = \"01:00\"\n").unwrap();
    assert_eq!(back.schedule[0].to, Slot::default().to);
}
