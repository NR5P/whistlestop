//! K7BOT, the demo station, running on its own real Mercury modem (Demo
//! mode with Mercury installed). It calls CQ with a real whistle and
//! CQFRAME every 45 s, answers connects, chats, takes and offers files, and
//! sits at Roundtables, all in WSP over real Mercury ARQ and broadcast.

use crate::audio::LiveOutput;
use crate::protocol::*;
use crate::rig::{Rig, RigctldClient};
use crate::tnc::{Tnc, TncEvent, parse_connected};
use crate::whistle::{WhistleParams, whistle};
use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;

pub const CALL: &str = "K7BOT";

pub struct BotPorts {
    pub mercury: u16,
    pub bcast: u16,
    pub rig: u16,
    pub audio: u16,
}

pub async fn run(p: BotPorts, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<()> {
    let (etx, mut erx) = mpsc::unbounded_channel();
    let mut tnc = None;
    for _ in 0..50 {
        match Tnc::connect("127.0.0.1", p.mercury, Some(p.bcast), etx.clone()).await {
            Ok(t) => {
                tnc = Some(t);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
    let mut tnc = tnc.ok_or_else(|| anyhow::anyhow!("K7BOT: Mercury did not start"))?;
    tnc.setup(CALL, 2300).await?;
    let _ = tnc.command("MODE 1").await;
    let mut rig = Rig::Rigctld(RigctldClient::connect(&format!("127.0.0.1:{}", p.rig)).await?);
    let audio = std::net::TcpStream::connect(("127.0.0.1", p.audio))?;
    let out = std::sync::Arc::new(LiveOutput::ether(audio.try_clone()?));
    // A whistle in progress (it plays in the background so K7BOT keeps
    // hearing Mercury: a connect can arrive mid-whistle).
    let mut whistling: Option<tokio::task::JoinHandle<()>> = None;
    let mut incoming_call = false;
    // Drain what the band sends this station's "speaker" (unused).
    std::thread::spawn(move || {
        use std::io::Read;
        let mut a = audio;
        let mut b = [0u8; 4096];
        while matches!(a.read(&mut b), Ok(n) if n > 0) {}
    });

    let mut dec = LinkDecoder::default();
    let mut peer: Option<String> = None;
    let mut chat_id = 0u64;
    let mut incoming: Option<(u64, u64, Vec<u8>, [u8; 32])> = None;
    let mut offer: Option<Outgoing> = None;
    let mut table_msg = 0u64;
    let mut next_cq = Instant::now() + Duration::from_secs(4);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut later: Vec<(Instant, Vec<u8>)> = vec![]; // delayed link data
    let mut later_bc: Vec<(Instant, Vec<u8>)> = vec![]; // delayed broadcast

    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::select! {
            ev = erx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    TncEvent::Line(l) if l == "PENDING" => incoming_call = true,
                    TncEvent::Line(l) if l == "CANCELPENDING" => incoming_call = false,
                    TncEvent::Line(l) if l.starts_with("CONNECTED") => {
                        let Some((who, _)) = parse_connected(&l, CALL) else { continue };
                        incoming_call = false;
                        if let Some(w) = whistling.take() {
                            w.abort();
                            out.abort();
                            rig.ptt(false).await?;
                        }
                        peer = Some(who.clone());
                        dec = LinkDecoder::default();
                        let mut w = Link::Hello { version: VERSION, caps: CAP_RESUME | CAP_ZIP, name: "Demo station".into(), grid: "DN31".into(), app: "Whistlestop demo (real Mercury)".into() }.encode();
                        chat_id += 1;
                        w.extend(Link::Chat { id: chat_id, text: format!("Hi {who}, K7BOT here on a real Mercury modem. Type anything, send me a file, or ask me for one.") }.encode());
                        tnc.send_data(&w).await?;
                    }
                    TncEvent::Line(l) if l.starts_with("DISCONNECTED") => {
                        peer = None;
                        incoming = None;
                        offer = None;
                        next_cq = Instant::now() + Duration::from_secs(20);
                    }
                    TncEvent::Line(_) => {}
                    TncEvent::Data(d) => {
                        let Ok(pkts) = dec.feed(&d) else { dec = LinkDecoder::default(); continue };
                        for pkt in pkts {
                            match pkt {
                                Link::Chat { id, text } => {
                                    tnc.send_data(&Link::Received { id }.encode()).await?;
                                    let t = text.to_lowercase();
                                    let reply = if t.contains("file") || t.contains("send me") {
                                        let body = "Welcome to Whistlestop!\n\nThis file came from K7BOT over a real Mercury link.\n73\n".to_string().repeat(20);
                                        let o = Outgoing::new(500, "welcome-from-k7bot.txt".into(), body.into_bytes(), false, 1);
                                        tnc.send_data(&o.offer().encode()).await?;
                                        offer = Some(o);
                                        "Sure, offering you a file now.".to_string()
                                    } else if t.contains("73") || t.contains("bye") {
                                        "73! Thanks for the contact.".to_string()
                                    } else {
                                        format!("Copy: \"{}\". Solid copy over real Mercury.", text.trim())
                                    };
                                    chat_id += 1;
                                    later.push((Instant::now() + Duration::from_secs(2), Link::Chat { id: chat_id, text: reply }.encode()));
                                }
                                Link::Offer { fid, size, sha, .. } => {
                                    incoming = Some((fid, size, vec![], sha));
                                    tnc.send_data(&Link::Answer { fid, accept: true, resume_from: 0 }.encode()).await?;
                                }
                                Link::Chunk { fid, offset, data } => {
                                    let mut done = None;
                                    if let Some((f, size, buf, sha)) = incoming.as_mut()
                                        && *f == fid && offset == buf.len() as u64 {
                                            buf.extend_from_slice(&data);
                                            if buf.len() as u64 >= *size {
                                                done = Some((fid, sha256(buf) == *sha));
                                            }
                                        }
                                    if let Some((fid, ok)) = done {
                                        incoming = None;
                                        tnc.send_data(&Link::Done { fid, ok }.encode()).await?;
                                        chat_id += 1;
                                        later.push((Instant::now() + Duration::from_secs(1), Link::Chat { id: chat_id, text: "File received, checksum good. Nice!".into() }.encode()));
                                    }
                                }
                                Link::Answer { fid, accept, resume_from } => {
                                    if let Some(mut o) = offer.take()
                                        && o.fid == fid && accept {
                                            o.next = Some(resume_from);
                                            let mut w = vec![];
                                            while let Some(c) = o.next_chunk() {
                                                w.extend(c.encode());
                                            }
                                            tnc.send_data(&w).await?;
                                        }
                                }
                                Link::Bye { .. } => {
                                    let _ = tnc.command("DISCONNECT").await;
                                }
                                _ => {}
                            }
                        }
                    }
                    TncEvent::Broadcast(f) => {
                        let Some(fr) = TableFrame::decode(&f) else { continue };
                        let text = match fr.body {
                            TableBody::Call { kind: CALL_TABLE, table_name, .. } | TableBody::Join { table_name, .. } => {
                                let j = TableFrame { table: fr.table, from: CALL.into(), body: TableBody::Join { table_name, snr_of_host: 9 } };
                                later_bc.push((Instant::now() + Duration::from_secs(4), j.encode()));
                                Some("K7BOT joining the table. Good evening all!")
                            }
                            TableBody::Text { part, total, .. } if part == total => Some("K7BOT: copy all, solid copy here."),
                            TableBody::Speed { mode_index } => {
                                let _ = tnc.command(&format!("MODE {mode_index}")).await;
                                None
                            }
                            _ => None,
                        };
                        if let Some(t) = text {
                            table_msg += 1;
                            if let Ok(frames) = table_text_frames(fr.table, CALL, table_msg, t, 123) {
                                for (i, f) in frames.into_iter().enumerate() {
                                    later_bc.push((Instant::now() + Duration::from_secs(9 + 4 * i as u64), f));
                                }
                            }
                        }
                    }
                    TncEvent::Closed(_) => break,
                }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                let (due, keep): (Vec<_>, Vec<_>) = later.drain(..).partition(|(at, _)| *at <= now);
                later = keep;
                for (_, d) in due {
                    if peer.is_some() {
                        tnc.send_data(&d).await?;
                    }
                }
                let (due, keep): (Vec<_>, Vec<_>) = later_bc.drain(..).partition(|(at, _)| *at <= now);
                later_bc = keep;
                for (_, f) in due {
                    tnc.send_broadcast(&f).await?;
                }
                // Call CQ every 45 s while nobody is linked or calling in: a
                // real whistle, then Mercury's CQFRAME.
                let idle = peer.is_none() && !incoming_call;
                if let Some(w) = &whistling {
                    if w.is_finished() {
                        whistling = None;
                        rig.ptt(false).await?;
                        if peer.is_none() && !incoming_call {
                            let _ = tnc.command(&format!("CQFRAME {CALL} 2300")).await;
                            let c = TableFrame { table: 0, from: CALL.into(), body: TableBody::Call { kind: CALL_CHAT, table_name: String::new(), bw_code: 4, grid: "DN31".into() } };
                            later_bc.push((Instant::now() + Duration::from_secs(5), c.encode()));
                        }
                    }
                } else if idle && now >= next_cq {
                    next_cq = now + Duration::from_secs(45);
                    rig.ptt(true).await?;
                    let o = out.clone();
                    whistling = Some(tokio::spawn(async move { o.play(&whistle(25.0, &WhistleParams::default())).await }));
                }
            }
        }
    }
    Ok(())
}
