//! Kenwood ASCII CAT (`FA00014074000;`), and the radios that speak it:
//! Elecraft (with its own extensions), QRP Labs QMX/QDX, truSDX and
//! FlexRadio's CAT ports.

use super::{Driver, Wire, ascii_query, ascii_set, cal, num, unkey_confirmed, unsupported, words};
use crate::rig::RigCaps;
use anyhow::{Result, anyhow, bail};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Variant {
    /// TS-590, TS-890, TS-990: data mode (DA), TX1 for USB audio, tuner.
    Kenwood,
    /// K3, K4, KX2, KX3.
    Elecraft,
    /// TS-480, TS-2000, QMX/QDX, truSDX: the common core.
    Plain,
}

pub struct Kenwood {
    w: Box<dyn Wire>,
    v: Variant,
    name: String,
    max_w: f32,
    /// TS-890: the mode is `OM0` (main band), with the data modes as modes
    /// of their own, rather than MD plus DA.
    om: bool,
    /// S-meter dots → dB over S9, for this model (RigHelm's / Hamlib's).
    sm: &'static [(f32, f32)],
}

// S-meter scales.
const SM_K30: &[(f32, f32)] = &[(0.0, -54.0), (6.0, 0.0), (9.0, 20.0), (12.0, 40.0), (15.0, 60.0)];
const SM_K31: &[(f32, f32)] = &[(0.0, -54.0), (9.0, 0.0), (13.0, 20.0), (17.0, 40.0), (21.0, 60.0)];
const SM_TS590: &[(f32, f32)] = &[(0.0, -54.0), (15.0, 0.0), (30.0, 60.0)];
const SM_TS2000: &[(f32, f32)] = &[(0.0, -54.0), (3.0, -48.0), (6.0, -36.0), (9.0, -24.0), (12.0, -12.0), (15.0, 0.0), (30.0, 60.0)];
const SM_TS480: &[(f32, f32)] =
    &[(0.0, -60.0), (3.0, -48.0), (5.0, -36.0), (7.0, -24.0), (9.0, -12.0), (11.0, 0.0), (14.0, 20.0), (17.0, 40.0), (20.0, 60.0)];
const SM_TS890: &[(f32, f32)] = &[(0.0, -54.0), (35.0, 0.0), (47.0, 20.0), (58.0, 40.0), (70.0, 60.0)];
const SM_PLAIN: &[(f32, f32)] = &[(0.0, -54.0), (9.0, 0.0), (15.0, 60.0)];

pub fn model_name(id: &str) -> &'static str {
    match id {
        "017" => "K3 / TS-570",
        "019" => "TS-2000",
        "020" => "TS-480 (or QMX/QDX/truSDX)",
        "021" => "TS-590S",
        "023" => "TS-590SG",
        "022" => "TS-990S",
        "024" => "TS-890S",
        _ => "radio",
    }
}

impl Kenwood {
    pub fn new(mut w: Box<dyn Wire>, id: &str, elecraft_hint: bool, max_w: f32) -> Result<Kenwood> {
        // Elecraft radios answer OM; (their options); Kenwoods don't.
        let elecraft =
            elecraft_hint || (id == "017" && ascii_query(w.as_mut(), "OM;", "OM").is_ok());
        let (v, name) = if elecraft {
            let k4 = ascii_query(w.as_mut(), "K4;", "K4").is_ok();
            (
                Variant::Elecraft,
                if k4 {
                    "Elecraft K4".to_string()
                } else {
                    "Elecraft K3/KX".to_string()
                },
            )
        } else {
            // The TS-990S (ID022) has no IF; and its own TX/RX commands:
            // half-working over direct CAT (no TX state to read) would be
            // worse than not at all.
            if id == "022" {
                bail!("the Kenwood TS-990S isn't supported over direct CAT yet: use Hamlib (Settings → Radio → Managed rigctld)");
            }
            let v = if matches!(id, "021" | "023" | "024") {
                Variant::Kenwood
            } else {
                Variant::Plain
            };
            (v, format!("Kenwood {}", model_name(id)))
        };
        // The TS-890S (ID024) sets modes with OM.
        let om = !elecraft && id == "024";
        let sm = if elecraft {
            // The K3/KX meter is 0-15 unless extended mode K31 is on
            // (another program may have set it); the K4 reads like K31.
            let k31 = name.contains("K4") || ascii_query(w.as_mut(), "K3;", "K3").is_ok_and(|r| r.ends_with('1'));
            if k31 { SM_K31 } else { SM_K30 }
        } else {
            match id {
                "019" => SM_TS2000,
                "020" => SM_TS480,
                "021" | "023" => SM_TS590,
                "024" => SM_TS890,
                _ => SM_PLAIN,
            }
        };
        let mut k = Kenwood {
            w,
            v,
            name,
            max_w,
            om,
            sm,
        };
        k.freq().map_err(|e| anyhow!("{}: {e}", k.name))?;
        Ok(k)
    }

    fn q(&mut self, cmd: &str, prefix: &str) -> Result<String> {
        ascii_query(self.w.as_mut(), cmd, prefix)
    }

    fn s(&mut self, cmd: &str) -> Result<()> {
        ascii_set(self.w.as_mut(), cmd)
    }

    fn elecraft(&self) -> bool {
        self.v == Variant::Elecraft
    }

    /// The IF; status word (frequency, RIT, TX state, mode, split).
    fn info(&mut self) -> Result<String> {
        let r = self.q("IF;", "IF")?;
        if r.len() < 33 {
            return Err(anyhow!("short IF reply: {r}"));
        }
        Ok(r)
    }

    /// Meter value for the TS-590/890 RM command (0..30).
    fn meter(&mut self, which: u8) -> Result<f32> {
        self.s(&format!("RM{which};"))?;
        let r = self.q("RM;", "RM")?;
        Ok(num(&r, r.len().saturating_sub(4), None)? as f32)
    }

    fn level_spec(&self, name: &str) -> Option<(&'static str, &'static str, f32, usize)> {
        // (query, reply prefix, full scale, digits)
        let e = self.elecraft();
        Some(match name {
            "AF" if e => ("AG;", "AG", 250.0, 3),
            "AF" => ("AG0;", "AG0", 255.0, 3),
            "RF" if e => ("RG;", "RG", 250.0, 3),
            "RF" => ("RG;", "RG", 255.0, 3),
            "SQL" if e => ("SQ;", "SQ", 29.0, 3),
            "SQL" => ("SQ0;", "SQ0", 255.0, 3),
            "MICGAIN" if e => ("MG;", "MG", 60.0, 3),
            "MICGAIN" => ("MG;", "MG", 100.0, 3),
            _ => return None,
        })
    }
}

/// TS-990 `OM` mode codes (Hamlib's Kenwood mode table, which the TS-990
/// backend uses with OM): 1 LSB, 2 USB, 3 CW, 4 FM, 5 AM, 6 FSK, 7 CW-R,
/// 9 FSK-R, A PSK, B PSK-R, C LSB-D, D USB-D, E FM-D, F AM-D. PSK and AM-D
/// have no name in our mode list and read as USB and AM.
fn om_mode_name(c: char) -> &'static str {
    match c {
        'C' => "PKTLSB",
        'D' => "PKTUSB",
        'E' => "PKTFM",
        'F' => "AM",
        _ => mode_name(c, false),
    }
}

fn om_code(mode_code: char, data: bool) -> Option<char> {
    Some(match (mode_code, data) {
        (c, false) => c,
        ('1', true) => 'C',
        ('2', true) => 'D',
        ('4', true) => 'E',
        _ => return None,
    })
}

fn mode_name(c: char, data: bool) -> &'static str {
    match (c, data) {
        ('1', false) => "LSB",
        ('2', false) => "USB",
        ('1', true) => "PKTLSB",
        ('2', true) => "PKTUSB",
        ('3', _) => "CW",
        ('4', false) => "FM",
        ('4', true) => "PKTFM",
        ('5', _) => "AM",
        ('6', _) => "RTTY",
        ('7', _) => "CWR",
        ('9', _) => "RTTYR",
        _ => "USB",
    }
}

impl Driver for Kenwood {
    fn describe(&self) -> String {
        format!("{} (direct CAT)", self.name)
    }

    fn caps(&self) -> RigCaps {
        let (gl, sl, f, ops) = match self.v {
            Variant::Kenwood => (
                "AF RF SQL MICGAIN RFPOWER PREAMP ATT STRENGTH SWR ALC",
                "AF RF SQL MICGAIN RFPOWER PREAMP ATT",
                "NB NR ANF COMP VOX TUNER",
                "TUNE CPY",
            ),
            Variant::Elecraft => (
                "AF RF SQL MICGAIN RFPOWER PREAMP ATT STRENGTH SWR",
                "AF RF SQL MICGAIN RFPOWER PREAMP ATT",
                "NB NR VOX",
                "",
            ),
            Variant::Plain => ("AF RF SQL RFPOWER STRENGTH", "AF RF SQL RFPOWER", "", ""),
        };
        RigCaps {
            model: self.name.clone(),
            get_levels: words(gl),
            set_levels: words(sl),
            get_funcs: words(f),
            set_funcs: words(f),
            vfo_ops: words(ops),
            preamp_db: if self.v == Variant::Plain {
                vec![]
            } else {
                vec![10]
            },
            att_db: if self.v == Variant::Plain {
                vec![]
            } else {
                vec![10]
            },
            modes: words("USB LSB PKTUSB PKTLSB CW CWR AM FM RTTY"),
        }
    }

    fn freq(&mut self) -> Result<f64> {
        let r = self.q("FA;", "FA")?;
        Ok(num(&r, 2, None)? as f64)
    }

    fn set_freq(&mut self, hz: f64) -> Result<()> {
        self.s(&format!("FA{:011};", hz.round() as u64))
    }

    fn mode(&mut self) -> Result<(String, i64)> {
        if self.om {
            let r = self.q("OM0;", "OM0")?;
            return Ok((om_mode_name(r.chars().nth(3).unwrap_or('2')).into(), 0));
        }
        let r = self.q("MD;", "MD")?;
        let c = r.chars().nth(2).unwrap_or('2');
        let (name, pb) = match self.v {
            Variant::Elecraft => {
                let pb = self
                    .q("BW;", "BW")
                    .ok()
                    .and_then(|b| num(&b, 2, None).ok())
                    .map(|n| n * 10)
                    .unwrap_or(0);
                let name = match c {
                    '6' => "PKTUSB",
                    '9' => "PKTLSB",
                    _ => mode_name(c, false),
                };
                (name, pb)
            }
            Variant::Kenwood => {
                let data = self
                    .q("DA;", "DA")
                    .map(|d| d.ends_with('1'))
                    .unwrap_or(false);
                (mode_name(c, data), 0)
            }
            Variant::Plain => (mode_name(c, false), 0),
        };
        Ok((name.into(), pb))
    }

    fn set_mode(&mut self, mode: &str, passband_hz: i64) -> Result<()> {
        let (code, data) = match mode {
            "LSB" => ('1', false),
            "USB" => ('2', false),
            "CW" => ('3', false),
            "FM" => ('4', false),
            "AM" => ('5', false),
            "RTTY" => ('6', false),
            "CWR" => ('7', false),
            "RTTYR" => ('9', false),
            "PKTLSB" => ('1', true),
            "PKTUSB" => ('2', true),
            "PKTFM" => ('4', true),
            _ => return Err(unsupported(mode)),
        };
        if self.om {
            let c = om_code(code, data).ok_or_else(|| unsupported(mode))?;
            return self.s(&format!("OM0{c};"));
        }
        match self.v {
            Variant::Elecraft => {
                if data {
                    // DATA (or DATA-REV) mode, sub-mode DATA A (sound card).
                    self.s(if code == '1' { "MD9;" } else { "MD6;" })?;
                    self.s("DT0;")?;
                } else {
                    self.s(&format!("MD{code};"))?;
                }
                if passband_hz > 0 {
                    self.s(&format!("BW{:04};", (passband_hz / 10).min(9999)))?;
                }
            }
            Variant::Kenwood => {
                self.s(&format!("MD{code};"))?;
                self.s(if data { "DA1;" } else { "DA0;" })?;
            }
            Variant::Plain => self.s(&format!("MD{code};"))?,
        }
        Ok(())
    }

    fn ptt(&mut self) -> Result<bool> {
        let r = self.info()?;
        Ok(r.chars().nth(28) == Some('1'))
    }

    fn set_ptt(&mut self, on: bool) -> Result<()> {
        // TX1 on Kenwood transmits the USB/ACC audio (data), not the mic.
        match (on, self.v) {
            (true, Variant::Kenwood) => self.s("TX1;"),
            (true, _) => self.s("TX;"),
            // Confirmed: see `unkey_confirmed`.
            (false, _) => unkey_confirmed(self, |k| k.s("RX;")),
        }
    }

    fn get_level(&mut self, name: &str) -> Result<f32> {
        if let Some((cmd, prefix, full, _)) = self.level_spec(name) {
            let r = self.q(cmd, prefix)?;
            return Ok(num(&r, prefix.len(), None)? as f32 / full);
        }
        let e = self.elecraft();
        Ok(match name {
            "RFPOWER" => {
                let r = self.q("PC;", "PC")?;
                num(&r, 2, Some(3))? as f32 / self.max_w.max(1.0)
            }
            "STRENGTH" => {
                let t = self.sm;
                let r = match self.v {
                    Variant::Elecraft => self.q("SM;", "SM")?,
                    _ => self.q("SM0;", "SM0")?,
                };
                let skip = r.len().saturating_sub(4);
                cal(t, num(&r, skip, None)? as f32)
            }
            "SWR" if e => num(&self.q("SW;", "SW")?, 2, None)? as f32 / 10.0,
            "SWR" if self.v == Variant::Kenwood => cal(
                &[
                    (0.0, 1.0),
                    (6.0, 1.5),
                    (10.0, 2.0),
                    (15.0, 3.0),
                    (30.0, 10.0),
                ],
                self.meter(1)?,
            ),
            "ALC" if self.v == Variant::Kenwood => (self.meter(3)? / 30.0).min(1.0),
            "PREAMP" if self.v != Variant::Plain => {
                if self.q("PA;", "PA")?.chars().nth(2) == Some('1') {
                    10.0
                } else {
                    0.0
                }
            }
            "ATT" if self.v != Variant::Plain => {
                let r = self.q("RA;", "RA")?;
                if num(&r, 2, Some(2))? != 0 { 10.0 } else { 0.0 }
            }
            _ => return Err(unsupported(name)),
        })
    }

    fn set_level(&mut self, name: &str, v: f32) -> Result<()> {
        if let Some((_, prefix, full, digits)) = self.level_spec(name) {
            let n = (v.clamp(0.0, 1.0) * full).round() as u32;
            return self.s(&format!("{prefix}{n:0digits$};"));
        }
        match name {
            // Three digits: a max power set above 999 W (allowed up to
            // 2000 for other radios) would make "PC2000;", which the radio
            // misreads.
            "RFPOWER" => self.s(&format!(
                "PC{:03};",
                ((v.clamp(0.0, 1.0) * self.max_w).round() as u32).min(999)
            )),
            "PREAMP" if self.v != Variant::Plain => self.s(&format!("PA{};", (v > 0.0) as u8)),
            "ATT" if self.v != Variant::Plain => self.s(&format!("RA{:02};", (v > 0.0) as u8)),
            _ => Err(unsupported(name)),
        }
    }

    fn get_func(&mut self, name: &str) -> Result<bool> {
        let (cmd, prefix) = match (name, self.v) {
            ("NB", Variant::Kenwood | Variant::Elecraft) => ("NB;", "NB"),
            ("NR", Variant::Kenwood | Variant::Elecraft) => ("NR;", "NR"),
            ("VOX", Variant::Kenwood | Variant::Elecraft) => ("VX;", "VX"),
            ("ANF", Variant::Kenwood) => ("BC;", "BC"),
            ("COMP", Variant::Kenwood) => ("PR;", "PR"),
            ("TUNER", Variant::Kenwood) => {
                let r = self.q("AC;", "AC")?;
                return Ok(r.chars().nth(3) == Some('1'));
            }
            _ => return Err(unsupported(name)),
        };
        let r = self.q(cmd, prefix)?;
        Ok(r.chars().nth(prefix.len()).is_some_and(|c| c != '0'))
    }

    fn set_func(&mut self, name: &str, on: bool) -> Result<()> {
        let p = match (name, self.v) {
            ("NB", Variant::Kenwood | Variant::Elecraft) => "NB",
            ("NR", Variant::Kenwood | Variant::Elecraft) => "NR",
            ("VOX", Variant::Kenwood | Variant::Elecraft) => "VX",
            ("ANF", Variant::Kenwood) => "BC",
            ("COMP", Variant::Kenwood) => "PR",
            ("TUNER", Variant::Kenwood) => return self.s(if on { "AC110;" } else { "AC000;" }),
            _ => return Err(unsupported(name)),
        };
        self.s(&format!("{p}{};", on as u8))
    }

    fn vfo_op(&mut self, op: &str) -> Result<()> {
        match (op, self.v) {
            ("TUNE", Variant::Kenwood) => self.s("AC111;"),
            ("CPY", Variant::Kenwood) => self.s("VV;"),
            _ => Err(unsupported(op)),
        }
    }

    fn vfo(&mut self) -> Result<String> {
        Ok(match self.q("FR;", "FR") {
            Ok(r) if r.ends_with('1') => "VFOB".into(),
            _ => "VFOA".into(),
        })
    }

    fn set_vfo(&mut self, vfo: &str) -> Result<()> {
        // The K3/KX always receives on VFO A: FR1 is ignored and FT1 turns
        // split on (transmitting on B), which isn't what was asked.
        if self.v == Variant::Elecraft && !self.name.contains("K4") && vfo == "VFOB" {
            return Err(anyhow!("{}: it always receives on VFO A (use split to transmit on B)", self.name));
        }
        let n = (vfo == "VFOB") as u8;
        self.s(&format!("FR{n};"))?;
        self.s(&format!("FT{n};"))
    }

    fn split(&mut self) -> Result<bool> {
        let rx = self.q("FR;", "FR")?;
        let tx = self.q("FT;", "FT")?;
        Ok(rx.chars().nth(2) != tx.chars().nth(2))
    }

    fn set_split(&mut self, on: bool) -> Result<()> {
        let rx = self
            .q("FR;", "FR")
            .map(|r| r.ends_with('1'))
            .unwrap_or(false);
        self.s(&format!("FT{};", (rx != on) as u8))
    }

    fn offset(&mut self, xit: bool) -> Result<i32> {
        let r = self.info()?;
        let on = r.chars().nth(if xit { 24 } else { 23 }) == Some('1');
        if !on {
            return Ok(0);
        }
        Ok(num(&r, 18, Some(5))? as i32)
    }

    fn set_offset(&mut self, xit: bool, hz: i32) -> Result<()> {
        match self.v {
            Variant::Elecraft => self.s(&format!(
                "RO{}{:04};",
                if hz < 0 { '-' } else { '+' },
                hz.unsigned_abs().min(9999)
            ))?,
            _ => {
                self.s("RC;")?;
                if hz != 0 {
                    self.s(&format!(
                        "{}{:05};",
                        if hz > 0 { "RU" } else { "RD" },
                        hz.unsigned_abs().min(99999)
                    ))?;
                }
            }
        }
        self.s(&format!(
            "{}{};",
            if xit { "XT" } else { "RT" },
            (hz != 0) as u8
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Just enough of a TS-890 for its mode: FA, and OM0 (no MD, no DA).
    struct Ts990 {
        mode: char,
        inb: Vec<u8>,
        out: Vec<u8>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Wire for Ts990 {
        fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
            self.inb.extend_from_slice(b);
            while let Some(i) = self.inb.iter().position(|&c| c == b';') {
                let c: Vec<u8> = self.inb.drain(..=i).collect();
                let cmd = String::from_utf8_lossy(&c[..c.len() - 1]).to_string();
                self.log.lock().unwrap().push(cmd.clone());
                let reply = match cmd.as_str() {
                    "FA" => "FA00014074000;".to_string(),
                    "OM0" => format!("OM0{};", self.mode),
                    s if s.starts_with("OM0") && s.len() == 4 => {
                        self.mode = s.chars().nth(3).unwrap();
                        String::new()
                    }
                    _ => "?;".to_string(),
                };
                self.out.extend(reply.as_bytes());
            }
            Ok(())
        }
        fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.out.len());
            buf[..n].copy_from_slice(&self.out[..n]);
            self.out.drain(..n);
            Ok(n)
        }
        fn clear_input(&mut self) {
            self.out.clear();
        }
    }

    /// A radio scripted by a closure: command (without ';') → reply.
    pub(crate) struct Script<F: FnMut(&str) -> String + Send> {
        pub f: F,
        pub inb: Vec<u8>,
        pub out: Vec<u8>,
    }
    impl<F: FnMut(&str) -> String + Send> Wire for Script<F> {
        fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
            self.inb.extend_from_slice(b);
            while let Some(i) = self.inb.iter().position(|&c| c == b';') {
                let c: Vec<u8> = self.inb.drain(..=i).collect();
                let cmd = String::from_utf8_lossy(&c[..c.len() - 1]).to_string();
                let r = (self.f)(&cmd);
                self.out.extend(r.as_bytes());
            }
            Ok(())
        }
        fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.out.len());
            buf[..n].copy_from_slice(&self.out[..n]);
            self.out.drain(..n);
            Ok(n)
        }
        fn clear_input(&mut self) {
            self.out.clear();
        }
    }

    /// A TS-590 whose IF says it's transmitting: RX it ignores ('?', busy).
    fn ts590(obeys_rx: bool) -> Kenwood {
        let mut tx = true;
        let f = move |c: &str| -> String {
            match c {
                "FA" => "FA00014074000;".into(),
                "RX" if obeys_rx => {
                    tx = false;
                    String::new()
                }
                "IF" => format!("IF00014074000     +000000000{}2000000 ;", if tx { 1 } else { 0 }),
                _ => "?;".into(),
            }
        };
        Kenwood::new(Box::new(Script { f, inb: vec![], out: vec![] }), "023", false, 100.0).unwrap()
    }

    /// An unkey the radio didn't take is an error (the engine keeps
    /// trying), not a success that leaves it on the air.
    #[test]
    fn a_refused_unkey_is_an_error() {
        let mut d = ts590(false);
        assert!(d.ptt().unwrap(), "scripted as transmitting");
        assert!(d.set_ptt(false).is_err());
        let mut d = ts590(true);
        d.set_ptt(false).unwrap();
        assert!(!d.ptt().unwrap());
    }

    /// ID022 is the TS-990S (not supported over direct CAT: no IF, own
    /// TX/RX), ID024 the TS-890S.
    #[test]
    fn kenwood_ids() {
        assert_eq!(model_name("022"), "TS-990S");
        assert_eq!(model_name("024"), "TS-890S");
        let f = |c: &str| -> String { if c == "FA" { "FA00014074000;".into() } else { "?;".into() } };
        let e = Kenwood::new(Box::new(Script { f, inb: vec![], out: vec![] }), "022", false, 100.0).err().unwrap();
        assert!(e.to_string().contains("TS-990S isn't supported over direct CAT"), "{e}");
    }

    #[test]
    fn ts890_uses_om() {
        let log = Arc::new(Mutex::new(vec![]));
        let w = Ts990 {
            mode: '2',
            inb: vec![],
            out: vec![],
            log: log.clone(),
        };
        let mut d = Kenwood::new(Box::new(w), "024", false, 200.0).unwrap();
        assert!(d.describe().contains("TS-890"));
        assert_eq!(d.mode().unwrap().0, "USB");
        for m in ["PKTUSB", "PKTLSB", "PKTFM", "LSB", "CW", "CWR", "AM", "FM", "RTTY", "USB"] {
            d.set_mode(m, 3000).unwrap();
            assert_eq!(d.mode().unwrap().0, m);
        }
        d.set_mode("PKTUSB", 0).unwrap();
        assert!(log.lock().unwrap().contains(&"OM0D".to_string()));
        assert!(
            !log.lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("MD") || c.starts_with("DA")),
            "{:?}",
            log.lock().unwrap()
        );
    }
}
