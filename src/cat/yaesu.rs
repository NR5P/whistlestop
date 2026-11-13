//! Yaesu CAT: the current ASCII protocol (FT-991, FT-891, FT-710, FTDX10,
//! FTDX101, FT-950, FTDX3000...) and the older 5-byte binary protocol of the
//! FT-817/818/857/897.

use super::{Driver, Wire, ascii_query, ascii_set, cal, num, unsupported, words};
use crate::rig::RigCaps;
use anyhow::{Result, anyhow, bail};
use std::time::{Duration, Instant};

pub fn model_name(id: &str) -> &'static str {
    match id {
        "0101" => "FTDX9000",
        "0241" => "FT-450",
        "0251" => "FT-2000",
        "0310" => "FT-950",
        "0362" => "FTDX5000",
        "0460" => "FTDX3000",
        "0570" => "FT-991",
        "0583" => "FTDX1200",
        "0650" => "FT-891",
        "0670" => "FT-991A",
        "0681" => "FTDX101D",
        "0682" => "FTDX101MP",
        "0761" => "FTDX10",
        "0800" => "FT-710",
        _ => "radio",
    }
}

pub struct Yaesu {
    w: Box<dyn Wire>,
    name: String,
    /// Digits in FA (9 on most, 8 on the FT-950/2000/3000 generation).
    fw: usize,
    max_w: f32,
    vfo: String,
}

const SMETER: [(f32, f32); 3] = [(0.0, -54.0), (130.0, 0.0), (255.0, 60.0)];
const SWR: [(f32, f32); 8] = [
    (0.0, 1.0),
    (26.0, 1.2),
    (52.0, 1.5),
    (89.0, 2.0),
    (126.0, 3.0),
    (173.0, 4.0),
    (236.0, 5.0),
    (255.0, 10.0),
];

impl Yaesu {
    pub fn new(w: Box<dyn Wire>, id: &str, max_w: f32) -> Result<Yaesu> {
        let max_w = if id == "0682" && max_w == 100.0 {
            200.0
        } else {
            max_w
        };
        let mut y = Yaesu {
            w,
            name: format!("Yaesu {}", model_name(id)),
            fw: 9,
            max_w,
            vfo: "VFOA".into(),
        };
        let fa = y.q("FA;", "FA").map_err(|e| anyhow!("{}: {e}", y.name))?;
        y.fw = fa.len().saturating_sub(2).clamp(8, 9);
        Ok(y)
    }

    fn q(&mut self, cmd: &str, prefix: &str) -> Result<String> {
        ascii_query(self.w.as_mut(), cmd, prefix)
    }

    fn s(&mut self, cmd: &str) -> Result<()> {
        ascii_set(self.w.as_mut(), cmd)
    }

    /// A 0..255 meter (RMn).
    fn meter(&mut self, n: u8) -> Result<f32> {
        let p = format!("RM{n}");
        let r = self.q(&format!("{p};"), &p)?;
        Ok(num(&r, 3, Some(3))? as f32)
    }
}

impl Driver for Yaesu {
    fn describe(&self) -> String {
        format!("{} (direct CAT)", self.name)
    }

    fn caps(&self) -> RigCaps {
        RigCaps {
            model: self.name.clone(),
            get_levels: words(
                "AF RF SQL MICGAIN RFPOWER PREAMP ATT STRENGTH SWR ALC RFPOWER_METER",
            ),
            set_levels: words("AF RF SQL MICGAIN RFPOWER PREAMP ATT"),
            get_funcs: words("NB NR ANF VOX TUNER"),
            set_funcs: words("NB NR ANF VOX TUNER"),
            vfo_ops: words("TUNE CPY XCHG"),
            preamp_db: vec![10, 20],
            att_db: vec![12],
            modes: words("USB LSB PKTUSB PKTLSB CW CWR AM FM PKTFM RTTY RTTYR"),
        }
    }

    fn freq(&mut self) -> Result<f64> {
        let r = self.q("FA;", "FA")?;
        Ok(num(&r, 2, None)? as f64)
    }

    fn set_freq(&mut self, hz: f64) -> Result<()> {
        let fw = self.fw;
        self.s(&format!("FA{:0fw$};", hz.round() as u64))
    }

    fn mode(&mut self) -> Result<(String, i64)> {
        let r = self.q("MD0;", "MD0")?;
        let name = match r.chars().nth(3).unwrap_or('2') {
            '1' => "LSB",
            '2' => "USB",
            '3' => "CW",
            '4' | 'B' | 'E' => "FM",
            '5' | 'D' => "AM",
            '6' => "RTTY",
            '7' => "CWR",
            '8' => "PKTLSB",
            '9' => "RTTYR",
            'A' => "PKTFM",
            'C' => "PKTUSB",
            _ => "USB",
        };
        Ok((name.into(), 0))
    }

    fn set_mode(&mut self, mode: &str, _passband_hz: i64) -> Result<()> {
        let c = match mode {
            "LSB" => '1',
            "USB" => '2',
            "CW" => '3',
            "FM" => '4',
            "AM" => '5',
            "RTTY" => '6',
            "CWR" => '7',
            "PKTLSB" => '8',
            "RTTYR" => '9',
            "PKTFM" => 'A',
            "PKTUSB" => 'C',
            _ => return Err(unsupported(mode)),
        };
        self.s(&format!("MD0{c};"))
    }

    fn ptt(&mut self) -> Result<bool> {
        let r = self.q("TX;", "TX")?;
        Ok(r.chars().nth(2).is_some_and(|c| c != '0'))
    }

    fn set_ptt(&mut self, on: bool) -> Result<()> {
        self.s(if on { "TX1;" } else { "TX0;" })
    }

    fn get_level(&mut self, name: &str) -> Result<f32> {
        let simple = |s: &mut Self, cmd: &str, p: &str, full: f32| -> Result<f32> {
            let r = s.q(cmd, p)?;
            Ok(num(&r, p.len(), Some(3))? as f32 / full)
        };
        Ok(match name {
            "AF" => simple(self, "AG0;", "AG0", 255.0)?,
            "RF" => simple(self, "RG0;", "RG0", 255.0)?,
            "SQL" => simple(self, "SQ0;", "SQ0", 100.0)?,
            "MICGAIN" => simple(self, "MG;", "MG", 100.0)?,
            "RFPOWER" => simple(self, "PC;", "PC", self.max_w.max(1.0))?,
            "STRENGTH" => {
                let r = self.q("SM0;", "SM0")?;
                cal(&SMETER, num(&r, 3, Some(3))? as f32)
            }
            "SWR" => cal(&SWR, self.meter(6)?),
            "ALC" => self.meter(4)? / 255.0,
            "RFPOWER_METER" => self.meter(5)? / 255.0,
            "PREAMP" => {
                let r = self.q("PA0;", "PA0")?;
                [0.0, 10.0, 20.0][num(&r, 3, Some(1))?.clamp(0, 2) as usize]
            }
            "ATT" => {
                let r = self.q("RA0;", "RA0")?;
                if num(&r, 3, Some(1))? != 0 { 12.0 } else { 0.0 }
            }
            _ => return Err(unsupported(name)),
        })
    }

    fn set_level(&mut self, name: &str, v: f32) -> Result<()> {
        let n = |full: f32| (v.clamp(0.0, 1.0) * full).round() as u32;
        match name {
            "AF" => self.s(&format!("AG0{:03};", n(255.0))),
            "RF" => self.s(&format!("RG0{:03};", n(255.0))),
            "SQL" => self.s(&format!("SQ0{:03};", n(100.0))),
            "MICGAIN" => self.s(&format!("MG{:03};", n(100.0))),
            // Three digits (see kenwood.rs): never more than 999.
            "RFPOWER" => self.s(&format!("PC{:03};", n(self.max_w).clamp(5, 999))),
            "PREAMP" => self.s(&format!(
                "PA0{};",
                if v >= 15.0 {
                    2
                } else if v > 0.0 {
                    1
                } else {
                    0
                }
            )),
            "ATT" => self.s(&format!("RA0{};", (v > 0.0) as u8)),
            _ => Err(unsupported(name)),
        }
    }

    fn get_func(&mut self, name: &str) -> Result<bool> {
        let (cmd, p) = match name {
            "NB" => ("NB0;", "NB0"),
            "NR" => ("NR0;", "NR0"),
            "ANF" => ("BC0;", "BC0"),
            "VOX" => ("VX;", "VX"),
            "TUNER" => {
                let r = self.q("AC;", "AC")?;
                return Ok(matches!(r.chars().nth(4), Some('1') | Some('2')));
            }
            _ => return Err(unsupported(name)),
        };
        let r = self.q(cmd, p)?;
        Ok(r.chars().nth(p.len()).is_some_and(|c| c != '0'))
    }

    fn set_func(&mut self, name: &str, on: bool) -> Result<()> {
        let p = match name {
            "NB" => "NB0",
            "NR" => "NR0",
            "ANF" => "BC0",
            "VOX" => "VX",
            "TUNER" => return self.s(if on { "AC001;" } else { "AC000;" }),
            _ => return Err(unsupported(name)),
        };
        self.s(&format!("{p}{};", on as u8))
    }

    fn vfo_op(&mut self, op: &str) -> Result<()> {
        match op {
            "TUNE" => self.s("AC002;"),
            "CPY" => self.s("AB;"),
            "XCHG" => {
                self.s("SV;")?;
                self.vfo = if self.vfo == "VFOA" {
                    "VFOB".into()
                } else {
                    "VFOA".into()
                };
                Ok(())
            }
            _ => Err(unsupported(op)),
        }
    }

    fn vfo(&mut self) -> Result<String> {
        Ok(self.vfo.clone())
    }

    fn set_vfo(&mut self, vfo: &str) -> Result<()> {
        self.s(&format!("VS{};", (vfo == "VFOB") as u8))?;
        self.vfo = if vfo == "VFOB" {
            "VFOB".into()
        } else {
            "VFOA".into()
        };
        Ok(())
    }

    fn split(&mut self) -> Result<bool> {
        let r = self.q("ST;", "ST")?;
        Ok(r.chars().nth(2).is_some_and(|c| c != '0'))
    }

    fn set_split(&mut self, on: bool) -> Result<()> {
        self.s(&format!("ST{};", on as u8))
    }

    fn offset(&mut self, xit: bool) -> Result<i32> {
        let r = self.q("IF;", "IF")?;
        let at = 2 + 3 + self.fw; // after memory channel and frequency
        let on = r.chars().nth(at + if xit { 6 } else { 5 }) == Some('1');
        if !on {
            return Ok(0);
        }
        Ok(num(&r, at, Some(5))? as i32)
    }

    fn set_offset(&mut self, xit: bool, hz: i32) -> Result<()> {
        self.s("RC;")?;
        if hz != 0 {
            self.s(&format!(
                "{}{:04};",
                if hz > 0 { "RU" } else { "RD" },
                hz.unsigned_abs().min(9999)
            ))?;
        }
        self.s(&format!(
            "{}{};",
            if xit { "XT" } else { "RT" },
            (hz != 0) as u8
        ))
    }
}

// ------------------------------------------------------ FT-817 / 857 / 897

pub struct Ft817 {
    w: Box<dyn Wire>,
    /// The PTT state the radio last confirmed; None = unknown (a key or
    /// unkey that may not have reached the radio).
    ptt: Option<bool>,
    /// The radio answers "read TX status" (F7). Cleared the first time it
    /// doesn't, so a radio or bridge without it isn't asked on every poll.
    tx_status_ok: bool,
}

/// A hard failure of the link to the radio (the port or bridge), as opposed
/// to the radio not answering.
fn is_io(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some()
}

fn bcd_be(v: u64) -> [u8; 4] {
    let mut b = [0u8; 4];
    let mut x = v;
    for i in (0..4).rev() {
        let lo = (x % 10) as u8;
        x /= 10;
        let hi = (x % 10) as u8;
        x /= 10;
        b[i] = (hi << 4) | lo;
    }
    b
}

fn from_bcd_be(b: &[u8]) -> Option<u64> {
    b.iter().try_fold(0u64, |acc, &x| {
        let (h, l) = (x >> 4, x & 0x0F);
        (h <= 9 && l <= 9).then_some(acc * 100 + h as u64 * 10 + l as u64)
    })
}

impl Ft817 {
    pub fn probe(w: Box<dyn Wire>) -> Result<Ft817> {
        let mut r = Ft817 {
            w,
            ptt: None,
            tx_status_ok: true,
        };
        r.read_fm().map_err(|e| anyhow!("FT-817 family: {e}"))?;
        Ok(r)
    }

    fn cmd(&mut self, b: [u8; 5], reply: usize) -> Result<Vec<u8>> {
        self.w.clear_input();
        self.w.write_all(&b)?;
        if reply == 0 {
            std::thread::sleep(Duration::from_millis(40));
            return Ok(vec![]);
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut acc = vec![];
        while acc.len() < reply && Instant::now() < deadline {
            let mut buf = [0u8; 16];
            let n = self.w.read_some(&mut buf)?;
            acc.extend_from_slice(&buf[..n]);
        }
        if acc.len() < reply {
            bail!("no answer from the radio");
        }
        acc.truncate(reply);
        Ok(acc)
    }

    /// Frequency (Hz) and mode byte.
    fn read_fm(&mut self) -> Result<(u64, u8)> {
        let r = self.cmd([0, 0, 0, 0, 0x03], 5)?;
        let f = from_bcd_be(&r[..4]).ok_or_else(|| anyhow!("not an FT-817 reply"))? * 10;
        if !matches!(
            r[4],
            0x00..=0x04 | 0x06 | 0x08 | 0x0A | 0x0C | 0x82 | 0x88 | 0x8A | 0x8C
        ) || f == 0
        {
            bail!("not an FT-817 reply");
        }
        Ok((f, r[4]))
    }

    /// Transmitting? "Read TX status" (00 00 00 00 F7) answers one byte
    /// whose bit 7 is low while the radio transmits (Hamlib reads it the
    /// same way). None if the radio doesn't answer it.
    fn tx_status(&mut self) -> Result<Option<bool>> {
        if !self.tx_status_ok {
            return Ok(None);
        }
        match self.cmd([0, 0, 0, 0, 0xF7], 1) {
            Ok(b) => Ok(Some(b[0] & 0x80 == 0)),
            Err(e) if is_io(&e) => Err(e),
            Err(_) => {
                self.tx_status_ok = false;
                Ok(None)
            }
        }
    }
}

impl Driver for Ft817 {
    fn describe(&self) -> String {
        "Yaesu FT-817/818/857/897 (direct CAT)".into()
    }

    fn caps(&self) -> RigCaps {
        RigCaps {
            model: "Yaesu FT-817/818/857/897".into(),
            get_levels: words("STRENGTH"),
            modes: words("USB LSB PKTUSB PKTFM CW CWR AM FM"),
            ..Default::default()
        }
    }

    fn freq(&mut self) -> Result<f64> {
        Ok(self.read_fm()?.0 as f64)
    }

    fn set_freq(&mut self, hz: f64) -> Result<()> {
        let b = bcd_be((hz / 10.0).round() as u64);
        self.cmd([b[0], b[1], b[2], b[3], 0x01], 0).map(|_| ())
    }

    fn mode(&mut self) -> Result<(String, i64)> {
        let name = match self.read_fm()?.1 & 0x7F {
            0x00 => "LSB",
            0x01 => "USB",
            0x02 => "CW",
            0x03 => "CWR",
            0x04 => "AM",
            0x06 | 0x08 => "FM",
            0x0A => "PKTUSB",
            0x0C => "PKTFM",
            _ => "USB",
        };
        Ok((name.into(), 0))
    }

    fn set_mode(&mut self, mode: &str, _passband_hz: i64) -> Result<()> {
        let m = match mode {
            "LSB" => 0x00,
            "USB" => 0x01,
            "CW" => 0x02,
            "CWR" => 0x03,
            "AM" => 0x04,
            "FM" => 0x08,
            // DIG (set its sub-mode to USER-U in the radio's menu).
            "PKTUSB" | "PKTLSB" => 0x0A,
            "PKTFM" => 0x0C,
            _ => return Err(unsupported(mode)),
        };
        self.cmd([m, 0, 0, 0, 0x07], 0).map(|_| ())
    }

    fn ptt(&mut self) -> Result<bool> {
        if let Some(on) = self.tx_status()? {
            self.ptt = Some(on);
            return Ok(on);
        }
        // No TX status from this radio: the last state it confirmed.
        self.ptt
            .ok_or_else(|| anyhow!("the radio's PTT state is unknown (it didn't confirm the last change)"))
    }

    fn set_ptt(&mut self, on: bool) -> Result<()> {
        let what = if on { "key" } else { "unkey" };
        // The radio acknowledges with one byte: 00 done, F0 already in
        // that state (both fine).
        let ack = match self.cmd([0, 0, 0, 0, if on { 0x08 } else { 0x88 }], 1) {
            Ok(b) => Some(b[0]),
            Err(e) if is_io(&e) => {
                self.ptt = None;
                return Err(e.context(format!("couldn't {what} the radio")));
            }
            Err(_) => None,
        };
        if matches!(ack, Some(0x00) | Some(0xF0)) {
            self.ptt = Some(on);
            return Ok(());
        }
        // No (or an odd) acknowledgement: ask the radio what it's doing.
        match self.tx_status() {
            Ok(Some(now)) => {
                self.ptt = Some(now);
                if now == on {
                    Ok(())
                } else {
                    Err(anyhow!("the radio didn't {what}"))
                }
            }
            Ok(None) => {
                self.ptt = None;
                Err(anyhow!("the radio didn't confirm the {what}"))
            }
            Err(e) => {
                self.ptt = None;
                Err(e.context(format!("couldn't {what} the radio")))
            }
        }
    }

    fn get_level(&mut self, name: &str) -> Result<f32> {
        if name != "STRENGTH" {
            return Err(unsupported(name));
        }
        let s = (self.cmd([0, 0, 0, 0, 0xE7], 1)?[0] & 0x0F) as f32;
        Ok(if s <= 9.0 {
            (s - 9.0) * 6.0
        } else {
            (s - 9.0) * 10.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ft817_bcd() {
        assert_eq!(bcd_be(1_407_400), [0x01, 0x40, 0x74, 0x00]);
        assert_eq!(from_bcd_be(&[0x01, 0x40, 0x74, 0x00]), Some(1_407_400));
        assert_eq!(from_bcd_be(&[0xFF, 0, 0, 0]), None);
    }

    use super::super::sim::{Kind, SimRadio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    const OK: u8 = 0;
    const WRITES_FAIL: u8 = 1;
    /// Acknowledge PTT changes with F0 ("already in that state").
    const ACK_F0: u8 = 2;

    /// A simulated FT-817 whose port can break, or that answers F0 to PTT
    /// commands. (The third field: the last command was a PTT command.)
    struct Flaky(SimRadio, Arc<AtomicU8>, bool);

    impl Wire for Flaky {
        fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
            match self.1.load(Ordering::SeqCst) {
                WRITES_FAIL => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "unplugged",
                )),
                _ => {
                    self.2 = matches!(b.last(), Some(0x08 | 0x88));
                    self.0.write_all(b)
                }
            }
        }
        fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read_some(buf)?;
            if self.1.load(Ordering::SeqCst) == ACK_F0 && self.2 && n == 1 && buf[0] == 0 {
                buf[0] = 0xF0;
            }
            Ok(n)
        }
        fn clear_input(&mut self) {
            self.0.clear_input()
        }
    }

    fn flaky() -> (Ft817, Arc<AtomicU8>) {
        let mode = Arc::new(AtomicU8::new(OK));
        let d = Ft817::probe(Box::new(Flaky(SimRadio::new(Kind::Ft817), mode.clone(), false))).unwrap();
        (d, mode)
    }

    #[test]
    fn ft817_failed_unkey_is_an_error_and_not_rx() {
        let (mut d, mode) = flaky();
        d.set_ptt(true).unwrap();
        assert!(d.ptt().unwrap());
        mode.store(WRITES_FAIL, Ordering::SeqCst);
        assert!(d.set_ptt(false).is_err(), "a failed write must not be Ok");
        assert!(d.ptt().is_err(), "can't read PTT through a broken port");
        // The port comes back, but the unkey never reached the radio: PTT
        // must not read as receive.
        mode.store(OK, Ordering::SeqCst);
        assert!(!matches!(d.ptt(), Ok(false)), "{:?}", d.ptt());
        d.set_ptt(false).unwrap();
        assert!(!d.ptt().unwrap());
    }

    #[test]
    fn ft817_failed_key_is_an_error() {
        let (mut d, mode) = flaky();
        mode.store(WRITES_FAIL, Ordering::SeqCst);
        assert!(d.set_ptt(true).is_err());
    }

    #[test]
    fn ft817_already_in_that_state_is_fine() {
        let (mut d, mode) = flaky();
        mode.store(ACK_F0, Ordering::SeqCst);
        d.set_ptt(false).unwrap();
        assert!(!d.ptt().unwrap());
        d.set_ptt(true).unwrap();
        assert!(d.ptt().unwrap());
    }
}
