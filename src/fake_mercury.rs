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
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;

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
}

impl Default for FakeOptions {
    fn default() -> Self {
        Self { fast: false, bot_call: "K7BOT".into(), bps: 1400.0, rig_freq: None, bot_freq: 14_103_150.0, answer_cq: true, bot_e2e: Policy::Prefer }
    }
}

pub struct FakeMercury {
    pub port: u16,
    pub bcast_port: u16,
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
        tokio::spawn(serve(ctl, data, bc, opts));
        Ok(Self { port, bcast_port })
    }
}

async fn serve(ctl: TcpListener, data: TcpListener, bc: TcpListener, opts: FakeOptions) {
    let bc_w: Arc<tokio::sync::Mutex<Option<OwnedWriteHalf>>> = Default::default();
    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<In>();
    {
        let bc_w = bc_w.clone();
        let in_tx = in_tx.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = bc.accept().await {
                let (mut r, w) = s.into_split();
                *bc_w.lock().await = Some(w);
                let in_tx = in_tx.clone();
                tokio::spawn(async move {
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
                });
            }
        });
    }
    loop {
        let Ok((cs, _)) = ctl.accept().await else { return };
        let Ok((ds, _)) = data.accept().await else { return };
        let (mut cr, mut cw) = cs.into_split();
        let (mut dr, mut dw) = ds.into_split();
        let t1 = in_tx.clone();
        let rd_ctl = tokio::spawn(async move {
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
        });
        let t2 = in_tx.clone();
        let rd_data = tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Ok(n) = dr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let _ = t2.send(In::Data(buf[..n].to_vec()));
            }
        });

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
            if dead || rd_ctl.is_finished() {
                break;
            }
        }
        rd_ctl.abort();
        rd_data.abort();
    }
}

/// A timed action.
struct Later {
    at: f64,
    what: Act,
}

enum Act {
    Line(String),
    Connect,
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
    incoming: Option<(u64, u64, Vec<u8>, [u8; 32])>, // fid, size, data, sha
    bot_offer: Option<Outgoing>,
    next_report: f64,
    next_cq: f64,
    table_msg_id: u64,
    answered_cq: bool,
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
            incoming: None,
            bot_offer: None,
            next_report: 5.0,
            next_cq: 3.0,
            table_msg_id: 0,
            answered_cq: false,
            t0: std::time::Instant::now(),
        }
    }

    fn delay(&self, s: f64) -> f64 {
        self.now + if self.o.fast { s * 0.01 } else { s }
    }

    fn after(&mut self, s: f64, what: Act) {
        let at = self.delay(s);
        self.later.push(Later { at, what });
    }

    fn reply(&mut self, l: &str) {
        self.ctl_out.push(l.to_string());
    }

    fn on_ctl(&mut self, line: &str) {
        if std::env::var_os("WS_FAKE_DEBUG").is_some() {
            eprintln!("[fake {:.1}s real {:.1}s] ctl < {line}  (peer {:?}, uplink {}, downlink {})", self.now, self.t0.elapsed().as_secs_f64(), self.peer, self.uplink.len(), self.downlink.len());
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
            "LISTEN" | "PUBLIC" | "CHAT" | "COMPRESSION" | "P2P" | "BUSYDETECT" | "CALLINT" | "RETRIES"
            | "BEACONMODE" | "MONITOR" | "BW500" | "BW2300" | "BW2750" => self.reply("OK"),
            "MODE" => {
                if w.next().is_some() {
                    self.reply("OK");
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
                    self.after(9.0, Act::Line("PENDING".into()));
                    self.after(12.0, Act::Connect);
                }
            }
            "CONNECT" => {
                let them = line.split_whitespace().nth(2).unwrap_or("").to_ascii_uppercase();
                if them.eq_ignore_ascii_case(&self.o.bot_call) {
                    self.reply("OK");
                    self.after(6.0, Act::Connect);
                } else {
                    // Nobody by that call is on the air.
                    self.reply("OK");
                    self.after(40.0, Act::Line("DISCONNECTED".into()));
                }
            }
            "DISCONNECT" | "ABORT" => {
                self.reply("OK");
                if self.ptt {
                    self.ptt = false;
                    self.reply("PTT OFF");
                }
                self.reply("DISCONNECTED");
                self.peer = None;
                self.ch = None;
                self.pending.clear();
                self.uplink.clear();
                self.downlink.clear();
            }
            "TUNE" => {
                self.reply("OK");
                let on = w.next() != Some("OFF");
                self.reply(if on { "PTT ON" } else { "PTT OFF" });
            }
            _ => self.reply("WRONG"),
        }
    }

    fn bot_send(&mut self, p: Link) {
        self.pending.push_back(p);
        self.flush();
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

    fn hang_up(&mut self) {
        self.reply("DISCONNECTED");
        self.peer = None;
        self.ch = None;
        self.pending.clear();
    }

    fn connect_bot(&mut self) {
        if std::env::var_os("WS_FAKE_DEBUG").is_some() {
            eprintln!("[fake {:.1}s] bot connects", self.now);
        }
        let bot = self.o.bot_call.clone();
        self.peer = Some(bot.clone());
        self.dec = LinkDecoder::default();
        self.pending.clear();
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
        self.bot_send(Link::Chat { id: self.bot_chat_id, text: hi });
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
                    let out = Outgoing::new(500, "welcome-from-k7bot.txt".into(), body.into_bytes(), false, 1);
                    self.bot_send(out.offer());
                    self.bot_offer = Some(out);
                    "Sure, offering you a file now.".to_string()
                } else if t.contains("73") || t.contains("bye") {
                    "73! Thanks for the contact.".to_string()
                } else {
                    format!("Copy: \"{}\". Signal here is 12 dB, we're running QAM16C2.", text.trim())
                };
                self.after(4.0, Act::BotChat(reply));
            }
            Link::Offer { fid, size, sha, .. } => {
                self.incoming = Some((fid, size, vec![], sha));
                self.bot_send(Link::Answer { fid, accept: true, resume_from: 0 });
            }
            Link::Chunk { fid, offset, data } => {
                let mut done = None;
                if let Some((f, size, buf, sha)) = &mut self.incoming
                    && *f == fid && offset == buf.len() as u64 {
                        buf.extend_from_slice(&data);
                        if buf.len() as u64 >= *size {
                            done = Some((fid, sha256(buf) == *sha));
                        }
                    }
                if let Some((fid, ok)) = done {
                    self.incoming = None;
                    self.bot_send(Link::Done { fid, ok });
                    self.after(2.0, Act::BotChat("File received fine, checksum good. Nice!".into()));
                }
            }
            Link::Answer { fid, accept, resume_from } => {
                if let Some(mut out) = self.bot_offer.take()
                    && out.fid == fid && accept {
                        out.next = Some(resume_from);
                        while let Some(c) = out.next_chunk() {
                            self.bot_send(c);
                        }
                    }
            }
            Link::Mail { id, .. } => self.bot_send(Link::MailAck { id }),
            Link::Bye { .. } => self.hang_up(),
            _ => {}
        }
    }

    fn on_bcast(&mut self, f: &[u8]) {
        let Some(fr) = TableFrame::decode(f) else { return };
        let bot = self.o.bot_call.clone();
        let reply_text = match fr.body {
            TableBody::Call { kind: CALL_TABLE, table_name, .. } | TableBody::Join { table_name, .. } => {
                let j = TableFrame { table: fr.table, from: bot.clone(), body: TableBody::Join { table_name, snr_of_host: 9 } };
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
                    let at = self.delay(9.0 + i as f64 * 3.2);
                    self.later.push(Later { at, what: Act::BotBcast(f) });
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
            match l.what {
                Act::Line(s) => {
                    if s == "DISCONNECTED" {
                        self.peer = None;
                        self.ch = None;
                    }
                    self.reply(&s)
                }
                Act::Connect => {
                    if self.peer.is_none() {
                        self.connect_bot()
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
        if self.peer.is_some() {
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
                                        let t = format!("🔒 Encrypted now. My screen shows the code {sas}; does yours?");
                                        self.pending.push_back(Link::Chat { id: self.bot_chat_id, text: t });
                                    }
                                    step.app.into_iter().for_each(|p| self.on_link(p));
                                }
                                // Policy refused, or tampering: hang up, like Whistlestop.
                                Err(_) => self.hang_up(),
                            }
                        }
                        self.flush();
                    }
                    Err(_) => self.dec = LinkDecoder::default(),
                }
            } else if self.ptt {
                self.ptt = false;
                self.reply("PTT OFF");
            }
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
        } else {
            self.uplink.clear();
        }

        // The bot calls CQ on its frequency; Mercury decodes the CQFRAME only
        // when our radio is tuned within its +/-55 Hz capture window.
        if let Some(rf) = &self.o.rig_freq {
            let f = *rf.lock().unwrap();
            if (f - self.o.bot_freq).abs() <= 55.0 && self.peer.is_none() && self.now >= self.next_cq {
                self.next_cq = self.delay(20.0);
                {
                    let l = format!("CQFRAME {} 2300", self.o.bot_call);
                    self.reply(&l);
                    let call = TableFrame {
                        table: 0,
                        from: self.o.bot_call.clone(),
                        body: TableBody::Call { kind: CALL_CHAT, table_name: String::new(), bw_code: 2, grid: "DN31".into() },
                    };
                    self.bcast_out.push(call.encode());
                }
            }
        }
    }
}
