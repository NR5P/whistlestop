//! Encryption policies across many links with K7BOT (whose own policy is
//! set per test): every combination gives the documented outcome on every
//! link, nothing goes out in plain text under Require, and no session
//! state survives into the next link.
//! Intended behaviour: docs/USER-GUIDE.md "Encryption (optional)".

mod soak_common;

use soak_common::*;
use std::time::Duration;
use whistlestop::e2e::Policy;
use whistlestop::engine::{Cmd, Dir, E2eView, Who, XferState};
use whistlestop::fake_mercury::FakeOptions;

fn bot(p: Policy) -> FakeOptions {
    FakeOptions { bot_e2e: p, ..Default::default() }
}

fn view(stn: &Stn) -> Option<E2eView> {
    stn.st().link.as_ref().map(|l| l.e2e.clone())
}

/// Both sides willing (Prefer/Require x Prefer/Require): encrypted every
/// time, fresh code every link, chat and a file work.
#[test]
fn willing_pairs_encrypt_every_link() {
    for (ours, theirs) in [
        (Policy::Prefer, Policy::Prefer),
        (Policy::Require, Policy::Prefer),
        (Policy::Prefer, Policy::Require),
        (Policy::Require, Policy::Require),
    ] {
        let stn = fast(&format!("N0E{}{}", ours as u8, theirs as u8), bot(theirs), |c| c.e2e = ours);
        let mut codes = std::collections::HashSet::new();
        for i in 0..8 {
            stn.send(Cmd::Connect("K7BOT".into()));
            stn.wait("encrypted link", 10.0, |s| {
                matches!(s.link.as_ref().map(|l| &l.e2e), Some(E2eView::Encrypted { .. }))
            });
            let Some(E2eView::Encrypted { sas }) = view(&stn) else { unreachable!() };
            assert!(codes.insert(sas.clone()), "{ours:?}/{theirs:?} link {i}: code repeated");
            stn.wait("K7BOT reads back the code", 10.0, |s| {
                s.chat.iter().any(|c| c.who == Who::Them && c.text.contains(&sas))
            });
            chat_roundtrip(&stn, &format!("{ours:?}-{theirs:?}-{i}"), 10.0);
            if i % 4 == 0 {
                let name = format!("e-{i}.bin");
                let p = write_file(&stn.root.join("out"), &name, &noise(9000, i));
                stn.send(Cmd::SendFiles { paths: vec![p], zip: false });
                stn.wait("sealed file delivered", 20.0, |s| {
                    newest_xfer(s, Dir::Out, &name).is_some_and(|t| t.state == XferState::Done(None))
                });
            }
            link_down(&stn, 10.0);
            std::thread::sleep(Duration::from_millis(100));
            assert_idle(&stn, &format!("{ours:?}/{theirs:?} link {i}"));
        }
    }
}

/// Prefer here, encryption off at K7BOT: plain text, clearly marked, and
/// chat works, on every link.
#[test]
fn prefer_with_plain_peer_falls_back_every_link() {
    let stn = fast("N0PFP", bot(Policy::Off), |c| c.e2e = Policy::Prefer);
    for i in 0..10 {
        link_up(&stn, 10.0);
        stn.wait("marked plain", 5.0, |s| {
            matches!(s.link.as_ref().map(|l| &l.e2e), Some(E2eView::Plain { .. }))
        });
        chat_roundtrip(&stn, &format!("plain {i}"), 10.0);
        link_down(&stn, 10.0);
        assert_idle(&stn, &format!("link {i}"));
    }
}

/// Require here, encryption off at K7BOT: every link is ended at once with
/// the reason, and a chat typed while the link was up never goes out.
#[test]
fn require_with_plain_peer_hangs_up_every_link() {
    let stn = fast("N0RQP", bot(Policy::Off), |c| c.e2e = Policy::Require);
    for i in 0..10 {
        let before = stn.st().chat.len();
        stn.send(Cmd::Connect("K7BOT".into()));
        stn.wait("link comes up", 10.0, |s| s.link.is_some() || s.chat.len() > before);
        stn.send(Cmd::Chat(format!("secret {i}")));
        stn.wait("link ended by policy", 10.0, |s| s.link.is_none());
        stn.wait("reason shown", 5.0, |s| {
            s.chat.iter().skip(before).any(|c| c.who == Who::System && c.text.contains("requires it"))
        });
        std::thread::sleep(Duration::from_millis(300));
        let g = stn.st();
        // Nothing from K7BOT's plain session is shown, and our line was
        // never delivered (K7BOT would have echoed it).
        assert!(
            !g.chat.iter().skip(before).any(|c| c.who == Who::Them),
            "link {i}: plain text from the peer accepted under Require"
        );
        assert!(
            !g.chat.iter().any(|c| c.who == Who::Me && c.text == format!("secret {i}") && c.delivered),
            "link {i}: chat delivered under Require with a plain peer"
        );
        drop(g);
        assert_idle(&stn, &format!("link {i}"));
    }
    assert!(responsive(&stn.h));
}

/// Encryption off here, K7BOT requires it: K7BOT hangs up; our view is Off
/// and everything is clean for the next link.
#[test]
fn off_with_requiring_peer_is_hung_up_every_link() {
    let stn = fast("N0OFR", bot(Policy::Require), |c| c.e2e = Policy::Off);
    for i in 0..10 {
        let before = stn.st().chat.len();
        stn.send(Cmd::Connect("K7BOT".into()));
        stn.wait("link up then down", 10.0, |s| {
            s.link.is_none() && s.chat.iter().skip(before).any(|c| c.text.contains("ended"))
        });
        assert_idle(&stn, &format!("link {i}"));
    }
}

/// The policy changed between links (Update, as the Settings window's
/// non-restart settings do): each link follows the policy in force when it
/// came up.
#[test]
fn policy_switch_between_links() {
    let stn = fast("N0SWP", bot(Policy::Prefer), |_| {});
    let order = [Policy::Off, Policy::Prefer, Policy::Require, Policy::Off, Policy::Require, Policy::Prefer];
    for (i, p) in order.iter().cycle().take(18).enumerate() {
        let mut c = stn.cfg.clone();
        c.e2e = *p;
        stn.send(Cmd::Update(c));
        link_up(&stn, 10.0);
        let want_encrypted = *p != Policy::Off;
        if want_encrypted {
            stn.wait("encrypted", 10.0, |s| {
                matches!(s.link.as_ref().map(|l| &l.e2e), Some(E2eView::Encrypted { .. }))
            });
        } else {
            assert_eq!(view(&stn), Some(E2eView::Off), "link {i} with {p:?}");
        }
        chat_roundtrip(&stn, &format!("switch {i}"), 10.0);
        link_down(&stn, 10.0);
        assert_idle(&stn, &format!("link {i} {p:?}"));
    }
}

/// Links that end in the middle of the key exchange (we hang up as soon as
/// the link shows): the next link starts a fresh exchange and encrypts.
#[test]
fn links_cut_during_key_exchange() {
    let stn = fast("N0CUT", bot(Policy::Prefer), |c| c.e2e = Policy::Prefer);
    for i in 0..15 {
        stn.send(Cmd::Connect("K7BOT".into()));
        stn.wait("link", 10.0, |s| s.link.is_some());
        stn.send(Cmd::Disconnect);
        stn.wait("down", 10.0, |s| s.link.is_none());
        std::thread::sleep(Duration::from_millis(100));
        assert_idle(&stn, &format!("cut {i}"));
        link_up(&stn, 10.0);
        stn.wait("encrypted after a cut", 10.0, |s| {
            matches!(s.link.as_ref().map(|l| &l.e2e), Some(E2eView::Encrypted { .. }))
        });
        chat_roundtrip(&stn, &format!("after cut {i}"), 10.0);
        link_down(&stn, 10.0);
        assert_idle(&stn, &format!("after cut {i}"));
    }
}

/// A packet from K7BOT still in flight when the link ends (here: its
/// RECEIVED for a chat typed just before Disconnect) arrives after
/// Mercury's DISCONNECTED line: the data and control ports are separate
/// sockets, read by separate tasks (tnc.rs). The engine keeps it in
/// `early` and replays it into the NEXT link, whoever that is with. Under
/// encryption the next link dies at once ("encrypted packet, but no keys
/// were agreed"); in plain text the old station's packet would be taken
/// as the new one's.
#[test]
fn hangup_race_does_not_poison_the_next_link() {
    let stn = fast("N0RAC", bot(Policy::Prefer), |c| c.e2e = Policy::Prefer);
    let mut poisoned = 0;
    for i in 0..30 {
        stn.send(Cmd::Connect("K7BOT".into()));
        stn.wait("encrypted", 10.0, |s| {
            s.link.is_none() && s.log.iter().any(|l| l.1.contains("no keys were agreed"))
                || matches!(s.link.as_ref().map(|l| &l.e2e), Some(E2eView::Encrypted { .. }))
        });
        if stn.st().link.is_none() {
            poisoned += 1;
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }
        // Chat and hang up at once: K7BOT's RECEIVED crosses the hang-up.
        stn.send(Cmd::Chat(format!("race {i}")));
        stn.send(Cmd::Disconnect);
        stn.wait("down", 10.0, |s| s.link.is_none());
        std::thread::sleep(Duration::from_millis(300));
    }
    let n = logs(&stn.h).iter().filter(|l| l.contains("no keys were agreed")).count();
    assert!(poisoned == 0 && n == 0, "{poisoned} of 30 links killed by the previous link's packet\n{}", dump(&stn.h));
}
