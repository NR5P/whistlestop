//! Simulated radios that speak each CAT protocol, for tests: a `Wire` that
//! answers like the real thing (including Icom's USB echo and the `?;` of a
//! Kenwood that doesn't know a command).

use super::Wire;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Icom(u8),
    /// Kenwood-protocol radio with this ID (e.g. "023" TS-590SG, "020" QMX).
    Kenwood(&'static str),
    /// A QRP Labs QMX, answering as a real one (firmware 1_02_005) did on
    /// the bench: ID020, two VFOs with the dial on whichever FR names,
    /// `SM;` but not `SM0;`, SWR and power only while transmitting, and
    /// `?;` to everything it doesn't have. `ssb_tx` false is that
    /// firmware's fault: TX; is ignored in LSB and USB.
    Qmx { ssb_tx: bool },
    Elecraft,
    /// Yaesu with this ID (e.g. "0670" FT-991A).
    Yaesu(&'static str),
    Ft817,
}

pub struct SimRadio {
    pub kind: Kind,
    pub freq: u64,
    /// Kenwood/Yaesu mode character, or Icom mode byte.
    pub mode: u8,
    pub data: bool,
    pub ptt: bool,
    pub vals: HashMap<String, String>,
    out: VecDeque<u8>,
    inb: Vec<u8>,
    /// Every command received (for tests).
    pub log: Vec<String>,
}

impl SimRadio {
    pub fn new(kind: Kind) -> SimRadio {
        SimRadio {
            kind,
            freq: 14_074_000,
            mode: match kind {
                Kind::Icom(_) => 0x01,
                Kind::Ft817 => 0x01,
                _ => b'2',
            },
            data: false,
            ptt: false,
            vals: HashMap::new(),
            out: VecDeque::new(),
            inb: vec![],
            log: vec![],
        }
    }

    fn reply(&mut self, s: &str) {
        self.out.extend(s.as_bytes());
    }

    fn qmx(&mut self, cmd: &str, ssb_tx: bool) {
        let (head, arg) = cmd.split_at(cmd.len().min(2));
        let on_b = self.vals.get("FR").is_some_and(|v| v == "1");
        let fb: u64 = self.vals.get("FB").and_then(|v| v.parse().ok()).unwrap_or(7_030_000);
        let active = if on_b { fb } else { self.freq };
        match (head, arg) {
            ("ID", "") => self.reply("ID020;"),
            ("VN", "") => self.reply("VN1_02_005QMX;"),
            ("FA", "") => {
                let f = self.freq;
                self.reply(&format!("FA{f:011};"));
            }
            ("FB", "") => self.reply(&format!("FB{fb:011};")),
            ("FA", a) => self.freq = a.parse().unwrap_or(self.freq),
            ("FB", a) => {
                self.vals.insert("FB".into(), a.trim_start_matches('0').to_string());
            }
            ("FR" | "FT", "") => self.reply(&format!("{head}{};", on_b as u8)),
            ("FR" | "FT", a) => {
                self.vals.insert("FR".into(), a.to_string());
            }
            ("MD", "") => {
                let m = self.mode as char;
                self.reply(&format!("MD{m};"));
            }
            ("MD", a) => self.mode = *a.as_bytes().last().unwrap(),
            ("TX", _) | ("TQ", "1") => self.ptt = ssb_tx || !matches!(self.mode, b'1' | b'2'),
            ("RX", "") | ("TQ", "0") => self.ptt = false,
            ("TQ", "") => {
                let p = self.ptt as u8;
                self.reply(&format!("TQ{p};"));
            }
            ("IF", "") => {
                let s = format!(
                    "IF{active:011}     +000000000{}{}{}00000 ;",
                    self.ptt as u8, self.mode as char, on_b as u8
                );
                self.reply(&s);
            }
            ("SM", "") => self.reply("SM013;"),
            ("SW", "") => self.reply(if self.ptt { "SW163;" } else { "SW;" }),
            ("PC", "") => self.reply(if self.ptt { "PC45;" } else { "PC00;" }),
            ("AG", "" | "0") => self.reply("AG0054;"),
            _ => self.reply("?;"),
        }
    }

    fn ascii(&mut self, cmd: &str) {
        self.log.push(cmd.to_string());
        if let Kind::Qmx { ssb_tx } = self.kind {
            return self.qmx(cmd, ssb_tx);
        }
        let yaesu = matches!(self.kind, Kind::Yaesu(_));
        let (head, arg) = cmd.split_at(cmd.len().min(2));
        let get = arg.is_empty()
            || (arg.len() == 1
                && (matches!(head, "AG" | "SQ" | "SM")
                    || (yaesu && matches!(head, "MD" | "RG" | "NB" | "NR" | "BC" | "PA" | "RA"))));
        match (head, get) {
            ("ID", true) => {
                let id = match self.kind {
                    Kind::Kenwood(id) | Kind::Yaesu(id) => id,
                    Kind::Elecraft => "017",
                    _ => "",
                };
                self.reply(&format!("ID{id};"));
            }
            ("OM", true) if self.kind == Kind::Elecraft => self.reply("OM AP----------;"),
            ("OM", true) | ("K4", true) => self.reply("?;"),
            ("FA", true) => {
                let w = if yaesu { 9 } else { 11 };
                let f = self.freq;
                self.reply(&format!("FA{f:0w$};"));
            }
            ("FA", false) => self.freq = arg.parse().unwrap_or(self.freq),
            ("MD", true) => {
                let m = self.mode as char;
                self.reply(&if yaesu {
                    format!("MD0{m};")
                } else {
                    format!("MD{m};")
                });
            }
            ("MD", false) => self.mode = *arg.as_bytes().last().unwrap(),
            ("DA", true) if self.kind == Kind::Kenwood("023") => {
                let d = self.data as u8;
                self.reply(&format!("DA{d};"));
            }
            ("DA", false) if self.kind == Kind::Kenwood("023") => self.data = arg == "1",
            ("TX", true) if yaesu => {
                let p = self.ptt as u8;
                self.reply(&format!("TX{p};"));
            }
            ("TX", _) => self.ptt = !yaesu || arg != "0",
            ("RX", true) => self.ptt = false,
            ("IF", true) => {
                // Kenwood layout: freq 11, 5 spaces, offset ±nnnn, RIT, XIT,
                // memory 3, TX, mode, VFO, scan, split, tone, tone#, P15.
                let s = format!(
                    "IF{:011}     +000000000{}{}0000000;",
                    self.freq, self.ptt as u8, self.mode as char
                );
                self.reply(&s);
            }
            ("SM", true) => self.reply(if yaesu { "SM0130;" } else { "SM00015;" }),
            ("SW", true) if self.kind == Kind::Elecraft => self.reply("SW015;"),
            ("BW", true) if self.kind == Kind::Elecraft => self.reply("BW0280;"),
            ("RM", false) if yaesu => {
                let v = if arg == "6" { "089" } else { "000" };
                self.reply(&format!("RM{arg}{v};"));
            }
            ("PC", true) => {
                let v = self.vals.get("PC").cloned().unwrap_or("050".into());
                self.reply(&format!("PC{v};"));
            }
            ("PC", false) => {
                self.vals.insert("PC".into(), arg.to_string());
            }
            (_, true) if yaesu && self.vals.contains_key(cmd) => {
                let v = self.vals[cmd].clone();
                self.reply(&format!("{cmd}{v};"));
            }
            (_, false) if yaesu && arg.len() > 1 => {
                // e.g. AG0128 → remember "AG0" = "128"
                self.vals
                    .insert(format!("{head}{}", &arg[..1]), arg[1..].to_string());
            }
            _ => {
                if get {
                    if let Some(v) = self.vals.get(head).cloned() {
                        self.reply(&format!("{head}{v};"));
                    } else if self.kind == Kind::Kenwood("020") {
                        self.reply("?;");
                    } else {
                        self.reply(&format!("{head}0;"));
                    }
                } else {
                    self.vals.insert(head.to_string(), arg.to_string());
                }
            }
        }
    }

    fn civ(&mut self, f: &[u8]) {
        // f = [to, from, body...]
        let Kind::Icom(addr) = self.kind else { return };
        if f.len() < 3 || (f[0] != addr && f[0] != 0x00) {
            return;
        }
        let body = &f[2..];
        self.log.push(format!("{body:02X?}"));
        let send = |out: &mut VecDeque<u8>, data: &[u8]| {
            out.extend([0xFE, 0xFE, 0xE0, addr]);
            out.extend(data);
            out.push_back(0xFD);
        };
        // USB echo of our own frame first, as real radios do.
        self.out.extend([0xFE, 0xFE]);
        self.out.extend(f);
        self.out.push_back(0xFD);
        let ok = [0xFB];
        match body {
            [0x19, 0x00] => send(&mut self.out, &[0x19, 0x00, addr]),
            [0x03] => {
                let mut d = vec![0x03];
                d.extend(super::icom::freq_bcd(self.freq, 5));
                send(&mut self.out, &d);
            }
            [0x05, rest @ ..] => {
                self.freq = super::icom::bcd_freq(rest);
                send(&mut self.out, &ok);
            }
            [0x04] => send(&mut self.out, &[0x04, self.mode, 0x01]),
            [0x06, m, ..] => {
                self.mode = *m;
                send(&mut self.out, &ok);
            }
            [0x1A, 0x06] => send(&mut self.out, &[0x1A, 0x06, self.data as u8, 0x01]),
            [0x1A, 0x06, d, ..] => {
                self.data = *d != 0;
                send(&mut self.out, &ok);
            }
            [0x1C, 0x00] => send(&mut self.out, &[0x1C, 0x00, self.ptt as u8]),
            [0x1C, 0x00, p] => {
                self.ptt = *p != 0;
                send(&mut self.out, &ok);
            }
            [0x15, 0x02] => send(&mut self.out, &[0x15, 0x02, 0x01, 0x20]), // 120 = S9
            [0x15, 0x12] => send(&mut self.out, &[0x15, 0x12, 0x00, 0x80]), // 80 = 2.0
            [0x14, c] => {
                let v = self
                    .vals
                    .get(&format!("14{c:02X}"))
                    .cloned()
                    .unwrap_or("0128".into());
                let b = super::icom::bcd2(v.parse().unwrap_or(128));
                send(&mut self.out, &[0x14, *c, b[0], b[1]]);
            }
            [0x14, c, a, b] => {
                self.vals.insert(
                    format!("14{c:02X}"),
                    super::icom::from_bcd2(&[*a, *b]).to_string(),
                );
                send(&mut self.out, &ok);
            }
            _ => send(&mut self.out, &[0xFA]),
        }
    }

    fn ft817(&mut self, b: &[u8]) {
        self.log.push(format!("{b:02X?}"));
        match b[4] {
            0x03 => {
                let f = self.freq / 10;
                let mut d = vec![];
                let mut x = f;
                let mut bytes = [0u8; 4];
                for i in (0..4).rev() {
                    let lo = (x % 10) as u8;
                    x /= 10;
                    let hi = (x % 10) as u8;
                    x /= 10;
                    bytes[i] = (hi << 4) | lo;
                }
                d.extend(bytes);
                d.push(self.mode);
                self.out.extend(d);
            }
            0x01 => {
                self.freq = b[..4].iter().fold(0u64, |a, &x| {
                    a * 100 + (x >> 4) as u64 * 10 + (x & 15) as u64
                }) * 10;
            }
            0x07 => self.mode = b[0],
            0x08 => {
                self.ptt = true;
                self.out.push_back(0);
            }
            0x88 => {
                self.ptt = false;
                self.out.push_back(0);
            }
            0xE7 => self.out.push_back(0x09),
            // TX status: bit 7 low while transmitting.
            0xF7 => self.out.push_back(if self.ptt { 0x00 } else { 0x80 }),
            _ => {}
        }
    }
}

impl Wire for SimRadio {
    fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
        self.inb.extend_from_slice(b);
        loop {
            match self.kind {
                Kind::Icom(_) => {
                    let Some((f, rest)) = super::icom::take_frame(&self.inb) else {
                        break;
                    };
                    self.inb = rest;
                    self.civ(&f);
                }
                Kind::Ft817 => {
                    if self.inb.len() < 5 {
                        break;
                    }
                    let c: Vec<u8> = self.inb.drain(..5).collect();
                    self.ft817(&c);
                }
                _ => {
                    let Some(i) = self.inb.iter().position(|&c| c == b';') else {
                        break;
                    };
                    let c: Vec<u8> = self.inb.drain(..=i).collect();
                    let s = String::from_utf8_lossy(&c[..c.len() - 1]).to_string();
                    self.ascii(&s);
                }
            }
        }
        Ok(())
    }

    fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.out.len());
        for b in buf.iter_mut().take(n) {
            *b = self.out.pop_front().unwrap();
        }
        Ok(n)
    }

    fn clear_input(&mut self) {
        // Only stale bytes are cleared; a real port can't see the future.
        self.out.clear();
    }
}

/// Serve a simulated radio on a local TCP port (like a serial-over-network
/// bridge), for whole-program tests. Returns the port.
pub fn serve_tcp(kind: Kind) -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut radio = SimRadio::new(kind);
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                s.set_read_timeout(Some(std::time::Duration::from_millis(5)))
                    .ok();
                let mut b = [0u8; 256];
                loop {
                    match s.read(&mut b) {
                        Ok(0) => break,
                        Ok(n) => {
                            radio.write_all(&b[..n]).ok();
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                    let mut out = [0u8; 256];
                    let n = radio.read_some(&mut out).unwrap_or(0);
                    if n > 0 && s.write_all(&out[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

#[cfg(test)]
mod tests {
    use super::super::{Driver, icom::Icom, kenwood::Kenwood, poll, yaesu::Ft817, yaesu::Yaesu};
    use super::*;

    fn exercise(d: &mut dyn Driver) {
        assert_eq!(d.freq().unwrap(), 14_074_000.0, "{}", d.describe());
        d.set_freq(7_074_500.0).unwrap();
        assert_eq!(d.freq().unwrap(), 7_074_500.0, "{}", d.describe());
        d.set_mode("PKTUSB", 3000).unwrap();
        let (m, _) = d.mode().unwrap();
        assert!(
            m == "PKTUSB" || d.describe().contains("QMX"),
            "{}: {m}",
            d.describe()
        );
        d.set_ptt(true).unwrap();
        assert!(d.ptt().unwrap(), "{}", d.describe());
        d.set_ptt(false).unwrap();
        assert!(!d.ptt().unwrap(), "{}", d.describe());
        let st = poll(d).unwrap();
        assert_eq!(st.freq_hz, 7_074_500.0);
    }

    #[test]
    fn icom_civ() {
        let mut d = Icom::probe(Box::new(SimRadio::new(Kind::Icom(0x94))), 0).unwrap();
        assert!(d.describe().contains("IC-7300"), "{}", d.describe());
        exercise(&mut d);
        assert!((d.get_level("STRENGTH").unwrap() - 0.0).abs() < 0.5);
        assert!((d.get_level("SWR").unwrap() - 2.0).abs() < 0.01);
        d.set_level("RFPOWER", 0.25).unwrap();
        assert!((d.get_level("RFPOWER").unwrap() - 0.25).abs() < 0.01);
    }

    #[test]
    fn kenwood_ts590() {
        let mut d = Kenwood::new(
            Box::new(SimRadio::new(Kind::Kenwood("023"))),
            "023",
            false,
            100.0,
        )
        .unwrap();
        assert!(d.describe().contains("TS-590SG"));
        exercise(&mut d);
        assert_eq!(d.get_level("STRENGTH").unwrap(), 0.0);
        d.set_level("RFPOWER", 0.3).unwrap();
        assert!((d.get_level("RFPOWER").unwrap() - 0.3).abs() < 0.01);
    }

    #[test]
    fn plain_kenwood_qmx() {
        // A QMX says ID020 and answers ?; to commands it doesn't have.
        let mut d = Kenwood::new(
            Box::new(SimRadio::new(Kind::Kenwood("020"))),
            "020",
            false,
            5.0,
        )
        .unwrap();
        exercise(&mut d);
        assert!(d.get_func("NB").is_err());
    }

    #[test]
    fn qmx_as_on_the_bench() {
        let mut d = Kenwood::new(Box::new(SimRadio::new(Kind::Qmx { ssb_tx: true })), "020", false, 5.0).unwrap();
        assert_eq!(d.describe(), "QRP Labs QMX (firmware 1_02_005) (direct CAT)");
        exercise(&mut d);
        // Its own meters; no RF gain, squelch or power setting to offer.
        let caps = d.caps();
        assert!(caps.can_get_level("STRENGTH") && caps.can_get_level("SWR") && caps.can_get_level("RFPOWER_METER"));
        assert!(!caps.can_get_level("RF") && !caps.can_get_level("RFPOWER"));
        // SM; is dB: 13 dB above S0 is a little over S2.
        assert!((d.get_level("STRENGTH").unwrap() + 41.0).abs() < 0.01);
        assert!(d.get_level("SWR").is_err(), "no SWR on receive");
        d.set_ptt(true).unwrap();
        assert!((d.get_level("SWR").unwrap() - 1.63).abs() < 0.001);
        assert!((d.get_level("RFPOWER_METER").unwrap() - 0.9).abs() < 0.001, "4.5 W of 5");
        d.set_ptt(false).unwrap();
        assert!(d.set_level("RFPOWER", 0.5).is_err());
    }

    #[test]
    fn qmx_on_vfo_b_is_tuned_on_b() {
        // The dial follows VFO B: reading or setting A would move nothing.
        let mut sim = SimRadio::new(Kind::Qmx { ssb_tx: true });
        sim.vals.insert("FR".into(), "1".into());
        sim.vals.insert("FB".into(), "7078000".into());
        let mut d = Kenwood::new(Box::new(sim), "020", false, 5.0).unwrap();
        assert_eq!(d.freq().unwrap(), 7_078_000.0);
        d.set_freq(7_103_000.0).unwrap();
        assert_eq!(d.freq().unwrap(), 7_103_000.0);
        assert_eq!(d.vfo().unwrap(), "VFOB");
        assert_eq!(poll(&mut d).unwrap().freq_hz, 7_103_000.0);
    }

    #[test]
    fn qmx_that_ignores_tx_in_ssb_says_so() {
        let mut d = Kenwood::new(Box::new(SimRadio::new(Kind::Qmx { ssb_tx: false })), "020", false, 5.0).unwrap();
        d.set_mode("USB", 0).unwrap();
        let e = d.set_ptt(true).unwrap_err().to_string();
        assert!(e.contains("SSB VOX") && e.contains("firmware"), "{e}");
        assert!(!d.ptt().unwrap());
        // Its Digi mode does key over CAT.
        d.set_mode("RTTY", 0).unwrap();
        d.set_ptt(true).unwrap();
        assert!(d.ptt().unwrap());
        d.set_ptt(false).unwrap();
    }

    #[test]
    fn elecraft() {
        let mut d =
            Kenwood::new(Box::new(SimRadio::new(Kind::Elecraft)), "017", false, 100.0).unwrap();
        assert!(d.describe().contains("Elecraft"), "{}", d.describe());
        exercise(&mut d);
        assert_eq!(d.get_level("SWR").unwrap(), 1.5);
    }

    #[test]
    fn yaesu_newcat() {
        let mut d =
            Yaesu::new(Box::new(SimRadio::new(Kind::Yaesu("0670"))), "0670", 100.0).unwrap();
        assert!(d.describe().contains("FT-991A"));
        exercise(&mut d);
        assert!((d.get_level("SWR").unwrap() - 2.0).abs() < 0.01);
        d.set_level("AF", 0.5).unwrap();
        assert!((d.get_level("AF").unwrap() - 0.5).abs() < 0.01);
    }

    #[test]
    fn yaesu_ft817() {
        let mut d = Ft817::probe(Box::new(SimRadio::new(Kind::Ft817))).unwrap();
        exercise(&mut d);
        assert_eq!(d.get_level("STRENGTH").unwrap(), 0.0);
    }

    #[test]
    fn ascii_probe_tells_families_apart() {
        let d = super::super::probe_ascii(
            Box::new(SimRadio::new(Kind::Yaesu("0800"))),
            super::super::Family::Kenwood,
            100.0,
        )
        .unwrap();
        assert!(d.describe().contains("FT-710"));
        let d = super::super::probe_ascii(
            Box::new(SimRadio::new(Kind::Elecraft)),
            super::super::Family::Kenwood,
            100.0,
        )
        .unwrap();
        assert!(d.describe().contains("Elecraft"));
    }
}
