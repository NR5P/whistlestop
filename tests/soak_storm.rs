//! Command storms: commands in odd orders, repeated, mixed, and settings
//! with odd values. The engine must never die (its thread stays alive and
//! keeps answering), must say what it refused in the Log, and must come
//! back to a clean idle state.
//! Intended behaviour: docs/USER-GUIDE.md ("Calling CQ", "Chat",
//! "Roundtables", "Finding stations", "Settings reference").

mod soak_common;

use soak_common::*;
use std::time::Duration;
use whistlestop::bandplan::SearchScope;
use whistlestop::engine::{Cmd, Dir, XferState};
use whistlestop::fake_mercury::FakeOptions;
use whistlestop::modes::Bandwidth;

fn quiet_bot() -> FakeOptions {
    FakeOptions { answer_cq: false, ..Default::default() }
}

fn count_sent(stn: &Stn, prefix: &str) -> usize {
    stn.sent_lines().iter().filter(|l| l.starts_with(prefix)).count()
}

#[test]
fn connect_twice_quickly_gives_one_clean_link() {
    let stn = fast("N0TWO", FakeOptions::default(), |_| {});
    for round in 0..10 {
        stn.send(Cmd::Connect("K7BOT".into()));
        stn.send(Cmd::Connect("k7bot ".into()));
        stn.wait("link", 10.0, |s| linked_with(s, "K7BOT"));
        stn.wait("calling cleared once linked", 3.0, |s| s.calling.is_none());
        chat_roundtrip(&stn, &format!("double {round}"), 10.0);
        link_down(&stn, 10.0);
        std::thread::sleep(Duration::from_millis(200));
        assert_idle(&stn, &format!("round {round}"));
    }
}

/// A second Connect during a link: like Depot → Connect ("already
/// linked"), it should be refused, not sent to Mercury, and must not leave
/// "Calling …" showing for the rest of the link.
#[test]
fn connect_while_linked_is_refused() {
    let stn = fast("N0CWL", FakeOptions::default(), |_| {});
    link_up(&stn, 10.0);
    let before = count_sent(&stn, "CONNECT ");
    stn.send(Cmd::Connect("K7BOT".into()));
    std::thread::sleep(Duration::from_millis(500));
    assert!(responsive(&stn.h));
    let calling = stn.st().calling.clone();
    let sent = count_sent(&stn, "CONNECT ") - before;
    assert!(
        calling.is_none() && sent == 0,
        "while linked: calling={calling:?}, CONNECT lines sent to Mercury={sent}\n{}",
        dump(&stn.h)
    );
}

/// The link stays up, and a CQ asked for during it is refused with a reason.
#[test]
fn cq_commands_in_odd_orders() {
    let stn = fast("N0CQS", quiet_bot(), |c| {
        c.cq_cycles = 3;
    });
    // Stop CQ with none running: harmless.
    for _ in 0..5 {
        stn.send(Cmd::StopCq);
    }
    assert!(responsive(&stn.h));
    // Call CQ twice quickly: one CQ, cycle 1.
    stn.send(Cmd::CallCq { table: None });
    stn.send(Cmd::CallCq { table: None });
    stn.wait("CQ running", 5.0, |s| s.cq.active);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(stn.st().cq.cycle, 1, "two Call CQs started two cycles\n{}", dump(&stn.h));
    stn.send(Cmd::StopCq);
    stn.wait("CQ stopped, PTT off", 3.0, |s| !s.cq.active && !s.tx);
    // StopCq right after CallCq, many times.
    for _ in 0..20 {
        stn.send(Cmd::CallCq { table: None });
        stn.send(Cmd::StopCq);
    }
    stn.wait("CQ stopped after a storm, PTT off", 3.0, |s| !s.cq.active && !s.tx);
    assert!(holds_for(&stn.h, 2.0, |s| !s.cq.active && !s.tx), "CQ came back\n{}", dump(&stn.h));
    // Call CQ while linked: refused.
    link_up(&stn, 10.0);
    stn.send(Cmd::CallCq { table: None });
    stn.wait("refusal logged", 3.0, |s| {
        s.log.iter().any(|l| l.1.contains("already linked; disconnect first"))
    });
    assert!(!stn.st().cq.active);
    chat_roundtrip(&stn, "still linked", 5.0);
    // Disconnect twice, then Disconnect with nothing going on.
    stn.send(Cmd::Disconnect);
    stn.send(Cmd::Disconnect);
    stn.wait("down", 10.0, |s| s.link.is_none());
    stn.send(Cmd::Disconnect);
    std::thread::sleep(Duration::from_millis(300));
    assert!(responsive(&stn.h));
    assert_idle(&stn, "after the CQ storm");
}

/// CQ with every number at its edge: 0 cycles, 0 s whistle, 0 s listening.
/// It runs one cycle and ends ("No answer to CQ") with the PTT off.
#[test]
fn cq_with_zero_settings_ends_cleanly() {
    let stn = fast("N0ZRO", quiet_bot(), |c| {
        c.cq_cycles = 0;
        c.whistle_s = 0.0;
        c.cq_listen_s = 0.0;
    });
    for _ in 0..3 {
        stn.send(Cmd::CallCq { table: None });
        stn.wait("CQ ends by itself", 20.0, |s| !s.cq.active && !s.tx && s.log.iter().any(|l| l.1 == "No answer to CQ"));
    }
    assert!(count_sent(&stn, "CQFRAME ") >= 1);
    assert!(responsive(&stn.h));
}

/// Listen after CQ from a hand-edited settings file (or a remote client's
/// Update) at -1 s: the engine thread must survive.
#[test]
fn negative_cq_listen_does_not_kill_the_engine() {
    let stn = fast("N0NEG", quiet_bot(), |_| {});
    let mut c = stn.cfg.clone();
    c.cq_listen_s = -1.0;
    stn.send(Cmd::Update(c));
    stn.send(Cmd::CallCq { table: None });
    std::thread::sleep(Duration::from_secs(9));
    assert!(alive(&stn.h) && responsive(&stn.h), "engine died\n{}", dump(&stn.h));
}

#[test]
fn nan_cq_listen_does_not_kill_the_engine() {
    let stn = fast("N0NAN", quiet_bot(), |c| c.cq_listen_s = f64::NAN);
    stn.send(Cmd::CallCq { table: None });
    std::thread::sleep(Duration::from_secs(9));
    assert!(alive(&stn.h) && responsive(&stn.h), "engine died\n{}", dump(&stn.h));
}

/// Station ID every "inf" minutes (TOML allows `inf`): the engine thread
/// must survive the first tick.
#[test]
fn infinite_beacon_interval_does_not_kill_the_engine() {
    let stn = fast("N0INF", quiet_bot(), |_| {});
    let mut c = stn.cfg.clone();
    c.beacon = true;
    c.beacon_minutes = f64::INFINITY;
    stn.send(Cmd::Update(c));
    std::thread::sleep(Duration::from_secs(2));
    assert!(alive(&stn.h) && responsive(&stn.h), "engine died\n{}", dump(&stn.h));
}

/// Demo mode with a negative whistle length (settings file): the CQ must
/// still move on (or be refused) rather than sit "whistling" with the PTT
/// keyed forever.
#[test]
fn demo_negative_whistle_does_not_stick() {
    let stn = demo("N0NWH", |c| c.whistle_s = -1.0);
    stn.send(Cmd::CallCq { table: None });
    stn.wait("CQ under way", 5.0, |s| s.cq.active || s.log.iter().any(|l| l.1.contains('⚠')));
    let moved_on = try_wait(&stn.h, 10.0, |s| !s.cq.active || s.cq.phase != "whistling");
    assert!(moved_on, "CQ stuck whistling\n{}", dump(&stn.h));
}

#[test]
fn lowercase_callsign_works() {
    let stn = fast("n0low", FakeOptions::default(), |_| {});
    assert_eq!(stn.st().mycall, "N0LOW");
    assert!(stn.sent_lines().iter().any(|l| l == "MYCALL N0LOW"), "{:?}", stn.sent_lines());
    link_up(&stn, 10.0);
    chat_roundtrip(&stn, "lower", 5.0);
    link_down(&stn, 10.0);
    assert_idle(&stn, "lowercase call");
}

/// An empty callsign: nothing may go on the air without one (the CQFRAME
/// is the station identification). Expected: Start or Call CQ refuses with
/// a clear message.
#[test]
fn empty_callsign_is_refused() {
    let stn = fast("", quiet_bot(), |_| {});
    stn.send(Cmd::CallCq { table: None });
    std::thread::sleep(Duration::from_secs(3));
    stn.send(Cmd::StopCq);
    let frames: Vec<String> =
        stn.sent_lines().into_iter().filter(|l| l.starts_with("CQFRAME")).collect();
    let warned = logs(&stn.h).iter().any(|l| l.contains('⚠') && l.to_lowercase().contains("callsign"));
    assert!(frames.is_empty() && warned, "CQFRAMEs sent: {frames:?}, warned: {warned}\n{}", dump(&stn.h));
}

/// A callsign with a space ("N0 SP"): Mercury takes only "N0" and the
/// CONNECT line becomes "CONNECT N0 SP K7BOT". Expected: refused with a
/// clear message, or trimmed/fixed so linking works.
#[test]
fn callsign_with_space_is_refused_or_works() {
    let stn = fast("N0 SP", FakeOptions::default(), |_| {});
    stn.send(Cmd::Connect("K7BOT".into()));
    let linked = try_wait(&stn.h, 5.0, |s| linked_with(s, "K7BOT"));
    let warned = logs(&stn.h).iter().any(|l| l.contains('⚠') && l.to_lowercase().contains("callsign"));
    assert!(linked || warned, "no link and no warning; sent {:?}\n{}", stn.sent_lines(), dump(&stn.h));
}

/// Bandwidth changed mid-link (the bandwidth buttons stay live during a
/// link): the link carries on, chat and a file still go through.
#[test]
fn bandwidth_change_mid_link() {
    // The owner's rule: the bandwidth is locked during a link (it was
    // agreed when the link came up); a change applies from the next link.
    let stn = fast("N0BWL", FakeOptions::default(), |_| {});
    link_up(&stn, 10.0);
    let agreed = stn.st().bandwidth;
    stn.send(Cmd::SetBandwidth(Bandwidth::Hz500));
    stn.wait("refused during the link", 3.0, |s| {
        s.log.iter().any(|l| l.1.contains("bandwidth can't change during a link"))
    });
    assert_eq!(stn.st().bandwidth, agreed, "bandwidth changed mid-link");
    link_down(&stn, 10.0);
    for (i, bw) in [Bandwidth::Hz500, Bandwidth::Hz250, Bandwidth::Hz2750, Bandwidth::Hz2300]
        .into_iter()
        .enumerate()
    {
        stn.send(Cmd::SetBandwidth(bw));
        stn.wait("bandwidth shown", 3.0, |s| s.bandwidth == Some(bw));
        link_up(&stn, 10.0);
        chat_roundtrip(&stn, &format!("bw {i}"), 5.0);
        let p = write_file(&stn.root.join("out"), &format!("bw-{i}.bin"), &noise(20_000, i as u64));
        stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
        stn.wait("file through", 20.0, |s| {
            newest_xfer(s, Dir::Out, &format!("bw-{i}.bin")).is_some_and(|t| t.state == XferState::Done(None))
        });
        link_down(&stn, 10.0);
    }
    assert_idle(&stn, "after bandwidth changes");
}

/// Search in Demo (real audio from the simulated band): odd dwell values,
/// retuning during a search, and searches asked for at the wrong time.
#[test]
fn search_storm_demo() {
    let stn = demo("N0SRC", |_| {});
    for dwell in [0.0, -1.0, f64::NAN, 0.05] {
        let mut c = stn.cfg.clone();
        c.dwell_s = dwell;
        stn.send(Cmd::Update(c));
        stn.send(Cmd::SetFreq(7_074_000.0)); // 40 m: nothing to find there
        stn.send(Cmd::StartSearch(SearchScope::CurrentBand));
        stn.wait("search running", 5.0, |s| s.search.active);
        let first = stn.st().search.index;
        let moved = try_wait(&stn.h, 6.0, |s| s.search.index != first);
        assert!(alive(&stn.h), "dwell {dwell}: engine died\n{}", dump(&stn.h));
        eprintln!("dwell {dwell}: search stepped: {moved}");
        // Retune by hand during the search: it stops.
        stn.send(Cmd::SetFreq(7_080_000.0));
        stn.wait("search stopped by hand tuning", 3.0, |s| !s.search.active && s.rig.freq_hz == 7_080_000.0);
        assert!(has_log(&stn.h, "Search tuned by hand"));
    }
    // Start twice, stop twice, stop with none.
    let mut c = stn.cfg.clone();
    c.dwell_s = 0.5;
    stn.send(Cmd::Update(c));
    stn.send(Cmd::StartSearch(SearchScope::CurrentBand));
    stn.send(Cmd::StartSearch(SearchScope::AllBands));
    stn.wait("search running", 5.0, |s| s.search.active && s.search.scope == "all bands");
    stn.send(Cmd::StopSearch);
    stn.send(Cmd::StopSearch);
    stn.wait("stopped", 3.0, |s| !s.search.active);
    // A band with no segment.
    stn.send(Cmd::StartSearch(SearchScope::Band("nonsense".into())));
    stn.wait("refusal", 3.0, |s| s.log.iter().any(|l| l.1.contains("no digital segment")));
    // During a CQ.
    stn.send(Cmd::CallCq { table: None });
    stn.wait("cq", 5.0, |s| s.cq.active);
    stn.send(Cmd::StartSearch(SearchScope::CurrentBand));
    stn.wait("refusal", 3.0, |s| s.log.iter().any(|l| l.1.contains("stop calling CQ first")));
    stn.send(Cmd::StopCq);
    stn.wait("cq off", 3.0, |s| !s.cq.active && !s.tx);
    // During a link: refused, and no retune.
    link_up(&stn, 30.0);
    let f = stn.st().rig.freq_hz;
    stn.send(Cmd::StartSearch(SearchScope::CurrentBand));
    stn.send(Cmd::SetFreq(14_200_000.0));
    stn.send(Cmd::Nudge(500.0));
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(stn.st().rig.freq_hz, f, "retuned mid-link");
    assert!(!stn.st().search.active);
    assert!(has_log(&stn.h, "not while linked"));
    link_down(&stn, 30.0);
    assert!(responsive(&stn.h));
}

/// Odd values for radio commands: the engine survives them all.
#[test]
fn odd_radio_values_demo() {
    let stn = demo("N0ODD", |_| {});
    let cmds = vec![
        Cmd::SetFreq(f64::NAN),
        Cmd::SetFreq(-5.0),
        Cmd::SetFreq(1e12),
        Cmd::Nudge(f64::INFINITY),
        Cmd::SetFreq(14_096_000.0),
        Cmd::SetPower(f32::NAN),
        Cmd::SetPower(-3.0),
        Cmd::SetMode(String::new()),
        Cmd::SetMode("NOTAMODE".into()),
        Cmd::RigLevel("BOGUS".into(), f32::NAN),
        Cmd::RigFunc(String::new(), true),
        Cmd::RigOp("NOPE".into()),
        Cmd::RigVfo("VFOZ".into()),
        Cmd::RigSplit(true),
        Cmd::RigSplit(false),
        Cmd::RigOffset { xit: true, hz: i32::MIN },
        Cmd::RigPassband(-1),
        Cmd::RigWatch(true),
        Cmd::TuneAntenna,
        Cmd::SetSdrSpan(f64::NAN),
        Cmd::SetSdrSpan(-1.0),
        Cmd::MonitorVolume(f32::NAN),
        Cmd::LockOn(f64::NAN),
        Cmd::StopSearch,
        Cmd::Tune(true),
        Cmd::Tune(false),
        Cmd::RigWatch(false),
        Cmd::SetFreq(14_096_000.0),
    ];
    for c in cmds {
        let what = format!("{c:?}");
        stn.send(c);
        std::thread::sleep(Duration::from_millis(150));
        assert!(alive(&stn.h), "engine died after {what}\n{}", dump(&stn.h));
    }
    std::thread::sleep(Duration::from_secs(2));
    assert!(responsive(&stn.h), "{}", dump(&stn.h));
    stn.wait("PTT off after Tune(false)", 5.0, |s| !s.tx);
    link_up(&stn, 30.0);
    chat_roundtrip(&stn, "after odd values", 30.0);
    link_down(&stn, 30.0);
}

/// Join a Roundtable, then connect 1:1. The Guide says the two don't mix
/// ("a Roundtable switches Mercury to broadcast modes, which would cut the
/// link"): Connect should be refused (or leave the table) rather than run
/// a link with the table still on.
#[test]
fn join_table_then_connect() {
    let stn = fast("N0JTC", FakeOptions::default(), |_| {});
    stn.send(Cmd::JoinTable("soaktable".into()));
    stn.wait("at the table", 5.0, |s| s.table.is_some());
    stn.wait("K7BOT joins", 10.0, |s| s.table.as_ref().is_some_and(|t| t.members.contains_key("K7BOT")));
    stn.send(Cmd::Connect("K7BOT".into()));
    let linked = try_wait(&stn.h, 5.0, |s| s.link.is_some());
    let table = stn.st().table.is_some();
    assert!(!(linked && table), "a 1:1 link with the Roundtable still joined\n{}", dump(&stn.h));
}

/// The owner's rule: a 1:1 link and a Roundtable never mix. Connect at a
/// table is refused (Leave first); after leaving, Connect works, and
/// joining a table during the link is refused too.
#[test]
fn join_table_then_connect_does_not_break() {
    let stn = fast("N0JTB", FakeOptions::default(), |_| {});
    stn.send(Cmd::JoinTable("soaktable".into()));
    stn.wait("at the table", 5.0, |s| s.table.is_some());
    stn.send(Cmd::TableSay("hello table".into()));
    stn.send(Cmd::Connect("K7BOT".into()));
    stn.wait("connect refused at a table", 3.0, |s| s.log.iter().any(|l| l.1.contains("Leave it first")));
    assert!(stn.st().link.is_none() && stn.st().calling.is_none(), "{}", dump(&stn.h));
    stn.send(Cmd::LeaveTable);
    stn.wait("left", 3.0, |s| s.table.is_none());
    stn.send(Cmd::Connect("K7BOT".into()));
    stn.wait("link", 15.0, |s| s.link.is_some());
    chat_roundtrip(&stn, "linked after leaving the table", 10.0);
    stn.send(Cmd::JoinTable("another".into()));
    stn.wait("join refused while linked", 3.0, |s| s.log.iter().any(|l| l.1.contains("you're in a 1:1 link")));
    link_down(&stn, 10.0);
    assert_idle(&stn, "after table + link");
    // Back in the calling mode afterwards.
    stn.wait("calling mode restored", 10.0, |s| s.listen_mode.is_some_and(|m| m.name == "DATAC3"));
}

/// Start twice quickly, Stop then more commands, Start again: the engine
/// survives, a stopped station ignores commands, a restarted one works.
#[test]
fn start_stop_storm() {
    let stn = fast("N0SSS", FakeOptions::default(), |_| {});
    let cfg = stn.cfg.clone();
    for _ in 0..3 {
        stn.send(Cmd::Start(cfg.clone()));
        stn.send(Cmd::Start(cfg.clone()));
    }
    stn.wait("running again", 20.0, |s| s.running && s.mercury_ok);
    link_up(&stn, 10.0);
    // Stop in the middle of a link, then commands at the stopped engine.
    stn.send(Cmd::Stop);
    stn.wait("stopped", 10.0, |s| !s.running);
    for c in [
        Cmd::Connect("K7BOT".into()),
        Cmd::Chat("into the void".into()),
        Cmd::CallCq { table: None },
        Cmd::Disconnect,
        Cmd::StartSearch(SearchScope::CurrentBand),
        Cmd::JoinTable("x".into()),
        Cmd::Stop,
        Cmd::Stop,
    ] {
        stn.send(c);
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(&stn.h));
    assert!(!stn.st().running);
    stn.send(Cmd::Start(cfg.clone()));
    stn.wait("restarted", 20.0, |s| s.running && s.mercury_ok);
    assert_idle(&stn, "fresh start");
    link_up(&stn, 10.0);
    chat_roundtrip(&stn, "after restart", 5.0);
    link_down(&stn, 10.0);
}

/// Thousands of commands in a pseudo-random order, with valid values, at
/// a link, a table and a CQ all at once. Afterwards the engine is alive
/// and, once things are wound down, links and chats normally.
#[test]
fn random_command_salvo() {
    let stn = fast("N0RND", FakeOptions::default(), |c| {
        c.accept_qsy = true;
    });
    let file = write_file(&stn.root.join("out"), "salvo.bin", &noise(30_000, 5));
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut rnd = |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    for i in 0..3000u64 {
        let fid = stn.st().transfers.last().map(|t| (t.fid, t.dir));
        let c = match rnd(26) {
            0 | 1 => Cmd::Connect("K7BOT".into()),
            2 => Cmd::Disconnect,
            3 | 4 => Cmd::Chat(format!("salvo {i}")),
            5 => Cmd::CallCq { table: None },
            6 => Cmd::StopCq,
            7 => Cmd::StartSearch(SearchScope::CurrentBand),
            8 => Cmd::StopSearch,
            9 => Cmd::SetFreq(14_096_000.0 + rnd(5000) as f64),
            10 => Cmd::SetBandwidth(Bandwidth::ALL[rnd(6) as usize]),
            11 => Cmd::JoinTable("salvo".into()),
            12 => Cmd::LeaveTable,
            13 => Cmd::TableSay(format!("t{i}")),
            14 => Cmd::SendFiles { paths: vec![file.clone()], zip: rnd(2) == 0 },
            15 => match fid {
                Some((fid, dir)) => Cmd::Cancel { fid, dir },
                None => Cmd::StopCq,
            },
            16 => match fid {
                Some((fid, _)) => Cmd::AnswerOffer { fid, accept: rnd(2) == 0 },
                None => Cmd::StopCq,
            },
            17 => Cmd::SetAway(rnd(2) == 0),
            18 => Cmd::DepotPost { to: "K7BOT".into(), via: None, text: format!("m{i}") },
            19 => Cmd::BeaconNow,
            20 => Cmd::Qsy(14_100_000.0 + rnd(3000) as f64),
            21 => Cmd::LibList(String::new()),
            22 => Cmd::LibFetch("k7bot-qsl.txt".into()),
            23 => Cmd::Chat("send me a file".into()),
            24 => Cmd::CallCq { table: Some("salvo".into()) },
            _ => Cmd::PassFloor("K7BOT".into()),
        };
        stn.send(c);
        if rnd(4) == 0 {
            std::thread::sleep(Duration::from_millis(rnd(15)));
        }
        if i % 500 == 0 {
            assert!(alive(&stn.h), "engine died at command {i}\n{}", dump(&stn.h));
        }
    }
    assert!(responsive(&stn.h), "{}", dump(&stn.h));
    // Wind down.
    for c in [Cmd::StopCq, Cmd::StopSearch, Cmd::LeaveTable, Cmd::SetAway(false), Cmd::Disconnect] {
        stn.send(c);
    }
    stn.wait("wound down", 30.0, |s| s.link.is_none() && !s.cq.active && !s.search.active && s.table.is_none());
    std::thread::sleep(Duration::from_secs(2));
    stn.send(Cmd::Disconnect);
    stn.wait("down", 10.0, |s| s.link.is_none());
    std::thread::sleep(Duration::from_secs(1));
    let g = stn.st();
    eprintln!(
        "after salvo: chat {}, transfers {}, log {}, depot departures {}, delivered {}, state JSON {} B",
        g.chat.len(),
        g.transfers.len(),
        g.log.len(),
        g.depot.departures.len(),
        g.depot.delivered.len(),
        serde_json::to_vec(&*g).unwrap().len()
    );
    drop(g);
    assert!(stn.st().log.len() <= 500);
    assert!(!stn.st().tx, "PTT left on\n{}", dump(&stn.h));
    link_up(&stn, 10.0);
    chat_roundtrip(&stn, "after the salvo", 10.0);
    link_down(&stn, 10.0);
    std::thread::sleep(Duration::from_millis(300));
    assert_idle(&stn, "after the salvo");
}

/// Depot → "Connect to …" with something that isn't a callsign: refused
/// like Connect is (`"…" isn't a callsign`), not sent to Mercury.
#[test]
fn depot_connect_validates_the_callsign() {
    let stn = fast("N0DPC", quiet_bot(), |_| {});
    for bad in ["", "   ", "not a call", "K7BOT;X"] {
        let before = count_sent(&stn, "CONNECT ");
        stn.send(Cmd::DepotConnect(bad.into()));
        std::thread::sleep(Duration::from_millis(400));
        let sent = count_sent(&stn, "CONNECT ") - before;
        let calling = stn.st().calling.clone();
        assert!(sent == 0 && calling.is_none(), "{bad:?}: CONNECT sent {sent}x, calling {calling:?}\n{:?}", stn.sent_lines());
    }
}

/// Connect while our own CQ is running (Demo: K7BOT takes 6 s to answer).
/// The Guide: a whistle or CQFRAME on top of Mercury's call handshake
/// makes it drop the call, so the CQ should stop (or the Connect be
/// refused) as soon as we call out.
#[test]
fn demo_connect_during_cq_stops_the_cq() {
    let stn = demo("N0CDQ", |c| c.whistle_s = 10.0);
    stn.send(Cmd::CallCq { table: None });
    stn.wait("whistling", 5.0, |s| s.cq.active && s.cq.phase == "whistling");
    stn.send(Cmd::Connect("K7BOT".into()));
    stn.wait("calling", 3.0, |s| s.calling.is_some() || s.log.iter().any(|l| l.1.contains("⚠")));
    let stopped = try_wait(&stn.h, 1.5, |s| !s.cq.active);
    let calling = stn.st().calling.clone();
    assert!(stopped || calling.is_none(), "CQ still running while calling\n{}", dump(&stn.h));
}

/// Disconnect while the call is still being placed (Demo: K7BOT answers
/// after about 6 s). The call is abandoned: no link may come up after it.
#[test]
fn demo_disconnect_while_calling_abandons_the_call() {
    let stn = demo("N0DWC", |_| {});
    stn.send(Cmd::Connect("K7BOT".into()));
    stn.wait("calling", 3.0, |s| s.calling.is_some());
    std::thread::sleep(Duration::from_secs(1));
    stn.send(Cmd::Disconnect);
    stn.wait("calling cleared", 3.0, |s| s.calling.is_none());
    assert!(holds_for(&stn.h, 10.0, |s| s.link.is_none()), "a link came up after Disconnect\n{}", dump(&stn.h));
}

/// Call a station that isn't there, give up, call K7BOT: the old call's
/// time-out (Demo: DISCONNECTED 40 s after the CONNECT) must not end the
/// new link.
#[test]
fn demo_stale_call_timeout_does_not_end_the_next_link() {
    let stn = demo("N0STL", |_| {});
    stn.send(Cmd::Connect("W9NONE".into()));
    stn.wait("calling", 3.0, |s| s.calling.is_some());
    std::thread::sleep(Duration::from_secs(1));
    stn.send(Cmd::Disconnect);
    stn.wait("calling cleared", 3.0, |s| s.calling.is_none());
    link_up(&stn, 30.0);
    assert!(holds_for(&stn.h, 45.0, |s| s.link.is_some()), "the link ended by itself\n{}", dump(&stn.h));
}
