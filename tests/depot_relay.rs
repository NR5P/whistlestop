//! The Depot: dedup, hop limit, relay loop guard, own vs in-transit mail,
//! and depot.json persistence (including damaged files).
//! Intended behaviour: docs/USER-GUIDE.md "Depot" and "Store and forward".

use std::path::PathBuf;
use whistlestop::depot::*;
use whistlestop::protocol::Link;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ws-itest-depot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mail(id: u64, from: &str, to: &str, hops: u8) -> Mail {
    Mail {
        id,
        from: from.into(),
        to: to.into(),
        created: 1_800_000_000,
        text: format!("msg {id}"),
        read: true,
        delivered_at: None,
        via: None,
        hops,
        handed: None,
    }
}

/// The MAIL packet the engine builds from a Depot entry (engine.rs).
fn packet(m: &Mail) -> Link {
    Link::Mail {
        id: m.id,
        from: m.from.clone(),
        to: m.to.clone(),
        created: m.created,
        hops: m.hops,
        text: m.text.clone(),
    }
}

/// What the receiving relay stores (engine.rs `Link::Mail` handling).
fn received(l: &Link) -> Mail {
    let Link::Mail { id, from, to, created, hops, text } = l.clone() else { panic!() };
    Mail {
        id,
        from,
        to,
        created,
        text,
        read: false,
        delivered_at: None,
        via: None,
        hops: hops.saturating_add(1),
        handed: None,
    }
}

#[test]
fn persistence_round_trip_of_every_box() {
    let dir = tmp("roundtrip");
    let p = dir.join("depot.json");
    let mut d = Depot::load(p.clone());
    d.post("n0call", " k7bot ", "one ✓ \"quoted\" \n newline").unwrap();
    d.post_via("N0CALL", "W1AW", Some(" k7bot "), "via").unwrap();
    assert!(d.arrive(mail(5, "K1ABC", "N0CALL", 0)));
    assert!(d.take_in_transit(mail(6, "AA1AA", "W2XYZ", 1), "aa1aa"));
    let id = d.departures[0].id;
    d.confirm(id, "k7bot").unwrap();
    let back = Depot::load(p.clone());
    assert_eq!(back, d, "everything survives a reload");
    assert_eq!(back.departures[0].via.as_deref(), Some("K7BOT"));
    assert_eq!(back.transit[0].handed.as_deref(), Some("AA1AA"));
    assert_eq!(back.delivered[0].handed.as_deref(), Some("K7BOT"));
    assert!(back.delivered[0].delivered_at.is_some());
    assert_eq!(back.unread(), 1);
    // No temp file left behind.
    assert!(!dir.join("depot.json.tmp").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_file_and_missing_folder_start_empty_and_save_creates_it() {
    let dir = tmp("missing");
    let p = dir.join("a/b/c/depot.json");
    let mut d = Depot::load(p.clone());
    assert!(d.arrivals.is_empty() && d.departures.is_empty() && d.transit.is_empty() && d.delivered.is_empty());
    d.post("N0CALL", "K7BOT", "hi").unwrap();
    assert!(p.exists(), "save creates the folders");
    assert_eq!(Depot::load(p).departures.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unknown_fields_from_a_newer_version_are_ignored() {
    let dir = tmp("unknown");
    let p = dir.join("depot.json");
    std::fs::write(
        &p,
        r#"{
          "arrivals": [{"id": 1, "from": "K1ABC", "to": "N0CALL", "created": 5, "text": "hi",
                        "priority": "high", "attachments": [1,2,3]}],
          "departures": [], "delivered": [],
          "outbox_v2": {"anything": true}
        }"#,
    )
    .unwrap();
    let d = Depot::load(p);
    assert_eq!(d.arrivals.len(), 1);
    assert!(!d.arrivals[0].read && d.arrivals[0].hops == 0 && d.transit.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn older_file_without_transit_or_optional_fields_loads() {
    let dir = tmp("older");
    let p = dir.join("depot.json");
    std::fs::write(
        &p,
        r#"{"arrivals": [], "departures": [{"id": 9, "from": "N0CALL", "to": "K7BOT", "created": 1, "text": "x"}], "delivered": []}"#,
    )
    .unwrap();
    let d = Depot::load(p);
    assert_eq!(d.departures.len(), 1);
    assert_eq!(d.waiting_for("k7bot").len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// A damaged depot.json (cut short by a crash or a full disk, or edited by
/// hand) loads as an empty Depot, and the next save then overwrites it: all
/// the mail in it is gone for good. The User Guide promises "mail survives
/// restarts"; the damaged file should at least be kept (e.g. renamed to
/// depot.json.bad) rather than silently replaced.
#[test]
fn damaged_depot_file_is_not_overwritten() {
    let dir = tmp("damaged");
    let p = dir.join("depot.json");
    let mut d = Depot::load(p.clone());
    for i in 0..5 {
        d.post("N0CALL", "K7BOT", &format!("important {i}")).unwrap();
    }
    let good = std::fs::read(&p).unwrap();
    let damaged = good[..good.len() / 2].to_vec();
    std::fs::write(&p, &damaged).unwrap();
    let mut d = Depot::load(p.clone());
    d.post("N0CALL", "W1AW", "new").unwrap(); // any change saves
    let kept = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .any(|e| std::fs::read(e.path()).is_ok_and(|b| b == damaged));
    assert!(kept, "the damaged depot.json was overwritten; its 5 messages are lost");
    let _ = std::fs::remove_dir_all(dir);
}

/// A damaged depot.json that can't be moved aside (locked by a sync or
/// virus checker; here its new name would be too long) is left alone too:
/// the session's mail is kept in memory only.
#[test]
fn damaged_depot_that_cannot_be_set_aside_is_not_overwritten() {
    let dir = tmp("stuck");
    let p = dir.join(format!("{}.json", "d".repeat(250)));
    std::fs::write(&p, b"{\"arrivals\": [{\"id\": 1, \"from\": \"K7BOT\", \"text\": \"cut sho").unwrap();
    let mut d = Depot::load(p.clone());
    let why = d.load_problem.clone().unwrap_or_default();
    assert!(why.contains("memory only"), "{why}");
    d.post("N0CALL", "W1AW", "new").unwrap();
    assert_eq!(d.departures.len(), 1, "still works in memory");
    assert_eq!(
        std::fs::read(&p).unwrap(),
        b"{\"arrivals\": [{\"id\": 1, \"from\": \"K7BOT\", \"text\": \"cut sho"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn damaged_or_odd_files_never_panic() {
    let dir = tmp("odd");
    let p = dir.join("depot.json");
    for content in [
        &b""[..],
        b"null",
        b"[]",
        b"{}",
        b"{\"arrivals\": 5}",
        b"\xff\xfe\x00garbage",
        b"{\"arrivals\": [{\"id\": -1}]}",
        b"{\"arrivals\": [], \"departures\": [], \"delivered\": [], \"transit\": [{\"id\": 1, \"from\": \"A\", \"to\": \"B\", \"created\": 1, \"text\": \"x\", \"hops\": 300}]}",
    ] {
        std::fs::write(&p, content).unwrap();
        let d = Depot::load(p.clone());
        let _ = d.waiting_calls();
    }
    // The path is a folder: load and save quietly do nothing harmful.
    let d = Depot::load(dir.clone());
    d.save();
    assert!(dir.is_dir());
    let _ = std::fs::remove_file(dir.with_extension("json.tmp"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn repeat_deliveries_are_ignored() {
    let mut d = Depot::default();
    assert!(d.arrive(mail(1, "K1ABC", "N0CALL", 0)));
    // Same id again, even with other content (a retry after a lost ack).
    let mut again = mail(1, "K1ABC", "N0CALL", 2);
    again.text = "changed".into();
    assert!(!d.arrive(again));
    assert_eq!(d.arrivals.len(), 1);
    assert_eq!(d.arrivals[0].text, "msg 1");
    assert_eq!(d.unread(), 1);
    d.mark_read(1);
    assert_eq!(d.unread(), 0);
    d.mark_read(999); // unknown id: nothing happens
}

#[test]
fn arrivals_are_always_unread() {
    let mut d = Depot::default();
    let mut m = mail(3, "K1ABC", "N0CALL", 0);
    m.read = true;
    assert!(d.arrive(m));
    assert_eq!(d.unread(), 1);
}

#[test]
fn in_transit_dedup_and_loop_guard() {
    let mut d = Depot::default();
    assert!(d.take_in_transit(mail(7, "AA1AA", "W1AW", 1), "aa1aa"));
    assert!(!d.take_in_transit(mail(7, "AA1AA", "W1AW", 1), "BB2BB"), "already carrying");
    assert_eq!(d.transit.len(), 1);
    assert_eq!(d.transit[0].handed.as_deref(), Some("AA1AA"));
    // Delivered onward: a copy coming round the loop again is refused.
    d.confirm(7, "W1AW").unwrap();
    assert!(d.transit.is_empty());
    assert!(!d.take_in_transit(mail(7, "CC3CC", "W1AW", 2), "CC3CC"), "already delivered");
    // A via set by the sender is not carried on (it named *us*).
    let mut m = mail(8, "AA1AA", "W1AW", 1);
    m.via = Some("ME".into());
    assert!(d.take_in_transit(m, "AA1AA"));
    assert_eq!(d.transit[0].via, None);
}

#[test]
fn hop_limit_boundary() {
    let mut d = Depot::default();
    assert!(d.take_in_transit(mail(1, "A", "Z", 0), "A"));
    assert!(d.take_in_transit(mail(2, "A", "Z", MAX_HOPS - 1), "A"));
    // `hops` counts the carrying relay itself: the 3rd relay stores 3.
    assert!(d.take_in_transit(mail(3, "A", "Z", MAX_HOPS), "A"));
    assert!(!d.take_in_transit(mail(5, "A", "Z", MAX_HOPS + 1), "A"));
    assert!(!d.take_in_transit(mail(4, "A", "Z", u8::MAX), "A"));
}

/// USER-GUIDE: "a message is relayed at most **3** times"; PROTOCOL: "`hops`
/// counts relays (at most 3)". Following the engine's convention (a relay
/// stores `hops + 1`), the chain N0CALL → R1 → R2 → R3 → W1AW is refused at
/// R3: only two relays can ever carry a message. Either the docs or the
/// check is off by one.
#[test]
fn three_relays_may_carry_a_message() {
    let mut origin = Depot::default();
    origin.post_via("N0CALL", "W1AW", Some("R1"), "far away").unwrap();
    let mut pkt = packet(&origin.departures[0]);
    for relay in ["R1", "R2", "R3"] {
        let mut d = Depot::default();
        let m = received(&pkt);
        assert!(d.take_in_transit(m, "prev"), "{relay} refused the message (hops on the wire {:?})", pkt);
        pkt = packet(&d.transit[0]);
    }
}

#[test]
fn relay_chain_loop_is_broken_by_hops() {
    // A ring of relays that keep handing the mail on: it must stop.
    let mut d0 = Depot::default();
    d0.post("N0CALL", "NOWHERE", "lost").unwrap();
    let mut pkt = packet(&d0.departures[0]);
    let mut carried = 0;
    for _ in 0..20 {
        let mut d = Depot::default(); // a fresh station each time
        if !d.take_in_transit(received(&pkt), "x") {
            break;
        }
        carried += 1;
        pkt = packet(&d.transit[0]);
    }
    assert!(carried <= MAX_HOPS as usize, "carried {carried} times");
}

#[test]
fn own_mail_vs_relayed_mail_waiting_for() {
    let mut d = Depot::default();
    d.post("N0CALL", "K7BOT", "direct").unwrap();
    d.post_via("N0CALL", "W1AW", Some("K7BOT"), "via k7bot").unwrap();
    d.post("N0CALL", "W1AW", "direct to w1aw").unwrap();
    assert!(d.take_in_transit(mail(50, "AA1AA", "K7BOT", 1), "AA1AA"));
    assert!(d.take_in_transit(mail(51, "AA1AA", "W1AW", 1), "AA1AA"));
    let k: Vec<&str> = d.waiting_for("k7bot").iter().map(|m| m.text.as_str()).collect();
    assert_eq!(k, ["direct", "via k7bot", "msg 50"]);
    let w: Vec<&str> = d.waiting_for("W1AW").iter().map(|m| m.text.as_str()).collect();
    // Mail for W1AW routed via K7BOT is also handed to W1AW itself if we
    // meet it first.
    assert_eq!(w, ["via k7bot", "direct to w1aw", "msg 51"]);
    assert_eq!(d.waiting_calls(), vec!["K7BOT".to_string(), "W1AW".to_string()]);
    assert!(d.waiting_for("").is_empty());
}

#[test]
fn confirm_and_delete() {
    let mut d = Depot::default();
    let id = d.post("N0CALL", "K7BOT", "x").unwrap().id;
    assert!(d.confirm(424242, "K7BOT").is_none());
    assert!(d.confirm(id, "k7bot").is_some());
    assert!(d.confirm(id, "k7bot").is_none(), "second ack does nothing");
    assert_eq!(d.delivered.len(), 1);
    d.delete(id);
    assert!(d.delivered.is_empty());
    assert!(d.take_in_transit(mail(9, "A", "B", 0), "A"));
    d.delete(9);
    assert!(d.transit.is_empty());
}

#[test]
fn ids_are_unique_and_nonzero() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..20_000 {
        let id = new_id();
        assert_ne!(id, 0);
        assert!(seen.insert(id), "duplicate id");
    }
}

#[test]
fn post_normalises_callsigns() {
    let mut d = Depot::default();
    let m = d.post_via("n0call", "  w1aw ", Some("   "), "x").unwrap().clone();
    assert_eq!((m.from.as_str(), m.to.as_str(), m.via.as_deref()), ("N0CALL", "W1AW", None));
    assert_eq!(m.hops, 0);
    assert!(m.read, "own mail isn't 'new'");
}

/// The Depot (and `Cmd::DepotPost` in the engine, which only trims and
/// upper-cases) accepts any destination text. Such mail is put on the air
/// as a MAIL packet that every receiver refuses ("not a callsign"), and the
/// receiver then hangs up the link (see e2e_handshake.rs). With Via set,
/// every link with the relay is dropped each time the mail is retried. An
/// over-long call is silently cut to 12 characters on the wire, so it
/// reaches (or never reaches) a different station than the one written.
#[test]
fn posted_mail_always_makes_a_decodable_packet() {
    for to in ["W1AW X", "K1ABC.", "VE3/K1ABC/QRP", "ÅÄÖ", "W1AW\r"] {
        let mut d = Depot::default();
        let _ = d.post_via("N0CALL", to, Some("K7BOT"), "hello");
        // Either refused at posting, or the packet survives the trip intact.
        if let Some(m) = d.departures.first() {
            let got = Link::decode(&packet(m).body());
            match got {
                Ok(Link::Mail { to: t, .. }) => assert_eq!(t, m.to, "destination changed on the air"),
                other => panic!("mail to {to:?} gives an undecodable packet: {other:?}"),
            }
        }
    }
}

/// Mail over the longest message is refused when written (a packet over
/// 1 MB makes the other station hang up, and unconfirmed mail goes again
/// on every link); the longest allowed makes a packet that decodes.
#[test]
fn overlong_mail_is_refused_when_posted() {
    use whistlestop::protocol::{LinkDecoder, MAX_TEXT_BYTES};
    let mut d = Depot::default();
    let e = d.post("N0CALL", "K7BOT", &"x".repeat(MAX_TEXT_BYTES + 1)).unwrap_err();
    assert!(e.to_string().contains("as a file"), "{e}");
    let e = d.post("N0CALL", "K7BOT", &"x".repeat(2 << 20)).unwrap_err();
    assert!(e.to_string().contains("2048 KB"), "{e}");
    assert!(d.departures.is_empty());
    let longest = "é".repeat(MAX_TEXT_BYTES / 2);
    d.post("N0CALL", "K7BOT", &longest).unwrap();
    let wire = packet(&d.departures[0]).encode();
    let got = LinkDecoder::default().feed(&wire).unwrap();
    assert!(matches!(&got[..], [Link::Mail { text, .. }] if *text == longest));
}
