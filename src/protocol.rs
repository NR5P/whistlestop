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
    if reserved {
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
        snr_of_host: i8,
    },
    Speed {
        mode_index: u8,
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
            } => {
                w.str(table_name).i8(*snr_of_host);
            }
            TableBody::Speed { mode_index } => {
                w.u8(*mode_index);
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
                },
                0x13 => TableBody::Speed {
                    mode_index: r.u8()?,
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
}

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
        if self.done.contains_key(&key) {
            return None;
        }
        let e = self.parts.entry(key.clone()).or_insert_with(|| Partial {
            total,
            got: HashMap::new(),
            last: now,
        });
        if total != e.total || part > e.total {
            // Disagrees with the parts already here (corrupt or forged):
            // keep what we have.
            return None;
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

    /// Messages with gaps: (from, id, bitmap of missing parts).
    pub fn missing(&self) -> Vec<(String, u64, u64)> {
        self.missing_at(Instant::now())
    }

    /// `missing` with an explicit clock (for tests): messages that have
    /// waited longer than `TABLE_PARTIAL_TTL` are not asked for.
    pub fn missing_at(&self, now: Instant) -> Vec<(String, u64, u64)> {
        self.parts
            .iter()
            .filter(|(_, p)| now.saturating_duration_since(p.last) < TABLE_PARTIAL_TTL)
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
            },
            TableBody::Speed { mode_index: 0 },
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
