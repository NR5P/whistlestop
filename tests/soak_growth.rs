//! Unbounded growth: long sessions (time compressed by the fast fake
//! Mercury where possible) must keep the State's collections bounded, and
//! must not leave files or threads behind. The whole State is also sent
//! as JSON to remote-operation clients 4x a second (remote.rs), so its
//! size is checked too.
//! Intended behaviour: docs/USER-GUIDE.md; bounds that the Guide doesn't
//! state are taken as "about the Log's 500 lines".

mod soak_common;

use soak_common::*;
use std::time::Duration;
use whistlestop::engine::{Cmd, Dir, Who, XferState};
use whistlestop::fake_mercury::FakeOptions;

/// Bounds the Guide/code state, checked after many links and much chat:
/// the Log keeps 500 lines, Heard 30 stations.
#[test]
fn log_and_heard_stay_bounded_over_many_links() {
    let stn = fast("N0BND", FakeOptions::default(), |_| {});
    for i in 0..60 {
        link_up(&stn, 10.0);
        for j in 0..5 {
            stn.send(Cmd::Chat(format!("l{i} c{j}")));
        }
        stn.wait("all delivered", 10.0, |s| s.chat.iter().filter(|c| c.who == Who::Me).all(|c| c.delivered));
        // Log noise: refused commands log a line each.
        for _ in 0..10 {
            stn.send(Cmd::JoinTable(String::new()));
        }
        link_down(&stn, 10.0);
    }
    let g = stn.st();
    eprintln!(
        "60 links: log {}, heard {}, chat {}, transfers {}, mail_alerts {}, audio_rows {}, state JSON {} B",
        g.log.len(),
        g.heard.len(),
        g.chat.len(),
        g.transfers.len(),
        g.mail_alerts.len(),
        g.audio_rows.len(),
        serde_json::to_vec(&*g).unwrap().len()
    );
    assert!(g.log.len() <= 500, "log {}", g.log.len());
    assert!(g.heard.len() <= 30);
    assert!(g.audio_rows.len() <= 64);
}

/// Chat history over a long session: about 3000 lines on one link (a
/// week of an Away station's greetings, away messages and "Linked/ended"
/// lines is in the same range). Expected: bounded like the Log, since the
/// whole history is re-sent to remote clients 4x a second and redrawn.
#[test]
fn chat_history_is_bounded() {
    let stn = fast("N0CHT", FakeOptions::default(), |_| {});
    link_up(&stn, 10.0);
    for batch in 0..15 {
        for j in 0..100 {
            stn.send(Cmd::Chat(format!("b{batch} line {j}")));
        }
        stn.wait("batch delivered", 30.0, |s| s.chat.iter().filter(|c| c.who == Who::Me).all(|c| c.delivered));
    }
    std::thread::sleep(Duration::from_millis(500));
    let n = stn.st().chat.len();
    let js = state_json_len(&stn.h);
    eprintln!("chat {n} lines, state JSON {js} B");
    assert!(n <= 1000, "chat kept {n} lines; state JSON {js} B");
}

/// The Files list over a long session: 250 small files on one link.
#[test]
fn transfer_list_is_bounded() {
    let stn = fast("N0XFL", FakeOptions::default(), |_| {});
    link_up(&stn, 10.0);
    let dir = stn.root.join("out");
    // One at a time: the Demo's K7BOT takes one incoming file at a time
    // (see two_files_offered_back_to_back_both_complete).
    for i in 0..250 {
        let name = format!("f{i}.txt");
        let p = write_file(&dir, &name, format!("file {i}\n").as_bytes());
        stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
        stn.wait("delivered", 10.0, |s| {
            newest_xfer(s, Dir::Out, &name).is_some_and(|t| t.state == XferState::Done(None))
        });
    }
    let n = stn.st().transfers.len();
    eprintln!("transfers {n} rows, state JSON {} B", state_json_len(&stn.h));
    assert!(n <= 200, "Files list kept {n} rows");
}

/// Two files dropped on the window one after the other (two offers before
/// the first is done): both must arrive (K7BOT, the Demo's station, once
/// kept a single incoming slot and left the first "Sending" for good).
#[test]
fn two_files_offered_back_to_back_both_complete() {
    let stn = fast("N0TWF", FakeOptions::default(), |_| {});
    link_up(&stn, 10.0);
    let dir = stn.root.join("out");
    let a = write_file(&dir, "a.bin", &noise(5000, 1));
    let b = write_file(&dir, "b.bin", &noise(5000, 2));
    stn.send(Cmd::SendFiles { paths: vec![a], zip: false });
    stn.send(Cmd::SendFiles { paths: vec![b], zip: false });
    stn.wait("both delivered", 10.0, |s| {
        ["a.bin", "b.bin"]
            .iter()
            .all(|n| newest_xfer(s, Dir::Out, n).is_some_and(|t| t.state == XferState::Done(None)))
    });
}

/// An offer left unanswered when the link ends must not leave an empty
/// file under the download folder's `.partial` (one per distinct offer,
/// forever). Same for an offer cancelled before any of it arrived.
#[test]
fn unanswered_offer_leaves_no_empty_partial() {
    let stn = fast("N0PRT", FakeOptions::default(), |_| {});
    link_up(&stn, 10.0);
    stn.send(Cmd::Chat("please send me a file".into()));
    stn.wait("offer", 10.0, |s| s.transfers.iter().any(|t| t.dir == Dir::In && t.state == XferState::WaitingForYou));
    link_down(&stn, 10.0);
    stn.wait("row says why", 5.0, |s| {
        s.transfers.iter().any(|t| matches!(&t.state, XferState::Failed(w) if w.contains("before you answered")))
    });
    std::thread::sleep(Duration::from_millis(300));
    let p = partials(&stn.dl());
    assert!(p.iter().all(|(_, n)| *n > 0), "empty partial files left: {p:?}");
}

/// Away for a long time: links end themselves after the idle time, the
/// away message goes out, chat lands in Arrivals, and nothing gets stuck
/// between links. Reports how the Depot and State grow.
#[test]
fn away_station_many_auto_hangups() {
    let stn = fast("N0AWY", FakeOptions::default(), |c| {
        c.away_idle_s = 0.5;
    });
    stn.send(Cmd::SetAway(true));
    stn.wait("away", 3.0, |s| s.away);
    let mut last_arrivals = 0;
    for i in 0..25 {
        link_up(&stn, 10.0);
        stn.wait("hung up by itself", 15.0, |s| s.link.is_none());
        assert!(has_log(&stn.h, "Hanging up with K7BOT (idle"), "link {i}");
        std::thread::sleep(Duration::from_millis(200));
        assert_idle(&stn, &format!("away link {i}"));
        let a = stn.st().depot.arrivals.len();
        assert!(a > last_arrivals, "link {i}: K7BOT's chat didn't reach Arrivals");
        last_arrivals = a;
    }
    let depot_bytes = std::fs::metadata(&stn.cfg.depot_file).map(|m| m.len()).unwrap_or(0);
    eprintln!("25 away links: depot.json {depot_bytes} B; {}", sizes(&stn.h));
}

/// The waterfall and the remote-listening tap stay bounded while the Demo
/// band plays for a while.
#[test]
fn demo_audio_buffers_bounded() {
    let stn = demo("N0AUD", |_| {});
    stn.h.pcm.lock().unwrap().on = true;
    let seq0 = stn.st().audio_seq;
    std::thread::sleep(Duration::from_secs(8));
    let g = stn.st();
    assert!(g.audio_seq > seq0 + 50, "waterfall not advancing ({} -> {})", seq0, g.audio_seq);
    assert!(g.audio_rows.len() <= 64, "audio_rows {}", g.audio_rows.len());
    drop(g);
    let n = stn.h.pcm.lock().unwrap().samples.len();
    assert!(n <= 16_000, "pcm tap holds {n} samples");
}

/// Settings → Save & connect in Demo restarts the station. Twenty restarts
/// must not leave sockets or threads behind.
#[test]
fn demo_restarts_do_not_leak() {
    let stn = demo("N0RST", |_| {});
    std::thread::sleep(Duration::from_secs(2));
    let (f0, t0) = (fds(), threads());
    for _ in 0..20 {
        stn.send(Cmd::Start(stn.cfg.clone()));
        std::thread::sleep(Duration::from_millis(400));
        stn.wait("restarted", 20.0, |s| s.running && s.mercury_ok);
    }
    std::thread::sleep(Duration::from_secs(3));
    let (f1, t1) = (fds(), threads());
    eprintln!("20 Demo restarts: fds {f0} -> {f1}, threads {t0} -> {t1}");
    assert!(f1 <= f0 + 10, "file descriptors {f0} -> {f1} after 20 restarts");
    assert!(t1 <= t0 + 5, "threads {t0} -> {t1} after 20 restarts");
}

/// A long Away soak (ignored by default): 400 links with chat, files both
/// ways and auto hang-ups. Prints the growth of every observable
/// collection.
#[test]
#[ignore = "soak"]
fn away_soak_400_links() {
    let stn = fast("N0SOK", FakeOptions::default(), |c| c.away_idle_s = 0.5);
    stn.send(Cmd::SetAway(true));
    let dir = stn.root.join("out");
    for i in 0..400 {
        link_up(&stn, 10.0);
        if i % 4 == 0 {
            let p = write_file(&dir, &format!("s{i}.txt"), b"soak\n");
            stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
        }
        if i % 5 == 0 {
            stn.send(Cmd::Chat("send me a file".into()));
        }
        stn.wait("hung up by itself", 20.0, |s| s.link.is_none());
        if i % 50 == 49 {
            let g = stn.st();
            eprintln!(
                "{:4} links: chat {:5} transfers {:4} log {} heard {} alerts {} arrivals {:4} json {:7} B partials {} fds {} threads {}",
                i + 1,
                g.chat.len(),
                g.transfers.len(),
                g.log.len(),
                g.heard.len(),
                g.mail_alerts.len(),
                g.depot.arrivals.len(),
                serde_json::to_vec(&*g).unwrap().len(),
                partials(&stn.dl()).len(),
                fds(),
                threads()
            );
        }
    }
    assert!(responsive(&stn.h));
    assert!(stn.st().log.len() <= 500);
}
