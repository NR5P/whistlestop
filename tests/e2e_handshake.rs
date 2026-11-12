//! End-to-end encryption channel: policy matrix, tampering, replay,
//! reordering, wrong suites, commit/reveal mismatches, long sessions.
//! Intended behaviour: docs/PROTOCOL.md, "Encryption".

use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use whistlestop::e2e::*;
use whistlestop::protocol::*;

fn hello(name: &str) -> Link {
    Link::Hello {
        version: VERSION,
        caps: CAP_RESUME | CAP_ZIP,
        name: name.into(),
        grid: "EM12".into(),
        app: "itest".into(),
    }
}

fn commitment(pubkey: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"WSP1 E2E commit");
    h.update([SUITE]);
    h.update(pubkey);
    h.finalize().into()
}

/// Split wire bytes into packet bodies.
fn bodies(wire: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut d = LinkDecoder::default();
    d.feed_bodies(&wire.concat()).unwrap()
}

/// Two stations joined by an in-order pipe; `tap` may rewrite or drop
/// bodies in flight (a man in the middle).
struct Pair {
    a: Channel,
    b: Channel,
    to_a: Vec<Vec<u8>>,
    to_b: Vec<Vec<u8>>,
    a_app: Vec<Link>,
    b_app: Vec<Link>,
    a_notes: Vec<Notice>,
    b_notes: Vec<Notice>,
}

type Tap<'a> = &'a mut dyn FnMut(bool, Vec<u8>) -> Option<Vec<u8>>;

impl Pair {
    fn new(pa: Policy, pb: Policy, now: Instant) -> Pair {
        let (a, oa) = Channel::open(pa, hello("A"), now);
        let (b, ob) = Channel::open(pb, hello("B"), now);
        Pair {
            a,
            b,
            to_a: bodies(&ob),
            to_b: bodies(&oa),
            a_app: vec![],
            b_app: vec![],
            a_notes: vec![],
            b_notes: vec![],
        }
    }
    /// Run until quiet. Err names the side ("A: ..." / "B: ...") that failed.
    fn run_tapped(&mut self, tap: Tap) -> Result<(), String> {
        let mut guard = 0;
        while !self.to_a.is_empty() || !self.to_b.is_empty() {
            guard += 1;
            assert!(guard < 1000, "handshake does not settle");
            for body in std::mem::take(&mut self.to_b) {
                let Some(body) = tap(true, body) else { continue };
                let s = self.b.on_body(&body).map_err(|e| format!("B: {e}"))?;
                self.to_a.extend(bodies(&s.send));
                self.b_notes.extend(s.notice);
                self.b_app.extend(s.app);
            }
            for body in std::mem::take(&mut self.to_a) {
                let Some(body) = tap(false, body) else { continue };
                let s = self.a.on_body(&body).map_err(|e| format!("A: {e}"))?;
                self.to_b.extend(bodies(&s.send));
                self.a_notes.extend(s.notice);
                self.a_app.extend(s.app);
            }
        }
        Ok(())
    }
    fn run(&mut self) -> Result<(), String> {
        self.run_tapped(&mut |_, b| Some(b))
    }
    fn a_sends(&mut self, p: &Link) {
        let w = self.a.wrap(p).expect("ready");
        self.to_b.extend(bodies(&[w]));
    }
    fn b_sends(&mut self, p: &Link) {
        let w = self.b.wrap(p).expect("ready");
        self.to_a.extend(bodies(&[w]));
    }
}

fn secure(now: Instant) -> Pair {
    let mut p = Pair::new(Policy::Require, Policy::Require, now);
    p.run().unwrap();
    assert!(p.a.encrypted() && p.b.encrypted());
    p
}

fn chat(id: u64, t: &str) -> Link {
    Link::Chat { id, text: t.into() }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Plain,
    Encrypted,
    Refused,
}

/// The full Off/Prefer/Require matrix, as PROTOCOL.md describes it.
#[test]
fn policy_matrix() {
    use Outcome::*;
    use Policy::*;
    let want = |a: Policy, b: Policy| match (a, b) {
        (Off, Off) | (Off, Prefer) | (Prefer, Off) => Plain,
        (Off, Require) | (Require, Off) => Refused,
        _ => Encrypted,
    };
    for pa in [Off, Prefer, Require] {
        for pb in [Off, Prefer, Require] {
            let now = Instant::now();
            let mut p = Pair::new(pa, pb, now);
            let r = p.run();
            let got = match r {
                Err(ref e) => {
                    // Only the Require side refuses, and it says why.
                    let side = if pa == Require { "A:" } else { "B:" };
                    assert!(e.starts_with(side) && e.contains("requires"), "{pa:?}/{pb:?}: {e}");
                    Refused
                }
                Ok(()) if p.a.encrypted() && p.b.encrypted() => Encrypted,
                Ok(()) => {
                    assert!(p.a.ready() && p.b.ready(), "{pa:?}/{pb:?} plain but not ready");
                    assert!(!p.a.encrypted() && !p.b.encrypted());
                    Plain
                }
            };
            assert_eq!(got, want(pa, pb), "{pa:?} vs {pb:?}");
            // Both sides saw the other's HELLO when the link came up.
            if got != Refused {
                assert!(matches!(p.a_app.first(), Some(Link::Hello { .. })));
                assert!(matches!(p.b_app.first(), Some(Link::Hello { .. })));
                // A Prefer side that went plain is told so.
                for (pol, notes) in [(pa, &p.a_notes), (pb, &p.b_notes)] {
                    if pol == Prefer && got == Plain {
                        assert!(matches!(notes[..], [Notice::Plain { .. }]), "{notes:?}");
                    }
                    if got == Encrypted {
                        assert!(matches!(notes[..], [Notice::Secure { .. }]), "{notes:?}");
                    }
                }
                // Application data flows both ways.
                p.a_sends(&chat(1, "from A"));
                p.b_sends(&chat(1, "from B"));
                p.run().unwrap();
                assert_eq!(p.b_app.last(), Some(&chat(1, "from A")));
                assert_eq!(p.a_app.last(), Some(&chat(1, "from B")));
            }
            if got == Encrypted {
                assert_eq!(p.a.sas(), p.b.sas());
            }
        }
    }
}

#[test]
fn hello_caps_follow_policy() {
    for (pol, set) in [(Policy::Off, false), (Policy::Prefer, true), (Policy::Require, true)] {
        let (_, out) = Channel::open(
            pol,
            Link::Hello { version: 1, caps: 0xFF, name: "x".into(), grid: "".into(), app: "".into() },
            Instant::now(),
        );
        let b = bodies(&out);
        let Link::Hello { caps, .. } = Link::decode(&b[0]).unwrap() else { panic!() };
        assert_eq!(caps & CAP_E2E != 0, set, "{pol:?}");
        assert_eq!(caps & !CAP_E2E, 0xFF & !CAP_E2E, "other caps kept");
        // Off sends only HELLO; the others add KEYC.
        assert_eq!(b.len(), if pol == Policy::Off { 1 } else { 2 });
    }
}

#[test]
fn many_packets_keep_nonce_counters_in_step() {
    let mut p = secure(Instant::now());
    // Interleave sends from both sides, in bursts, 3000 packets.
    for i in 0..3000u64 {
        if i % 3 == 0 {
            p.b_sends(&chat(i, &format!("b{i}")));
        } else {
            p.a_sends(&chat(i, &format!("a{i}")));
        }
        if i % 7 == 0 {
            p.run().unwrap();
        }
    }
    p.run().unwrap();
    let a_got = p.a_app.iter().filter(|l| matches!(l, Link::Chat { .. })).count();
    let b_got = p.b_app.iter().filter(|l| matches!(l, Link::Chat { .. })).count();
    assert_eq!((a_got, b_got), (1000, 2000));
    // Identical plaintexts never give identical ciphertexts (fresh nonce).
    let w1 = p.a.wrap(&chat(1, "same")).unwrap();
    let w2 = p.a.wrap(&chat(1, "same")).unwrap();
    assert_ne!(w1, w2);
}

#[test]
fn dropped_packet_breaks_the_session() {
    let mut p = secure(Instant::now());
    let _lost = p.a.wrap(&chat(1, "lost")).unwrap();
    p.a_sends(&chat(2, "next"));
    let e = p.run().unwrap_err();
    assert!(e.starts_with("B:") && e.contains("authentication"), "{e}");
}

#[test]
fn every_single_bit_flip_in_a_sealed_packet_is_caught() {
    let now = Instant::now();
    let mut p = secure(now);
    let w = p.a.wrap(&chat(1, "hello there")).unwrap();
    let body = bodies(&[w]).remove(0);
    for i in 1..body.len() {
        for bit in 0..8 {
            let mut fresh = secure(now);
            let w = fresh.a.wrap(&chat(1, "hello there")).unwrap();
            let mut b = bodies(&[w]).remove(0);
            b[i] ^= 1 << bit;
            assert!(fresh.b.on_body(&b).is_err(), "flip at {i}.{bit} accepted");
        }
    }
    // Flipping the type byte turns it into something else entirely, which
    // on an encrypted link is refused too.
    let mut fresh = secure(now);
    let w = fresh.a.wrap(&chat(1, "hello there")).unwrap();
    let mut b = bodies(&[w]).remove(0);
    for t in 0u8..=255 {
        if t == T_SEALED {
            continue;
        }
        b[0] = t;
        let r = fresh.b.on_body(&b);
        if let Ok(step) = r {
            // Only an unknown type may be skipped silently.
            assert!(step.app.is_empty(), "type {t:#x} delivered {:?}", step.app);
        }
    }
}

#[test]
fn sealed_packets_cannot_be_reflected_back() {
    // A's own sealed packet sent back to A uses the wrong direction key.
    let mut p = secure(Instant::now());
    let w = p.a.wrap(&chat(1, "echo")).unwrap();
    let b = bodies(&[w]).remove(0);
    assert!(p.a.on_body(&b).is_err());
}

#[test]
fn tampered_hello_gives_different_keys() {
    // The man in the middle changes B's name in its HELLO on the way to A.
    let mut p = Pair::new(Policy::Require, Policy::Require, Instant::now());
    let mut tap = |to_b: bool, body: Vec<u8>| -> Option<Vec<u8>> {
        if !to_b && body[0] == T_HELLO {
            if let Ok(Link::Hello { version, caps, grid, app, .. }) = Link::decode(&body) {
                return Some(Link::Hello { version, caps, name: "EVIL".into(), grid, app }.body());
            }
        }
        Some(body)
    };
    p.run_tapped(&mut tap).unwrap();
    assert!(p.a.encrypted() && p.b.encrypted());
    assert_ne!(p.a.sas(), p.b.sas(), "SAS must differ when a HELLO was altered");
    p.a_sends(&chat(1, "hi"));
    assert!(p.run().unwrap_err().contains("authentication"));
}

#[test]
fn stripping_cap_e2e_downgrades_prefer_but_not_require() {
    let strip = |to_b: bool, body: Vec<u8>| -> Option<Vec<u8>> {
        let _ = to_b;
        if body[0] == T_HELLO {
            if let Ok(Link::Hello { version, caps, name, grid, app }) = Link::decode(&body) {
                return Some(Link::Hello { version, caps: caps & !CAP_E2E, name, grid, app }.body());
            }
        }
        // Drop the key packets too, so neither side notices.
        if body[0] == T_KEYC || body[0] == T_KEYX {
            return None;
        }
        Some(body)
    };
    let mut p = Pair::new(Policy::Prefer, Policy::Prefer, Instant::now());
    p.run_tapped(&mut { strip }).unwrap();
    assert!(p.a.ready() && !p.a.encrypted(), "documented: an attacker can force plain under prefer");
    let mut p = Pair::new(Policy::Require, Policy::Prefer, Instant::now());
    let e = p.run_tapped(&mut { strip }).unwrap_err();
    assert!(e.starts_with("A:") && e.contains("requires"), "{e}");
}

#[test]
fn wrong_suite_is_refused() {
    for kind in [T_KEYC, T_KEYX] {
        let mut p = Pair::new(Policy::Prefer, Policy::Prefer, Instant::now());
        let mut tap = |to_b: bool, body: Vec<u8>| -> Option<Vec<u8>> {
            if to_b && body[0] == kind {
                let mut b = body;
                b[1] = 2; // suite 2
                return Some(b);
            }
            Some(body)
        };
        let e = p.run_tapped(&mut tap).unwrap_err();
        assert!(e.starts_with("B:"), "{e}");
        assert!(!p.b.encrypted());
    }
}

#[test]
fn key_that_does_not_match_its_commitment_is_refused() {
    let mut p = Pair::new(Policy::Prefer, Policy::Prefer, Instant::now());
    let mut tap = |to_b: bool, body: Vec<u8>| -> Option<Vec<u8>> {
        if to_b && body[0] == T_KEYX {
            let mut b = body;
            b[10] ^= 0x40;
            return Some(b);
        }
        Some(body)
    };
    let e = p.run_tapped(&mut tap).unwrap_err();
    assert!(e.starts_with("B:") && e.contains("commitment"), "{e}");
}

#[test]
fn keyx_before_keyc_and_keyc_before_hello_are_refused() {
    let now = Instant::now();
    // KEYX first (after HELLO).
    let (mut b, _) = Channel::open(Policy::Prefer, hello("B"), now);
    let mut h = hello("A");
    if let Link::Hello { caps, .. } = &mut h {
        *caps |= CAP_E2E;
    }
    b.on_body(&h.body()).unwrap();
    let kx = Link::KeyX { suite: SUITE, pubkey: [9; 32] }.body();
    assert!(b.on_body(&kx).unwrap_err().to_string().contains("before its commitment"));
    // KEYC before HELLO.
    let (mut b, _) = Channel::open(Policy::Prefer, hello("B"), now);
    let kc = Link::KeyCommit { suite: SUITE, commit: [1; 32] }.body();
    assert!(b.on_body(&kc).is_err());
}

#[test]
fn second_keyc_or_keyx_after_agreement_is_refused() {
    let mut p = secure(Instant::now());
    let kc = Link::KeyCommit { suite: SUITE, commit: [1; 32] }.body();
    assert!(p.b.on_body(&kc).is_err());
    let mut p = secure(Instant::now());
    let kx = Link::KeyX { suite: SUITE, pubkey: [9; 32] }.body();
    assert!(p.b.on_body(&kx).is_err());
    let mut p = secure(Instant::now());
    assert!(p.b.on_body(&hello("A").body()).is_err(), "second HELLO");
}

#[test]
fn weak_public_keys_are_refused() {
    // All-zero and the other small-order points give an all-zero secret.
    let small_order: [[u8; 32]; 2] = [[0; 32], {
        let mut k = [0u8; 32];
        k[0] = 1;
        k
    }];
    for weak in small_order {
        let now = Instant::now();
        let (mut b, out) = Channel::open(Policy::Require, hello("B"), now);
        assert!(out.len() == 2);
        let mut h = hello("A");
        if let Link::Hello { caps, .. } = &mut h {
            *caps |= CAP_E2E;
        }
        b.on_body(&h.body()).unwrap();
        b.on_body(&Link::KeyCommit { suite: SUITE, commit: commitment(&weak) }.body()).unwrap();
        let e = b.on_body(&Link::KeyX { suite: SUITE, pubkey: weak }.body()).unwrap_err();
        assert!(e.to_string().contains("weak"), "{e}");
        assert!(!b.encrypted());
    }
}

#[test]
fn reflected_handshake_is_refused() {
    // Everything A sends comes straight back to A.
    let now = Instant::now();
    let (mut a, out) = Channel::open(Policy::Require, hello("A"), now);
    let mut err = None;
    let mut q = bodies(&out);
    for _ in 0..10 {
        let mut next = vec![];
        for b in q {
            match a.on_body(&b) {
                Ok(s) => next.extend(bodies(&s.send)),
                Err(e) => {
                    err = Some(e.to_string());
                    break;
                }
            }
        }
        if err.is_some() || next.is_empty() {
            break;
        }
        q = next;
    }
    assert!(!a.encrypted(), "a reflected handshake must never 'succeed'");
    assert!(err.is_some(), "a reflected handshake should be refused");
}

#[test]
fn handshake_packets_inside_sealed_are_refused() {
    for inner in [
        hello("X"),
        Link::KeyCommit { suite: 1, commit: [0; 32] },
        Link::KeyX { suite: 1, pubkey: [0; 32] },
        Link::Sealed { data: vec![0; 20] },
    ] {
        let mut p = secure(Instant::now());
        p.a_sends(&inner); // wrap() seals whatever it's given
        assert!(p.run().unwrap_err().contains("handshake packet"), "{inner:?}");
    }
}

#[test]
fn short_sealed_and_sealed_on_plain_links_are_refused() {
    let mut p = secure(Instant::now());
    assert!(p.b.on_body(&[T_SEALED, 1, 2, 3]).is_err());
    let mut p = Pair::new(Policy::Off, Policy::Off, Instant::now());
    p.run().unwrap();
    assert!(p.b.on_body(&Link::Sealed { data: vec![0; 40] }.body()).is_err());
}

#[test]
fn plain_packets_before_hello() {
    let now = Instant::now();
    // Prefer: a peer that never sends HELLO (not Whistlestop) gets plain.
    let (mut b, _) = Channel::open(Policy::Prefer, hello("B"), now);
    let s = b.on_body(&chat(1, "hi").body()).unwrap();
    assert!(matches!(s.notice, Some(Notice::Plain { .. })));
    assert_eq!(s.app, vec![chat(1, "hi")]);
    assert!(b.ready() && !b.encrypted());
    // Require: refused.
    let (mut b, _) = Channel::open(Policy::Require, hello("B"), now);
    assert!(b.on_body(&chat(1, "hi").body()).is_err());
    // Off: just plain.
    let (mut b, _) = Channel::open(Policy::Off, hello("B"), now);
    assert_eq!(b.on_body(&chat(1, "hi").body()).unwrap().app, vec![chat(1, "hi")]);
}

#[test]
fn off_station_ignores_key_packets() {
    let (mut b, _) = Channel::open(Policy::Off, hello("B"), Instant::now());
    let s = b.on_body(&Link::KeyCommit { suite: 9, commit: [0; 32] }.body()).unwrap();
    assert!(s.send.is_empty() && s.app.is_empty());
    let s = b.on_body(&Link::KeyX { suite: 9, pubkey: [0; 32] }.body()).unwrap();
    assert!(s.send.is_empty() && s.app.is_empty());
}

#[test]
fn queued_until_keys_agreed_and_tick_timeouts() {
    let now = Instant::now();
    for pol in [Policy::Prefer, Policy::Require] {
        let (mut a, _) = Channel::open(pol, hello("A"), now);
        assert!(!a.ready());
        assert!(a.wrap(&chat(1, "wait")).is_none());
        assert!(a.tick(now + TIMEOUT - Duration::from_millis(1)).unwrap().is_none());
        let r = a.tick(now + TIMEOUT);
        match pol {
            // No HELLO ever: Prefer falls back to plain.
            Policy::Prefer => {
                assert!(matches!(r, Ok(Some(Notice::Plain { .. }))), "{r:?}");
                assert!(a.ready() && a.wrap(&chat(1, "now")).is_some());
            }
            _ => assert!(r.is_err()),
        }
    }
    // Prefer with a HELLO that wants encryption but no keys: stalls → error.
    let (mut a, _) = Channel::open(Policy::Prefer, hello("A"), now);
    let mut h = hello("B");
    if let Link::Hello { caps, .. } = &mut h {
        *caps |= CAP_E2E;
    }
    a.on_body(&h.body()).unwrap();
    assert!(a.tick(now + TIMEOUT).is_err());
    // Off is ready at once and never times out.
    let (mut a, _) = Channel::open(Policy::Off, hello("A"), now);
    assert!(a.ready() && a.tick(now + TIMEOUT * 10).unwrap().is_none());
}

/// PROTOCOL.md (Encoding): "a packet or frame with any other callsign is
/// dropped whole". On a link without encryption, a MAIL whose callsign this
/// version refuses should be dropped, not end the whole link. Today
/// `Channel::on_body` returns an error for any undecodable packet, and the
/// engine treats every `on_body` error as an encryption failure
/// (`e2e_drop`: "⚠ Encryption: not a callsign. Disconnecting.") even when
/// encryption is off.
#[test]
fn malformed_packet_on_plain_link_is_dropped_not_fatal() {
    let mut p = Pair::new(Policy::Off, Policy::Off, Instant::now());
    p.run().unwrap();
    let mut w = Writer::default();
    w.u8(T_MAIL).uv(1).str("K1ABC").str("W1AW X").uv(0).u8(0).str("hi");
    let r = p.b.on_body(&w.0);
    assert!(r.is_ok(), "{:?}", r.err());
    // And the link still works.
    p.a_sends(&chat(2, "still here"));
    p.run().unwrap();
    assert_eq!(p.b_app.last(), Some(&chat(2, "still here")));
}

#[test]
fn random_bodies_never_panic_any_policy() {
    let mut x: u64 = 0x1234_5678_9ABC_DEF1;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let now = Instant::now();
    for pol in [Policy::Off, Policy::Prefer, Policy::Require] {
        for _ in 0..300 {
            let (mut c, _) = Channel::open(pol, hello("A"), now);
            for _ in 0..30 {
                let n = (rnd() % 80) as usize;
                let mut b: Vec<u8> = (0..n).map(|_| rnd() as u8).collect();
                if !b.is_empty() && rnd() % 2 == 0 {
                    b[0] = [T_HELLO, T_KEYC, T_KEYX, T_SEALED, 2, 0x0E][(rnd() % 6) as usize];
                }
                let _ = c.on_body(&b);
                let _ = c.wrap(&chat(1, "x"));
                let _ = c.tick(now + Duration::from_secs(rnd() % 300));
            }
        }
    }
}
