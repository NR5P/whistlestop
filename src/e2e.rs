//! Optional end-to-end encryption for the 1:1 link plane (chat and files).
//! Specification: docs/PROTOCOL.md, "Encryption".
//!
//! **Not legal on amateur bands** in the US (47 CFR 97.113(a)(4)) and most
//! other countries: messages there may not be encoded to obscure their
//! meaning. It is off by default.
//!
//! Each link gets a fresh X25519 key pair. Both stations first send a
//! commitment to their public key (KEYC) and reveal the key (KEYX) only once
//! the other's commitment has arrived, so a man in the middle cannot pick
//! keys that make the short authentication string (SAS) come out equal on
//! both sides. The shared secret goes through HKDF-SHA256, salted with a hash
//! of both HELLOs and both public keys, into one ChaCha20-Poly1305 key per
//! direction plus the 6-digit SAS. The nonce is a per-direction packet
//! counter that is never sent: Mercury's ARQ keeps order, so a replayed,
//! dropped or reordered packet simply fails authentication.
//!
//! The stations are ordered by public key, not by who called, so the
//! handshake does not depend on how Mercury words its CONNECTED line.
//!
//! All secrets live in types that wipe themselves on drop, and nothing is
//! ever written to disk: dropping the `Channel` at DISCONNECTED forgets the
//! session for good.

use crate::protocol::*;
use anyhow::{Result, anyhow, bail};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use x25519_dalek::{EphemeralSecret, PublicKey};
use zeroize::Zeroizing;

/// Suite 1: X25519, HKDF-SHA256, ChaCha20-Poly1305 (16-byte tag).
pub const SUITE: u8 = 1;
pub const TAG_LEN: usize = 16;
/// Air bytes SEALED adds to a packet: its type byte plus the tag (the
/// length prefix grows by one more byte if the packet crosses 128 bytes).
pub const SEALED_OVERHEAD: usize = 1 + TAG_LEN;
/// How long to wait for the key exchange before giving up. HF links can be
/// slow (about 140 bytes each way at 50 bps, plus ARQ turnarounds).
pub const TIMEOUT: Duration = Duration::from_secs(120);

pub const LEGAL_WARNING: &str = "Encryption is NOT legal on amateur radio bands in the US \
(47 CFR 97.113(a)(4)) and in most other countries: amateur transmissions may not be encoded \
to obscure their meaning. Use it only where it is lawful (for example on non-amateur \
frequencies you are licensed for, or over a wired or test link).";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Never encrypt (default).
    #[default]
    Off,
    /// Encrypt when the other station also wants to; otherwise plain text.
    Prefer,
    /// Encrypt or hang up: never fall back to plain text.
    Require,
}

impl Policy {
    pub fn label(self) -> &'static str {
        match self {
            Policy::Off => "Off",
            Policy::Prefer => "Prefer",
            Policy::Require => "Require",
        }
    }
}

type Key = Zeroizing<[u8; 32]>;

fn commitment(pubkey: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"WSP1 E2E commit");
    h.update([SUITE]);
    h.update(pubkey);
    h.finalize().into()
}

/// SHA-256 over both HELLOs and both public keys, lower public key first.
fn transcript(lo: (&[u8], &[u8; 32]), hi: (&[u8], &[u8; 32])) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"WSP1 E2E transcript");
    h.update([SUITE]);
    for hello in [lo.0, hi.0] {
        h.update((hello.len() as u32).to_be_bytes());
        h.update(hello);
    }
    h.update(lo.1);
    h.update(hi.1);
    h.finalize().into()
}

fn nonce(n: u64) -> chacha20poly1305::Nonce {
    let mut b = [0u8; 12];
    b[4..].copy_from_slice(&n.to_le_bytes());
    b.into()
}

/// Session keys for one link, one per direction.
pub struct Session {
    tx_key: Key,
    rx_key: Key,
    tx_n: u64,
    rx_n: u64,
    sas: String,
}

impl Session {
    /// `i_am_lo`: our public key sorts below the peer's.
    fn derive(shared: &[u8; 32], salt: &[u8; 32], i_am_lo: bool) -> Session {
        let hk = Hkdf::<Sha256>::new(Some(salt), shared);
        let mut lo_hi: Key = Zeroizing::new([0; 32]);
        let mut hi_lo: Key = Zeroizing::new([0; 32]);
        let mut sas = Zeroizing::new([0u8; 4]);
        hk.expand(b"WSP1 E2E lo->hi", &mut lo_hi[..])
            .expect("32 bytes");
        hk.expand(b"WSP1 E2E hi->lo", &mut hi_lo[..])
            .expect("32 bytes");
        hk.expand(b"WSP1 E2E SAS", &mut sas[..]).expect("4 bytes");
        let code = u32::from_be_bytes(*sas) % 1_000_000;
        let (tx_key, rx_key) = if i_am_lo {
            (lo_hi, hi_lo)
        } else {
            (hi_lo, lo_hi)
        };
        Session {
            tx_key,
            rx_key,
            tx_n: 0,
            rx_n: 0,
            sas: format!("{:03} {:03}", code / 1000, code % 1000),
        }
    }

    /// Encrypt one inner packet body; returns the SEALED payload.
    pub fn seal(&mut self, inner_body: &[u8]) -> Vec<u8> {
        let c = ChaCha20Poly1305::new_from_slice(&self.tx_key[..]).expect("32-byte key");
        let ct = c
            .encrypt(
                &nonce(self.tx_n),
                Payload {
                    msg: inner_body,
                    aad: &[T_SEALED],
                },
            )
            .expect("in range");
        self.tx_n += 1;
        ct
    }

    /// Decrypt the next SEALED payload. Anything tampered with, replayed or
    /// out of order fails, because the nonce is the expected counter.
    pub fn open(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < TAG_LEN {
            bail!("encrypted packet too short");
        }
        let c = ChaCha20Poly1305::new_from_slice(&self.rx_key[..]).expect("32-byte key");
        let pt = c
            .decrypt(&nonce(self.rx_n), Payload { msg: data, aad: &[T_SEALED] })
            .map_err(|_| anyhow!("encrypted packet failed authentication (tampered with, replayed or out of order)"))?;
        self.rx_n += 1;
        Ok(pt)
    }

    pub fn sas(&self) -> &str {
        &self.sas
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// Keys agreed: show the SAS.
    Secure { sas: String },
    /// Policy Prefer, but this link runs in plain text.
    Plain { why: String },
}

/// What the channel made of one incoming packet.
#[derive(Default, Debug)]
pub struct Step {
    /// Handshake packets to send now (already encoded for the wire).
    pub send: Vec<Vec<u8>>,
    /// Application packets for the engine (decrypted if need be).
    pub app: Vec<Link>,
    pub notice: Option<Notice>,
}

/// The link plane's security layer for one connection.
pub struct Channel {
    policy: Policy,
    my_hello: Vec<u8>,
    peer_hello: Option<Vec<u8>>,
    secret: Option<EphemeralSecret>,
    my_pub: [u8; 32],
    peer_commit: Option<[u8; 32]>,
    session: Option<Session>,
    plain: bool,
    deadline: Instant,
}

impl Channel {
    /// Start a link. `hello` is our HELLO (CAP_E2E is set or cleared here
    /// to match `policy`). Returns the packets to send first: HELLO, and
    /// KEYC when encryption is on.
    pub fn open(policy: Policy, hello: Link, now: Instant) -> (Channel, Vec<Vec<u8>>) {
        let hello = match hello {
            Link::Hello {
                version,
                caps,
                name,
                grid,
                app,
            } => {
                let caps = if policy == Policy::Off {
                    caps & !CAP_E2E
                } else {
                    caps | CAP_E2E
                };
                Link::Hello {
                    version,
                    caps,
                    name,
                    grid,
                    app,
                }
            }
            other => other,
        };
        let mut out = vec![hello.encode()];
        let (secret, my_pub) = if policy == Policy::Off {
            (None, [0; 32])
        } else {
            let s = EphemeralSecret::random();
            let p = PublicKey::from(&s).to_bytes();
            out.push(
                Link::KeyCommit {
                    suite: SUITE,
                    commit: commitment(&p),
                }
                .encode(),
            );
            (Some(s), p)
        };
        let ch = Channel {
            policy,
            my_hello: hello.body(),
            peer_hello: None,
            secret,
            my_pub,
            peer_commit: None,
            session: None,
            plain: policy == Policy::Off,
            deadline: now + TIMEOUT,
        };
        (ch, out)
    }

    /// Application packets may flow (plain or sealed).
    pub fn ready(&self) -> bool {
        self.plain || self.session.is_some()
    }

    pub fn encrypted(&self) -> bool {
        self.session.is_some()
    }

    pub fn sas(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.sas())
    }

    /// Encode an application packet for the wire: sealed once keys are
    /// agreed, plain on a plain link, and None while the handshake runs
    /// (the caller keeps it queued).
    pub fn wrap(&mut self, p: &Link) -> Option<Vec<u8>> {
        if let Some(s) = self.session.as_mut() {
            let body = Zeroizing::new(p.body());
            Some(
                Link::Sealed {
                    data: s.seal(&body),
                }
                .encode(),
            )
        } else if self.plain {
            Some(p.encode())
        } else {
            None
        }
    }

    fn go_plain(&mut self) {
        self.plain = true;
        self.secret = None;
    }

    /// Handle one packet body from the peer. An error means the link can't
    /// be trusted (or policy refuses it) and should be dropped.
    pub fn on_body(&mut self, body: &[u8]) -> Result<Step> {
        let mut step = Step::default();
        let p = Link::decode(body)?;
        match p {
            Link::Hello { caps, .. } => {
                if self.peer_hello.is_some() {
                    if !self.plain {
                        bail!("second HELLO on an encrypting link");
                    }
                } else {
                    self.peer_hello = Some(body.to_vec());
                    if !self.plain && caps & CAP_E2E == 0 {
                        if self.policy == Policy::Require {
                            bail!(
                                "the other station does not have encryption on, and this station requires it"
                            );
                        }
                        self.go_plain();
                        step.notice = Some(Notice::Plain {
                            why: "the other station has encryption off".into(),
                        });
                    }
                }
                step.app.push(p);
            }
            Link::KeyCommit { suite, commit } => {
                if self.plain {
                    return Ok(step);
                }
                if suite != SUITE {
                    bail!(
                        "the other station offered encryption suite {suite}, which this version doesn't know"
                    );
                }
                if self.peer_hello.is_none() {
                    bail!("key commitment before HELLO");
                }
                if self.peer_commit.is_some() {
                    bail!("second key commitment");
                }
                if commit == commitment(&self.my_pub) {
                    bail!("our own key commitment came back (reflected)");
                }
                self.peer_commit = Some(commit);
                // Only now, with their commitment in hand, reveal our key.
                step.send.push(
                    Link::KeyX {
                        suite: SUITE,
                        pubkey: self.my_pub,
                    }
                    .encode(),
                );
            }
            Link::KeyX { suite, pubkey } => {
                if self.plain {
                    return Ok(step);
                }
                let Some(commit) = self.peer_commit else {
                    bail!("public key before its commitment")
                };
                if suite != SUITE || self.session.is_some() {
                    bail!("unexpected key exchange packet");
                }
                if commitment(&pubkey) != commit {
                    bail!(
                        "the other station's key does not match its commitment: someone may be tampering with the link"
                    );
                }
                if pubkey == self.my_pub {
                    bail!("our own public key came back (reflected)");
                }
                let secret = self
                    .secret
                    .take()
                    .ok_or_else(|| anyhow!("no key pair for this link"))?;
                let shared = secret.diffie_hellman(&PublicKey::from(pubkey));
                if !shared.was_contributory() {
                    bail!("the other station sent a weak public key");
                }
                let peer_hello = self.peer_hello.as_deref().unwrap_or_default();
                let (mine, theirs) = ((&self.my_hello[..], &self.my_pub), (peer_hello, &pubkey));
                let i_am_lo = self.my_pub < pubkey;
                let salt = if i_am_lo {
                    transcript(mine, theirs)
                } else {
                    transcript(theirs, mine)
                };
                let s = Session::derive(shared.as_bytes(), &salt, i_am_lo);
                step.notice = Some(Notice::Secure { sas: s.sas.clone() });
                self.session = Some(s);
            }
            Link::Sealed { data } => {
                let s = self
                    .session
                    .as_mut()
                    .ok_or_else(|| anyhow!("encrypted packet, but no keys were agreed"))?;
                let inner = Zeroizing::new(s.open(&data)?);
                let q = Link::decode(&inner)?;
                if matches!(
                    q,
                    Link::Hello { .. }
                        | Link::KeyCommit { .. }
                        | Link::KeyX { .. }
                        | Link::Sealed { .. }
                ) {
                    bail!("handshake packet inside an encrypted one");
                }
                step.app.push(q);
            }
            Link::Unknown(_) => {}
            other => {
                if !self.plain && self.peer_hello.is_none() && self.policy == Policy::Prefer {
                    // Not Whistlestop, or something older that skips HELLO.
                    self.go_plain();
                    step.notice = Some(Notice::Plain {
                        why: "the other station sent plain text".into(),
                    });
                }
                if !self.plain {
                    bail!(
                        "unencrypted packet on a link that is {}",
                        if self.encrypted() {
                            "encrypted"
                        } else {
                            "setting up encryption"
                        }
                    );
                }
                step.app.push(other);
            }
        }
        Ok(step)
    }

    /// Call now and then: gives up on a handshake that never finishes.
    pub fn tick(&mut self, now: Instant) -> Result<Option<Notice>> {
        if self.ready() || now < self.deadline {
            return Ok(None);
        }
        match (self.policy, self.peer_hello.is_some()) {
            (Policy::Require, _) => {
                bail!(
                    "no key exchange within {} s, and this station requires encryption",
                    TIMEOUT.as_secs()
                )
            }
            (_, false) => {
                self.go_plain();
                Ok(Some(Notice::Plain {
                    why: "the other station never sent HELLO (not Whistlestop?)".into(),
                }))
            }
            (_, true) => bail!("the encryption handshake stalled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(name: &str) -> Link {
        Link::Hello {
            version: VERSION,
            caps: CAP_RESUME | CAP_ZIP,
            name: name.into(),
            grid: "EM12".into(),
            app: "test".into(),
        }
    }

    /// Two stations joined by an in-order byte pipe.
    struct Pair {
        a: Channel,
        b: Channel,
        a_rx: LinkDecoder,
        b_rx: LinkDecoder,
        to_a: Vec<u8>,
        to_b: Vec<u8>,
        a_notes: Vec<Notice>,
        b_notes: Vec<Notice>,
        a_app: Vec<Link>,
        b_app: Vec<Link>,
    }

    impl Pair {
        fn new(pa: Policy, pb: Policy) -> Pair {
            let now = Instant::now();
            let (a, oa) = Channel::open(pa, hello("A"), now);
            let (b, ob) = Channel::open(pb, hello("B"), now);
            Pair {
                a,
                b,
                a_rx: LinkDecoder::default(),
                b_rx: LinkDecoder::default(),
                to_a: ob.concat(),
                to_b: oa.concat(),
                a_notes: vec![],
                b_notes: vec![],
                a_app: vec![],
                b_app: vec![],
            }
        }
        /// Deliver everything in flight until both ends are quiet.
        fn run(&mut self) -> Result<()> {
            while !self.to_a.is_empty() || !self.to_b.is_empty() {
                for body in self.b_rx.feed_bodies(&std::mem::take(&mut self.to_b))? {
                    let s = self.b.on_body(&body)?;
                    self.to_a.extend(s.send.concat());
                    self.b_notes.extend(s.notice);
                    self.b_app.extend(s.app);
                }
                for body in self.a_rx.feed_bodies(&std::mem::take(&mut self.to_a))? {
                    let s = self.a.on_body(&body)?;
                    self.to_b.extend(s.send.concat());
                    self.a_notes.extend(s.notice);
                    self.a_app.extend(s.app);
                }
            }
            Ok(())
        }
    }

    fn secure_pair() -> Pair {
        let mut p = Pair::new(Policy::Prefer, Policy::Require);
        p.run().unwrap();
        p
    }

    #[test]
    fn handshake_agrees_on_keys_and_sas() {
        let p = secure_pair();
        assert!(p.a.encrypted() && p.b.encrypted());
        let (sa, sb) = (p.a.session.as_ref().unwrap(), p.b.session.as_ref().unwrap());
        assert_eq!(*sa.tx_key, *sb.rx_key);
        assert_eq!(*sa.rx_key, *sb.tx_key);
        assert_ne!(*sa.tx_key, *sa.rx_key, "one key per direction");
        assert_eq!(p.a.sas(), p.b.sas());
        let sas = p.a.sas().unwrap();
        assert!(
            sas.len() == 7 && sas.chars().filter(|c| c.is_ascii_digit()).count() == 6,
            "{sas}"
        );
        assert_eq!(p.a_notes, vec![Notice::Secure { sas: sas.into() }]);
        assert_eq!(p.b_notes, p.a_notes);
        // Both HELLOs reached the engine.
        assert!(
            matches!(p.a_app[..], [Link::Hello { ref name, caps, .. }] if name == "B" && caps & CAP_E2E != 0)
        );
        // A fresh link gets fresh keys.
        assert_ne!(secure_pair().a.sas(), Some(sas)); // (1 in a million to flake)
    }

    #[test]
    fn roundtrip_and_overhead() {
        let mut p = secure_pair();
        let msgs = [
            Link::Chat {
                id: 1,
                text: "73".into(),
            },
            Link::Chunk {
                fid: 1,
                offset: 0,
                data: vec![0xAB; CHUNK],
            },
            Link::Received { id: 9 },
        ];
        for m in &msgs {
            let w = p.a.wrap(m).unwrap();
            let extra = w.len() - m.encode().len();
            assert!(
                extra == SEALED_OVERHEAD || extra == SEALED_OVERHEAD + 1,
                "{extra}"
            );
            p.to_b.extend(w);
            p.to_a.extend(p.b.wrap(m).unwrap());
        }
        // "73" is 6 bytes in the clear and 6 + 17 = 23 sealed.
        assert_eq!(
            Link::Chat {
                id: 5,
                text: "73".into()
            }
            .encode()
            .len()
                + SEALED_OVERHEAD,
            23
        );
        p.run().unwrap();
        assert_eq!(p.b_app[1..], msgs);
        assert_eq!(p.a_app[1..], msgs);
        // The text is not on the air in the clear.
        let w =
            p.a.wrap(&Link::Chat {
                id: 2,
                text: "secret meeting place".into(),
            })
            .unwrap();
        assert!(!w.windows(6).any(|x| x == b"secret"));
    }

    #[test]
    fn tamper_is_detected() {
        let mut p = secure_pair();
        let mut w =
            p.a.wrap(&Link::Chat {
                id: 1,
                text: "hello".into(),
            })
            .unwrap();
        let n = w.len();
        w[n - 20] ^= 0x01; // a ciphertext bit
        p.to_b.extend(w);
        assert!(p.run().unwrap_err().to_string().contains("authentication"));
    }

    #[test]
    fn replay_and_reorder_are_rejected() {
        let mut p = secure_pair();
        let w1 =
            p.a.wrap(&Link::Chat {
                id: 1,
                text: "one".into(),
            })
            .unwrap();
        p.to_b.extend(&w1);
        p.run().unwrap();
        p.to_b.extend(&w1); // replay
        assert!(p.run().is_err());

        let mut p = secure_pair();
        let w1 =
            p.a.wrap(&Link::Chat {
                id: 1,
                text: "one".into(),
            })
            .unwrap();
        let w2 =
            p.a.wrap(&Link::Chat {
                id: 2,
                text: "two".into(),
            })
            .unwrap();
        p.to_b.extend(&w2); // reordered
        p.to_b.extend(&w1);
        assert!(p.run().is_err());
    }

    #[test]
    fn plaintext_injection_is_rejected() {
        let mut p = secure_pair();
        p.to_b.extend(
            Link::Chat {
                id: 7,
                text: "forged".into(),
            }
            .encode(),
        );
        assert!(p.run().unwrap_err().to_string().contains("unencrypted"));
    }

    #[test]
    fn queued_until_keys_agreed() {
        let now = Instant::now();
        let (mut a, _) = Channel::open(Policy::Prefer, hello("A"), now);
        assert!(!a.ready());
        assert!(
            a.wrap(&Link::Chat {
                id: 1,
                text: "wait".into()
            })
            .is_none()
        );
        let (mut off, _) = Channel::open(Policy::Off, hello("O"), now);
        assert_eq!(
            off.wrap(&Link::Received { id: 1 }),
            Some(Link::Received { id: 1 }.encode())
        );
    }

    #[test]
    fn require_refuses_plain_peer() {
        let mut p = Pair::new(Policy::Require, Policy::Off);
        let e = p.run().unwrap_err().to_string();
        assert!(e.contains("requires"), "{e}");
        assert!(!p.a.ready());

        // A peer that never answers at all: refused at the timeout.
        let now = Instant::now();
        let (mut a, _) = Channel::open(Policy::Require, hello("A"), now);
        assert!(a.tick(now + Duration::from_secs(10)).unwrap().is_none());
        assert!(a.tick(now + TIMEOUT).is_err());
        assert!(!a.ready());
    }

    #[test]
    fn prefer_falls_back_to_plain() {
        let mut p = Pair::new(Policy::Prefer, Policy::Off);
        p.run().unwrap();
        assert!(p.a.ready() && !p.a.encrypted() && !p.b.encrypted());
        assert!(matches!(p.a_notes[..], [Notice::Plain { .. }]));
        assert!(p.b_notes.is_empty());
        let w = p.a.wrap(&Link::Received { id: 3 }).unwrap();
        assert_eq!(w, Link::Received { id: 3 }.encode());
        // An old station that ignores KEYC entirely looks the same.
        let mut p = Pair::new(Policy::Off, Policy::Off);
        p.run().unwrap();
        assert!(p.a.ready() && p.b.ready() && p.a_notes.is_empty());
    }

    /// A relay that runs its own handshake with each side: the keys work,
    /// but the two SAS codes differ, so the operators can tell.
    #[test]
    fn man_in_the_middle_shows_different_sas() {
        let left = secure_pair(); // A <-> Mallory
        let right = secure_pair(); // Mallory <-> B
        assert!(left.a.encrypted() && right.b.encrypted());
        assert_ne!(left.a.sas(), right.b.sas());
    }

    #[test]
    fn swapped_key_breaks_commitment() {
        let now = Instant::now();
        let (mut a, oa) = Channel::open(Policy::Prefer, hello("A"), now);
        let (_b, ob) = Channel::open(Policy::Prefer, hello("B"), now);
        drop(oa);
        for body in LinkDecoder::default().feed_bodies(&ob.concat()).unwrap() {
            a.on_body(&body).unwrap();
        }
        // Mallory substitutes a key of her own for B's.
        let m = EphemeralSecret::random();
        let forged = Link::KeyX {
            suite: SUITE,
            pubkey: PublicKey::from(&m).to_bytes(),
        }
        .body();
        assert!(
            a.on_body(&forged)
                .unwrap_err()
                .to_string()
                .contains("commitment")
        );
    }

    #[test]
    fn secrets_wipe_on_drop() {
        fn wipes<T: zeroize::ZeroizeOnDrop>() {}
        wipes::<Key>();
        wipes::<EphemeralSecret>();
        wipes::<x25519_dalek::SharedSecret>();
        wipes::<ChaCha20Poly1305>();
        // After the handshake the private key is gone from the channel.
        let p = secure_pair();
        assert!(p.a.secret.is_none() && p.b.secret.is_none());
    }

    #[test]
    fn policy_toml_names() {
        #[derive(Serialize, Deserialize)]
        struct W {
            p: Policy,
        }
        assert_eq!(
            toml::to_string(&W { p: Policy::Require }).unwrap().trim(),
            "p = \"require\""
        );
    }
}
