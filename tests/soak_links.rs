//! Many links in a row with the demo bot (K7BOT): connect, chat, files both
//! ways, cancels, disconnect, again and again. Nothing may leak from one
//! link into the next (calling / incoming_call / PTT stuck, transfer rows
//! left live, encryption state), and chat must stay prompt however many
//! files went before it.
//! Intended behaviour: docs/USER-GUIDE.md "Chat" and "Files".

mod soak_common;

use soak_common::*;
use std::time::{Duration, Instant};
use whistlestop::e2e::Policy;
use whistlestop::engine::{Cmd, Dir, E2eView, XferState};
use whistlestop::fake_mercury::FakeOptions;

const WELCOME: &str = "welcome-from-k7bot.txt";

fn rows_named(stn: &Stn, dir: Dir, name: &str) -> usize {
    stn.st().transfers.iter().filter(|t| t.dir == dir && t.name == name).count()
}

/// Ask K7BOT for its file and wait for the new offer. Returns its fid.
fn request_bot_file(stn: &Stn, secs: f64) -> u64 {
    let before = rows_named(stn, Dir::In, WELCOME);
    stn.send(Cmd::Chat("please send me a file".into()));
    stn.wait("K7BOT's file offer", secs, |s| {
        s.transfers.iter().filter(|t| t.dir == Dir::In && t.name == WELCOME).count() > before
    });
    newest_xfer(&stn.st(), Dir::In, WELCOME).unwrap().fid
}

/// Offer a file to K7BOT and wait for "delivered".
fn send_file(stn: &Stn, name: &str, data: &[u8], secs: f64) -> f64 {
    let p = write_file(&stn.root.join("out"), name, data);
    let t = Instant::now();
    stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
    stn.wait(&format!("{name} delivered"), secs, |s| {
        newest_xfer(s, Dir::Out, name).is_some_and(|t| t.state == XferState::Done(None))
    });
    t.elapsed().as_secs_f64()
}

/// 40 links on the fast fake: chat every time, a file out every 3rd link,
/// K7BOT's file in every 4th, a big file cancelled mid-way every 5th, and
/// a file queued while idle (offered when the next link comes up).
#[test]
fn fast_forty_links_chat_files_cancel() {
    let stn = fast("N0LNK", FakeOptions::default(), |_| {});
    let mut connect_s: Vec<f64> = vec![];
    let mut down_s: Vec<f64> = vec![];
    let mut chat_s: Vec<f64> = vec![];
    for i in 0..40 {
        connect_s.push(link_up(&stn, 10.0));
        assert_eq!(
            stn.st().link.as_ref().map(|l| l.e2e.clone()),
            Some(E2eView::Off),
            "link {i}: encryption off in Settings"
        );
        chat_s.push(chat_roundtrip(&stn, &format!("ping {i}"), 10.0));
        if i % 3 == 0 {
            send_file(&stn, &format!("out-{i}.bin"), &noise(3000 + i * 50, i as u64), 20.0);
        }
        if i % 4 == 1 {
            let fid = request_bot_file(&stn, 10.0);
            stn.send(Cmd::AnswerOffer { fid, accept: true });
            stn.wait("K7BOT's file saved", 10.0, |s| {
                newest_xfer(s, Dir::In, WELCOME)
                    .is_some_and(|t| matches!(t.state, XferState::Done(Some(_))))
            });
        }
        if i % 5 == 2 {
            // A big file, cancelled once it's moving.
            let name = format!("big-{i}.bin");
            let p = write_file(&stn.root.join("out"), &name, &noise(1_500_000, 99 + i as u64));
            stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
            stn.wait("big file moving", 20.0, |s| {
                newest_xfer(s, Dir::Out, &name)
                    .is_some_and(|t| t.state == XferState::Sending && t.done > 20_000)
            });
            let t = newest_xfer(&stn.st(), Dir::Out, &name).unwrap();
            assert!(t.done < t.size, "link {i}: the big file finished before the cancel");
            stn.send(Cmd::Cancel { fid: t.fid, dir: Dir::Out });
            stn.wait("cancel shown", 5.0, |s| {
                newest_xfer(s, Dir::Out, &name)
                    .is_some_and(|t| t.state == XferState::Failed("cancelled".into()))
            });
            let done_then = newest_xfer(&stn.st(), Dir::Out, &name).unwrap().done;
            // Chat right after a cancel goes out at once.
            chat_s.push(chat_roundtrip(&stn, &format!("after cancel {i}"), 10.0));
            let after = newest_xfer(&stn.st(), Dir::Out, &name).unwrap();
            assert_eq!(after.state, XferState::Failed("cancelled".into()), "link {i}");
            assert!(after.done <= done_then + 4096, "link {i}: kept sending after cancel");
        }
        if i % 10 == 7 {
            stn.wait("file queued while idle delivered", 20.0, |s| {
                newest_xfer(s, Dir::Out, &format!("queued-{}.bin", i - 1))
                    .is_some_and(|t| t.state == XferState::Done(None))
            });
        }
        down_s.push(link_down(&stn, 10.0));
        std::thread::sleep(Duration::from_millis(150));
        assert_idle(&stn, &format!("after link {i}"));
        eprintln!("link {i}: connect {:.2} s, down {:.2} s", connect_s[i], down_s[i]);
        if i % 10 == 6 {
            // Queued while idle: goes out on the next link.
            let p = write_file(&stn.root.join("out"), &format!("queued-{i}.bin"), &noise(5000, 7));
            stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
            stn.wait("queued offer listed", 5.0, |s| {
                newest_xfer(s, Dir::Out, &format!("queued-{i}.bin"))
                    .is_some_and(|t| t.state == XferState::Offered)
            });
        }
    }
    assert!(responsive(&stn.h));
    let max = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
    eprintln!(
        "40 links: connect max {:.2} s, chat delivery max {:.2} s (last {:.2}), disconnect max {:.2} s; {}",
        max(&connect_s),
        max(&chat_s),
        chat_s.last().unwrap(),
        max(&down_s),
        sizes(&stn.h)
    );
    assert!(max(&chat_s) < 3.0, "chat delivery got slow: {chat_s:?}");
    assert!(max(&down_s) < 3.0, "disconnect got slow: {down_s:?}");
    assert!(stn.st().log.len() <= 500);
    // Nothing was cancelled on the receiving side: no partial files.
    assert!(partials(&stn.dl()).is_empty(), "{:?}", partials(&stn.dl()));
}

/// A file going out when the link drops is offered again on the next link
/// and completes; the row ends Done, not stuck at "link ended".
#[test]
fn fast_link_drop_mid_send_resends_next_link() {
    let stn = fast("N0DRP", FakeOptions::default(), |_| {});
    for round in 0..5 {
        link_up(&stn, 10.0);
        let name = format!("drop-{round}.bin");
        let p = write_file(&stn.root.join("out"), &name, &noise(1_200_000, round));
        stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
        stn.wait("moving", 20.0, |s| {
            newest_xfer(s, Dir::Out, &name).is_some_and(|t| t.done > 50_000)
        });
        link_down(&stn, 10.0);
        stn.wait("row shows the drop", 5.0, |s| {
            newest_xfer(s, Dir::Out, &name).is_some_and(|t| {
                matches!(&t.state, XferState::Failed(w) if w.contains("will resume"))
            })
        });
        assert_idle(&stn, &format!("round {round} after the drop"));
        link_up(&stn, 10.0);
        stn.wait("re-offered and delivered", 60.0, |s| {
            newest_xfer(s, Dir::Out, &name).is_some_and(|t| t.state == XferState::Done(None))
        });
        chat_roundtrip(&stn, &format!("still here {round}"), 5.0);
        link_down(&stn, 10.0);
        assert_idle(&stn, &format!("round {round}"));
    }
}

/// The product's Demo mode, real-time: six links with a file out, K7BOT's
/// file in, a receive cancelled mid-way and then resumed, a send cancelled
/// mid-way, and plain chat.
#[test]
fn demo_six_links_files_cancel_resume() {
    let stn = demo("N0DLK", |_| {});
    let dl = stn.dl();
    // 1: chat and a small file out.
    link_up(&stn, 30.0);
    chat_roundtrip(&stn, "first link", 30.0);
    send_file(&stn, "small.txt", b"QSO log line\n".repeat(30).as_slice(), 60.0);
    link_down(&stn, 30.0);
    assert_idle_opts(&stn, "link 1", false);

    // 2: K7BOT's file, cancelled half way: a partial is kept for resume.
    link_up(&stn, 30.0);
    let fid = request_bot_file(&stn, 30.0);
    stn.send(Cmd::AnswerOffer { fid, accept: true });
    stn.wait("receiving part of it", 60.0, |s| {
        newest_xfer(s, Dir::In, WELCOME).is_some_and(|t| t.done > 0 && t.done < t.size / 2)
    });
    stn.send(Cmd::Cancel { fid, dir: Dir::In });
    stn.wait("cancel shown", 5.0, |s| {
        newest_xfer(s, Dir::In, WELCOME).is_some_and(|t| t.state == XferState::Failed("cancelled".into()))
    });
    let kept: u64 = partials(&dl).iter().map(|p| p.1).sum();
    assert!(kept > 0, "a cancelled receive keeps its partial: {:?}", partials(&dl));
    // K7BOT stops sending on our CANCEL (within a chunk); the chat echo
    // shows the link still works before hanging up.
    chat_roundtrip(&stn, "after the cancel in", 60.0);
    link_down(&stn, 30.0);
    assert_idle_opts(&stn, "link 2", false);

    // 3: the same file again resumes from the partial and completes; the
    // partial is gone afterwards.
    link_up(&stn, 30.0);
    let fid = request_bot_file(&stn, 30.0);
    stn.wait("the second offer resumes", 5.0, |s| {
        s.log.iter().any(|l| l.1.contains("already here — will resume"))
    });
    stn.send(Cmd::AnswerOffer { fid, accept: true });
    stn.wait("resumed file saved", 60.0, |s| {
        newest_xfer(s, Dir::In, WELCOME).is_some_and(|t| matches!(t.state, XferState::Done(Some(_))))
    });
    assert!(partials(&dl).is_empty(), "{:?}", partials(&dl));
    link_down(&stn, 30.0);
    assert_idle_opts(&stn, "link 3", false);

    // 4: a send cancelled mid-way, then chat goes out at once.
    link_up(&stn, 30.0);
    let p = write_file(&stn.root.join("out"), "cancel-me.bin", &noise(6000, 3));
    stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
    stn.wait("sending", 30.0, |s| {
        newest_xfer(s, Dir::Out, "cancel-me.bin").is_some_and(|t| t.state == XferState::Sending)
    });
    let fid = newest_xfer(&stn.st(), Dir::Out, "cancel-me.bin").unwrap().fid;
    stn.send(Cmd::Cancel { fid, dir: Dir::Out });
    let d = chat_roundtrip(&stn, "after the cancel", 40.0);
    assert!(d < 20.0, "chat after a cancel took {d:.1} s");
    assert_eq!(
        newest_xfer(&stn.st(), Dir::Out, "cancel-me.bin").unwrap().state,
        XferState::Failed("cancelled".into())
    );
    link_down(&stn, 30.0);
    assert_idle_opts(&stn, "link 4", false);

    // 5, 6: plain chat.
    for i in 5..=6 {
        link_up(&stn, 30.0);
        chat_roundtrip(&stn, &format!("link {i}"), 30.0);
        link_down(&stn, 30.0);
        assert_idle_opts(&stn, &format!("link {i}"), false);
    }
    assert!(responsive(&stn.h));
}

/// Demo: cancel a file K7BOT is sending, hang up at once, call again. The
/// new link must start clean: K7BOT's greeting arrives and chat works.
/// Seen instead: the next link starts with the rest of the old file's
/// bytes ("⚠ Skipped a packet … (packet too short)", then a bogus "K7BOT
/// asked to QSY to 0.105 kHz (…file text…)"), the greeting is lost and
/// the stream stays misframed.
#[test]
fn demo_cancelled_receive_then_hang_up_next_link_clean() {
    let stn = demo("N0PSN", |_| {});
    link_up(&stn, 30.0);
    let fid = request_bot_file(&stn, 30.0);
    stn.send(Cmd::AnswerOffer { fid, accept: true });
    stn.wait("receiving part of it", 60.0, |s| {
        newest_xfer(s, Dir::In, WELCOME).is_some_and(|t| t.done > 0 && t.done < t.size / 2)
    });
    stn.send(Cmd::Cancel { fid, dir: Dir::In });
    link_down(&stn, 30.0);
    link_up(&stn, 30.0);
    chat_roundtrip(&stn, "clean start?", 30.0);
    assert!(!has_log(&stn.h, "didn't make sense"), "{}", dump(&stn.h));
}

/// Thirty Demo links in a row (about 7 minutes). Chat latency must not
/// creep up from the first links to the last.
#[test]
#[ignore = "soak"]
fn demo_thirty_links() {
    let stn = demo("N0D30", |_| {});
    let mut lat = vec![];
    for i in 0..30 {
        link_up(&stn, 30.0);
        lat.push(chat_roundtrip(&stn, &format!("soak {i}"), 30.0));
        if i % 5 == 0 {
            send_file(&stn, &format!("d30-{i}.txt"), format!("line {i}\n").repeat(40).as_bytes(), 60.0);
        }
        link_down(&stn, 30.0);
        std::thread::sleep(Duration::from_millis(300));
        assert_idle_opts(&stn, &format!("link {i}"), false);
    }
    let first: f64 = lat[..5].iter().sum::<f64>() / 5.0;
    let last: f64 = lat[25..].iter().sum::<f64>() / 5.0;
    eprintln!("demo 30 links: chat delivery first5 {first:.2} s, last5 {last:.2} s; {}", sizes(&stn.h));
    assert!(last < first * 2.0 + 1.0, "latency crept up: {lat:?}");
}

/// Encryption on (Prefer) across links: every link gets new keys, chat and
/// files work, and nothing of the old session is left.
#[test]
fn fast_encrypted_links_back_to_back() {
    let stn = fast(
        "N0ENC",
        FakeOptions { bot_e2e: Policy::Prefer, ..Default::default() },
        |c| c.e2e = Policy::Prefer,
    );
    let mut codes = std::collections::HashSet::new();
    for i in 0..15 {
        link_up(&stn, 10.0);
        stn.wait("encrypted", 10.0, |s| {
            matches!(s.link.as_ref().map(|l| &l.e2e), Some(E2eView::Encrypted { .. }))
        });
        let sas = match stn.st().link.as_ref().map(|l| l.e2e.clone()) {
            Some(E2eView::Encrypted { sas }) => sas,
            other => panic!("{other:?}"),
        };
        assert!(codes.insert(sas.clone()), "link {i}: check code {sas} repeated");
        chat_roundtrip(&stn, &format!("sealed {i}"), 10.0);
        if i % 3 == 0 {
            send_file(&stn, &format!("sealed-{i}.bin"), &noise(4000, i as u64), 20.0);
            let fid = request_bot_file(&stn, 10.0);
            stn.send(Cmd::AnswerOffer { fid, accept: true });
            stn.wait("sealed file in", 10.0, |s| {
                newest_xfer(s, Dir::In, WELCOME).is_some_and(|t| matches!(t.state, XferState::Done(Some(_))))
            });
        }
        link_down(&stn, 10.0);
        assert_idle(&stn, &format!("encrypted link {i}"));
    }
}

/// Demo: after Disconnect (73) the transmit indicator goes off. Seen: it
/// can stay lit for good, because the Demo modem hangs up on our BYE
/// without a PTT OFF, the simulated radio was keyed by Mercury's PTT ON,
/// and the radio poll sets `tx` again from it.
#[test]
fn demo_ptt_released_after_links() {
    let stn = demo("N0PTT", |_| {});
    for i in 0..6 {
        link_up(&stn, 30.0);
        chat_roundtrip(&stn, &format!("ptt {i}"), 30.0);
        link_down(&stn, 30.0);
        let off = try_wait(&stn.h, 3.0, |s| !s.tx && !s.rig.ptt);
        assert!(off, "link {i}: PTT still on 3 s after the link ended\n{}", dump(&stn.h));
    }
}
