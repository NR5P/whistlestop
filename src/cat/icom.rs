//! Icom CI-V: binary frames `FE FE <to> <from> <cmd> [sub] [data] FD`.
//! Whistlestop is controller E0h. The radio's address is asked for with a
//! broadcast "read transceiver ID" (19 00), so it's found even if changed
//! from the factory setting. On USB the radio may echo our own frames back;
//! those are skipped (they're addressed to the radio, not to us).

use super::{Driver, Wire, cal, unsupported, words};
use crate::rig::RigCaps;
use anyhow::{Result, anyhow, bail};
use std::time::{Duration, Instant};

const ME: u8 = 0xE0;
const OK: u8 = 0xFB;
const NG: u8 = 0xFA;

/// Factory CI-V addresses.
pub fn model_name(addr: u8) -> &'static str {
    match addr {
        0x94 => "IC-7300",
        0xA4 => "IC-705",
        0x98 => "IC-7610",
        0xA2 => "IC-9700",
        0x88 => "IC-7100",
        0x8E => "IC-7851",
        0xAC => "IC-905",
        0xB6 => "IC-7300MK2",
        0x70 => "IC-7000 (or Xiegu)",
        0x76 => "IC-7200",
        0x80 => "IC-7410",
        0x7C => "IC-9100",
        0x7A => "IC-7600",
        0x6E => "IC-756PROIII",
        0x66 => "IC-746PRO",
        0x5E => "IC-718",
        0x58 => "IC-703",
        0x4E => "IC-706MKIIG",
        _ => "Icom radio",
    }
}

pub struct Icom {
    w: Box<dyn Wire>,
    addr: u8,
    vfo: String,
    /// Does the radio have the data-mode command (1A 06)?
    data_cmd: bool,
}

pub(crate) fn frame(to: u8, body: &[u8]) -> Vec<u8> {
    let mut f = vec![0xFE, 0xFE, to, ME];
    f.extend_from_slice(body);
    f.push(0xFD);
    f
}

/// Frequency as CI-V BCD, least significant byte first.
pub(crate) fn freq_bcd(hz: u64, n: usize) -> Vec<u8> {
    let mut v = hz;
    (0..n)
        .map(|_| {
            let lo = (v % 10) as u8;
            v /= 10;
            let hi = (v % 10) as u8;
            v /= 10;
            (hi << 4) | lo
        })
        .collect()
}

pub(crate) fn bcd_freq(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |acc, &x| acc * 100 + ((x >> 4) as u64) * 10 + (x & 0x0F) as u64)
}

/// 0..=255 (or 0..=9999) as two BCD bytes, most significant first.
pub(crate) fn bcd2(v: u16) -> [u8; 2] {
    let d = [(v / 1000) % 10, (v / 100) % 10, (v / 10) % 10, v % 10];
    [((d[0] << 4) | d[1]) as u8, ((d[2] << 4) | d[3]) as u8]
}

pub(crate) fn from_bcd2(b: &[u8]) -> u16 {
    let n = |x: u8| ((x >> 4) as u16) * 10 + (x & 0x0F) as u16;
    n(b[0]) * 100 + n(*b.get(1).unwrap_or(&0))
}

const SMETER: [(f32, f32); 3] = [(0.0, -54.0), (120.0, 0.0), (241.0, 60.0)];
const SWR: [(f32, f32); 5] = [(0.0, 1.0), (48.0, 1.5), (80.0, 2.0), (120.0, 3.0), (255.0, 10.0)];

fn level_cmd(name: &str) -> Option<u8> {
    Some(match name {
        "AF" => 0x01,
        "RF" => 0x02,
        "SQL" => 0x03,
        "NR" => 0x06,
        "RFPOWER" => 0x0A,
        "MICGAIN" => 0x0B,
        "COMP" => 0x0E,
        _ => return None,
    })
}

fn func_cmd(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "NB" => &[0x16, 0x22],
        "NR" => &[0x16, 0x40],
        "ANF" => &[0x16, 0x41],
        "COMP" => &[0x16, 0x44],
        "MON" => &[0x16, 0x45],
        "VOX" => &[0x16, 0x46],
        "TUNER" => &[0x1C, 0x01],
        _ => return None,
    })
}

impl Icom {
    /// Find the radio (asking its address unless given) and check it answers.
    pub fn probe(mut w: Box<dyn Wire>, addr: u8) -> Result<Icom> {
        let addr = if addr != 0 {
            addr
        } else {
            w.clear_input();
            w.write_all(&frame(0x00, &[0x19, 0x00]))?;
            let deadline = Instant::now() + Duration::from_millis(600);
            let mut found = None;
            let mut acc = vec![];
            while found.is_none() && Instant::now() < deadline {
                let mut b = [0u8; 64];
                let n = w.read_some(&mut b)?;
                acc.extend_from_slice(&b[..n]);
                while let Some((f, rest)) = take_frame(&acc) {
                    acc = rest;
                    if f.len() >= 5 && f[0] == ME && f[2] == 0x19 && f[3] == 0x00 {
                        found = Some(f.get(4).copied().unwrap_or(f[1]));
                    }
                }
            }
            found.ok_or_else(|| anyhow!("no Icom CI-V radio answered"))?
        };
        let mut r = Icom { w, addr, vfo: "VFOA".into(), data_cmd: true };
        r.freq().map_err(|e| anyhow!("Icom at {addr:02X}h: {e}"))?;
        r.data_cmd = r.query(&[0x1A, 0x06]).is_ok();
        Ok(r)
    }

    /// Send a command and return the radio's reply body (cmd + data).
    fn cmd(&mut self, body: &[u8]) -> Result<Vec<u8>> {
        self.w.clear_input();
        self.w.write_all(&frame(self.addr, body))?;
        let deadline = Instant::now() + Duration::from_millis(600);
        let mut acc = vec![];
        while Instant::now() < deadline {
            let mut b = [0u8; 64];
            let n = self.w.read_some(&mut b)?;
            acc.extend_from_slice(&b[..n]);
            while let Some((f, rest)) = take_frame(&acc) {
                acc = rest;
                // f = [to, from, body...]; skip echoes and transceive chatter.
                if f.len() >= 3 && f[0] == ME && f[1] == self.addr {
                    return Ok(f[2..].to_vec());
                }
            }
        }
        bail!("no answer from the radio (CI-V {:02X}h)", self.addr)
    }

    /// A read: the reply must echo the command; returns the data after it.
    fn query(&mut self, body: &[u8]) -> Result<Vec<u8>> {
        let r = self.cmd(body)?;
        if r == [NG] {
            bail!("the radio doesn't support that command");
        }
        if !r.starts_with(body) {
            bail!("unexpected reply from the radio");
        }
        Ok(r[body.len()..].to_vec())
    }

    fn set(&mut self, body: &[u8]) -> Result<()> {
        match self.cmd(body)?.as_slice() {
            [OK] => Ok(()),
            [NG] => bail!("the radio refused that"),
            _ => Ok(()),
        }
    }
}

/// Pull one `FE FE … FD` frame (without the FE FE / FD) from a buffer.
pub(crate) fn take_frame(buf: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let start = buf.windows(2).position(|w| w == [0xFE, 0xFE])?;
    let mut i = start + 2;
    while i < buf.len() && buf[i] == 0xFE {
        i += 1;
    }
    let end = buf[i..].iter().position(|&b| b == 0xFD)? + i;
    Some((buf[i..end].to_vec(), buf[end + 1..].to_vec()))
}

fn mode_code(m: &str) -> Option<(u8, bool)> {
    Some(match m {
        "LSB" => (0x00, false),
        "USB" => (0x01, false),
        "AM" => (0x02, false),
        "CW" => (0x03, false),
        "RTTY" => (0x04, false),
        "FM" => (0x05, false),
        "CWR" => (0x07, false),
        "RTTYR" => (0x08, false),
        "PKTLSB" => (0x00, true),
        "PKTUSB" => (0x01, true),
        "PKTAM" => (0x02, true),
        "PKTFM" => (0x05, true),
        _ => return None,
    })
}

fn passband(mode: u8, fil: u8) -> i64 {
    let t: [i64; 3] = match mode {
        0x02 => [9000, 6000, 3000],
        0x03 | 0x07 => [1200, 500, 250],
        0x04 | 0x08 => [2400, 500, 250],
        0x05 => [15000, 10000, 7000],
        _ => [3000, 2400, 1800],
    };
    t.get((fil.max(1) - 1) as usize).copied().unwrap_or(0)
}

impl Driver for Icom {
    fn describe(&self) -> String {
        format!("Icom {} (CI-V {:02X}h, direct CAT)", model_name(self.addr).trim_start_matches("Icom "), self.addr)
    }

    fn caps(&self) -> RigCaps {
        RigCaps {
            model: format!("Icom {}", model_name(self.addr)),
            get_levels: words("AF RF SQL NR RFPOWER MICGAIN COMP PREAMP ATT STRENGTH SWR ALC RFPOWER_METER"),
            set_levels: words("AF RF SQL NR RFPOWER MICGAIN COMP PREAMP ATT"),
            get_funcs: words("NB NR ANF COMP VOX MON TUNER"),
            set_funcs: words("NB NR ANF COMP VOX MON TUNER"),
            vfo_ops: words("CPY XCHG TUNE"),
            preamp_db: vec![10, 20],
            att_db: vec![20],
            modes: words("USB LSB PKTUSB PKTLSB CW CWR AM FM RTTY"),
        }
    }

    fn freq(&mut self) -> Result<f64> {
        let d = self.query(&[0x03])?;
        if d.len() < 4 {
            bail!("short frequency reply");
        }
        Ok(bcd_freq(&d) as f64)
    }

    fn set_freq(&mut self, hz: f64) -> Result<()> {
        let mut b = vec![0x05];
        // The IC-905 (up to 10 GHz) uses six bytes; HF radios five.
        b.extend(freq_bcd(hz.round() as u64, if hz >= 10e9 || self.addr == 0xAC { 6 } else { 5 }));
        self.set(&b)
    }

    fn mode(&mut self) -> Result<(String, i64)> {
        let d = self.query(&[0x04])?;
        let (m, fil) = (*d.first().ok_or_else(|| anyhow!("short mode reply"))?, *d.get(1).unwrap_or(&1));
        let data = self.data_cmd && self.query(&[0x1A, 0x06]).map(|x| x.first().copied().unwrap_or(0) != 0).unwrap_or(false);
        let name = match (m, data) {
            (0x00, false) => "LSB",
            (0x01, false) => "USB",
            (0x02, false) => "AM",
            (0x03, _) => "CW",
            (0x04, _) => "RTTY",
            (0x05, false) => "FM",
            (0x07, _) => "CWR",
            (0x08, _) => "RTTYR",
            (0x00, true) => "PKTLSB",
            (0x01, true) => "PKTUSB",
            (0x02, true) => "PKTAM",
            (0x05, true) => "PKTFM",
            _ => "USB",
        };
        Ok((name.into(), passband(m, fil)))
    }

    fn set_mode(&mut self, mode: &str, passband_hz: i64) -> Result<()> {
        let (code, data) = mode_code(mode).ok_or_else(|| unsupported(mode))?;
        let fil = match passband_hz {
            p if p <= 0 => 0,
            p if p >= passband(code, 1) => 1,
            p if p >= passband(code, 2) => 2,
            _ => 3,
        };
        if fil == 0 { self.set(&[0x06, code])? } else { self.set(&[0x06, code, fil])? }
        if self.data_cmd {
            let r = self.set(&[0x1A, 0x06, data as u8, if data { fil.max(1) } else { 0 }]);
            if data {
                r?;
            }
        } else if data {
            bail!("this radio has no data mode over CI-V; use USB");
        }
        Ok(())
    }

    fn ptt(&mut self) -> Result<bool> {
        Ok(self.query(&[0x1C, 0x00])?.first().copied().unwrap_or(0) != 0)
    }

    fn set_ptt(&mut self, on: bool) -> Result<()> {
        self.set(&[0x1C, 0x00, on as u8])
    }

    fn get_level(&mut self, name: &str) -> Result<f32> {
        let v = |d: Vec<u8>| -> Result<f32> {
            if d.len() < 2 {
                bail!("short reply");
            }
            Ok(from_bcd2(&d) as f32)
        };
        Ok(match name {
            "STRENGTH" => cal(&SMETER, v(self.query(&[0x15, 0x02])?)?),
            "SWR" => cal(&SWR, v(self.query(&[0x15, 0x12])?)?),
            "ALC" => (v(self.query(&[0x15, 0x13])?)? / 120.0).min(1.0),
            "RFPOWER_METER" => (v(self.query(&[0x15, 0x11])?)? / 213.0).min(1.0),
            "PREAMP" => [0.0, 10.0, 20.0][self.query(&[0x16, 0x02])?.first().copied().unwrap_or(0).min(2) as usize],
            "ATT" => {
                if self.query(&[0x11])?.first().copied().unwrap_or(0) != 0 {
                    20.0
                } else {
                    0.0
                }
            }
            _ => {
                let c = level_cmd(name).ok_or_else(|| unsupported(name))?;
                v(self.query(&[0x14, c])?)? / 255.0
            }
        })
    }

    fn set_level(&mut self, name: &str, val: f32) -> Result<()> {
        match name {
            "PREAMP" => self.set(&[0x16, 0x02, if val >= 15.0 { 2 } else if val > 0.0 { 1 } else { 0 }]),
            "ATT" => self.set(&[0x11, if val > 0.0 { 0x20 } else { 0x00 }]),
            _ => {
                let c = level_cmd(name).ok_or_else(|| unsupported(name))?;
                let b = bcd2((val.clamp(0.0, 1.0) * 255.0).round() as u16);
                self.set(&[0x14, c, b[0], b[1]])
            }
        }
    }

    fn get_func(&mut self, name: &str) -> Result<bool> {
        let c = func_cmd(name).ok_or_else(|| unsupported(name))?;
        Ok(self.query(c)?.first().copied().unwrap_or(0) != 0)
    }

    fn set_func(&mut self, name: &str, on: bool) -> Result<()> {
        let mut c = func_cmd(name).ok_or_else(|| unsupported(name))?.to_vec();
        c.push(on as u8);
        self.set(&c)
    }

    fn vfo_op(&mut self, op: &str) -> Result<()> {
        match op {
            "TUNE" => self.set(&[0x1C, 0x01, 0x02]),
            "CPY" => self.set(&[0x07, 0xA0]),
            "XCHG" => {
                self.set(&[0x07, 0xB0])?;
                self.vfo = if self.vfo == "VFOA" { "VFOB".into() } else { "VFOA".into() };
                Ok(())
            }
            _ => Err(unsupported(op)),
        }
    }

    fn vfo(&mut self) -> Result<String> {
        Ok(self.vfo.clone())
    }

    fn set_vfo(&mut self, vfo: &str) -> Result<()> {
        self.set(&[0x07, if vfo == "VFOB" { 0x01 } else { 0x00 }])?;
        self.vfo = if vfo == "VFOB" { "VFOB".into() } else { "VFOA".into() };
        Ok(())
    }

    fn split(&mut self) -> Result<bool> {
        Ok(self.query(&[0x0F])?.first().copied().unwrap_or(0) == 1)
    }

    fn set_split(&mut self, on: bool) -> Result<()> {
        self.set(&[0x0F, on as u8])
    }

    /// Icom radios share one offset between RIT and XIT.
    fn offset(&mut self, xit: bool) -> Result<i32> {
        let on = self.query(&[0x21, if xit { 0x02 } else { 0x01 }])?.first().copied().unwrap_or(0) != 0;
        if !on {
            return Ok(0);
        }
        let d = self.query(&[0x21, 0x00])?;
        if d.len() < 3 {
            bail!("short reply");
        }
        let n = |x: u8| ((x >> 4) as i32) * 10 + (x & 0x0F) as i32;
        let hz = n(d[1]) * 100 + n(d[0]);
        Ok(if d[2] != 0 { -hz } else { hz })
    }

    fn set_offset(&mut self, xit: bool, hz: i32) -> Result<()> {
        let a = hz.unsigned_abs().min(9999) as u64;
        let b = freq_bcd(a, 2);
        self.set(&[0x21, 0x00, b[0], b[1], (hz < 0) as u8])?;
        self.set(&[0x21, if xit { 0x02 } else { 0x01 }, (hz != 0) as u8])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd() {
        assert_eq!(freq_bcd(14_074_000, 5), vec![0x00, 0x40, 0x07, 0x14, 0x00]);
        assert_eq!(bcd_freq(&[0x00, 0x40, 0x07, 0x14, 0x00]), 14_074_000);
        assert_eq!(bcd2(128), [0x01, 0x28]);
        assert_eq!(from_bcd2(&[0x02, 0x55]), 255);
    }

    #[test]
    fn frames_skip_noise() {
        let buf = [0x00, 0xFE, 0xFE, 0xE0, 0x94, 0xFB, 0xFD, 0xFE];
        let (f, rest) = take_frame(&buf).unwrap();
        assert_eq!(f, vec![0xE0, 0x94, 0xFB]);
        assert_eq!(rest, vec![0xFE]);
    }
}
