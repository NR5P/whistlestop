//! Fuzz-style and round-trip tests for every WSP decoder (docs/PROTOCOL.md).
//! Nothing that arrives from the air may ever panic a decoder.

use whistlestop::protocol::*;

/// Deterministic xorshift64* PRNG (no extra dependencies).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    /// Bytes biased towards interesting values (type codes, varint bits,
    /// KISS specials, magic).
    fn spicy(&mut self, n: usize) -> Vec<u8> {
        const SPICE: [u8; 14] = [
            0x00, 0x01, 0x7F, 0x80, 0xFF, 0xC0, 0xDB, 0xDC, 0xDD, 0x57, 0x0D, 0x0E, 0x11, 0x23,
        ];
        (0..n)
            .map(|_| {
                if self.below(3) == 0 {
                    SPICE[self.below(SPICE.len())]
                } else {
                    self.next() as u8
                }
            })
            .collect()
    }
}

fn call(n: usize) -> String {
    "ABCDEFGHIJKLMNOP"[..n].to_string()
}

/// One of every link packet, with edge-case field values.
fn all_links() -> Vec<Link> {
    let long = "x".repeat(5000);
    let uni = "73 ✓ de Å — 日本".to_string();
    vec![
        Link::Hello {
            version: 0,
            caps: 0,
            name: String::new(),
            grid: String::new(),
            app: String::new(),
        },
        Link::Hello {
            version: 255,
            caps: 255,
            name: uni.clone(),
            grid: "EM12ab".into(),
            app: long.clone(),
        },
        Link::Chat { id: 0, text: String::new() },
        Link::Chat { id: u64::MAX, text: long.clone() },
        Link::Chat { id: 127, text: uni.clone() },
        Link::Received { id: u64::MAX },
        Link::Offer {
            fid: u64::MAX,
            name: "../../etc/passwd".into(),
            size: u64::MAX,
            sha: [0xFF; 32],
            zipped: true,
            files: u64::MAX,
        },
        Link::Offer {
            fid: 0,
            name: String::new(),
            size: 0,
            sha: [0; 32],
            zipped: false,
            files: 0,
        },
        Link::Answer { fid: 1, accept: true, resume_from: u64::MAX },
        Link::Answer { fid: 1, accept: false, resume_from: 0 },
        Link::Chunk { fid: 3, offset: u64::MAX, data: vec![0xC0; CHUNK] },
        Link::Chunk { fid: 3, offset: 0, data: vec![] },
        Link::Done { fid: 9, ok: true },
        Link::Done { fid: 9, ok: false },
        Link::Cancel { fid: u64::MAX },
        Link::Report { snr_db: i8::MIN, mode_index: 255 },
        Link::Report { snr_db: i8::MAX, mode_index: 0 },
        Link::Bye { text: String::new() },
        Link::Bye { text: long.clone() },
        Link::Mail {
            id: u64::MAX,
            from: call(12),
            to: "K7BOT/P".into(),
            created: u64::MAX,
            hops: 255,
            text: long.clone(),
        },
        Link::Mail {
            id: 0,
            from: "A".into(),
            to: "B-1".into(),
            created: 0,
            hops: 0,
            text: String::new(),
        },
        Link::MailAck { id: u64::MAX },
        Link::Qsy { id: 1, dial_hz: u64::MAX, why: uni.clone() },
        Link::QsyAnswer { id: 1, accept: true, reason: String::new() },
        Link::QsyAnswer { id: 2, accept: false, reason: long.clone() },
        Link::ListReq { id: 0, path: String::new() },
        Link::ListAns {
            id: 5,
            path: "maps/".into(),
            entries: (0..200)
                .map(|i| LibEntry {
                    name: format!("{i:0>200}"),
                    size: u64::MAX - i,
                    dir: i % 2 == 0,
                })
                .collect(),
            note: String::new(),
        },
        Link::ListAns {
            id: 6,
            path: String::new(),
            entries: vec![],
            note: "there's no file library here".into(),
        },
        Link::Fetch { id: 7, path: "a/b/c.txt".into() },
        Link::FetchAnswer { id: 7, ok: false, reason: uni },
        Link::FetchAnswer { id: 7, ok: true, reason: String::new() },
        Link::KeyCommit { suite: 1, commit: [0xAB; 32] },
        Link::KeyX { suite: 255, pubkey: [0; 32] },
        Link::Sealed { data: vec![] },
        Link::Sealed { data: vec![0xC0; 3000] },
        Link::Unknown(0x7F),
        Link::Unknown(0xFF),
    ]
}

fn all_table_frames() -> Vec<TableFrame> {
    let mut v = vec![];
    let bodies = vec![
        TableBody::Call { kind: CALL_CHAT, table_name: String::new(), bw_code: 0, grid: String::new() },
        TableBody::Call { kind: 255, table_name: "Evening Net ✓".into(), bw_code: 255, grid: "EM12".into() },
        TableBody::Text { id: u64::MAX, part: 64, total: 64, data: vec![0xC0; 1024] },
        TableBody::Text { id: 0, part: 1, total: 1, data: vec![] },
        TableBody::Join { table_name: "X".into(), snr_of_host: i8::MIN },
        TableBody::Speed { mode_index: 255, last_id: 0 },
        TableBody::Speed { mode_index: 255, last_id: u64::MAX },
        TableBody::Again { of: call(12), id: u64::MAX, missing: u64::MAX },
        TableBody::Floor { to: "W1AW".into() },
        TableBody::Leave,
        TableBody::Beacon { grid: String::new(), away: false, mail_for: vec![] },
        TableBody::Beacon {
            grid: "FN31pr".into(),
            away: true,
            mail_for: (0..32).map(|i| format!("K{i}ABC")).collect(),
        },
        TableBody::Notice { to: "K7BOT".into(), count: 255 },
    ];
    for (i, body) in bodies.into_iter().enumerate() {
        for table in [0u16, 1, u16::MAX, table_id("Evening Net")] {
            v.push(TableFrame { table, from: call(1 + i % 12), body: body.clone() });
        }
    }
    v
}

#[test]
fn every_link_packet_round_trips() {
    for p in all_links() {
        let body = p.body();
        assert_eq!(Link::decode(&body).unwrap(), p, "body round trip");
        // encode() = varint(len) ‖ body, and the stream decoder agrees.
        let wire = p.encode();
        let mut d = LinkDecoder::default();
        assert_eq!(d.feed(&wire).unwrap(), vec![p.clone()]);
        let mut d = LinkDecoder::default();
        assert_eq!(d.feed_bodies(&wire).unwrap(), vec![body]);
    }
}

#[test]
fn link_stream_survives_any_split() {
    let pkts = all_links();
    let wire: Vec<u8> = pkts.iter().flat_map(|p| p.encode()).collect();
    let mut rng = Rng::new(1);
    for _ in 0..200 {
        let mut d = LinkDecoder::default();
        let mut got = vec![];
        let mut i = 0;
        while i < wire.len() {
            let n = 1 + rng.below(300);
            let end = (i + n).min(wire.len());
            got.extend(d.feed(&wire[i..end]).unwrap());
            i = end;
        }
        assert_eq!(got, pkts);
    }
}

#[test]
fn every_table_frame_round_trips() {
    for f in all_table_frames() {
        assert_eq!(TableFrame::decode(&f.encode()), Some(f));
    }
}

#[test]
fn truncated_link_packets_are_errors_not_panics() {
    for p in all_links() {
        let body = p.body();
        for cut in 0..body.len() {
            let r = Link::decode(&body[..cut]);
            match p {
                // SEALED's data is "the rest of the body", and an unknown
                // type has no fields: any prefix with the type byte decodes.
                Link::Sealed { .. } | Link::Unknown(_) if cut >= 1 => {}
                _ => assert!(r.is_err(), "{p:?} cut at {cut} decoded as {r:?}"),
            }
        }
    }
}

#[test]
fn truncated_table_frames_are_dropped() {
    for f in all_table_frames() {
        let b = f.encode();
        // SPEED's newest-message id is optional (older stations send none):
        // cut anywhere in it, the frame is still a SPEED without it.
        let speed_base = match &f.body {
            TableBody::Speed { mode_index, .. } => {
                Some(TableFrame { body: TableBody::Speed { mode_index: *mode_index, last_id: 0 }, ..f.clone() }.encode().len())
            }
            _ => None,
        };
        for cut in 0..b.len() {
            let r = TableFrame::decode(&b[..cut]);
            if speed_base.is_some_and(|n| cut >= n) {
                assert!(matches!(r, Some(TableFrame { body: TableBody::Speed { last_id: 0, .. }, .. })), "{f:?} cut at {cut} decoded as {r:?}");
                continue;
            }
            // LEAVE has no fields: the full header is the whole frame.
            if !(matches!(f.body, TableBody::Leave) && cut == b.len()) {
                assert!(r.is_none(), "{f:?} cut at {cut} decoded as {r:?}");
            }
        }
    }
}

#[test]
fn random_bytes_never_panic_link_decode() {
    let mut rng = Rng::new(2);
    for i in 0..200_000 {
        let n = rng.below(64);
        let mut b = if i % 2 == 0 { rng.bytes(n) } else { rng.spicy(n) };
        // Steer half the cases onto a real type byte.
        if i % 4 < 2 && !b.is_empty() {
            const T: [u8; 22] = [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0x20, 0x21, 0x22, 0x23, 0x24,
                0x25, 0x26,
            ];
            b[0] = T[rng.below(T.len())];
        }
        let _ = Link::decode(&b);
    }
}

#[test]
fn mutated_valid_packets_never_panic() {
    let mut rng = Rng::new(3);
    let pkts = all_links();
    let frames = all_table_frames();
    for _ in 0..100_000 {
        let mut b = pkts[rng.below(pkts.len())].body();
        let mut t = frames[rng.below(frames.len())].encode();
        for buf in [&mut b, &mut t] {
            for _ in 0..1 + rng.below(4) {
                if buf.is_empty() {
                    break;
                }
                let i = rng.below(buf.len());
                match rng.below(4) {
                    0 => buf[i] ^= 1 << rng.below(8),
                    1 => buf[i] = 0xFF,
                    2 => buf.truncate(i),
                    _ => buf.insert(i, rng.next() as u8),
                }
            }
        }
        let _ = Link::decode(&b);
        let _ = TableFrame::decode(&t);
    }
}

#[test]
fn random_bytes_never_panic_table_decode() {
    let mut rng = Rng::new(4);
    for i in 0..200_000 {
        let n = rng.below(130);
        let mut b = rng.spicy(n);
        if b.len() >= 2 && i % 3 != 0 {
            b[0] = TABLE_MAGIC;
            b[1] = 0x10 + rng.below(10) as u8;
        }
        if let Some(f) = TableFrame::decode(&b) {
            // Whatever decodes carries a valid callsign and sane parts.
            assert!(valid_call(&f.from), "{f:?}");
            if let TableBody::Text { part, total, .. } = f.body {
                assert!(1 <= part && part <= total && total <= 64, "{part}/{total}");
            }
        }
    }
}

#[test]
fn huge_lengths_inside_packets_are_refused() {
    // Every length-prefixed field claiming u64::MAX / 2^63 bytes.
    for t in [0x02u8, 0x04, 0x06, 0x0A, 0x0E, 0x20, 0x22, 0x23, 0x24, 0x25] {
        for len in [u64::MAX, 1 << 63, 1 << 40, usize::MAX as u64] {
            let mut w = Writer::default();
            w.u8(t).uv(1);
            if t == 0x06 {
                w.uv(0);
            }
            w.uv(len).raw(b"abc");
            assert!(Link::decode(&w.0).is_err(), "type {t:#x} len {len}");
        }
    }
    // LISTANS claiming 2^63 entries must not spin or allocate them.
    let mut w = Writer::default();
    w.u8(T_LISTANS).uv(1).str("").str("").uv(1 << 63);
    let t0 = std::time::Instant::now();
    assert!(Link::decode(&w.0).is_err());
    assert!(t0.elapsed().as_millis() < 500);
    // BEACON claiming 2^63 calls.
    let mut w = Writer::default();
    w.u8(TABLE_MAGIC).u8(0x17).uv(0).call("N0CALL").str("").u8(0).uv(1 << 63);
    assert!(TableFrame::decode(&w.0).is_none());
}

#[test]
fn varint_edges() {
    // 10-byte maximal varint.
    let mut w = Writer::default();
    w.uv(u64::MAX);
    assert_eq!(w.0.len(), 10);
    assert_eq!(Reader::new(&w.0).uv().unwrap(), u64::MAX);
    // An endless run of continuation bytes is an error, not a loop.
    assert!(Reader::new(&[0x80; 64]).uv().is_err());
    assert!(Reader::new(&[0xFF; 11]).uv().is_err());
    assert!(Reader::new(&[]).uv().is_err());
    // Random varints round-trip.
    let mut rng = Rng::new(5);
    for _ in 0..50_000 {
        let v = rng.next() >> rng.below(64);
        let mut w = Writer::default();
        w.uv(v);
        let mut r = Reader::new(&w.0);
        assert_eq!(r.uv().unwrap(), v);
        assert!(r.rest().is_empty());
    }
}

#[test]
fn link_decoder_limits() {
    // Exactly the 1 MiB limit is fine; one more byte is a corrupt stream.
    let mut w = Writer::default();
    w.uv(1 << 20);
    let mut d = LinkDecoder::default();
    assert!(d.feed_bodies(&w.0).unwrap().is_empty(), "waits for the body");
    let mut w = Writer::default();
    w.uv((1 << 20) + 1);
    let mut d = LinkDecoder::default();
    assert!(d.feed_bodies(&w.0).is_err());
    // A zero-length packet yields an empty body, which `feed` refuses.
    let mut d = LinkDecoder::default();
    assert_eq!(d.feed_bodies(&[0]).unwrap(), vec![Vec::<u8>::new()]);
    let mut d = LinkDecoder::default();
    assert!(d.feed(&[0]).is_err());
}

/// A length prefix of 5+ varint bytes (4 continuation bytes) can never be a
/// valid packet (the limit is 1 MiB, which fits in 3 bytes). The decoder
/// must report the stream as corrupt instead of waiting forever and
/// buffering everything that follows.
#[test]
fn overlong_length_prefix_is_corrupt_not_a_stall() {
    let mut d = LinkDecoder::default();
    let mut wire = vec![0x80, 0x80, 0x80, 0x80, 0x00];
    wire.extend(Link::Chat { id: 1, text: "hi".into() }.encode());
    let r = d.feed_bodies(&wire);
    assert!(r.is_err(), "expected a corrupt-stream error, got {r:?}");
}

#[test]
fn random_streams_never_panic_link_decoder() {
    let mut rng = Rng::new(6);
    for _ in 0..3_000 {
        let mut d = LinkDecoder::default();
        for _ in 0..20 {
            // Mostly small length prefixes so packets actually complete.
            let mut b = vec![rng.below(40) as u8];
            let n = rng.below(60);
            b.extend(rng.spicy(n));
            let _ = d.feed(&b);
        }
    }
}

#[test]
fn kiss_round_trips_and_never_panics() {
    let mut rng = Rng::new(7);
    for _ in 0..5_000 {
        let n = rng.below(300);
        let payload = rng.spicy(n);
        let cmd = rng.below(16) as u8;
        let enc = kiss_encode(&payload, cmd);
        // No FEND inside the frame body.
        assert!(!enc[1..enc.len() - 1].contains(&0xC0));
        let mut d = KissDecoder::default();
        // Leading junk outside any frame is ignored.
        let jn = rng.below(10);
        let junk: Vec<u8> = rng.spicy(jn).into_iter().filter(|&b| b != 0xC0).collect();
        let mut got = d.feed(&junk);
        // Fed in random pieces.
        let mut i = 0;
        while i < enc.len() {
            let end = (i + 1 + rng.below(40)).min(enc.len());
            got.extend(d.feed(&enc[i..end]));
            i = end;
        }
        assert_eq!(got, vec![(cmd, payload)]);
    }
    for _ in 0..5_000 {
        let mut d = KissDecoder::default();
        let n = rng.below(500);
        let _ = d.feed(&rng.spicy(n));
    }
}

#[test]
fn kiss_back_to_back_and_empty_frames() {
    let mut d = KissDecoder::default();
    let mut wire = vec![0xC0, 0xC0, 0xC0]; // empty frames are skipped
    wire.extend(kiss_encode(b"one", 0));
    wire.extend(kiss_encode(b"", 0));
    wire.extend(kiss_encode(&[0xC0, 0xDB], 0));
    assert_eq!(
        d.feed(&wire),
        vec![(0, b"one".to_vec()), (0, vec![]), (0, vec![0xC0, 0xDB])]
    );
}

#[test]
fn table_text_frames_fit_and_reassemble_random() {
    let mut rng = Rng::new(8);
    let alphabet: Vec<char> = "abc XYZ 0123 ✓Å日本🛰".chars().collect();
    for _ in 0..1_500 {
        let len = rng.below(1500);
        let text: String = (0..len).map(|_| alphabet[rng.below(alphabet.len())]).collect();
        let frame_bytes = 30 + rng.below(1200);
        let from = call(1 + rng.below(12));
        let id = rng.next() >> rng.below(64);
        let t = rng.next() as u16;
        let Ok(frames) = table_text_frames(t, &from, id, &text, frame_bytes) else {
            continue; // too small a frame, or more than 64 parts
        };
        assert!(frames.len() <= 64);
        let mut a = TableAssembler::default();
        let mut out = None;
        // Deliver in a random order.
        let mut order: Vec<usize> = (0..frames.len()).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        for &i in &order {
            let f = &frames[i];
            assert!(f.len() <= frame_bytes, "{} > {frame_bytes}", f.len());
            let TableFrame { from: fr, table, body: TableBody::Text { id: mid, part, total, data } } =
                TableFrame::decode(f).expect("decodes")
            else {
                panic!("not TEXT")
            };
            assert_eq!((table, mid), (t, id));
            if let Some(s) = a.add(&fr, mid, part, total, &data) {
                assert!(out.is_none(), "completed twice");
                out = Some(s);
            }
        }
        assert_eq!(out.as_deref(), Some(text.as_str()));
        assert!(a.missing().is_empty());
    }
}

#[test]
fn table_text_frames_refuse_too_small_or_too_long() {
    assert!(table_text_frames(1, "N0CALL", 1, "hi", 5).is_err());
    assert!(table_text_frames(1, "N0CALL", 1, &"x".repeat(100_000), 123).is_err());
    // Empty text is one empty part.
    let f = table_text_frames(1, "N0CALL", 1, "", 123).unwrap();
    assert_eq!(f.len(), 1);
}

#[test]
fn assembler_missing_bitmap_uses_all_64_bits() {
    let mut a = TableAssembler::default();
    for p in 1..=63u8 {
        assert!(a.add("K1ABC", 9, p, 64, b"x").is_none());
    }
    assert_eq!(a.missing(), vec![("K1ABC".to_string(), 9, 1u64 << 63)]);
    assert!(a.add("K1ABC", 9, 64, 64, b"y").is_some());
    assert!(a.missing().is_empty());
}

#[test]
fn assembler_keeps_senders_and_ids_apart() {
    let mut a = TableAssembler::default();
    assert!(a.add("A", 1, 1, 2, b"a1").is_none());
    assert!(a.add("B", 1, 1, 2, b"b1").is_none());
    assert!(a.add("A", 2, 1, 2, b"x1").is_none());
    assert_eq!(a.add("B", 1, 2, 2, b"b2").as_deref(), Some("b1b2"));
    assert_eq!(a.add("A", 1, 2, 2, b"a2").as_deref(), Some("a1a2"));
    assert_eq!(a.missing(), vec![("A".to_string(), 2, 0b10)]);
}

/// In a Roundtable, parts are resent when *someone* asks AGAIN, so a station
/// that already has the whole message hears those parts a second time. That
/// must not show the message twice, and must not make this station ask
/// AGAIN for the rest of a message it already has (the engine sends AGAIN
/// for every entry of `missing()` every 20 s, which would set off a resend
/// storm across the table).
#[test]
fn resent_part_after_completion_is_ignored() {
    let mut a = TableAssembler::default();
    assert!(a.add("K1ABC", 5, 1, 3, b"one ").is_none());
    assert!(a.add("K1ABC", 5, 2, 3, b"two ").is_none());
    assert_eq!(a.add("K1ABC", 5, 3, 3, b"three").as_deref(), Some("one two three"));
    // Part 2 is resent for another member who missed it.
    assert!(a.add("K1ABC", 5, 2, 3, b"two ").is_none());
    assert!(a.missing().is_empty(), "asks AGAIN for a message it already showed: {:?}", a.missing());
    assert!(a.add("K1ABC", 5, 1, 3, b"one ").is_none());
    assert!(a.add("K1ABC", 5, 3, 3, b"three").is_none(), "message shown twice");
}

/// Unfinished messages are given up after TABLE_PARTIAL_TTL (no AGAIN for
/// them forever), and finished ones are forgotten after TABLE_DONE_TTL or
/// once TABLE_DONE_MAX newer ones finished.
#[test]
fn assembler_expires_stale_partials_and_old_completions() {
    use std::time::{Duration, Instant};
    let t0 = Instant::now();
    let mut a = TableAssembler::default();
    assert!(a.add_at(t0, "K1ABC", 7, 1, 2, b"a").is_none());
    assert_eq!(a.missing_at(t0).len(), 1);
    let later = t0 + TABLE_PARTIAL_TTL + Duration::from_secs(1);
    assert!(a.missing_at(later).is_empty(), "still asking AGAIN for a stale message");
    // The late part starts over rather than completing a stale message.
    assert!(a.add_at(later, "K1ABC", 7, 2, 2, b"b").is_none());
    assert_eq!(a.missing_at(later), vec![("K1ABC".to_string(), 7, 0b01)]);
    // A finished message is remembered for a while, then forgotten.
    assert!(a.add_at(t0, "K2X", 1, 1, 1, b"x").is_some());
    assert!(a.add_at(t0, "K2X", 1, 1, 1, b"x").is_none());
    let much_later = t0 + TABLE_DONE_TTL + Duration::from_secs(1);
    assert!(a.add_at(much_later, "K2X", 1, 1, 1, b"x").is_some());
    // Bounded memory: the oldest completions are dropped first.
    let mut a = TableAssembler::default();
    for id in 0..(TABLE_DONE_MAX as u64 + 1) {
        assert!(a.add_at(t0, "K3Y", id, 1, 1, b"m").is_some());
    }
    assert!(a.add_at(t0, "K3Y", 0, 1, 1, b"m").is_some(), "oldest forgotten");
    assert!(a.add_at(t0, "K3Y", TABLE_DONE_MAX as u64, 1, 1, b"m").is_none(), "newest remembered");
}

/// Parts of one message that disagree on `total` (a corrupted or forged
/// frame). The assembler must not throw away the parts it has without
/// delivering, and must never panic.
#[test]
fn assembler_inconsistent_totals_do_not_panic() {
    let mut a = TableAssembler::default();
    let _ = a.add("K1ABC", 1, 1, 2, b"a");
    let _ = a.add("K1ABC", 1, 3, 3, b"c");
    let _ = a.missing();
    let _ = a.add("K1ABC", 1, 2, 2, b"b");
    let _ = a.missing();
}

#[test]
fn valid_call_matches_the_spec() {
    // 1–12 of A–Z a–z 0–9 / -
    let mut rng = Rng::new(9);
    for _ in 0..20_000 {
        let n = rng.below(16);
        let s: String = (0..n).map(|_| (32 + rng.below(95)) as u8 as char).collect();
        let want = (1..=12).contains(&s.len())
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'/' || b == b'-');
        assert_eq!(valid_call(&s), want, "{s:?}");
    }
    assert!(!valid_call("N0CALL\0"));
    assert!(!valid_call("ÅÅ"));
}

#[test]
fn writer_call_never_emits_more_than_12_bytes() {
    for s in ["", "a", "abcdefghijklmnop", "ÅÅÅÅÅÅÅÅ", "日本日本日本日本", "🛰🛰🛰🛰"] {
        let mut w = Writer::default();
        w.call(s);
        let got = Reader::new(&w.0).str().unwrap();
        assert!(got.len() <= 12, "{s:?} -> {got:?}");
        assert!(s.to_uppercase().starts_with(&got) || s.to_ascii_uppercase().starts_with(&got));
    }
}

#[test]
fn wire_size_helpers_match_encoding() {
    for t in ["", "73", &"x".repeat(200), &"✓".repeat(100)] {
        assert_eq!(chat_wire_size(t), Link::Chat { id: 1000, text: t.to_string() }.encode().len());
    }
    // file_wire_size is an estimate; it must not be below the real cost of
    // the offer plus the data.
    for size in [0u64, 1, 1023, 1024, 1025, 100_000] {
        let real_min = Link::Offer { fid: 1, name: "a.zip".into(), size, sha: [0; 32], zipped: true, files: 1 }
            .encode()
            .len()
            + size as usize;
        assert!(file_wire_size("a.zip", size) >= real_min);
    }
}

/// A one-frame Roundtable message lost whole leaves nothing to ask AGAIN
/// for, unless the gap in the sender's ids shows it: the next message, or
/// the host's heartbeat (its newest id), gives it away.
#[test]
fn assembler_asks_for_messages_skipped_whole() {
    let mut a = TableAssembler::default();
    assert!(a.add("HOST", 3, 1, 1, b"three").is_some());
    // 4 and 5 never heard; 6 arrives.
    assert!(a.add("HOST", 6, 1, 1, b"six").is_some());
    let mut m = a.missing();
    m.sort();
    assert_eq!(m, vec![("HOST".to_string(), 4, u64::MAX), ("HOST".to_string(), 5, u64::MAX)]);
    // A resent part of 4 turns it into an ordinary partial message.
    assert!(a.add("HOST", 4, 1, 2, b"fo").is_none());
    assert_eq!(a.add("HOST", 4, 2, 2, b"ur").as_deref(), Some("four"));
    assert_eq!(a.missing(), vec![("HOST".to_string(), 5, u64::MAX)]);
    // The heartbeat says 7 went out too.
    a.note_latest("HOST", 7, true);
    let mut m = a.missing();
    m.sort();
    assert_eq!(m, vec![("HOST".to_string(), 5, u64::MAX), ("HOST".to_string(), 7, u64::MAX)]);
}

#[test]
fn assembler_gap_rules() {
    let mut a = TableAssembler::default();
    // The first id from a sender: no asking for the table's past.
    a.note_latest("HOST", 40, true);
    assert!(a.missing().is_empty());
    // A huge jump is not chased (at most 16 messages).
    a.note_latest("HOST", 100, true);
    assert!(a.missing().is_empty());
    // A sender that restarted counts from 1 again: no gap storm.
    assert!(a.add("HOST", 1, 1, 1, b"hi").is_some());
    assert!(a.missing().is_empty());
    assert!(a.add("HOST", 2, 1, 1, b"again").is_some());
    assert!(a.missing().is_empty());
}
