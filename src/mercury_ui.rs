//! Mercury's UI port: the WebSocket it serves when started with `-G`
//! (port 10000 unless `-U` says otherwise). Whistlestop uses it for one
//! thing Mercury's TNC port can't do: set the transmit level
//! (`set_tx_gain`, in dB, which Mercury applies at once and saves to its
//! mercury.ini). Mercury's status messages on the same socket report the
//! level back (`tx_gain_db`), which confirms it took.
//!
//! A connection per change: they're rare (a Settings change, or one step of
//! the automatic level after a transmission).

use anyhow::{Result, anyhow, bail};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The lowest and highest levels Mercury accepts (it clamps to these).
pub const MIN_DB: f32 = -20.0;
pub const MAX_DB: f32 = 20.0;

/// How long to wait for Mercury to answer at all, and for its status to
/// show the new level (it publishes status every half second or so).
const CONNECT: Duration = Duration::from_secs(2);
const CONFIRM: Duration = Duration::from_secs(4);

/// Set Mercury's transmit level to `db` and return the level it then
/// reports.
pub async fn set_tx_gain(host: &str, port: u16, db: f32) -> Result<f32> {
    let db = db.clamp(MIN_DB, MAX_DB);
    let mut s = tokio::time::timeout(CONNECT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| anyhow!("Mercury's UI port {host}:{port} didn't answer"))?
        .map_err(|e| anyhow!("can't reach Mercury's UI port {host}:{port} ({e}); start Mercury with -G"))?;
    let _ = s.set_nodelay(true);
    tokio::time::timeout(CONFIRM, async {
        handshake(&mut s, host, port).await?;
        let cmd = format!("{{\"command\":\"set_tx_gain\",\"value\":\"{db:.1}\"}}");
        s.write_all(&frame(1, cmd.as_bytes())).await?;
        loop {
            let (op, payload) = read_frame(&mut s).await?;
            match op {
                1 => {
                    if let Some(v) = status_gain(&String::from_utf8_lossy(&payload))
                        && (v - db).abs() < 0.06
                    {
                        let _ = s.write_all(&frame(8, &[])).await;
                        return Ok(v);
                    }
                }
                8 => bail!("Mercury closed its UI connection"),
                9 => s.write_all(&frame(10, &payload)).await?,
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| anyhow!("Mercury didn't confirm the new level"))?
}

async fn handshake(s: &mut TcpStream, host: &str, port: u16) -> Result<()> {
    // The key's value doesn't matter to a client that doesn't check the
    // answer to it (there's no proxy here to fool).
    let req = format!(
        "GET /websocket HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: d2hpc3RsZXN0b3AtdHhsdg==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 || s.read(&mut b).await? == 0 {
            bail!("Mercury's UI port didn't open a WebSocket");
        }
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head);
    if !head.starts_with("HTTP/1.1 101") {
        bail!(
            "Mercury's UI port refused the connection ({})",
            head.lines().next().unwrap_or("")
        );
    }
    Ok(())
}

/// One client frame (always masked, RFC 6455 5.3), final.
fn frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let mask = [0x57u8, 0x53, 0x74, 0x78];
    let mut f = vec![0x80 | op];
    match payload.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n <= 0xffff => {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(&mask);
    f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

/// One server frame: its opcode and payload. Big ones (the spectrum) are
/// read and dropped.
async fn read_frame(s: &mut TcpStream) -> Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).await?;
    let op = h[0] & 0x0f;
    let masked = h[1] & 0x80 != 0;
    let len = match h[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).await?;
            u16::from_be_bytes(b) as u64
        }
        127 => {
            let mut b = [0u8; 8];
            s.read_exact(&mut b).await?;
            u64::from_be_bytes(b)
        }
        n => n as u64,
    };
    let mut mask = [0u8; 4];
    if masked {
        s.read_exact(&mut mask).await?;
    }
    if len > 1 << 20 {
        let mut rest = len;
        let mut buf = vec![0u8; 64 * 1024];
        while rest > 0 {
            let n = rest.min(buf.len() as u64) as usize;
            s.read_exact(&mut buf[..n]).await?;
            rest -= n as u64;
        }
        return Ok((0, Vec::new()));
    }
    let mut p = vec![0u8; len as usize];
    s.read_exact(&mut p).await?;
    if masked {
        for (i, b) in p.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok((op, p))
}

/// The level in one of Mercury's status messages
/// (`{"type":"status",...,"tx_gain_db":-8.0,...}`).
fn status_gain(text: &str) -> Option<f32> {
    if !text.contains("\"type\":\"status\"") {
        return None;
    }
    let at = text.find("\"tx_gain_db\":")? + "\"tx_gain_db\":".len();
    let rest = &text[at..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.' || c == '+'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// A stand-in for Mercury's UI port (Demo mode's Mercury, and tests):
/// answers the WebSocket handshake, takes `set_tx_gain` (clamped like
/// Mercury's) and reports the level in a status message. Stops when
/// dropped.
pub struct FakeUi {
    pub port: u16,
    /// Every level it was set to, in order.
    pub set: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeUi {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn fake_ui() -> Result<FakeUi> {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = l.local_addr()?.port();
    let set = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let got = set.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut c, _)) = l.accept().await {
            let got = got.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if c.read(&mut b).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(b[0]);
                }
                let _ = c
                    .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: x\r\n\r\n")
                    .await;
                let mut db = 0.0f32;
                while let Ok((op, p)) = read_frame(&mut c).await {
                    if op == 8 {
                        return;
                    }
                    let t = String::from_utf8_lossy(&p).to_string();
                    if let Some(v) = t.split("\"value\":\"").nth(1).and_then(|r| r.split('"').next()) {
                        db = v.parse::<f32>().unwrap_or(0.0).clamp(MIN_DB, MAX_DB);
                        got.lock().unwrap().push(db);
                    }
                    let st = format!("{{\"type\":\"status\",\"tx_gain_db\":{db:.1}}}");
                    let mut f = vec![0x81u8, st.len() as u8];
                    f.extend_from_slice(st.as_bytes());
                    if c.write_all(&f).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    Ok(FakeUi { port, set, task })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sets_the_fake_level() {
        let ui = fake_ui().await.unwrap();
        assert_eq!(set_tx_gain("127.0.0.1", ui.port, -6.5).await.unwrap(), -6.5);
        assert_eq!(*ui.set.lock().unwrap(), [-6.5]);
        // Stopped when dropped.
        let port = ui.port;
        drop(ui);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(set_tx_gain("127.0.0.1", port, -6.5).await.is_err());
        // Nothing on the port: a clear error, naming -G.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = set_tx_gain("127.0.0.1", free, -6.5).await.unwrap_err().to_string();
        assert!(e.contains("-G"), "{e}");
    }

    #[test]
    fn reads_the_level_from_a_status_message() {
        let s = r#"{"type":"status","bitrate":0,"snr":0.0,"tx_gain_db":-8.0,"tx_peak_dbfs":-3.1}"#;
        assert_eq!(status_gain(s), Some(-8.0));
        assert_eq!(status_gain(r#"{"type":"status","tx_gain_db":2.5}"#), Some(2.5));
        assert_eq!(status_gain(r#"{"type":"spectrum","tx_gain_db":1.0}"#), None);
        assert_eq!(status_gain(r#"{"type":"status"}"#), None);
    }

    #[test]
    fn frames_are_masked() {
        let f = frame(1, b"hi");
        assert_eq!(&f[..2], &[0x81, 0x82]);
        assert_eq!(f.len(), 2 + 4 + 2);
        assert_eq!(f[6] ^ f[2], b'h');
        let f = frame(1, &[0u8; 300]);
        assert_eq!(&f[1..4], &[0x80 | 126, 1, 44]);
    }

    /// Against a real Mercury started with -G (WS_MERCURY_UI=host:port).
    /// Sets the level, then puts it back.
    #[tokio::test]
    #[ignore]
    async fn sets_a_real_mercurys_level() {
        let Ok(at) = std::env::var("WS_MERCURY_UI") else { return };
        let (host, port) = at.rsplit_once(':').unwrap();
        let port: u16 = port.parse().unwrap();
        assert_eq!(set_tx_gain(host, port, -7.5).await.unwrap(), -7.5);
        assert_eq!(set_tx_gain(host, port, -3.0).await.unwrap(), -3.0);
        assert_eq!(set_tx_gain(host, port, -40.0).await.unwrap(), MIN_DB);
    }
}
