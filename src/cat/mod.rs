//! Direct CAT: Whistlestop speaks the radio's own control protocol over its
//! serial port (or a TCP serial bridge), with no Hamlib or rigctld needed.
//!
//! * **Icom CI-V** (IC-7300, IC-705, IC-7610, IC-9700, IC-7100, IC-905 and
//!   the rest, plus Xiegu radios, which speak CI-V).
//! * **Kenwood** (TS-480, TS-590, TS-890, TS-990, TS-2000) and radios that
//!   speak Kenwood: QRP Labs QMX/QDX, truSDX, FlexRadio's CAT ports.
//! * **Elecraft** (K3, K4, KX2, KX3: Kenwood with Elecraft's extensions).
//! * **Yaesu** current CAT (FT-991/A, FT-891, FT-710, FTDX10, FTDX101,
//!   FT-950, FTDX3000, FTDX1200...).
//! * **Yaesu FT-817/818/857/897** (the older 5-byte binary protocol).
//!
//! A radio's serial port can only be opened by one program, so Whistlestop
//! also serves the rigctld protocol on a local port (see `rigserve`): point
//! Mercury (Hamlib model 2, NET rigctl) there and it keys the radio through
//! Whistlestop.
//!
//! The port is opened with RTS and DTR held low: on many radios those lines
//! key the transmitter or reset the CAT interface.
//!
//! Each driver is plain blocking code that owns its `Wire`; `CatHandle`
//! runs it on its own thread so the async engine never blocks on the port.

use crate::rig::{RigCaps, RigStatus};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub mod icom;
pub mod kenwood;
pub mod sim;
pub mod yaesu;

/// Which protocol the radio speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Family {
    /// Work it out: from the USB description, else by asking.
    #[default]
    Auto,
    Icom,
    Kenwood,
    Elecraft,
    Yaesu,
    Yaesu817,
}

impl Family {
    pub const ALL: [Family; 6] = [
        Family::Auto,
        Family::Icom,
        Family::Kenwood,
        Family::Elecraft,
        Family::Yaesu,
        Family::Yaesu817,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Family::Auto => "Automatic (detect)",
            Family::Icom => "Icom CI-V (and Xiegu)",
            Family::Kenwood => "Kenwood (and QRP Labs, truSDX, Flex)",
            Family::Elecraft => "Elecraft K3 / K4 / KX",
            Family::Yaesu => "Yaesu (FT-991, FT-891, FT-710, FTDX…)",
            Family::Yaesu817 => "Yaesu FT-817 / 818 / 857 / 897",
        }
    }

    /// Serial speeds to try, most likely first.
    pub fn bauds(&self) -> &'static [u32] {
        match self {
            Family::Icom => &[19200, 115200, 9600, 38400, 57600, 4800],
            Family::Kenwood => &[115200, 57600, 38400, 9600, 19200, 4800],
            Family::Elecraft => &[38400, 4800, 9600, 19200, 57600, 115200],
            Family::Yaesu => &[38400, 4800, 9600, 19200, 115200],
            Family::Yaesu817 => &[4800, 9600, 38400],
            Family::Auto => &[19200, 38400, 115200, 9600, 4800, 57600],
        }
    }

    /// The family a detected radio's description suggests.
    pub fn from_description(what: &str) -> Family {
        let w = what.to_uppercase();
        if w.starts_with("ICOM") || w.contains("XIEGU") {
            Family::Icom
        } else if w.contains("FT-817")
            || w.contains("FT-818")
            || w.contains("FT-857")
            || w.contains("FT-897")
        {
            Family::Yaesu817
        } else if w.starts_with("YAESU") {
            Family::Yaesu
        } else if w.starts_with("ELECRAFT") {
            Family::Elecraft
        } else if w.starts_with("KENWOOD") || w.contains("QRP LABS") || w.contains("TRUSDX") {
            Family::Kenwood
        } else {
            Family::Auto
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatConfig {
    pub family: Family,
    /// Serial port (`/dev/cu.…`, `COM3`, `/dev/ttyUSB0`) or `host:port` for
    /// a TCP serial bridge; empty = the first radio detected on USB.
    pub port: String,
    /// 0 = try the usual speeds for the family.
    pub baud: u32,
    /// Icom CI-V address; 0 = ask the radio.
    pub civ_addr: u8,
    /// The radio's full power, so the power slider reads in watts.
    pub max_power_w: f32,
    /// Serve rigctld here so Mercury can key the radio (0 = off).
    pub share_port: u16,
}

impl Default for CatConfig {
    fn default() -> Self {
        Self {
            family: Family::Auto,
            port: String::new(),
            baud: 0,
            civ_addr: 0,
            max_power_w: 100.0,
            share_port: 4532,
        }
    }
}

// ------------------------------------------------------------------- wires

/// A byte pipe to the radio.
pub trait Wire: Send {
    fn write_all(&mut self, b: &[u8]) -> std::io::Result<()>;
    /// Read what's there, waiting at most a short while; Ok(0) if nothing.
    fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    /// Throw away anything unread (stale replies, transceive chatter).
    fn clear_input(&mut self);
}

struct SerialWire(Box<dyn serialport::SerialPort>);

impl Wire for SerialWire {
    fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        self.0.write_all(b)?;
        self.0.flush()
    }
    fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::Read;
        match self.0.read(buf) {
            Ok(n) => Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }
    fn clear_input(&mut self) {
        let _ = self.0.clear(serialport::ClearBuffer::Input);
    }
}

struct TcpWire(std::net::TcpStream);

impl Wire for TcpWire {
    fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        self.0.write_all(b)
    }
    fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::Read;
        match self.0.read(buf) {
            Ok(0) => Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "radio bridge closed the connection",
            )),
            Ok(n) => Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }
    fn clear_input(&mut self) {
        use std::io::Read;
        let _ = self.0.set_nonblocking(true);
        let mut b = [0u8; 256];
        while matches!(self.0.read(&mut b), Ok(n) if n > 0) {}
        let _ = self.0.set_nonblocking(false);
    }
}

/// `host:port` (TCP bridge) rather than a serial device?
pub fn is_tcp(port: &str) -> bool {
    let p = port.strip_prefix("tcp:").unwrap_or(port);
    !port.starts_with('/')
        && !port.to_uppercase().starts_with("COM")
        && p.rsplit_once(':')
            .is_some_and(|(_, n)| n.parse::<u16>().is_ok())
}

pub fn open_wire(port: &str, baud: u32) -> Result<Box<dyn Wire>> {
    if is_tcp(port) {
        let addr = port.strip_prefix("tcp:").unwrap_or(port);
        let s = std::net::TcpStream::connect(addr)
            .map_err(|e| anyhow!("can't reach the radio at {addr}: {e}"))?;
        s.set_read_timeout(Some(Duration::from_millis(40)))?;
        s.set_nodelay(true)?;
        return Ok(Box::new(TcpWire(s)));
    }
    let mut p = serialport::new(port, baud)
        .timeout(Duration::from_millis(40))
        .dtr_on_open(false)
        .open()
        .map_err(|e| anyhow!("can't open {port}: {e}"))?;
    // Hold RTS and DTR low: on many radios they key the transmitter.
    let _ = p.write_request_to_send(false);
    let _ = p.write_data_terminal_ready(false);
    Ok(Box::new(SerialWire(p)))
}

// ------------------------------------------------------- ASCII protocols

/// Send an ASCII query (`FA;`) and return the reply that starts with
/// `prefix`, without the `;`. Other replies (transceive updates, an old
/// `?;`) are skipped; a `?;` right after our query means "not supported".
pub(crate) fn ascii_query(w: &mut dyn Wire, cmd: &str, prefix: &str) -> Result<String> {
    w.clear_input();
    w.write_all(cmd.as_bytes())?;
    let deadline = Instant::now() + Duration::from_millis(700);
    let mut acc: Vec<u8> = vec![];
    let mut b = [0u8; 128];
    while Instant::now() < deadline {
        let n = w.read_some(&mut b)?;
        acc.extend_from_slice(&b[..n]);
        while let Some(i) = acc.iter().position(|&c| c == b';') {
            let reply: Vec<u8> = acc.drain(..=i).collect();
            let text = String::from_utf8_lossy(&reply[..reply.len() - 1])
                .trim()
                .to_string();
            if text.starts_with(prefix) {
                return Ok(text);
            }
            if text == "?" || text == "E" || text == "N" {
                bail!("the radio doesn't support {}", cmd.trim_end_matches(';'));
            }
        }
    }
    bail!("no answer from the radio to {}", cmd.trim_end_matches(';'))
}

/// Send ASCII commands that have no reply.
pub(crate) fn ascii_set(w: &mut dyn Wire, cmd: &str) -> Result<()> {
    w.write_all(cmd.as_bytes())?;
    // Give slow CAT interfaces a moment between commands.
    std::thread::sleep(Duration::from_millis(15));
    Ok(())
}

/// Digits of a reply after `skip` characters, as a number.
pub(crate) fn num(reply: &str, skip: usize, len: Option<usize>) -> Result<i64> {
    let s = reply.get(skip..).unwrap_or("");
    let s = match len {
        Some(l) => s.get(..l).unwrap_or(s),
        None => s,
    };
    s.trim()
        .trim_start_matches('+')
        .parse::<i64>()
        .map_err(|_| anyhow!("odd reply from the radio: {reply}"))
}

/// Piecewise-linear calibration (raw meter value → real units).
pub(crate) fn cal(table: &[(f32, f32)], v: f32) -> f32 {
    if v <= table[0].0 {
        return table[0].1;
    }
    for w in table.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if v <= x1 {
            return y0 + (y1 - y0) * (v - x0) / (x1 - x0);
        }
    }
    table[table.len() - 1].1
}

// ------------------------------------------------------------------ driver

pub(crate) fn unsupported(what: &str) -> anyhow::Error {
    anyhow!("{what} isn't available over direct CAT for this radio")
}

/// One radio. Levels, functions and VFO operations use Hamlib's names and
/// units (levels 0..1, STRENGTH in dB relative to S9, SWR as a ratio,
/// PREAMP/ATT in dB), so the rest of Whistlestop doesn't care how the radio
/// is reached.
pub trait Driver: Send {
    fn describe(&self) -> String;
    fn caps(&self) -> RigCaps;
    fn freq(&mut self) -> Result<f64>;
    fn set_freq(&mut self, hz: f64) -> Result<()>;
    /// Hamlib mode name and passband (0 if the radio doesn't say).
    fn mode(&mut self) -> Result<(String, i64)>;
    fn set_mode(&mut self, mode: &str, passband_hz: i64) -> Result<()>;
    fn ptt(&mut self) -> Result<bool>;
    fn set_ptt(&mut self, on: bool) -> Result<()>;
    fn get_level(&mut self, name: &str) -> Result<f32> {
        Err(unsupported(name))
    }
    fn set_level(&mut self, name: &str, _v: f32) -> Result<()> {
        Err(unsupported(name))
    }
    fn get_func(&mut self, name: &str) -> Result<bool> {
        Err(unsupported(name))
    }
    fn set_func(&mut self, name: &str, _on: bool) -> Result<()> {
        Err(unsupported(name))
    }
    fn vfo_op(&mut self, op: &str) -> Result<()> {
        Err(unsupported(op))
    }
    fn vfo(&mut self) -> Result<String> {
        Ok("VFOA".into())
    }
    fn set_vfo(&mut self, _vfo: &str) -> Result<()> {
        Err(unsupported("Choosing the VFO"))
    }
    fn split(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn set_split(&mut self, _on: bool) -> Result<()> {
        Err(unsupported("Split"))
    }
    fn offset(&mut self, _xit: bool) -> Result<i32> {
        Err(unsupported("RIT/XIT"))
    }
    fn set_offset(&mut self, _xit: bool, _hz: i32) -> Result<()> {
        Err(unsupported("RIT/XIT"))
    }
}

/// What the GUI polls a few times a second.
pub fn poll(d: &mut dyn Driver) -> Result<RigStatus> {
    let freq_hz = d.freq()?;
    let (mode, passband_hz) = d.mode().unwrap_or_default();
    let ptt = d.ptt().unwrap_or(false);
    let caps = d.caps();
    let strength_db = if caps.can_get_level("STRENGTH") {
        d.get_level("STRENGTH").ok().map(|v| v.round() as i32)
    } else {
        None
    };
    let rf_power = if caps.can_get_level("RFPOWER") {
        d.get_level("RFPOWER").ok()
    } else {
        None
    };
    Ok(RigStatus {
        freq_hz,
        mode,
        passband_hz,
        ptt,
        strength_db,
        rf_power,
    })
}

// ------------------------------------------------------------------ handle

type Job = Box<dyn FnOnce(&mut dyn Driver) + Send>;

/// A radio running on its own thread; cheap to clone and share (the engine
/// and the rigctld server both hold one).
#[derive(Clone)]
pub struct CatHandle {
    tx: std::sync::mpsc::Sender<Job>,
    pub desc: String,
    pub caps: RigCaps,
}

impl CatHandle {
    pub fn start(d: Box<dyn Driver>) -> CatHandle {
        let desc = d.describe();
        let caps = d.caps();
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("cat".into())
            .spawn(move || {
                let mut d = d;
                while let Ok(job) = rx.recv() {
                    job(d.as_mut());
                }
            })
            .expect("CAT thread");
        CatHandle { tx, desc, caps }
    }

    /// Run `f` on the radio's thread and wait for its answer.
    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn Driver) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (otx, orx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |d| {
                let _ = otx.send(f(d));
            }))
            .map_err(|_| anyhow!("the radio connection has closed"))?;
        orx.await
            .map_err(|_| anyhow!("the radio connection has closed"))?
    }
}

// ---------------------------------------------------------------- connect

/// Open the radio described by `cfg`: find the port, then the protocol and
/// speed (trying each until the radio answers). Blocking; run it off the
/// async threads.
pub fn connect(cfg: &CatConfig) -> Result<Box<dyn Driver>> {
    let (port, mut family) = if cfg.port.trim().is_empty() {
        let found = crate::rigsetup::detect();
        let g = found.first().ok_or_else(|| anyhow!("no radio found on USB: plug it in, or choose the port in Settings → Radio connection"))?;
        (
            g.port.clone(),
            if cfg.family == Family::Auto {
                Family::from_description(&g.what)
            } else {
                cfg.family
            },
        )
    } else {
        (cfg.port.trim().to_string(), cfg.family)
    };
    if family == Family::Auto && cfg.civ_addr != 0 {
        family = Family::Icom;
    }
    let bauds: Vec<u32> = if cfg.baud > 0 || is_tcp(&port) {
        vec![cfg.baud.max(1)]
    } else {
        family.bauds().to_vec()
    };
    let families: Vec<Family> = if family == Family::Auto {
        vec![Family::Icom, Family::Kenwood, Family::Yaesu817]
    } else {
        vec![family]
    };
    let mut last = anyhow!("the radio on {port} didn't answer");
    for &baud in &bauds {
        for &f in &families {
            // A previous connection may still be letting go of the port.
            let mut w = open_wire(&port, baud);
            for _ in 0..10 {
                if w.is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
                w = open_wire(&port, baud);
            }
            let w = w?;
            let tried = match f {
                Family::Icom => {
                    icom::Icom::probe(w, cfg.civ_addr).map(|d| Box::new(d) as Box<dyn Driver>)
                }
                // "ID;" tells Kenwood, Elecraft and Yaesu apart.
                Family::Kenwood | Family::Elecraft | Family::Yaesu => {
                    probe_ascii(w, f, cfg.max_power_w)
                }
                Family::Yaesu817 => yaesu::Ft817::probe(w).map(|d| Box::new(d) as Box<dyn Driver>),
                Family::Auto => unreachable!(),
            };
            match tried {
                Ok(d) => {
                    let at = if is_tcp(&port) {
                        port.clone()
                    } else {
                        format!("{port} at {baud} baud")
                    };
                    return Ok(Box::new(Described { inner: d, at }));
                }
                Err(e) => last = e,
            }
        }
    }
    Err(last.context(format!(
        "no radio answered on {port} (tried {} at {:?} baud)",
        families
            .iter()
            .map(|f| f.label())
            .collect::<Vec<_>>()
            .join(", "),
        bauds
    )))
}

/// Ask `ID;` and build the matching ASCII driver.
fn probe_ascii(mut w: Box<dyn Wire>, hint: Family, max_w: f32) -> Result<Box<dyn Driver>> {
    let id = ascii_query(w.as_mut(), "ID;", "ID")?;
    let digits = id[2..].trim().to_string();
    if digits.len() >= 4 {
        Ok(Box::new(yaesu::Yaesu::new(w, &digits, max_w)?))
    } else {
        Ok(Box::new(kenwood::Kenwood::new(
            w,
            &digits,
            hint == Family::Elecraft,
            max_w,
        )?))
    }
}

/// A driver plus where it is ("IC-7300 (CI-V 94h) on /dev/cu.… at 19200").
struct Described {
    inner: Box<dyn Driver>,
    at: String,
}

impl Driver for Described {
    fn describe(&self) -> String {
        format!("{} on {}", self.inner.describe(), self.at)
    }
    fn caps(&self) -> RigCaps {
        self.inner.caps()
    }
    fn freq(&mut self) -> Result<f64> {
        self.inner.freq()
    }
    fn set_freq(&mut self, hz: f64) -> Result<()> {
        self.inner.set_freq(hz)
    }
    fn mode(&mut self) -> Result<(String, i64)> {
        self.inner.mode()
    }
    fn set_mode(&mut self, m: &str, pb: i64) -> Result<()> {
        self.inner.set_mode(m, pb)
    }
    fn ptt(&mut self) -> Result<bool> {
        self.inner.ptt()
    }
    fn set_ptt(&mut self, on: bool) -> Result<()> {
        self.inner.set_ptt(on)
    }
    fn get_level(&mut self, n: &str) -> Result<f32> {
        self.inner.get_level(n)
    }
    fn set_level(&mut self, n: &str, v: f32) -> Result<()> {
        self.inner.set_level(n, v)
    }
    fn get_func(&mut self, n: &str) -> Result<bool> {
        self.inner.get_func(n)
    }
    fn set_func(&mut self, n: &str, on: bool) -> Result<()> {
        self.inner.set_func(n, on)
    }
    fn vfo_op(&mut self, op: &str) -> Result<()> {
        self.inner.vfo_op(op)
    }
    fn vfo(&mut self) -> Result<String> {
        self.inner.vfo()
    }
    fn set_vfo(&mut self, v: &str) -> Result<()> {
        self.inner.set_vfo(v)
    }
    fn split(&mut self) -> Result<bool> {
        self.inner.split()
    }
    fn set_split(&mut self, on: bool) -> Result<()> {
        self.inner.set_split(on)
    }
    fn offset(&mut self, xit: bool) -> Result<i32> {
        self.inner.offset(xit)
    }
    fn set_offset(&mut self, xit: bool, hz: i32) -> Result<()> {
        self.inner.set_offset(xit, hz)
    }
}

pub(crate) fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_or_serial() {
        assert!(is_tcp("127.0.0.1:4992"));
        assert!(is_tcp("tcp:radio.local:5000"));
        assert!(!is_tcp("/dev/cu.SLAB_USBtoUART"));
        assert!(!is_tcp("COM3"));
        assert!(!is_tcp("/dev/ttyUSB0"));
    }

    #[test]
    fn families_from_usb_names() {
        assert_eq!(Family::from_description("Icom IC-7300"), Family::Icom);
        assert_eq!(Family::from_description("Yaesu FT-991A"), Family::Yaesu);
        assert_eq!(Family::from_description("Yaesu FT-817"), Family::Yaesu817);
        assert_eq!(
            Family::from_description("QRP Labs QMX/QDX"),
            Family::Kenwood
        );
        assert_eq!(
            Family::from_description("USB serial (choose the model)"),
            Family::Auto
        );
    }

    #[test]
    fn calibration_tables() {
        let t = [(0.0, 1.0), (48.0, 1.5), (80.0, 2.0)];
        assert_eq!(cal(&t, 0.0), 1.0);
        assert_eq!(cal(&t, 64.0), 1.75);
        assert_eq!(cal(&t, 200.0), 2.0);
    }
}
