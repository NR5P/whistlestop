//! WSP, the Whistlestop Protocol. Specification: docs/PROTOCOL.md.

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const VERSION: u8 = 1;
pub const CAP_RESUME: u8 = 1;
pub const CAP_ZIP: u8 = 2;
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
        self.str(&c[..c.len().min(12)])
    }
    pub fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
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
        if self.pos + n > self.b.len() {
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
    pub fn hash(&mut self) -> Result<[u8; 32]> {
        Ok(self.take(32)?.try_into()?)
    }
}

fn read_varint_prefix(b: &[u8]) -> Option<(usize, usize)> {
    let mut v = 0usize;
    for (i, &byte) in b.iter().enumerate().take(4) {
        v |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

// -------------------------------------------------------------- link plane

#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Hello { version: u8, caps: u8, name: String, grid: String, app: String },
    Chat { id: u64, text: String },
    Seen { id: u64 },
    Offer { fid: u64, name: String, size: u64, sha: [u8; 32], zipped: bool, files: u64 },
    Answer { fid: u64, accept: bool, resume_from: u64 },
    Chunk { fid: u64, offset: u64, data: Vec<u8> },
    Done { fid: u64, ok: bool },
    Cancel { fid: u64 },
    Report { snr_db: i8, mode_index: u8 },
    Bye { text: String },
    Unknown(u8),
}

impl Link {
    /// Body without the length prefix.
    pub fn body(&self) -> Vec<u8> {
        let mut w = Writer::default();
        match self {
            Link::Hello { version, caps, name, grid, app } => {
                w.u8(0x01).u8(*version).u8(*caps).str(name).str(grid).str(app);
            }
            Link::Chat { id, text } => {
                w.u8(0x02).uv(*id).str(text);
            }
            Link::Seen { id } => {
                w.u8(0x03).uv(*id);
            }
            Link::Offer { fid, name, size, sha, zipped, files } => {
                w.u8(0x04).uv(*fid).str(name).uv(*size).raw(sha).u8(*zipped as u8).uv(*files);
            }
            Link::Answer { fid, accept, resume_from } => {
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
            0x01 => Link::Hello { version: r.u8()?, caps: r.u8()?, name: r.str()?, grid: r.str()?, app: r.str()? },
            0x02 => Link::Chat { id: r.uv()?, text: r.str()? },
            0x03 => Link::Seen { id: r.uv()? },
            0x04 => Link::Offer {
                fid: r.uv()?,
                name: r.str()?,
                size: r.uv()?,
                sha: r.hash()?,
                zipped: r.u8()? & 1 == 1,
                files: r.uv()?,
            },
            0x05 => Link::Answer { fid: r.uv()?, accept: r.u8()? == 1, resume_from: r.uv()? },
            0x06 => Link::Chunk { fid: r.uv()?, offset: r.uv()?, data: r.bytes()?.to_vec() },
            0x07 => Link::Done { fid: r.uv()?, ok: r.u8()? == 1 },
            0x08 => Link::Cancel { fid: r.uv()? },
            0x09 => Link::Report { snr_db: r.i8()?, mode_index: r.u8()? },
            0x0A => Link::Bye { text: r.str()? },
            t => Link::Unknown(t),
        })
    }
}

/// Splits Mercury's ARQ byte stream into WSP link packets.
#[derive(Default)]
pub struct LinkDecoder {
    buf: Vec<u8>,
}

impl LinkDecoder {
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<Link>> {
        self.buf.extend_from_slice(data);
        let mut out = vec![];
        while let Some((n, hdr)) = read_varint_prefix(&self.buf) {
            if n > MAX_PACKET {
                bail!("corrupt link stream (packet length {n})");
            }
            if self.buf.len() < hdr + n {
                break;
            }
            out.push(Link::decode(&self.buf[hdr..hdr + n])?);
            self.buf.drain(..hdr + n);
        }
        Ok(out)
    }
}

/// Air bytes a chat line costs, for the "how long will this take" display.
pub fn chat_wire_size(text: &str) -> usize {
    Link::Chat { id: 1000, text: text.into() }.encode().len()
}

/// Air bytes a file of `size` costs: offer + chunks + done.
pub fn file_wire_size(name: &str, size: u64) -> usize {
    let chunks = (size as usize).div_ceil(CHUNK).max(1);
    let offer = Link::Offer { fid: 1, name: name.into(), size, sha: [0; 32], zipped: true, files: 1 }.encode().len();
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
        Self { fid, name, data, sha, zipped, files, next: None }
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
        Some(Link::Chunk { fid: self.fid, offset: off, data: self.data[off as usize..end].to_vec() })
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
        let data = if data.len() as u64 <= size { data } else { vec![] };
        Self { fid, name, size, sha, zipped, data, partial_path }
    }
    pub fn have(&self) -> u64 {
        self.data.len() as u64
    }
    /// Apply a chunk. Out-of-order chunks are ignored (ARQ keeps order).
    pub fn chunk(&mut self, offset: u64, data: &[u8]) {
        if offset == self.have() {
            self.data.extend_from_slice(data);
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
        // Never trust a path from the air: keep only the file name.
        let name = Path::new(&self.name).file_name().map(|s| s.to_string_lossy().into_owned());
        let name = name.filter(|n| !n.is_empty() && n != "..").unwrap_or_else(|| "received.bin".into());
        let path = unique_path(&dir.join(name));
        std::fs::write(&path, &self.data)?;
        let _ = std::fs::remove_file(&self.partial_path);
        Ok(path)
    }
}

fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..).map(|i| p.with_file_name(format!("{stem}-{i}{ext}"))).find(|c| !c.exists()).unwrap()
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
    Call { kind: u8, table_name: String, bw_code: u8, grid: String },
    Text { id: u64, part: u8, total: u8, data: Vec<u8> },
    Join { table_name: String, snr_of_host: i8 },
    Speed { mode_index: u8 },
    Again { of: String, id: u64, missing: u64 },
    Floor { to: String },
    Leave,
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
        };
        w.u8(TABLE_MAGIC).u8(t).uv(self.table as u64).call(&self.from);
        match &self.body {
            TableBody::Call { kind, table_name, bw_code, grid } => {
                w.u8(*kind).str(table_name).u8(*bw_code).str(grid);
            }
            TableBody::Text { id, part, total, data } => {
                w.uv(*id).u8(*part).u8(*total).bytes(data);
            }
            TableBody::Join { table_name, snr_of_host } => {
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
        let from = r.str().ok()?;
        let body = (|| -> Result<TableBody> {
            Ok(match t {
                0x10 => TableBody::Call { kind: r.u8()?, table_name: r.str()?, bw_code: r.u8()?, grid: r.str()? },
                0x11 => TableBody::Text { id: r.uv()?, part: r.u8()?, total: r.u8()?, data: r.bytes()?.to_vec() },
                0x12 => TableBody::Join { table_name: r.str()?, snr_of_host: r.i8()? },
                0x13 => TableBody::Speed { mode_index: r.u8()? },
                0x14 => TableBody::Again { of: r.str()?, id: r.uv()?, missing: r.uv()? },
                0x15 => TableBody::Floor { to: r.str()? },
                0x16 => TableBody::Leave,
                _ => bail!("unknown"),
            })
        })()
        .ok()?;
        Some(TableFrame { table, from, body })
    }
}

/// Split a Roundtable message into TEXT frames that each fit `frame_bytes`.
pub fn table_text_frames(table: u16, from: &str, id: u64, text: &str, frame_bytes: usize) -> Result<Vec<Vec<u8>>> {
    let overhead = TableFrame {
        table,
        from: from.into(),
        body: TableBody::Text { id, part: 255, total: 255, data: vec![0; 127] },
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
            TableFrame { table, from: from.into(), body: TableBody::Text { id, part: i as u8 + 1, total, data: d.to_vec() } }
                .encode()
        })
        .collect())
}

/// Reassembles TEXT parts and reports what is missing (for AGAIN).
#[derive(Default)]
pub struct TableAssembler {
    parts: HashMap<(String, u64), (u8, HashMap<u8, Vec<u8>>)>,
}

impl TableAssembler {
    pub fn add(&mut self, from: &str, id: u64, part: u8, total: u8, data: &[u8]) -> Option<String> {
        let key = (from.to_string(), id);
        let e = self.parts.entry(key.clone()).or_insert_with(|| (total, HashMap::new()));
        e.1.insert(part, data.to_vec());
        if e.1.len() == total as usize {
            let (_, got) = self.parts.remove(&key).unwrap();
            let mut all = vec![];
            for i in 1..=total {
                all.extend_from_slice(got.get(&i)?);
            }
            return Some(String::from_utf8_lossy(&all).into_owned());
        }
        None
    }

    /// Messages with gaps: (from, id, bitmap of missing parts).
    pub fn missing(&self) -> Vec<(String, u64, u64)> {
        self.parts
            .iter()
            .map(|((from, id), (total, got))| {
                let mut m = 0u64;
                for p in 1..=*total {
                    if !got.contains_key(&p) {
                        m |= 1 << (p - 1);
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
        assert_eq!(Link::Chat { id: 5, text: "73".into() }.encode().len(), 6);
    }

    #[test]
    fn link_stream_roundtrip_in_pieces() {
        let pkts = vec![
            Link::Hello { version: 1, caps: 3, name: "Nate".into(), grid: "EM12".into(), app: "Whistlestop 0.1".into() },
            Link::Chat { id: 1, text: "hello ✓".into() },
            Link::Offer { fid: 9, name: "a.zip".into(), size: 5000, sha: [7; 32], zipped: true, files: 3 },
            Link::Answer { fid: 9, accept: true, resume_from: 2048 },
            Link::Chunk { fid: 9, offset: 2048, data: vec![1; 1024] },
            Link::Done { fid: 9, ok: true },
            Link::Report { snr_db: -7, mode_index: 1 },
            Link::Bye { text: "73".into() },
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
        let Link::Offer { size, sha, .. } = out.offer() else { panic!() };

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
            let TableFrame { from, body: TableBody::Text { id, part, total, data }, .. } = TableFrame::decode(f).unwrap() else {
                panic!()
            };
            done = a.add(&from, id, part, total, &data);
        }
        assert!(done.is_none());
        assert_eq!(a.missing(), vec![("N0CALL".to_string(), 42, 1)]); // part 1 missing
        let TableFrame { from, body: TableBody::Text { id, part, total, data }, .. } = TableFrame::decode(&frames[0]).unwrap() else {
            panic!()
        };
        assert_eq!(a.add(&from, id, part, total, &data).unwrap(), text);
    }

    #[test]
    fn table_control_frames() {
        for body in [
            TableBody::Call { kind: CALL_TABLE, table_name: "EVENING".into(), bw_code: 2, grid: "EM12".into() },
            TableBody::Join { table_name: "EVENING".into(), snr_of_host: -4 },
            TableBody::Speed { mode_index: 0 },
            TableBody::Again { of: "K1ABC".into(), id: 7, missing: 0b101 },
            TableBody::Floor { to: "K1ABC".into() },
            TableBody::Leave,
        ] {
            let f = TableFrame { table: 77, from: "N0CALL".into(), body };
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
}
