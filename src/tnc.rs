//! Async client for Mercury's VARA-style TNC (Mercury docs/TNC.md).
//!
//! Control port (default 8300): CR-terminated ASCII commands and status.
//! Data port (control + 1): the ARQ byte pipe.
//! Broadcast port (8100): KISS frames, for the Roundtable plane.

use crate::protocol::{KissDecoder, kiss_encode};
use anyhow::{Context, Result, anyhow, bail};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum TncEvent {
    /// An asynchronous status line (CONNECTED, BUFFER, PTT ON, CQFRAME ...).
    Line(String),
    /// Bytes from the ARQ data port.
    Data(Vec<u8>),
    /// One frame from the broadcast plane.
    Broadcast(Vec<u8>),
    Closed(&'static str),
}

pub struct Tnc {
    ctl: OwnedWriteHalf,
    data: OwnedWriteHalf,
    bcast: Option<OwnedWriteHalf>,
    replies: mpsc::UnboundedReceiver<String>,
}

const REPLIES: [&str; 3] = ["OK", "WRONG", "BUSY"];

/// Mercury answered BUSY: it is in (or still closing) an ARQ link and
/// won't take this command yet. Worth trying again once it's free.
#[derive(Debug)]
pub struct Busy(pub String);

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mercury: {:?} -> BUSY", self.0)
    }
}

impl std::error::Error for Busy {}

/// Was this error Mercury saying BUSY?
pub fn is_busy(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Busy>().is_some()
}

async fn dial(addr: &str) -> Result<TcpStream> {
    let s = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("Mercury at {addr} did not answer"))?
        .with_context(|| format!("connecting to Mercury at {addr} (is it running?)"))?;
    s.set_nodelay(true).ok();
    Ok(s)
}

impl Tnc {
    /// Connect to Mercury. Events go to `events`.
    pub async fn connect(
        host: &str,
        port: u16,
        bcast_port: Option<u16>,
        events: mpsc::UnboundedSender<TncEvent>,
    ) -> Result<Self> {
        let (mut cr, ctl) = dial(&format!("{host}:{port}")).await?.into_split();
        let (mut dr, data) = dial(&format!("{host}:{}", port + 1)).await?.into_split();
        let (rtx, replies) = mpsc::unbounded_channel();

        let ev = events.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut line = Vec::new();
            loop {
                let n = match cr.read(&mut buf).await {
                    Ok(0) | Err(_) => {
                        let _ = ev.send(TncEvent::Closed("control"));
                        return;
                    }
                    Ok(n) => n,
                };
                for &b in &buf[..n] {
                    if b == b'\r' || b == b'\n' {
                        let s = String::from_utf8_lossy(&line).trim().to_string();
                        line.clear();
                        if s.is_empty() || s == "IAMALIVE" {
                            continue;
                        }
                        if REPLIES.contains(&s.as_str()) {
                            let _ = rtx.send(s);
                        } else {
                            let _ = ev.send(TncEvent::Line(s));
                        }
                    } else {
                        line.push(b);
                    }
                }
            }
        });

        let ev = events.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                match dr.read(&mut buf).await {
                    Ok(0) | Err(_) => {
                        let _ = ev.send(TncEvent::Closed("data"));
                        return;
                    }
                    Ok(n) => {
                        let _ = ev.send(TncEvent::Data(buf[..n].to_vec()));
                    }
                }
            }
        });

        let bcast = match bcast_port {
            Some(bp) => {
                let (mut br, bw) = dial(&format!("{host}:{bp}")).await?.into_split();
                let ev = events.clone();
                tokio::spawn(async move {
                    let mut dec = KissDecoder::default();
                    let mut buf = vec![0u8; 8192];
                    loop {
                        match br.read(&mut buf).await {
                            Ok(0) | Err(_) => {
                                let _ = ev.send(TncEvent::Closed("broadcast"));
                                return;
                            }
                            Ok(n) => {
                                for (cmd, payload) in dec.feed(&buf[..n]) {
                                    if cmd <= 0x02 {
                                        let _ = ev.send(TncEvent::Broadcast(payload));
                                    }
                                }
                            }
                        }
                    }
                });
                Some(bw)
            }
            None => None,
        };
        Ok(Self {
            ctl,
            data,
            bcast,
            replies,
        })
    }

    /// Send a command and wait for OK. WRONG/BUSY become errors.
    pub async fn command(&mut self, cmd: &str) -> Result<()> {
        // One command per line: a CR or LF inside (say, in a callsign from
        // the air) would start a second command.
        if cmd.chars().any(|c| c.is_control()) {
            bail!("refused a Mercury command with control characters: {cmd:?}");
        }
        while self.replies.try_recv().is_ok() {} // drop stale replies
        self.ctl.write_all(format!("{cmd}\r").as_bytes()).await?;
        let r = tokio::time::timeout(Duration::from_secs(5), self.replies.recv())
            .await
            .map_err(|_| anyhow!("Mercury did not reply to {cmd:?}"))?
            .ok_or_else(|| anyhow!("Mercury connection closed"))?;
        if r == "BUSY" {
            return Err(Busy(cmd.to_string()).into());
        }
        if r != "OK" {
            bail!("Mercury: {cmd:?} -> {r}");
        }
        Ok(())
    }

    /// Fire-and-forget (queries whose answers arrive as status lines).
    pub async fn send_line(&mut self, cmd: &str) -> Result<()> {
        self.ctl.write_all(format!("{cmd}\r").as_bytes()).await?;
        Ok(())
    }

    pub async fn setup(&mut self, mycall: &str, arq_bw: u32) -> Result<()> {
        // Without a real callsign Mercury isn't given one (the engine refuses
        // to call or CQ until there is one).
        let mycall = mycall.trim().to_ascii_uppercase();
        if crate::protocol::valid_call(&mycall) {
            self.command(&format!("MYCALL {mycall}")).await?;
        }
        self.command(&format!("BW{arq_bw}")).await?;
        self.command("LISTEN ON").await
    }

    pub async fn send_data(&mut self, bytes: &[u8]) -> Result<()> {
        self.data.write_all(bytes).await?;
        Ok(())
    }

    pub async fn send_broadcast(&mut self, frame: &[u8]) -> Result<()> {
        let w = self
            .bcast
            .as_mut()
            .ok_or_else(|| anyhow!("broadcast port not open"))?;
        w.write_all(&kiss_encode(frame, 0x00)).await?;
        Ok(())
    }
}

/// `CONNECTED <initiator> <called> <bw>` -> (peer, bw), given our call.
pub fn parse_connected(line: &str, mycall: &str) -> Option<(String, u32)> {
    let p: Vec<&str> = line.split_whitespace().collect();
    if p.len() < 3 || p[0] != "CONNECTED" {
        return None;
    }
    let peer = if p[1].eq_ignore_ascii_case(mycall) {
        p[2]
    } else {
        p[1]
    };
    Some((
        peer.to_string(),
        p.get(3).and_then(|b| b.parse().ok()).unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_mercury::{FakeMercury, FakeOptions};
    use crate::protocol::Link;

    #[test]
    fn connected_parsing() {
        assert_eq!(
            parse_connected("CONNECTED N0CALL K7BOT 2300", "N0CALL"),
            Some(("K7BOT".into(), 2300))
        );
        assert_eq!(
            parse_connected("CONNECTED K7BOT N0CALL 500", "n0call"),
            Some(("K7BOT".into(), 500))
        );
    }

    #[tokio::test]
    async fn session_against_fake_mercury() {
        let fake = FakeMercury::start(FakeOptions {
            fast: true,
            ..Default::default()
        })
        .await
        .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut t = Tnc::connect("127.0.0.1", fake.port, Some(fake.bcast_port), tx)
            .await
            .unwrap();
        t.setup("N0CALL", 2300).await.unwrap();
        assert!(t.command("BOGUS").await.is_err());
        t.command("CONNECT N0CALL K7BOT").await.unwrap();
        let mut peer = None;
        while peer.is_none() {
            if let Some(TncEvent::Line(l)) = rx.recv().await {
                peer = parse_connected(&l, "N0CALL");
            }
        }
        assert_eq!(peer.unwrap().0, "K7BOT");
        t.send_data(
            &Link::Chat {
                id: 1,
                text: "hello".into(),
            }
            .encode(),
        )
        .await
        .unwrap();
        // The bot says HELLO and answers the chat.
        let mut dec = crate::protocol::LinkDecoder::default();
        let mut got_chat = false;
        while !got_chat {
            if let Some(TncEvent::Data(d)) = rx.recv().await {
                got_chat = dec
                    .feed(&d)
                    .unwrap()
                    .iter()
                    .any(|p| matches!(p, Link::Chat { .. }));
            }
        }
    }
}
