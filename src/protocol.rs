//! WSP, the Whistlestop Protocol. Specification: docs/PROTOCOL.md.

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const VERSION: u8 = 1;
pub const CAP_RESUME: u8 = 1;
pub const CAP_ZIP: u8 = 2;
/// Set in HELLO when this station's encryption setting is Prefer or Require
/// (not merely when it could encrypt). See `crate::e2e`.
pub const CAP_E2E: u8 = 4;
pub const CHUNK: usize = 1024;
pub const TABLE_MAGIC: u8 = 0x57;
const MAX_PACKET: usize = 1 << 20;

// ------------------------------------------------------------- primitives

#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn i8(&mut self, v: i8) -> &mut Self {
        self.0.push(v as u8);
        self
    }
    pub fn uv(&mut self, mut v: u64) -> &mut Self {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(b);
                return self;
            }
            self.0.push(b | 0x80);
        }
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.uv(b.len() as u64);
        self.0.extend_from_slice(b);
        self
    }
    pub fn str(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes())
    }
    pub fn call(&mut self, c: &str) -> &mut Self {
        let c = c.to_ascii_uppercase();
        // At most 12 bytes, cut on a character boundary.
        let mut end = c.len().min(12);
        while !c.is_char_boundary(end) {
            end -= 1;
        }
        self.str(&c[..end])
    }
    pub fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
}

/// Does this look like a callsign (or a tactical name like BRAVO)? 1–12
/// characters: letters, digits, `/` and `-`.
pub fn valid_call(c: &str) -> bool {
    !c.is_empty()
        && c.len() <= 12
        && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'/' || b == b'-')
}

pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        // A length from the air can be anything: no overflow.
        if self.pos.checked_add(n).is_none_or(|end| end > self.b.len()) {
            bail!("packet too short");
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn uv(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("varint too long")
    }
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.uv()? as usize;
        self.take(n)
    }
    pub fn str(&mut self) -> Result<String> {
        Ok(std::str::from_utf8(self.bytes()?)?.to_string())
    }
    /// A callsign: refused unless it looks like one (see `valid_call`), so
    /// nothing from the air can smuggle text into a Mercury command.
    pub fn call(&mut self) -> Result<String> {
        let c = self.str()?;
        if !valid_call(&c) {
            bail!("not a callsign");
        }
        Ok(c)
    }
    pub fn hash(&mut self) -> Result<[u8; 32]> {
        Ok(self.take(32)?.try_into()?)
    }
    /// Everything left in the packet.
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.pos..];
        self.pos = self.b.len();
        s
    }
}

/// The length prefix at the start of `b`: Ok(Some((length, prefix bytes))),
/// Ok(None) if more bytes are needed, or an error if no valid prefix can
/// start here (more than 4 bytes; MAX_PACKET needs only 3).
fn read_varint_prefix(b: &[u8]) -> Result<Option<(usize, usize)>> {
    let mut v = 0usize;
    for (i, &byte) in b.iter().enumerate().take(4) {
        v |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(Some((v, i + 1)));
        }
    }
    if b.len() >= 4 {
        bail!("corrupt link stream (length prefix too long)");
    }
    Ok(None)
}

// -------------------------------------------------------------- link plane

pub const T_HELLO: u8 = 0x01;
pub const T_KEYC: u8 = 0x0B;
pub const T_KEYX: u8 = 0x0C;
pub const T_SEALED: u8 = 0x0D;
pub const T_MAIL: u8 = 0x0E;
pub const T_MAILACK: u8 = 0x0F;
pub const T_QSY: u8 = 0x20;
pub const T_QSYANS: u8 = 0x21;
pub const T_LIST: u8 = 0x22;
pub const T_LISTANS: u8 = 0x23;
pub const T_FETCH: u8 = 0x24;
pub const T_FETCHANS: u8 = 0x25;

/// One entry of a station's file library listing.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LibEntry {
    pub name: String,
    pub size: u64,
    pub dir: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Hello {
        version: u8,
        caps: u8,
        name: String,
        grid: String,
        app: String,
    },
    Chat {
        id: u64,
        text: String,
    },
    /// The CHAT `id` arrived (sent automatically on receipt).
    Received {
        id: u64,
    },
    Offer {
        fid: u64,
        name: String,
        size: u64,
        sha: [u8; 32],
        zipped: bool,
        files: u64,
    },
    Answer {
        fid: u64,
        accept: bool,
        resume_from: u64,
    },
    Chunk {
        fid: u64,
        offset: u64,
        data: Vec<u8>,
    },
    Done {
        fid: u64,
        ok: bool,
    },
    Cancel {
        fid: u64,
    },
    Report {
        snr_db: i8,
        mode_index: u8,
    },
    Bye {
        text: String,
    },
    /// Depot mail, store-and-forward (sent automatically to the station
    /// it's addressed to when linked).
    Mail {
        id: u64,
        from: String,
        to: String,
        created: u64,
        hops: u8,
        text: String,
    },
    /// Mail `id` is safely in the other station's Depot.
    MailAck {
        id: u64,
    },
    /// E2E handshake, step 1: a commitment to the ephemeral public key.
    KeyCommit {
        suite: u8,
        commit: [u8; 32],
    },
    /// E2E handshake, step 2: the public key itself (sent only after the
    /// peer's commitment has arrived).
    KeyX {
        suite: u8,
        pubkey: [u8; 32],
    },
    /// One encrypted inner link packet body (ciphertext + 16-byte tag).
    Sealed {
        data: Vec<u8>,
    },
    /// "Let's both move to `dial_hz`", optionally with a reason.
    Qsy {
        id: u64,
        dial_hz: u64,
        why: String,
    },
    /// The answer to QSY `id`; when accepted, both stations move once this
    /// has crossed, and both go home when the link ends.
    QsyAnswer {
        id: u64,
        accept: bool,
        reason: String,
    },
    /// "What's in your library at `path`?" ("" is the top).
    ListReq {
        id: u64,
        path: String,
    },
    /// The listing (or, with no entries, why not in `note`).
    ListAns {
        id: u64,
        path: String,
        entries: Vec<LibEntry>,
        note: String,
    },
    /// "Please send me `path` from your library."
    Fetch {
        id: u64,
        path: String,
    },
    /// Refused (with a reason), or OK: an OFFER with fid = `id` follows.
    FetchAnswer {
        id: u64,
        ok: bool,
        reason: String,
    },
    Unknown(u8),
}

impl Link {
    /// Body without the length prefix.
    pub fn body(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Link::Hello {
                version,
                caps,
                name,
                grid,
                app,
            } => {
                w.u8(0x01)
                    .u8(*version)
                    .u8(*caps)
                    .str(name)
                    .str(grid)
                    .str(app);
            }
            Link::Chat { id, text } => {
                w.u8(0x02).uv(*id).str(text);
            }
            Link::Received { id } => {
                w.u8(0x03).uv(*id);
            }
            Link::Offer {
                fid,
                name,
                size,
                sha,
                zipped,
                files,
            } => {
                w.u8(0x04)
                    .uv(*fid)
                    .str(name)
                    .uv(*size)
                    .raw(sha)
                    .u8(*zipped as u8)
                    .uv(*files);
            }
            Link::Answer {
                fid,
                accept,
                resume_from,
            } => {
                w.u8(0x05).uv(*fid).u8(*accept as u8).uv(*resume_from);
            }
            Link::Chunk { fid, offset, data } => {
                w.u8(0x06).uv(*fid).uv(*offset).bytes(data);
            }
            Link::Done { fid, ok } => {
                w.u8(0x07).uv(*fid).u8(*ok as u8);
            }
            Link::Cancel { fid } => {
                w.u8(0x08).uv(*fid);
            }
            Link::Report { snr_db, mode_index } => {
                w.u8(0x09).i8(*snr_db).u8(*mode_index);
            }
            Link::Bye { text } => {
                w.u8(0x0A).str(text);
            }
            Link::Mail {
                id,
                from,
                to,
                created,
                hops,
                text,
            } => {
                w.u8(T_MAIL)
                    .uv(*id)
                    .call(from)
                    .call(to)
                    .uv(*created)
                    .u8(*hops)
                    .str(text);
            }
            Link::MailAck { id } => {
                w.u8(T_MAILACK).uv(*id);
            }
            Link::Qsy { id, dial_hz, why } => {
                w.u8(T_QSY).uv(*id).uv(*dial_hz).str(why);
            }
            Link::QsyAnswer { id, accept, reason } => {
                w.u8(T_QSYANS).uv(*id).u8(*accept as u8).str(reason);
            }
            Link::ListReq { id, path } => {
                w.u8(T_LIST).uv(*id).str(path);
            }
            Link::ListAns {
                id,
                path,
                entries,
                note,
            } => {
                w.u8(T_LISTANS)
                    .uv(*id)
                    .str(path)
                    .str(note)
                    .uv(entries.len() as u64);
                for e in entries {
                    w.str(&e.name).uv(e.size).u8(e.dir as u8);
                }
            }
            Link::Fetch { id, path } => {
                w.u8(T_FETCH).uv(*id).str(path);
            }
            Link::FetchAnswer { id, ok, reason } => {
                w.u8(T_FETCHANS).uv(*id).u8(*ok as u8).str(reason);
            }
            Link::KeyCommit { suite, commit } => {
                w.u8(T_KEYC).u8(*suite).raw(commit);
            }
            Link::KeyX { suite, pubkey } => {
                w.u8(T_KEYX).u8(*suite).raw(pubkey);
            }
            Link::Sealed { data } => {
                w.u8(T_SEALED).raw(data);
            }
            Link::Unknown(t) => {
                w.u8(*t);
            }
        }
        w.0
    }

    /// The packet as it goes on the wire, length prefix included.
    pub fn encode(&self) -> Vec<u8> {
        let body = self.body();
        let mut w = Writer::default();
        w.uv(body.len() as u64).raw(&body);
        w.0
    }

    pub fn decode(body: &[u8]) -> Result<Link> {
        let mut r = Reader::new(body);
        Ok(match r.u8()? {
            0x01 => Link::Hello {
                version: r.u8()?,
                caps: r.u8()?,
                name: r.str()?,
                grid: r.str()?,
                app: r.str()?,
            },
            0x02 => Link::Chat {
                id: r.uv()?,
                text: r.str()?,
            },
            0x03 => Link::Received { id: r.uv()? },
            0x04 => Link::Offer {
                fid: r.uv()?,
                name: r.str()?,
                size: r.uv()?,
                sha: r.hash()?,
                zipped: r.u8()? & 1 == 1,
                files: r.uv()?,
            },
            0x05 => Link::Answer {
                fid: r.uv()?,
                accept: r.u8()? == 1,
                resume_from: r.uv()?,
            },
            0x06 => Link::Chunk {
                fid: r.uv()?,
                offset: r.uv()?,
                data: r.bytes()?.to_vec(),
            },
            0x07 => Link::Done {
                fid: r.uv()?,
                ok: r.u8()? == 1,
            },
            0x08 => Link::Cancel { fid: r.uv()? },
            0x09 => Link::Report {
                snr_db: r.i8()?,
                mode_index: r.u8()?,
            },
            0x0A => Link::Bye { text: r.str()? },
            T_MAIL => Link::Mail {
                id: r.uv()?,
                from: r.call()?,
                to: r.call()?,
                created: r.uv()?,
                hops: r.u8()?,
                text: r.str()?,
            },
            T_MAILACK => Link::MailAck { id: r.uv()? },
            T_QSY => Link::Qsy {
                id: r.uv()?,
                dial_hz: r.uv()?,
                why: r.str()?,
            },
            T_QSYANS => Link::QsyAnswer {
                id: r.uv()?,
                accept: r.u8()? != 0,
                reason: r.str()?,
            },
            T_LIST => Link::ListReq {
                id: r.uv()?,
                path: r.str()?,
            },
            T_LISTANS => {
                let (id, path, note) = (r.uv()?, r.str()?, r.str()?);
                let n = r.uv()?.min(10_000);
                let mut entries = vec![];
                for _ in 0..n {
                    entries.push(LibEntry {
                        name: r.str()?,
                        size: r.uv()?,
                        dir: r.u8()? != 0,
                    });
                }
                Link::ListAns {
                    id,
                    path,
                    entries,
                    note,
                }
            }
            T_FETCH => Link::Fetch {
                id: r.uv()?,
                path: r.str()?,
            },
            T_FETCHANS => Link::FetchAnswer {
                id: r.uv()?,
                ok: r.u8()? != 0,
                reason: r.str()?,
            },
            T_KEYC => Link::KeyCommit {
                suite: r.u8()?,
                commit: r.hash()?,
            },
            T_KEYX => Link::KeyX {
                suite: r.u8()?,
                pubkey: r.hash()?,
            },
            T_SEALED => Link::Sealed {
                data: r.rest().to_vec(),
            },
            t => Link::Unknown(t),
        })
    }
}

/// Splits Mercury's ARQ byte stream into WSP link packets.
///
/// The stream has no resync marker, so once a length prefix is corrupt
/// nothing after it can be trusted: the decoder drops what it buffered and
/// refuses everything until `reset` (or a new decoder for the next link).
#[derive(Default)]
pub struct LinkDecoder {
    buf: Vec<u8>,
    corrupt: bool,
}

impl LinkDecoder {
    /// Whether the stream has been found corrupt (see the type's docs).
    pub fn is_corrupt(&self) -> bool {
        self.corrupt
    }

    /// Start over, as for a new link.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<Link>> {
        self.feed_bodies(data)?
            .iter()
            .map(|b| Link::decode(b))
            .collect()
    }

    /// Like `feed`, but hands back the raw packet bodies (the E2E handshake
    /// hashes the HELLOs exactly as they were sent).
    pub fn feed_bodies(&mut self, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        if self.corrupt {
            bail!("corrupt link stream");
        }
        self.buf.extend_from_slice(data);
        let mut out = vec![];
        loop {
            let (n, hdr) = match read_varint_prefix(&self.buf) {
                Ok(Some(x)) => x,
                Ok(None) => break,
                Err(e) => {
                    self.buf = vec![];
                    self.corrupt = true;
                    return Err(e);
                }
            };
            if n > MAX_PACKET {
                self.buf = vec![];
                self.corrupt = true;
                bail!("corrupt link stream (packet length {n})");
            }
            if self.buf.len() < hdr + n {
                break;
            }
            out.push(self.buf[hdr..hdr + n].to_vec());
            self.buf.drain(..hdr + n);
        }
        Ok(out)
    }
}

/// Air bytes a chat line costs, for the "how long will this take" display.
pub fn chat_wire_size(text: &str) -> usize {
    Link::Chat {
        id: 1000,
        text: text.into(),
    }
    .encode()
    .len()
}

/// Air bytes a file of `size` costs: offer + chunks + done.
pub fn file_wire_size(name: &str, size: u64) -> usize {
    let chunks = (size as usize).div_ceil(CHUNK).max(1);
    let offer = Link::Offer {
        fid: 1,
        name: name.into(),
        size,
        sha: [0; 32],
        zipped: true,
        files: 1,
    }
    .encode()
    .len();
    offer + size as usize + chunks * 8 + 6
}

// ------------------------------------------------------- file transfer state

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// An outgoing file waiting for (or in) transfer.
pub struct Outgoing {
    pub fid: u64,
    pub name: String,
    pub data: Vec<u8>,
    pub sha: [u8; 32],
    pub zipped: bool,
    pub files: u64,
    /// Next offset to hand to the modem; None until the peer accepts.
    pub next: Option<u64>,
    /// The station it's for: offered (and resumed) only on a link with them.
    /// None: picked while not linked; it goes to the next station linked.
    pub to: Option<String>,
}

impl Outgoing {
    pub fn new(fid: u64, name: String, data: Vec<u8>, zipped: bool, files: u64) -> Self {
        let sha = sha256(&data);
        Self {
            fid,
            name,
            data,
            sha,
            zipped,
            files,
            next: None,
            to: None,
        }
    }
    pub fn offer(&self) -> Link {
        Link::Offer {
            fid: self.fid,
            name: self.name.clone(),
            size: self.data.len() as u64,
            sha: self.sha,
            zipped: self.zipped,
            files: self.files,
        }
    }
    /// Next chunk to send, if accepted and not finished.
    pub fn next_chunk(&mut self) -> Option<Link> {
        let off = self.next?;
        if off as usize >= self.data.len() {
            return None;
        }
        let end = (off as usize + CHUNK).min(self.data.len());
        self.next = Some(end as u64);
        Some(Link::Chunk {
            fid: self.fid,
            offset: off,
            data: self.data[off as usize..end].to_vec(),
        })
    }
    pub fn finished_sending(&self) -> bool {
        self.next.is_some_and(|n| n as usize >= self.data.len())
    }
}

/// An incoming file. Partial data is kept on disk under its SHA-256 so a
/// broken transfer can resume.
pub struct Incoming {
    pub fid: u64,
    pub name: String,
    pub size: u64,
    pub sha: [u8; 32],
    pub zipped: bool,
    pub data: Vec<u8>,
    /// Set once the operator (or a library fetch) accepted the offer:
    /// pieces that arrive before that are not ours to keep.
    pub accepted: bool,
    partial_path: PathBuf,
}

impl Incoming {
    pub fn new(dir: &Path, fid: u64, name: String, size: u64, sha: [u8; 32], zipped: bool) -> Self {
        let partial_path = dir.join(".partial").join(hex::encode(sha));
        let data = std::fs::read(&partial_path).unwrap_or_default();
        let data = if data.len() as u64 <= size {
            data
        } else {
            vec![]
        };
        Self {
            fid,
            name,
            size,
            sha,
            zipped,
            data,
            accepted: false,
            partial_path,
        }
    }
    pub fn have(&self) -> u64 {
        self.data.len() as u64
    }
    /// Apply a chunk. Out-of-order chunks are ignored (ARQ keeps order).
    pub fn chunk(&mut self, offset: u64, data: &[u8]) {
        if offset == self.have() {
            // Never more than the offered size.
            let room = self.size.saturating_sub(self.have()).min(data.len() as u64) as usize;
            self.data.extend_from_slice(&data[..room]);
        }
    }
    pub fn complete(&self) -> bool {
        self.have() >= self.size
    }
    pub fn save_partial(&self) {
        // Nothing received yet: nothing to resume from (and no empty file).
        if self.data.is_empty() {
            return;
        }
        if let Some(p) = self.partial_path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::write(&self.partial_path, &self.data);
    }
    /// Verify and save. Returns the final path.
    pub fn finish(self, dir: &Path) -> Result<PathBuf> {
        if sha256(&self.data) != self.sha {
            let _ = std::fs::remove_file(&self.partial_path);
            bail!("checksum mismatch on {}", self.name);
        }
        std::fs::create_dir_all(dir)?;
        let path = unique_path(&dir.join(safe_file_name(&self.name)));
        std::fs::write(&path, &self.data)?;
        let _ = std::fs::remove_file(&self.partial_path);
        Ok(path)
    }
}

/// Longest received file name kept, in bytes (filesystems allow 255; this
/// leaves room for the "-N" that `unique_path` may add).
pub const MAX_NAME_BYTES: usize = 200;

/// A file name from the air made safe to save on any of our platforms:
/// only the last path component (either separator), no control characters
/// or characters Windows refuses, no trailing dots or spaces, Windows
/// device names (CON, NUL, COM1.txt, ...) prefixed with '_', at most
/// `MAX_NAME_BYTES` (keeping the extension), and "received.bin" if
/// nothing is left.
pub fn safe_file_name(raw: &str) -> String {
    const FALLBACK: &str = "received.bin";
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut name: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect();
    let trimmed = name.trim_end_matches(['.', ' ']).len();
    name.truncate(trimmed);
    if name.trim().is_empty() {
        return FALLBACK.into();
    }
    // Windows device names, with or without an extension.
    let device = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((device.starts_with("COM") || device.starts_with("LPT"))
            && device.len() == 4
            && matches!(device.as_bytes()[3], b'1'..=b'9'));
    // A leading dot would hide the file, and ".partial" would take the
    // name of the folder that resumable downloads are kept in.
    if reserved || name.starts_with('.') {
        name.insert(0, '_');
    }
    if name.len() > MAX_NAME_BYTES {
        // Keep a sensible extension (".txt", ".tar"), shorten the rest.
        let ext = match name.rfind('.') {
            Some(i) if i > 0 && name.len() - i <= 16 => name[i..].to_string(),
            _ => String::new(),
        };
        let mut cut = MAX_NAME_BYTES - ext.len();
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        let stem = name[..cut].trim_end_matches(['.', ' ']).to_string();
        if stem.trim().is_empty() {
            return FALLBACK.into();
        }
        name = stem + &ext;
    }
    name
}

fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|i| p.with_file_name(format!("{stem}-{i}{ext}")))
        .find(|c| !c.exists())
        .unwrap()
}

// ------------------------------------------------------------- table plane

/// FNV-1a of the table name, folded to 16 bits.
pub fn table_id(name: &str) -> u16 {
    let mut h: u32 = 0x811c9dc5;
    for b in name.trim().to_ascii_uppercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    ((h >> 16) ^ (h & 0xffff)) as u16
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableBody {
    Call {
        kind: u8,
        table_name: String,
        bw_code: u8,
        grid: String,
    },
    Text {
        id: u64,
        part: u8,
        total: u8,
        data: Vec<u8>,
    },
    Join {
        table_name: String,
        /// How well this member hears the host, dB; `SNR_UNKNOWN` until it
        /// has measured the host's signal.
        snr_of_host: i8,
        /// The member's bandwidth setting (an index into `Bandwidth::ALL`,
        /// like CALL's), so the host never picks a mode wider than it. A
        /// trailing byte: older versions leave it off, and skip it.
        bw_code: Option<u8>,
    },
    Speed {
        mode_index: u8,
        /// The host's newest message id (0: none). Optional on the wire
        /// (older stations don't send it, and ignore it).
        last_id: u64,
    },
    Again {
        of: String,
        id: u64,
        missing: u64,
    },
    Floor {
        to: String,
    },
    Leave,
    /// Station ID beacon: who, where, whether away, and who has mail here.
    Beacon {
        grid: String,
        away: bool,
        mail_for: Vec<String>,
    },
    /// "I have `count` messages for `to`: connect to me to collect them."
    Notice {
        to: String,
        count: u8,
    },
}

pub const CALL_CHAT: u8 = 0;
pub const CALL_TABLE: u8 = 1;

/// A JOIN/report's `snr_of_host` when the member hasn't measured the host
/// yet: the host leaves it out of its speed choice. (An older host reads it
/// as a very weak member and slows down: the safe side.)
pub const SNR_UNKNOWN: i8 = i8::MIN;

#[derive(Clone, Debug, PartialEq)]
pub struct TableFrame {
    pub table: u16,
    pub from: String,
    pub body: TableBody,
}

impl TableFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        let t = match &self.body {
            TableBody::Call { .. } => 0x10,
            TableBody::Text { .. } => 0x11,
            TableBody::Join { .. } => 0x12,
            TableBody::Speed { .. } => 0x13,
            TableBody::Again { .. } => 0x14,
            TableBody::Floor { .. } => 0x15,
            TableBody::Leave => 0x16,
            TableBody::Beacon { .. } => 0x17,
            TableBody::Notice { .. } => 0x18,
        };
        w.u8(TABLE_MAGIC)
            .u8(t)
            .uv(self.table as u64)
            .call(&self.from);
        match &self.body {
            TableBody::Call {
                kind,
                table_name,
                bw_code,
                grid,
            } => {
                w.u8(*kind).str(table_name).u8(*bw_code).str(grid);
            }
            TableBody::Text {
                id,
                part,
                total,
                data,
            } => {
                w.uv(*id).u8(*part).u8(*total).bytes(data);
            }
            TableBody::Join {
                table_name,
                snr_of_host,
                bw_code,
            } => {
                w.str(table_name).i8(*snr_of_host);
                if let Some(b) = bw_code {
                    w.u8(*b);
                }
            }
            TableBody::Speed { mode_index, last_id } => {
                w.u8(*mode_index);
                if *last_id > 0 {
                    w.uv(*last_id);
                }
            }
            TableBody::Again { of, id, missing } => {
                w.call(of).uv(*id).uv(*missing);
            }
            TableBody::Floor { to } => {
                w.call(to);
            }
            TableBody::Leave => {}
            TableBody::Beacon {
                grid,
                away,
                mail_for,
            } => {
                w.str(grid).u8(*away as u8).uv(mail_for.len() as u64);
                for c in mail_for {
                    w.call(c);
                }
            }
            TableBody::Notice { to, count } => {
                w.call(to).u8(*count);
            }
        }
        w.0
    }

    /// None if this broadcast frame isn't WSP (another app's traffic).
    pub fn decode(b: &[u8]) -> Option<TableFrame> {
        let mut r = Reader::new(b);
        if r.u8().ok()? != TABLE_MAGIC {
            return None;
        }
        let t = r.u8().ok()?;
        let table = r.uv().ok()? as u16;
        let from = r.call().ok()?;
        let body = (|| -> Result<TableBody> {
            Ok(match t {
                0x10 => TableBody::Call {
                    kind: r.u8()?,
                    table_name: r.str()?,
                    bw_code: r.u8()?,
                    grid: r.str()?,
                },
                0x11 => {
                    let (id, part, total) = (r.uv()?, r.u8()?, r.u8()?);
                    // Missing parts are tracked in a 64-bit map.
                    if total == 0 || total > 64 || part == 0 || part > total {
                        bail!("bad part number");
                    }
                    TableBody::Text {
                        id,
                        part,
                        total,
                        data: r.bytes()?.to_vec(),
                    }
                }
                0x12 => TableBody::Join {
                    table_name: r.str()?,
                    snr_of_host: r.i8()?,
                    bw_code: r.u8().ok(),
                },
                0x13 => TableBody::Speed {
                    mode_index: r.u8()?,
                    last_id: r.uv().unwrap_or(0),
                },
                0x14 => TableBody::Again {
                    of: r.call()?,
                    id: r.uv()?,
                    missing: r.uv()?,
                },
                0x15 => TableBody::Floor { to: r.call()? },
                0x16 => TableBody::Leave,
                0x17 => {
                    let grid = r.str()?;
                    let away = r.u8()? & 1 == 1;
                    let n = r.uv()?.min(32);
                    let mut mail_for = vec![];
                    for _ in 0..n {
                        mail_for.push(r.call()?);
                    }
                    TableBody::Beacon {
                        grid,
                        away,
                        mail_for,
                    }
                }
                0x18 => TableBody::Notice {
                    to: r.call()?,
                    count: r.u8()?,
                },
                _ => bail!("unknown"),
            })
        })()
        .ok()?;
        Some(TableFrame { table, from, body })
    }
}

/// Split a Roundtable message into TEXT frames that each fit `frame_bytes`.
pub fn table_text_frames(
    table: u16,
    from: &str,
    id: u64,
    text: &str,
    frame_bytes: usize,
) -> Result<Vec<Vec<u8>>> {
    let overhead = TableFrame {
        table,
        from: from.into(),
        body: TableBody::Text {
            id,
            part: 255,
            total: 255,
            data: vec![0; 127],
        },
    }
    .encode()
    .len()
        - 127;
    if frame_bytes < overhead + 8 {
        bail!("frame of {frame_bytes} bytes is too small for Roundtable text");
    }
    // A data length >= 128 takes a 2-byte varint; leave room for it.
    let room = (frame_bytes - overhead - 1).min(CHUNK);
    let mut parts: Vec<&[u8]> = vec![];
    let mut rest = text;
    while !rest.is_empty() {
        let mut cut = room.min(rest.len());
        while !rest.is_char_boundary(cut) {
            cut -= 1; // never split a UTF-8 character
        }
        parts.push(&rest.as_bytes()[..cut]);
        rest = &rest[cut..];
    }
    if parts.is_empty() {
        parts.push(b"");
    }
    if parts.len() > 64 {
        bail!("message too long for one Roundtable message (max 64 frames)");
    }
    let total = parts.len() as u8;
    Ok(parts
        .iter()
        .enumerate()
        .map(|(i, d)| {
            TableFrame {
                table,
                from: from.into(),
                body: TableBody::Text {
                    id,
                    part: i as u8 + 1,
                    total,
                    data: d.to_vec(),
                },
            }
            .encode()
        })
        .collect())
}

/// A table frame (not TEXT) made to fit a broadcast frame of `max` bytes:
/// a JOIN leaves its table name out (the host knows its table by number),
/// an AGAIN asks only for the lowest of its missing parts (the rest are
/// asked for in later rounds). None if it can't be made to fit. A TEXT
/// frame isn't shrunk: its whole message is split again instead.
pub fn shrink_table_frame(f: &[u8], max: usize) -> Option<Vec<u8>> {
    if f.len() <= max {
        return Some(f.to_vec());
    }
    let mut fr = TableFrame::decode(f)?;
    match &mut fr.body {
        TableBody::Join { table_name, .. } => table_name.clear(),
        TableBody::Again { .. } => loop {
            let e = fr.encode();
            if e.len() <= max {
                return Some(e);
            }
            let TableBody::Again { missing, .. } = &mut fr.body else {
                unreachable!()
            };
            if missing.count_ones() <= 1 {
                return None;
            }
            *missing &= !(1u64 << (63 - missing.leading_zeros()));
        },
        _ => return None,
    }
    let e = fr.encode();
    (e.len() <= max).then_some(e)
}

/// The largest table number, and a message id beyond any session's (ids
/// start below 2^20): the worst case when sizing frames.
const WORST_TABLE: u16 = u16::MAX;
const WORST_ID: u64 = 1 << 20;

/// The two longest of `calls` (the second may be the first again).
fn two_longest(calls: &[String]) -> Option<(&str, &str)> {
    let mut v: Vec<&str> = calls.iter().map(String::as_str).collect();
    v.sort_by_key(|c| std::cmp::Reverse(c.len()));
    let a = *v.first()?;
    Some((a, v.get(1).copied().unwrap_or(a)))
}

/// How many parts a message may have so that an AGAIN for any one of them,
/// between any two of these `calls`, fits a broadcast frame of `max` bytes
/// (the bitmap grows with the part's number). 64 in all but the smallest
/// frames; 0 if not even part 1 fits.
pub fn again_parts_limit(max: usize, calls: &[String]) -> usize {
    let Some((a, b)) = two_longest(calls) else { return 64 };
    (1..=64)
        .rev()
        .find(|n| {
            TableFrame {
                table: WORST_TABLE,
                from: a.into(),
                body: TableBody::Again {
                    of: b.into(),
                    id: WORST_ID,
                    missing: 1 << (n - 1),
                },
            }
            .encode()
            .len()
                <= max
        })
        .unwrap_or(0)
}

/// Whether a table whose stations have these `calls` can work in broadcast
/// frames of `max` bytes: each can send TEXT (`table_text_frames` takes
/// the frame), in messages of 8 parts or more that can all be asked for again
/// (JOIN, SPEED, FLOOR and LEAVE are smaller). Long callsigns don't fit the
/// smallest modes.
pub fn table_frames_fit(max: usize, calls: &[String]) -> bool {
    let Some((a, _)) = two_longest(calls) else { return true };
    let text = table_text_frames(WORST_TABLE, a, WORST_ID, "x", max).is_ok();
    text && again_parts_limit(max, calls) >= 8
}

/// `text` as Roundtable messages of at most `limit` parts each in frames
/// of `frame_bytes` (see `again_parts_limit`): as it is when it fits,
/// otherwise cut into several, between words where it can be.
pub fn split_for_parts(from: &str, text: &str, frame_bytes: usize, limit: usize) -> Result<Vec<String>> {
    let parts = |s: &str| table_text_frames(WORST_TABLE, from, WORST_ID, s, frame_bytes).map(|v| v.len());
    if limit >= 64 || parts(text).is_ok_and(|n| n <= limit) {
        return Ok(vec![text.to_string()]);
    }
    let first = text.chars().next().map(char::len_utf8).unwrap_or(0);
    parts(&text[..first])?; // the frames can carry text at all
    let mut out = vec![];
    let mut rest = text;
    while !rest.is_empty() {
        if parts(rest).is_ok_and(|n| n <= limit) {
            out.push(rest.to_string());
            break;
        }
        // The longest start that fits (binary search on char boundaries).
        let (mut lo, mut hi) = (0, rest.len());
        while lo < hi {
            let mut mid = (lo + hi).div_ceil(2);
            while !rest.is_char_boundary(mid) {
                mid -= 1;
            }
            if mid > lo && parts(&rest[..mid]).is_ok_and(|n| n <= limit) {
                lo = mid;
            } else {
                hi = mid.max(lo + 1) - 1;
                while !rest.is_char_boundary(hi) {
                    hi -= 1;
                }
            }
        }
        if lo == 0 {
            bail!("the frames at this speed are too small for Roundtable text");
        }
        let cut = match rest[..lo].rfind(' ') {
            Some(i) if i > lo / 2 => i + 1,
            _ => lo,
        };
        let piece = rest[..cut].trim_end();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        rest = rest[cut..].trim_start();
    }
    Ok(out)
}

/// How long an unfinished message waits for its missing parts.
pub const TABLE_PARTIAL_TTL: Duration = Duration::from_secs(10 * 60);
/// How long (and how many) finished messages are remembered, so parts
/// resent later for another member are recognised and ignored.
pub const TABLE_DONE_TTL: Duration = Duration::from_secs(10 * 60);
pub const TABLE_DONE_MAX: usize = 256;

/// One unfinished message: total parts, parts received by number, and
/// when the last part arrived.
struct Partial {
    total: u8,
    got: HashMap<u8, Vec<u8>>,
    last: Instant,
}

/// Reassembles TEXT parts and reports what is missing (for AGAIN).
///
/// A message is delivered once. Parts of a message finished in the last
/// `TABLE_DONE_TTL` (at most the last `TABLE_DONE_MAX` messages) are
/// ignored: in a Roundtable parts are resent whenever *any* member asks
/// AGAIN, and those must neither show the message twice nor make this
/// station ask AGAIN itself. Unfinished messages are given up after
/// `TABLE_PARTIAL_TTL` without a new part.
#[derive(Default)]
pub struct TableAssembler {
    parts: HashMap<(String, u64), Partial>,
    done: HashMap<(String, u64), Instant>,
    done_order: VecDeque<((String, u64), Instant)>,
    /// Each sender's newest message id seen: a jump means messages we never
    /// heard a single frame of (a one-frame message lost whole).
    latest: HashMap<String, u64>,
    /// Those messages (asked for whole), and since when.
    wanted: HashMap<(String, u64), Instant>,
}

/// At most this many skipped messages per sender are asked for.
const TABLE_GAP_MAX: u64 = 16;

impl TableAssembler {
    pub fn add(&mut self, from: &str, id: u64, part: u8, total: u8, data: &[u8]) -> Option<String> {
        self.add_at(Instant::now(), from, id, part, total, data)
    }

    /// `add` with an explicit clock (for tests).
    pub fn add_at(
        &mut self,
        now: Instant,
        from: &str,
        id: u64,
        part: u8,
        total: u8,
        data: &[u8],
    ) -> Option<String> {
        self.expire(now);
        if total == 0 || total > 64 || part == 0 || part > total {
            return None;
        }
        let key = (from.to_string(), id);
        self.wanted.remove(&key);
        self.note_latest_at(now, from, id, false);
        if self.done.contains_key(&key) {
            return None;
        }
        let e = self.parts.entry(key.clone()).or_insert_with(|| Partial {
            total,
            got: HashMap::new(),
            last: now,
        });
        if total != e.total {
            // Its sender split it again, into smaller parts: the table slowed
            // down since, and the old parts didn't fit the new speed. (A
            // re-split always has a different total.) Start it over.
            *e = Partial {
                total,
                got: HashMap::new(),
                last: now,
            };
        }
        e.got.insert(part, data.to_vec());
        e.last = now;
        if (1..=e.total).all(|i| e.got.contains_key(&i)) {
            let p = self.parts.remove(&key)?;
            let mut all = vec![];
            for i in 1..=p.total {
                all.extend_from_slice(p.got.get(&i)?);
            }
            self.remember_done(key, now);
            return Some(String::from_utf8_lossy(&all).into_owned());
        }
        None
    }

    fn remember_done(&mut self, key: (String, u64), now: Instant) {
        self.done.insert(key.clone(), now);
        self.done_order.push_back((key, now));
        while self.done_order.len() > TABLE_DONE_MAX {
            if let Some((k, t)) = self.done_order.pop_front()
                && self.done.get(&k) == Some(&t)
            {
                self.done.remove(&k);
            }
        }
    }

    /// Forget unfinished messages that have waited too long, and finished
    /// ones old enough that their parts won't be resent.
    pub fn expire(&mut self, now: Instant) {
        self.parts
            .retain(|_, p| now.saturating_duration_since(p.last) < TABLE_PARTIAL_TTL);
        self.wanted
            .retain(|_, t| now.saturating_duration_since(*t) < TABLE_PARTIAL_TTL);
        if self.latest.len() > 256 {
            self.latest.clear();
        }
        while let Some((k, t)) = self.done_order.front() {
            if now.saturating_duration_since(*t) < TABLE_DONE_TTL {
                break;
            }
            if self.done.get(k) == Some(t) {
                self.done.remove(k);
            }
            self.done_order.pop_front();
        }
    }

    /// `from` has sent messages up to `id` (inclusive: the host's heartbeat
    /// says so; not inclusive: a frame of message `id` itself arrived). Any
    /// we never heard of are asked for whole. The first id heard from a
    /// sender only sets where we start (no asking for the table's past).
    pub fn note_latest(&mut self, from: &str, id: u64, inclusive: bool) {
        self.note_latest_at(Instant::now(), from, id, inclusive)
    }

    pub fn note_latest_at(&mut self, now: Instant, from: &str, id: u64, inclusive: bool) {
        if id == 0 {
            return;
        }
        let Some(&h) = self.latest.get(from) else {
            self.latest.insert(from.to_string(), if inclusive { id } else { id - 1 });
            return;
        };
        if id + 64 < h {
            // The sender restarted (ids count from 1 again).
            self.latest.insert(from.to_string(), id);
            return;
        }
        let end = if inclusive { id } else { id - 1 };
        if end > h && end - h <= TABLE_GAP_MAX {
            for g in h + 1..=end {
                let k = (from.to_string(), g);
                if !self.done.contains_key(&k) && !self.parts.contains_key(&k) {
                    self.wanted.entry(k).or_insert(now);
                }
            }
        }
        if id > h {
            self.latest.insert(from.to_string(), id);
        }
    }

    /// Whether an earlier message from `from` than `id` is still coming
    /// in: some of its parts here, or known missed, and not given up.
    pub fn pending_before(&self, from: &str, id: u64) -> bool {
        let now = Instant::now();
        self.parts.iter().any(|((f, i), p)| {
            f == from && *i < id && now.saturating_duration_since(p.last) < TABLE_PARTIAL_TTL
        }) || self.wanted.iter().any(|((f, i), t)| {
            f == from && *i < id && now.saturating_duration_since(*t) < TABLE_PARTIAL_TTL
        })
    }

    /// Messages with gaps: (from, id, bitmap of missing parts).
    pub fn missing(&self) -> Vec<(String, u64, u64)> {
        self.missing_at(Instant::now())
    }

    /// `missing` with an explicit clock (for tests): messages that have
    /// waited longer than `TABLE_PARTIAL_TTL` are not asked for.
    pub fn missing_at(&self, now: Instant) -> Vec<(String, u64, u64)> {
        self.missing_quiet_at(now, Duration::ZERO)
    }

    /// `missing`, leaving out messages whose last part came less than
    /// `quiet` ago: their sender is likely still sending the rest, and an
    /// AGAIN now would go out on top of it.
    pub fn missing_quiet(&self, quiet: Duration) -> Vec<(String, u64, u64)> {
        self.missing_quiet_at(Instant::now(), quiet)
    }

    pub fn missing_quiet_at(&self, now: Instant, quiet: Duration) -> Vec<(String, u64, u64)> {
        self.parts
            .iter()
            .filter(|(_, p)| now.saturating_duration_since(p.last) < TABLE_PARTIAL_TTL)
            .filter(|(_, p)| now.saturating_duration_since(p.last) >= quiet)
            .map(|((from, id), p)| {
                let mut m = 0u64;
                for n in 1..=p.total.min(64) {
                    if !p.got.contains_key(&n) {
                        m |= 1 << (n - 1);
                    }
                }
                (from.clone(), *id, m)
            })
            .filter(|x| x.2 != 0)
            .chain(
                self.wanted
                    .iter()
                    .filter(|(k, t)| {
                        now.saturating_duration_since(**t) < TABLE_PARTIAL_TTL
                            && !self.done.contains_key(*k)
                            && !self.parts.contains_key(*k)
                    })
                    .map(|((from, id), _)| (from.clone(), *id, u64::MAX)),
            )
            .collect()
    }
}

// -------------------------------------------------------------------- KISS

const FEND: u8 = 0xC0;
const FESC: u8 = 0xDB;
const TFEND: u8 = 0xDC;
const TFESC: u8 = 0xDD;

pub fn kiss_encode(payload: &[u8], cmd: u8) -> Vec<u8> {
    let mut out = vec![FEND, cmd];
    for &b in payload {
        match b {
            FEND => out.extend([FESC, TFEND]),
            FESC => out.extend([FESC, TFESC]),
            _ => out.push(b),
        }
    }
    out.push(FEND);
    out
}

#[derive(Default)]
pub struct KissDecoder {
    buf: Vec<u8>,
    inside: bool,
    esc: bool,
}

impl KissDecoder {
    pub fn feed(&mut self, data: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = vec![];
        for &b in data {
            if b == FEND {
                if self.inside && !self.buf.is_empty() {
                    out.push((self.buf[0], self.buf[1..].to_vec()));
                }
                self.buf.clear();
                self.inside = true;
                self.esc = false;
                continue;
            }
            if !self.inside {
                continue;
            }
            if self.esc {
                self.buf.push(match b {
                    TFEND => FEND,
                    TFESC => FESC,
                    o => o,
                });
                self.esc = false;
            } else if b == FESC {
                self.esc = true;
            } else {
                self.buf.push(b);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A later message from a station is known to wait for an earlier one
    /// still being filled in; another station's doesn't.
    #[test]
    fn an_earlier_message_still_coming_in_is_known() {
        let mut a = TableAssembler::default();
        assert!(a.add("W1ABC", 5, 1, 2, b"first ").is_none());
        assert!(a.add("W1ABC", 6, 1, 1, b"second").is_some());
        assert!(a.pending_before("W1ABC", 6));
        assert!(!a.pending_before("W1ABC", 5));
        assert!(!a.pending_before("K7XYZ", 9));
        assert!(a.add("W1ABC", 5, 2, 2, b"half").is_some());
        assert!(!a.pending_before("W1ABC", 6));
    }

    /// Parts of a message still coming in aren't asked for again yet: the
    /// request would go out on top of them.
    #[test]
    fn a_message_still_coming_in_isnt_asked_for_yet() {
        let mut a = TableAssembler::default();
        let t0 = Instant::now();
        a.add_at(t0, "W1ABC", 9, 1, 4, b"one ");
        a.add_at(t0 + Duration::from_secs(4), "W1ABC", 9, 2, 4, b"two ");
        let quiet = Duration::from_secs(9);
        assert!(a.missing_quiet_at(t0 + Duration::from_secs(6), quiet).is_empty(), "still sending");
        let m = a.missing_quiet_at(t0 + Duration::from_secs(14), quiet);
        assert_eq!(m, vec![("W1ABC".to_string(), 9, 0b1100)]);
        assert_eq!(a.missing_at(t0 + Duration::from_secs(6)).len(), 1, "missing() still lists it");
    }

    /// DATAC15's broadcast frames carry 27 bytes.
    const SMALL: usize = 27;

    /// A member's report at a slow speed: without the table name it fits,
    /// and the host still reads how it hears the host.
    #[test]
    fn a_report_that_doesnt_fit_leaves_the_name_out() {
        let f = TableFrame {
            table: table_id("Sunday Morning Net"),
            from: "KD2ABC/P".into(),
            body: TableBody::Join {
                table_name: "Sunday Morning Net".into(),
                snr_of_host: -3,
                bw_code: Some(1),
            },
        };
        let big = f.encode();
        assert!(big.len() > SMALL);
        assert_eq!(shrink_table_frame(&big, 200).unwrap(), big, "one that fits is left alone");
        let small = shrink_table_frame(&big, SMALL).unwrap();
        assert!(small.len() <= SMALL);
        let back = TableFrame::decode(&small).unwrap();
        assert_eq!((back.table, back.from.as_str()), (f.table, "KD2ABC/P"));
        assert!(matches!(back.body, TableBody::Join { ref table_name, snr_of_host: -3, bw_code: Some(1) } if table_name.is_empty()));
    }

    /// "Send the whole message again" (all ones) doesn't fit DATAC15: it asks
    /// for the lowest parts instead; the rest are asked for later.
    #[test]
    fn a_resend_request_that_doesnt_fit_asks_for_fewer_parts() {
        let f = TableFrame {
            table: table_id("Evening Table"),
            from: "W1ABC".into(),
            body: TableBody::Again {
                of: "K7XYZ".into(),
                id: 40000,
                missing: u64::MAX,
            },
        }
        .encode();
        assert!(f.len() > SMALL);
        let small = shrink_table_frame(&f, SMALL).unwrap();
        assert!(small.len() <= SMALL);
        let Some(TableFrame {
            body: TableBody::Again { of, id, missing },
            ..
        }) = TableFrame::decode(&small)
        else {
            panic!("not an AGAIN")
        };
        assert_eq!((of.as_str(), id), ("K7XYZ", 40000));
        assert!(missing & 1 == 1 && missing.count_ones() >= 4, "asks for parts 1..: {missing:#b}");
        assert_eq!(missing & (missing + 1), 0, "the lowest parts, in a row");
        // Nothing else is shrunk.
        let text = table_text_frames(1, "W1ABC", 1, &"x".repeat(100), 120).unwrap();
        assert!(shrink_table_frame(&text[0], SMALL).is_none());
    }

    /// Which speeds a table's callsigns fit: DATAC15 takes calls of up to 7
    /// characters (W1ABC/P); 8 leave no room for text (KD2ABC/P); DATAC4
    /// (51 bytes) takes any.
    #[test]
    fn long_callsigns_dont_fit_the_smallest_frames() {
        let calls = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(table_frames_fit(SMALL, &calls(&["W1ABC", "K7XYZ", "KD2ABC"])));
        assert!(table_frames_fit(SMALL, &calls(&["W1ABC/P", "KD2ABC/"])));
        assert!(!table_frames_fit(SMALL, &calls(&["KD2ABC/P", "W1ABC"])));
        assert!(!table_frames_fit(SMALL, &calls(&["VE3/KD2ABC/P", "W1ABC"])));
        assert!(table_frames_fit(51, &calls(&["VE3/KD2ABC/P", "VE3/KD2ABD/M"])));
        assert!(table_frames_fit(SMALL, &[]));
        // Everyday calls: the parts of a message can all be asked for again
        // only up to a point; any number in frames of 51 bytes and more.
        let us = calls(&["KD2ABC", "W1ABC"]);
        let n = again_parts_limit(SMALL, &us);
        assert!((8..64).contains(&n), "{n}");
        assert_eq!(again_parts_limit(51, &us), 64);
        assert_eq!(again_parts_limit(SMALL, &calls(&["VE3/KD2ABC/P", "VE3/KD2ABD/M"])), 0);
    }

    /// At the slowest speed a long message goes out as several, each short
    /// enough for all its parts to be asked for again; the words survive.
    #[test]
    fn a_long_message_at_the_slowest_speed_goes_as_several() {
        let text = "Seventy two degrees and sunny here today, the wind has dropped, and the new dipole is up at forty feet. ".repeat(4);
        let text = text.trim();
        let limit = again_parts_limit(SMALL, &["KD2ABC".to_string(), "W1ABC".to_string()]);
        let pieces = split_for_parts("KD2ABC", text, SMALL, limit).unwrap();
        assert!(pieces.len() > 1);
        for p in &pieces {
            let n = table_text_frames(WORST_TABLE, "KD2ABC", WORST_ID, p, SMALL).unwrap().len();
            assert!(n <= limit, "{n} parts > {limit}");
            assert!(!p.starts_with(' ') && !p.ends_with(' '));
        }
        assert_eq!(pieces.join(" "), text, "cut between words, nothing lost");
        // Short, or a roomy speed: one message, as it was.
        assert_eq!(split_for_parts("KD2ABC", "73 all", SMALL, limit).unwrap(), vec!["73 all"]);
        assert_eq!(split_for_parts("KD2ABC", text, 120, 64).unwrap(), vec![text.to_string()]);
        // A word longer than a whole message is cut inside it.
        let long_word = "x".repeat(2000);
        let pieces = split_for_parts("KD2ABC", &long_word, SMALL, limit).unwrap();
        assert_eq!(pieces.concat(), long_word);
    }

    /// A message split again into smaller parts (the table slowed down
    /// before it was all heard): a station holding parts of the old split
    /// starts over and gets the whole message.
    #[test]
    fn a_message_split_again_is_put_together_from_the_new_parts() {
        let text = "Seventy degrees and sunny here, the antenna is finally back up after the storm.";
        let old = table_text_frames(7, "W1ABC", 3, text, 60).unwrap();
        let mut new_room = 27;
        let new = loop {
            let v = table_text_frames(7, "W1ABC", 3, text, new_room).unwrap();
            if v.len() != old.len() {
                break v;
            }
            new_room -= 1;
        };
        let parts = |fs: &[Vec<u8>]| -> Vec<(u8, u8, Vec<u8>)> {
            fs.iter()
                .map(|f| match TableFrame::decode(f).unwrap().body {
                    TableBody::Text { part, total, data, .. } => (part, total, data),
                    _ => unreachable!(),
                })
                .collect()
        };
        let mut a = TableAssembler::default();
        let (p, t, d) = &parts(&old)[0];
        assert!(a.add("W1ABC", 3, *p, *t, d).is_none());
        let mut got = None;
        for (p, t, d) in parts(&new) {
            got = got.or(a.add("W1ABC", 3, p, t, &d));
        }
        assert_eq!(got.as_deref(), Some(text));
        assert!(a.missing().is_empty());
    }

    /// Names from the air are saved as plain, visible files in the
    /// download folder and nowhere else.
    #[test]
    fn received_file_names_are_made_safe() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\x\\a.txt"), "a.txt");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name("a?b*.txt"), "a_b_.txt");
        assert_eq!(safe_file_name("..."), "received.bin");
        assert_eq!(safe_file_name(""), "received.bin");
        // Hidden names, and the folder resumable downloads live in.
        assert_eq!(safe_file_name(".partial"), "_.partial");
        assert_eq!(safe_file_name(".bashrc"), "_.bashrc");
        assert!(safe_file_name(&"x".repeat(300)).len() <= MAX_NAME_BYTES);
    }

    /// An empty file still arrives: nothing is left to send once it's
    /// accepted, so it's complete at once.
    #[test]
    fn an_empty_file_is_complete_at_once() {
        let dir = std::env::temp_dir().join(format!("ws-empty-{}", std::process::id()));
        let inc = Incoming::new(&dir, 1, "empty.txt".into(), 0, sha256(b""), false);
        assert!(inc.complete());
        let p = inc.finish(&dir).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn varints() {
        for v in [0u64, 1, 127, 128, 300, 1 << 20, u64::MAX >> 1] {
            let mut w = Writer::default();
            w.uv(v);
            assert_eq!(Reader::new(&w.0).uv().unwrap(), v);
        }
    }

    #[test]
    fn chat_is_compact() {
        // "73" costs 2 bytes of text + 4 of WSP.
        assert_eq!(
            Link::Chat {
                id: 5,
                text: "73".into()
            }
            .encode()
            .len(),
            6
        );
    }

    #[test]
    fn link_stream_roundtrip_in_pieces() {
        let pkts = vec![
            Link::Hello {
                version: 1,
                caps: 3,
                name: "Nate".into(),
                grid: "EM12".into(),
                app: "Whistlestop 0.1".into(),
            },
            Link::Chat {
                id: 1,
                text: "hello ✓".into(),
            },
            Link::Offer {
                fid: 9,
                name: "a.zip".into(),
                size: 5000,
                sha: [7; 32],
                zipped: true,
                files: 3,
            },
            Link::Answer {
                fid: 9,
                accept: true,
                resume_from: 2048,
            },
            Link::Chunk {
                fid: 9,
                offset: 2048,
                data: vec![1; 1024],
            },
            Link::Done { fid: 9, ok: true },
            Link::Report {
                snr_db: -7,
                mode_index: 1,
            },
            Link::Bye { text: "73".into() },
            Link::Mail {
                id: 0xDEAD_BEEF,
                from: "N0CALL".into(),
                to: "K7BOT".into(),
                created: 1_800_000_000,
                hops: 1,
                text: "See you on 40 m ✓".into(),
            },
            Link::MailAck { id: 0xDEAD_BEEF },
            Link::Qsy {
                id: 7,
                dial_hz: 14_105_250,
                why: "file transfer".into(),
            },
            Link::QsyAnswer {
                id: 7,
                accept: false,
                reason: "outside my allowed bands".into(),
            },
            Link::ListReq {
                id: 8,
                path: "maps/".into(),
            },
            Link::ListAns {
                id: 8,
                path: "maps/".into(),
                entries: vec![
                    LibEntry {
                        name: "grid.png".into(),
                        size: 48_211,
                        dir: false,
                    },
                    LibEntry {
                        name: "old".into(),
                        size: 0,
                        dir: true,
                    },
                ],
                note: String::new(),
            },
            Link::Fetch {
                id: 9,
                path: "maps/grid.png".into(),
            },
            Link::FetchAnswer {
                id: 9,
                ok: true,
                reason: String::new(),
            },
            Link::KeyCommit {
                suite: 1,
                commit: [3; 32],
            },
            Link::KeyX {
                suite: 1,
                pubkey: [4; 32],
            },
            Link::Sealed { data: vec![9; 40] },
        ];
        let wire: Vec<u8> = pkts.iter().flat_map(|p| p.encode()).collect();
        let mut d = LinkDecoder::default();
        let mut got = vec![];
        for c in wire.chunks(7) {
            got.extend(d.feed(c).unwrap());
        }
        assert_eq!(got, pkts);
    }

    #[test]
    fn file_transfer_with_resume() {
        let dir = std::env::temp_dir().join(format!("wsp-{}", std::process::id()));
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8).collect();
        let mut out = Outgoing::new(1, "../../etc/x.bin".into(), data.clone(), false, 1);
        let Link::Offer { size, sha, .. } = out.offer() else {
            panic!()
        };

        // First attempt: link drops after two chunks.
        let mut inc = Incoming::new(&dir, 1, out.name.clone(), size, sha, false);
        out.next = Some(inc.have());
        for _ in 0..2 {
            if let Some(Link::Chunk { offset, data, .. }) = out.next_chunk() {
                inc.chunk(offset, &data);
            }
        }
        inc.save_partial();

        // Offered again: the receiver resumes from what it has.
        let mut inc = Incoming::new(&dir, 2, out.name.clone(), size, sha, false);
        assert_eq!(inc.have(), 2048);
        out.next = Some(inc.have());
        while let Some(Link::Chunk { offset, data, .. }) = out.next_chunk() {
            inc.chunk(offset, &data);
        }
        assert!(inc.complete());
        let path = inc.finish(&dir).unwrap();
        assert_eq!(path, dir.join("x.bin")); // no path escape
        assert_eq!(std::fs::read(&path).unwrap(), data);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn table_frames_fit_and_reassemble() {
        let t = table_id("Evening Net");
        assert_eq!(t, table_id("evening net "));
        let text = "Good evening everyone, this is a longer message for the table ✓ ".repeat(3);
        let frames = table_text_frames(t, "N0CALL", 42, &text, 123).unwrap();
        assert!(frames.len() > 1 && frames.iter().all(|f| f.len() <= 123));
        let mut a = TableAssembler::default();
        let mut done = None;
        for f in frames.iter().skip(1) {
            let TableFrame {
                from,
                body:
                    TableBody::Text {
                        id,
                        part,
                        total,
                        data,
                    },
                ..
            } = TableFrame::decode(f).unwrap()
            else {
                panic!()
            };
            done = a.add(&from, id, part, total, &data);
        }
        assert!(done.is_none());
        assert_eq!(a.missing(), vec![("N0CALL".to_string(), 42, 1)]); // part 1 missing
        let TableFrame {
            from,
            body:
                TableBody::Text {
                    id,
                    part,
                    total,
                    data,
                },
            ..
        } = TableFrame::decode(&frames[0]).unwrap()
        else {
            panic!()
        };
        assert_eq!(a.add(&from, id, part, total, &data).unwrap(), text);
    }

    #[test]
    fn table_control_frames() {
        for body in [
            TableBody::Call {
                kind: CALL_TABLE,
                table_name: "EVENING".into(),
                bw_code: 2,
                grid: "EM12".into(),
            },
            TableBody::Join {
                table_name: "EVENING".into(),
                snr_of_host: -4,
                bw_code: Some(2),
            },
            TableBody::Join {
                table_name: "EVENING".into(),
                snr_of_host: SNR_UNKNOWN,
                bw_code: None,
            },
            TableBody::Speed { mode_index: 0, last_id: 0 },
            TableBody::Speed { mode_index: 1, last_id: 300 },
            TableBody::Again {
                of: "K1ABC".into(),
                id: 7,
                missing: 0b101,
            },
            TableBody::Floor { to: "K1ABC".into() },
            TableBody::Leave,
            TableBody::Beacon {
                grid: "EM12".into(),
                away: true,
                mail_for: vec!["K7BOT".into(), "W1AW".into()],
            },
            TableBody::Notice {
                to: "K7BOT".into(),
                count: 2,
            },
        ] {
            let f = TableFrame {
                table: 77,
                from: "N0CALL".into(),
                body,
            };
            assert_eq!(TableFrame::decode(&f.encode()), Some(f));
        }
        assert_eq!(TableFrame::decode(b"\x01not ours"), None);
    }

    #[test]
    fn kiss_roundtrip() {
        let p = vec![0xC0, 1, 0xDB, 2];
        let mut d = KissDecoder::default();
        let w = kiss_encode(&p, 0);
        let (a, b) = w.split_at(3);
        assert!(d.feed(a).is_empty());
        assert_eq!(d.feed(b), vec![(0, p)]);
    }

    #[test]
    fn huge_length_from_the_air_is_refused_not_a_panic() {
        // Table frame, type TEXT, table 0, then a "from" whose length varint
        // is near u64::MAX: pos + len used to wrap and pass the check.
        let mut b = vec![TABLE_MAGIC, 0x11, 0x00];
        b.extend_from_slice(&[0xFF; 9]);
        b.push(0x01);
        b.extend_from_slice(b"ABC");
        assert!(TableFrame::decode(&b).is_none());
        let mut r = Reader::new(&b[3..]);
        assert!(r.bytes().is_err());
    }

    #[test]
    fn callsigns_from_the_air_are_checked() {
        assert!(valid_call("N0CALL"));
        assert!(valid_call("K7BOT/P"));
        assert!(valid_call("BRAVO"));
        assert!(!valid_call(""));
        assert!(!valid_call("X\rLISTEN OFF"));
        assert!(!valid_call("A B"));
        assert!(!valid_call("ABCDEFGHIJKLM"));
        // A NOTICE "from" with a CR in it is dropped whole.
        let mut w = Writer::default();
        w.u8(TABLE_MAGIC).u8(0x18).uv(0).str("X\rLISTEN OFF").str("N0CALL").u8(1);
        assert!(TableFrame::decode(&w.0).is_none());
    }

    #[test]
    fn long_non_ascii_callsign_is_cut_on_a_character_boundary() {
        let mut w = Writer::default();
        w.call("AÅÅÅÅÅÅ"); // 13 bytes: byte 12 is inside a character
        let mut r = Reader::new(&w.0);
        assert_eq!(r.str().unwrap(), "AÅÅÅÅÅ");
    }

    #[test]
    fn text_with_too_many_parts_is_refused() {
        for (part, total) in [(1u8, 65u8), (0, 3), (4, 3), (1, 0)] {
            let mut w = Writer::default();
            w.u8(TABLE_MAGIC).u8(0x11).uv(1).str("N0CALL").uv(7).u8(part).u8(total).bytes(b"hi");
            assert!(TableFrame::decode(&w.0).is_none(), "{part}/{total}");
        }
    }
}
