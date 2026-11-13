//! A stand-in for Mercury that speaks its TNC protocol, with a built-in
//! demo station (K7BOT by default) on the other end of the "air".
//!
//! Used by Demo mode, so the whole app can be tried without a radio, and by
//! the tests. It models what the app sees from Mercury: OK/WRONG replies,
//! CQFRAME/PENDING/CONNECTED/DISCONNECTED, BUFFER draining at a set bit
//! rate, BITRATE and SN reports, PTT lines, and the broadcast plane.

use crate::e2e::{Channel, Notice, Policy};
use crate::protocol::*;
use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
pub struct FakeOptions {
    /// Shrink every delay (tests).
    pub fast: bool,
    pub bot_call: String,
    /// Simulated ARQ goodput in bits/s.
    pub bps: f64,
    /// Where our radio is tuned (shared with the simulated rig), and where
    /// the bot calls CQ. When they line up, Mercury "decodes" its CQFRAME.
    pub rig_freq: Option<Arc<Mutex<f64>>>,
    pub bot_freq: f64,
    pub answer_cq: bool,
    /// The bot's encryption setting (Prefer: encrypts if you do).
    pub bot_e2e: Policy,
    /// Tests: the bot's CQs are for this Roundtable (it hosts it).
    pub bot_cq_table: Option<String>,
    /// Tests: the bot sends a file's pieces right after offering it,
    /// without waiting for the answer (a misbehaving station).
    pub bot_pushy: bool,
    /// Tests: as host of `bot_cq_table`, the bot answers a JOIN with its
    /// SPEED (so the member knows who hosts) and closes the table (LEAVE)
    /// this many seconds later.
    pub bot_table_leave_after: Option<f64>,
}

impl Default for FakeOptions {
    fn default() -> Self {
        Self {
            fast: false,
            bot_call: "K7BOT".into(),
            bps: 1400.0,
            rig_freq: None,
            bot_freq: 14_103_150.0,
            answer_cq: true,
            bot_e2e: Policy::Prefer,
            bot_pushy: false,
            bot_cq_table: None,
            bot_table_leave_after: None,
        }
    }
}

pub struct FakeMercury {
    pub port: u16,
    pub bcast_port: u16,
    stop: watch::Sender<bool>,
}

enum In {
    Ctl(String),
    Data(Vec<u8>),
    Bcast(Vec<u8>),
}

impl FakeMercury {
    pub async fn start(opts: FakeOptions) -> Result<Self> {
        // Control and data ports must be consecutive, as in Mercury.
        let (ctl, data, port) = loop {
            let ctl = TcpListener::bind("127.0.0.1:0").await?;
            let p = ctl.local_addr()?.port();
            if let Ok(d) = TcpListener::bind(("127.0.0.1", p + 1)).await {
                break (ctl, d, p);
            }
        };
        let bc = TcpListener::bind("127.0.0.1:0").await?;
        let bcast_port = bc.local_addr()?.port();
        let (stop, stop_rx) = watch::channel(false);
        tokio::spawn(serve(ctl, data, bc, opts, stop_rx));
        Ok(Self {
            port,
            bcast_port,
            stop,
        })
    }

    /// Shut this Mercury down: its three listening sockets, any open
    /// control/data/broadcast connections and all of its tasks end (at
    /// their next await, i.e. at once). Safe to call more than once.
    ///
    /// Dropping a `FakeMercury` does NOT stop it (tests keep only its
    /// ports); whoever starts one for a while (the engine's Demo mode) keeps
    /// it and calls `stop()` when done with it.
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

/// Resolves once `stop()` is called (never, if the FakeMercury is dropped
/// without it).
async fn stopped(mut rx: watch::Receiver<bool>) {
    if rx.wait_for(|s| *s).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// A task that is aborted when this handle is dropped.
struct Task(tokio::task::JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn serve(
    ctl: TcpListener,
    data: TcpListener,
    bc: TcpListener,
    opts: FakeOptions,
    stop: watch::Receiver<bool>,
) {
    let bc_w: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>> = Default::default();
    let (in_tx, in_rx) = mpsc::unbounded_channel::<In>();
    let _bc_task = {
        let bc_w = bc_w.clone();
        let in_tx = in_tx.clone();
        Task(tokio::spawn(async move {
            // Readers end with this task (aborted when serve() ends).
            let mut readers: Vec<Task> = vec![];
            while let Ok((s, _)) = bc.accept().await {
                readers.retain(|t| !t.0.is_finished());
                let (mut r, w) = s.into_split();
                *bc_w.lock().await = Some(w);
                let in_tx = in_tx.clone();
                readers.push(Task(tokio::spawn(async move {
                    let mut dec = KissDecoder::default();
                    let mut buf = vec![0u8; 8192];
                    while let Ok(n) = r.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        for (_, p) in dec.feed(&buf[..n]) {
                            let _ = in_tx.send(In::Bcast(p));
                        }
                    }
                })));
            }
        }))
    };
    tokio::select! {
        _ = stopped(stop) => {}
        _ = serve_links(ctl, data, &bc_w, opts, in_tx, in_rx) => {}
    }
    // Everything else (listeners, connections, reader tasks) is dropped,
    // closed or aborted here.
    bc_w.lock().await.take();
}

async fn serve_links(
    ctl: TcpListener,
    data: TcpListener,
    bc_w: &tokio::sync::Mutex<Option<OwnedWriteHalf>>,
    opts: FakeOptions,
    in_tx: mpsc::UnboundedSender<In>,
    mut in_rx: mpsc::UnboundedReceiver<In>,
) {
    loop {
        let Ok((cs, _)) = ctl.accept().await else {
            return;
        };
        let Ok((ds, _)) = data.accept().await else {
            return;
        };
        let (mut cr, mut cw) = cs.into_split();
        let (mut dr, mut dw) = ds.into_split();
        let t1 = in_tx.clone();
        let rd_ctl = Task(tokio::spawn(async move {
            let mut buf = vec![0u8; 1024];
            let mut line = vec![];
            while let Ok(n) = cr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                for &b in &buf[..n] {
                    if b == b'\r' {
                        let _ = t1.send(In::Ctl(String::from_utf8_lossy(&line).trim().to_string()));
                        line.clear();
                    } else {
                        line.push(b);
                    }
                }
            }
        }));
        let t2 = in_tx.clone();
        let _rd_data = Task(tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Ok(n) = dr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let _ = t2.send(In::Data(buf[..n].to_vec()));
            }
        }));

        let mut s = Session::new(opts.clone());
        let tick_ms = if opts.fast { 5 } else { 200 };
        let mut tick = tokio::time::interval(Duration::from_millis(tick_ms));
        loop {
            tokio::select! {
                m = in_rx.recv() => {
                    let Some(m) = m else { return };
                    match m {
                        In::Ctl(l) => s.on_ctl(&l),
                        In::Data(d) => s.uplink.extend(d),
                        In::Bcast(f) => s.on_bcast(&f),
                    }
                }
                _ = tick.tick() => s.tick(tick_ms as f64 / 1000.0),
            }
            // Flush what the session produced. Data goes first: real Mercury
            // can deliver link data before its CONNECTED line arrives, and
            // the app must cope (this keeps that race covered).
            let mut dead = false;
            if !s.data_out.is_empty() {
                let d: Vec<u8> = std::mem::take(&mut s.data_out);
                dead |= dw.write_all(&d).await.is_err();
            }
            for l in s.ctl_out.drain(..) {
                dead |= cw.write_all(format!("{l}\r").as_bytes()).await.is_err();
            }
            if !s.bcast_out.is_empty() {
                if let Some(w) = bc_w.lock().await.as_mut() {
                    for f in s.bcast_out.drain(..) {
                        let _ = w.write_all(&kiss_encode(&f, 0x00)).await;
                    }
                } else {
                    s.bcast_out.clear();
                }
            }
            if dead || rd_ctl.0.is_finished() {
                break;
            }
        }
        // The readers are aborted as rd_ctl/_rd_data drop here.
    }
}

/// A timed action. `epoch`: the link (or call) it belongs to; it is
/// dropped if that has ended by the time it is due.
struct Later {
    at: f64,
    what: Act,
    epoch: Option<u64>,
}

enum Act {
    Line(String),
    Connect,
    /// The bot answering our CQ: Mercury's PENDING, then the link (both
    /// skipped if the CQ turned out to be for a Roundtable).
    AnswerCq { pending: bool },
    BotChat(String),
    BotBcast(Vec<u8>),
}

struct Session {
    o: FakeOptions,
    now: f64,
    mycall: String,
    peer: Option<String>,
    uplink: VecDeque<u8>,   // us -> bot, waiting to be "sent"
    downlink: VecDeque<u8>, // bot -> us
    last_buffer: usize,
    ptt: bool,
    later: Vec<Later>,
    ctl_out: Vec<String>,
    data_out: Vec<u8>,
    bcast_out: Vec<Vec<u8>>,
    dec: LinkDecoder,
    /// The bot's end of the link's security layer, and what waits for keys.
    ch: Option<Channel>,
    pending: VecDeque<Link>,
    bot_chat_id: u64,
    /// Files coming to the bot, by fid: size, data so far, sha.
    incoming: HashMap<u64, (u64, Vec<u8>, [u8; 32])>,
    /// The bot's files: offered (`next` None) or being sent, a chunk at a
    /// time as the air takes them (so a CANCEL stops them).
    bot_out: Vec<Outgoing>,
    /// Bumped whenever a link or call ends (or a link starts): timed
    /// actions of an older epoch are stale and dropped.
    epoch: u64,
    next_report: f64,
    next_cq: f64,
    table_msg_id: u64,
    answered_cq: bool,
    /// Our last CQ's CALL packet said Roundtable.
    cq_is_table: bool,
    t0: std::time::Instant,
}

impl Session {
    fn new(o: FakeOptions) -> Self {
        Self {
            o,
            now: 0.0,
            mycall: "N0CALL".into(),
            peer: None,
            uplink: VecDeque::new(),
            downlink: VecDeque::new(),
            last_buffer: 0,
            ptt: false,
            later: vec![],
            ctl_out: vec![],
            data_out: vec![],
            bcast_out: vec![],
            dec: LinkDecoder::default(),
            ch: None,
            pending: VecDeque::new(),
            bot_chat_id: 0,
            incoming: HashMap::new(),
            bot_out: vec![],
            epoch: 0,
            next_report: 5.0,
            next_cq: 3.0,
            table_msg_id: 0,
            answered_cq: false,
            cq_is_table: false,
            t0: std::time::Instant::now(),
        }
    }

    fn delay(&self, s: f64) -> f64 {
        self.now + if self.o.fast { s * 0.01 } else { s }
    }

    fn after(&mut self, s: f64, what: Act) {
        let at = self.delay(s);
        self.later.push(Later {
            at,
            what,
            epoch: None,
        });
    }

    /// Like `after`, but only if the current call/link is still on then.
    fn after_link(&mut self, s: f64, what: Act) {
        let at = self.delay(s);
        self.later.push(Later {
            at,
            what,
            epoch: Some(self.epoch),
        });
    }

    fn reply(&mut self, l: &str) {
        self.ctl_out.push(l.to_string());
    }

    fn on_ctl(&mut self, line: &str) {
        if std::env::var_os("WS_FAKE_DEBUG").is_some() {
            eprintln!(
                "[fake {:.1}s real {:.1}s] ctl < {line}  (peer {:?}, uplink {}, downlink {})",
                self.now,
                self.t0.elapsed().as_secs_f64(),
                self.peer,
                self.uplink.len(),
                self.downlink.len()
            );
        }
        let mut w = line.split_whitespace();
        let cmd = w.next().unwrap_or("").to_ascii_uppercase();
        match cmd.as_str() {
            "MYCALL" => {
                self.mycall = w.next().unwrap_or("N0CALL").to_ascii_uppercase();
                self.reply("OK");
                let r = format!("REGISTERED {}", self.mycall);
                self.reply(&r);
            }
            "LISTEN" | "PUBLIC" | "CHAT" | "COMPRESSION" | "P2P" | "BUSYDETECT" | "CALLINT"
            | "RETRIES" | "BEACONMODE" | "MONITOR" | "BW500" | "BW2300" | "BW2750" => {
                self.reply("OK")
            }
            "MODE" => {
                if w.next().is_some() {
                    // Like real Mercury: no listening-mode change mid-link.
                    self.reply(if self.peer.is_some() { "BUSY" } else { "OK" });
                } else {
                    self.reply("MODE 1 DATAC3");
                }
            }
            "SN" => self.reply("SN 12.4"),
            // What real Mercury 1.9 answers (it imitates VARA here).
            "VERSION" => self.reply("VARA version 4.9.0 registered"),
            "BITRATE" => self.reply("BITRATE (25) 3100 BPS"),
            "CQFRAME" => {
                self.reply("OK");
                self.reply("PTT ON");
                self.reply("PENDING");
                self.after(3.1, Act::Line("PTT OFF".into()));
                self.after(3.1, Act::Line("CANCELPENDING".into()));
                if self.o.answer_cq && self.peer.is_none() && !self.answered_cq {
                    self.answered_cq = true;
                    // Unless its CALL packet says it's for a Roundtable
                    // (then the bot joins the table instead).
                    self.cq_is_table = false;
                    self.after_link(9.0, Act::AnswerCq { pending: true });
                    self.after_link(12.0, Act::AnswerCq { pending: false });
                }
            }
            "CONNECT" => {
                let them = line
                    .split_whitespace()
                    .nth(2)
                    .unwrap_or("")
                    .to_ascii_uppercase();
                // Both are cancelled by a DISCONNECT/ABORT before they're due.
                if them.eq_ignore_ascii_case(&self.o.bot_call) {
                    self.reply("OK");
                    self.after_link(6.0, Act::Connect);
                } else {
                    // Nobody by that call is on the air.
                    self.reply("OK");
                    self.after_link(40.0, Act::Line("DISCONNECTED".into()));
                }
            }
            "DISCONNECT" | "ABORT" => {
                self.reply("OK");
                self.end_link();
            }
            "TUNE" => {
                self.reply("OK");
                let on = w.next() != Some("OFF");
                self.reply(if on { "PTT ON" } else { "PTT OFF" });
            }
            // Every command gets an answer (unknown ones WRONG), so the app
            // never waits out its reply time-out. (BUSYDETECT ON/OFF/bare
            // are all OK above.)
            _ => self.reply("WRONG"),
        }
    }

    fn bot_send(&mut self, p: Link) {
        self.pending.push_back(p);
        self.flush();
    }

    /// Offer a file (a new offer of the same fid replaces the old one).
    fn offer_file(&mut self, out: Outgoing) {
        self.bot_send(out.offer());
        self.bot_out.retain(|o| o.fid != out.fid);
        self.bot_out.push(out);
    }

    /// A file to the bot is complete.
    fn file_in(&mut self, fid: u64, ok: bool) {
        self.bot_send(Link::Done { fid, ok });
        let t = if ok {
            "File received fine, checksum good. Nice!"
        } else {
            "That file arrived damaged (checksum wrong), sorry."
        };
        self.after_link(2.0, Act::BotChat(t.into()));
    }

    /// Hand the air the next chunks of the bot's accepted files, only as
    /// fast as it takes them (about `budget` bytes per tick), so a CANCEL
    /// stops a file within a chunk or so.
    fn pump_files(&mut self, budget: usize) {
        while self.ch.is_some() && self.pending.len() < 2 && self.downlink.len() < budget {
            let Some(i) = self.bot_out.iter().position(|o| o.next.is_some()) else {
                break;
            };
            match self.bot_out[i].next_chunk() {
                Some(c) => self.bot_send(c),
                None => {
                    self.bot_out.remove(i);
                }
            }
        }
    }

    /// Send what the bot's security layer lets through (all of it once
    /// the link is plain or keys are agreed).
    fn flush(&mut self) {
        let Some(ch) = self.ch.as_mut() else { return };
        while let Some(p) = self.pending.front() {
            let Some(w) = ch.wrap(p) else { break };
            self.downlink.extend(w);
            self.pending.pop_front();
        }
    }

    /// The bot hangs up (BYE, or a security failure).
    fn hang_up(&mut self) {
        self.end_link();
    }

    /// The link (or the call being placed) ends, whoever ended it: unkey,
    /// say DISCONNECTED, forget everything of it (nothing of it may reach
    /// the next link) and cancel what was scheduled for it.
    fn end_link(&mut self) {
        if self.ptt {
            self.ptt = false;
            self.reply("PTT OFF");
        }
        if self.last_buffer != 0 {
            self.last_buffer = 0;
            self.reply("BUFFER 0");
        }
        self.reply("DISCONNECTED");
        self.peer = None;
        self.ch = None;
        self.dec = LinkDecoder::default();
        self.pending.clear();
        self.uplink.clear();
        self.downlink.clear();
        self.incoming.clear();
        self.bot_out.clear();
        self.epoch += 1;
    }

    fn connect_bot(&mut self) {
        if std::env::var_os("WS_FAKE_DEBUG").is_some() {
            eprintln!("[fake {:.1}s] bot connects", self.now);
        }
        let bot = self.o.bot_call.clone();
        self.peer = Some(bot.clone());
        self.dec = LinkDecoder::default();
        self.pending.clear();
        self.uplink.clear();
        self.downlink.clear();
        self.incoming.clear();
        self.bot_out.clear();
        // A new link: what an earlier call scheduled (an absent station's
        // time-out, another answer) no longer applies.
        self.epoch += 1;
        let l = format!("CONNECTED {} {} 2300", bot, self.mycall);
        self.reply(&l);
        let hello = Link::Hello {
            version: VERSION,
            caps: CAP_RESUME | CAP_ZIP,
            name: "Demo station".into(),
            grid: "DN31".into(),
            app: "Whistlestop demo".into(),
        };
        let (ch, first) = Channel::open(self.o.bot_e2e, hello, std::time::Instant::now());
        self.ch = Some(ch);
        for w in first {
            self.downlink.extend(w);
        }
        self.bot_chat_id += 1;
        let hi = format!(
            "Hi {}, K7BOT demo station here. Good signal! Type anything, try sending a file, or ask me for a file.",
            self.mycall
        );
        self.bot_send(Link::Chat {
            id: self.bot_chat_id,
            text: hi,
        });
        // Greeting goes out with CONNECTED, ahead of it on the wire.
        let early: Vec<u8> = self.downlink.drain(..).collect();
        self.data_out.extend(early);
    }

    fn on_link(&mut self, p: Link) {
        match p {
            Link::Chat { id, text } => {
                self.bot_send(Link::Received { id });
                let t = text.to_lowercase();
                let reply = if t.contains("file") || t.contains("send me") {
                    let body = format!(
                        "Welcome to Whistlestop!\n\nThis file came from the K7BOT demo station.\nSent to {} over a simulated Mercury link.\n73\n",
                        self.mycall
                    )
                    .repeat(20);
                    let out = Outgoing::new(
                        500,
                        "welcome-from-k7bot.txt".into(),
                        body.into_bytes(),
                        false,
                        1,
                    );
                    if self.o.bot_pushy {
                        // A misbehaving station: the pieces right behind the
                        // offer, without waiting for the answer.
                        let mut early = Outgoing::new(out.fid, out.name.clone(), out.data.clone(), false, 1);
                        early.next = Some(0);
                        self.bot_send(out.offer());
                        while let Some(c) = early.next_chunk() {
                            self.bot_send(c);
                        }
                    }
                    self.offer_file(out);
                    "Sure, offering you a file now.".to_string()
                } else if t.contains("73") || t.contains("bye") {
                    "73! Thanks for the contact.".to_string()
                } else {
                    format!(
                        "Copy: \"{}\". Signal here is 12 dB, we're running QAM16C2.",
                        text.trim()
                    )
                };
                self.after_link(4.0, Act::BotChat(reply));
            }
            // Any number of files at once, told apart by fid.
            Link::Offer { fid, size, sha, .. } => {
                self.bot_send(Link::Answer {
                    fid,
                    accept: true,
                    resume_from: 0,
                });
                if size == 0 {
                    // No chunks will come.
                    self.file_in(fid, sha256(&[]) == sha);
                } else {
                    self.incoming.insert(fid, (size, vec![], sha));
                }
            }
            Link::Chunk { fid, offset, data } => {
                let mut done = None;
                if let Some((size, buf, sha)) = self.incoming.get_mut(&fid)
                    && offset == buf.len() as u64
                {
                    buf.extend_from_slice(&data);
                    if buf.len() as u64 >= *size {
                        done = Some(sha256(buf) == *sha);
                    }
                }
                if let Some(ok) = done {
                    self.incoming.remove(&fid);
                    self.file_in(fid, ok);
                }
            }
            // Either way: a file coming in is dropped, one going out stops.
            Link::Cancel { fid } => {
                self.incoming.remove(&fid);
                self.bot_out.retain(|o| o.fid != fid);
            }
            Link::Answer {
                fid,
                accept,
                resume_from,
            } => {
                if accept {
                    if let Some(out) = self.bot_out.iter_mut().find(|o| o.fid == fid) {
                        out.next = Some(resume_from);
                    }
                } else {
                    self.bot_out.retain(|o| o.fid != fid);
                }
            }
            Link::Mail { id, .. } => self.bot_send(Link::MailAck { id }),
            // K7BOT agrees to any QSY within 20 m, and has a small library.
            Link::Qsy { id, dial_hz, .. } => {
                let ok = (14_000_000..=14_350_000).contains(&dial_hz);
                self.bot_send(Link::QsyAnswer {
                    id,
                    accept: ok,
                    reason: if ok {
                        String::new()
                    } else {
                        "K7BOT stays on 20 m".into()
                    },
                });
            }
            Link::ListReq { id, path } => {
                let entries = if path.is_empty() {
                    vec![
                        LibEntry {
                            name: "maps".into(),
                            size: 0,
                            dir: true,
                        },
                        LibEntry {
                            name: "k7bot-qsl.txt".into(),
                            size: 1240,
                            dir: false,
                        },
                    ]
                } else {
                    vec![LibEntry {
                        name: "grid.txt".into(),
                        size: 600,
                        dir: false,
                    }]
                };
                self.bot_send(Link::ListAns {
                    id,
                    path,
                    entries,
                    note: String::new(),
                });
            }
            Link::Fetch { id, path } => {
                if path.ends_with("k7bot-qsl.txt") {
                    self.bot_send(Link::FetchAnswer {
                        id,
                        ok: true,
                        reason: String::new(),
                    });
                    let out = Outgoing::new(
                        id,
                        "k7bot-qsl.txt".into(),
                        "QSL from K7BOT, Whistlestop demo station. 73!\n"
                            .repeat(26)
                            .into_bytes(),
                        false,
                        1,
                    );
                    self.offer_file(out);
                } else {
                    self.bot_send(Link::FetchAnswer {
                        id,
                        ok: false,
                        reason: "no such file".into(),
                    });
                }
            }
            Link::Bye { .. } => self.hang_up(),
            _ => {}
        }
    }

    fn on_bcast(&mut self, f: &[u8]) {
        let Some(fr) = TableFrame::decode(f) else {
            return;
        };
        let bot = self.o.bot_call.clone();
        if matches!(fr.body, TableBody::Call { kind: CALL_TABLE, .. }) {
            self.cq_is_table = true;
        }
        let is_join = matches!(fr.body, TableBody::Join { .. });
        let reply_text = match fr.body {
            TableBody::Call {
                kind: CALL_TABLE,
                table_name,
                ..
            }
            | TableBody::Join { table_name, .. } => {
                if let Some(after) = self.o.bot_table_leave_after
                    && is_join
                    && self.o.bot_cq_table.as_ref() == Some(&table_name)
                {
                    let id = table_id(&table_name);
                    let speed = TableFrame {
                        table: id,
                        from: bot.clone(),
                        body: TableBody::Speed {
                            mode_index: crate::modes::mode_by_name("DATAC3").unwrap().index,
                            last_id: 0,
                        },
                    };
                    self.after(2.0, Act::BotBcast(speed.encode()));
                    let leave = TableFrame {
                        table: id,
                        from: bot.clone(),
                        body: TableBody::Leave,
                    };
                    self.after(2.0 + after, Act::BotBcast(leave.encode()));
                    return;
                }
                let j = TableFrame {
                    table: fr.table,
                    from: bot.clone(),
                    body: TableBody::Join {
                        table_name,
                        snr_of_host: 9,
                    },
                };
                let b = j.encode();
                self.after(5.0, Act::BotBcast(b));
                Some("K7BOT joining the table. Good evening all!".to_string())
            }
            TableBody::Text { .. } => Some("K7BOT: copy all, solid copy here.".to_string()),
            _ => None,
        };
        if let Some(t) = reply_text {
            self.table_msg_id += 1;
            if let Ok(frames) = table_text_frames(fr.table, &bot, self.table_msg_id, &t, 123) {
                for (i, f) in frames.into_iter().enumerate() {
                    self.after(9.0 + i as f64 * 3.2, Act::BotBcast(f));
                }
            }
        }
    }

    fn tick(&mut self, dt: f64) {
        self.now += dt;
        let now = self.now;
        let due: Vec<Later> = {
            let (due, keep): (Vec<_>, Vec<_>) = self.later.drain(..).partition(|l| l.at <= now);
            self.later = keep;
            due
        };
        for l in due {
            if l.epoch.is_some_and(|e| e != self.epoch) {
                continue; // its call or link has ended since
            }
            match l.what {
                Act::Line(s) if s == "DISCONNECTED" => self.end_link(),
                Act::Line(s) => self.reply(&s),
                Act::Connect => {
                    if self.peer.is_none() {
                        self.connect_bot()
                    }
                }
                Act::AnswerCq { pending } => {
                    if self.peer.is_none() && !self.cq_is_table {
                        if pending {
                            self.reply("PENDING");
                        } else {
                            self.connect_bot()
                        }
                    }
                }
                Act::BotChat(t) => {
                    if self.peer.is_some() {
                        self.bot_chat_id += 1;
                        let id = self.bot_chat_id;
                        self.bot_send(Link::Chat { id, text: t });
                    }
                }
                Act::BotBcast(f) => self.bcast_out.push(f),
            }
        }

        // The "air": move bytes at the link's rate, both ways (half duplex
        // in reality; good enough to show progress and ETAs).
        let speed = if self.o.fast { 1e9 } else { self.o.bps };
        let budget = ((speed / 8.0) * dt).ceil() as usize;
        self.air(budget);

        // The bot calls CQ on its frequency; Mercury decodes the CQFRAME only
        // when our radio is tuned within its +/-55 Hz capture window.
        if let Some(rf) = &self.o.rig_freq {
            let f = *rf.lock().unwrap();
            if (f - self.o.bot_freq).abs() <= 55.0
                && self.peer.is_none()
                && self.now >= self.next_cq
            {
                self.next_cq = self.delay(20.0);
                {
                    let l = format!("CQFRAME {} 2300", self.o.bot_call);
                    self.reply(&l);
                    let (kind, table_name) = match &self.o.bot_cq_table {
                        Some(t) => (CALL_TABLE, t.clone()),
                        None => (CALL_CHAT, String::new()),
                    };
                    let call = TableFrame {
                        table: 0,
                        from: self.o.bot_call.clone(),
                        body: TableBody::Call {
                            kind,
                            table_name,
                            bw_code: 2,
                            grid: "DN31".into(),
                        },
                    };
                    self.bcast_out.push(call.encode());
                }
            }
        }
    }

    /// One tick of the link: `budget` bytes each way.
    fn air(&mut self, budget: usize) {
        if self.peer.is_none() {
            self.uplink.clear();
            if self.ptt {
                self.ptt = false;
                self.reply("PTT OFF");
            }
            return;
        }
        let n = budget.min(self.uplink.len());
        if n > 0 {
            if !self.ptt {
                self.ptt = true;
                self.reply("PTT ON");
            }
            let bytes: Vec<u8> = self.uplink.drain(..n).collect();
            match self.dec.feed_bodies(&bytes) {
                Ok(bodies) => {
                    for b in bodies {
                        let Some(ch) = self.ch.as_mut() else { break };
                        match ch.on_body(&b) {
                            Ok(step) => {
                                for w in step.send {
                                    self.downlink.extend(w);
                                }
                                if let Some(Notice::Secure { sas }) = step.notice {
                                    self.bot_chat_id += 1;
                                    let t = format!(
                                        "🔒 Encrypted now. My screen shows the code {sas}; does yours?"
                                    );
                                    self.pending.push_back(Link::Chat {
                                        id: self.bot_chat_id,
                                        text: t,
                                    });
                                }
                                for p in step.app {
                                    // After a BYE the rest is not for this link.
                                    if self.peer.is_none() {
                                        break;
                                    }
                                    self.on_link(p);
                                }
                            }
                            // Policy refused, or tampering: hang up, like Whistlestop.
                            Err(_) => self.hang_up(),
                        }
                    }
                    self.flush();
                }
                Err(_) => self.dec = LinkDecoder::default(),
            }
            if self.peer.is_none() {
                return; // the bot hung up; end_link said all there is to say
            }
        } else if self.ptt {
            self.ptt = false;
            self.reply("PTT OFF");
        }
        self.pump_files(budget);
        let n = budget.min(self.downlink.len());
        self.data_out.extend(self.downlink.drain(..n));
        if self.uplink.len() != self.last_buffer {
            self.last_buffer = self.uplink.len();
            let l = format!("BUFFER {}", self.last_buffer);
            self.reply(&l);
        }
        if self.now >= self.next_report {
            self.next_report = self.delay(5.0);
            self.reply("BITRATE (25) 3100 BPS");
            self.reply("SN 12.4");
        }
    }
}

/// Tests: demo stations started with this callsign get a K7BOT whose CQs
/// are for a Roundtable (see `FakeOptions::bot_cq_table`).
#[cfg(test)]
pub static TEST_BOT_TABLE: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

pub fn test_bot_table(_mycall: &str) -> Option<String> {
    #[cfg(test)]
    {
        return TEST_BOT_TABLE
            .lock()
            .unwrap()
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(_mycall))
            .map(|(_, t)| t.clone());
    }
    #[cfg(not(test))]
    None
}
