//! Finding the radio and running rigctld for it.
//!
//! **Detection is passive**: it reads the USB descriptions the operating
//! system already has and never opens a port, because opening a serial port
//! raises RTS/DTR, which on some radios keys the transmitter. Radios
//! recognised:
//!
//! * Icom: the radio's name is in the CP210x serial number ("IC-7300
//!   03001234"); the IC-705 and IC-905 have Icom's own USB IDs.
//! * Yaesu: the CP2105 dual UART (the "Enhanced" port carries CAT).
//! * Kenwood, Elecraft, QRP Labs, Xiegu and others: by USB product names,
//!   or as "a USB serial radio, choose the model".
//!
//! The model name is matched against Hamlib's own list (`rigctl -l`), so
//! model numbers are never hard-coded. Whistlestop can then run `rigctld`
//! itself, with RTS and DTR held off, and both Whistlestop and Mercury share
//! it.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct HamlibModel {
    pub id: u32,
    pub mfg: String,
    pub model: String,
}

impl HamlibModel {
    pub fn label(&self) -> String {
        format!("{} {} (#{})", self.mfg, self.model, self.id)
    }
}

/// Where rigctld and rigctl live (Homebrew, MacPorts, Linux, the Windows
/// Hamlib installer, or PATH).
pub fn hamlib_bin(name: &str) -> Option<String> {
    let mut dirs: Vec<std::path::PathBuf> = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/opt/local/bin",
        "/usr/bin",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .collect();
    if cfg!(windows) {
        // The Hamlib installer uses C:\Program Files\hamlib-w64-<version>\bin.
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var)
                && let Ok(rd) = std::fs::read_dir(pf)
            {
                for e in rd.flatten() {
                    if e.file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .starts_with("hamlib")
                    {
                        dirs.push(e.path().join("bin"));
                    }
                }
            }
        }
    }
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    dirs.into_iter()
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
        .or_else(|| on_path(name))
        .map(|p| p.to_string_lossy().to_string())
}

/// A program on PATH (with `.exe` on Windows).
pub fn on_path(name: &str) -> Option<std::path::PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// Parse `rigctl -l`.
pub fn parse_model_list(text: &str) -> Vec<HamlibModel> {
    let mut out = vec![];
    for line in text.lines().skip(1) {
        // Columns: Rig #, Mfg, Model, Version, Status, Macro -- separated by
        // runs of 2+ spaces.
        let cols: Vec<&str> = line
            .split("  ")
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();
        if cols.len() >= 3
            && let Ok(id) = cols[0].parse::<u32>()
        {
            out.push(HamlibModel {
                id,
                mfg: cols[1].to_string(),
                model: cols[2].to_string(),
            });
        }
    }
    out
}

/// Hamlib's radio list (cached after the first call). Empty if Hamlib isn't
/// installed.
pub fn hamlib_models() -> &'static [HamlibModel] {
    static LIST: OnceLock<Vec<HamlibModel>> = OnceLock::new();
    LIST.get_or_init(|| {
        let Some(bin) = hamlib_bin("rigctl") else {
            return vec![];
        };
        Command::new(bin)
            .arg("-l")
            .output()
            .map(|o| parse_model_list(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsbSerial {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
}

/// USB serial ports, without opening any of them.
pub fn usb_serial_ports() -> Vec<UsbSerial> {
    // Radios the platform opens itself (the Android app's USB host).
    let v = crate::cat::usb_host().map(|h| h.list()).unwrap_or_default();
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let v = [v, usb_serial_ports_desktop()].concat();
    v
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn usb_serial_ports_desktop() -> Vec<UsbSerial> {
    let Ok(ports) = serialport::available_ports() else {
        return vec![];
    };
    let mut out: Vec<UsbSerial> = ports
        .into_iter()
        .filter_map(|p| match p.port_type {
            serialport::SerialPortType::UsbPort(u) => Some(UsbSerial {
                path: p.port_name,
                vid: u.vid,
                pid: u.pid,
                manufacturer: u.manufacturer.unwrap_or_default(),
                product: u.product.unwrap_or_default(),
                serial: u.serial_number.unwrap_or_default(),
            }),
            _ => None,
        })
        // macOS lists each port twice; the call-out device (cu.) is the one
        // to use.
        .filter(|u| !u.path.starts_with("/dev/tty.") || !cfg!(target_os = "macos"))
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Every serial port path (for choosing by hand), USB or not.
pub fn all_serial_paths() -> Vec<String> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    return vec![];
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    all_serial_paths_desktop()
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn all_serial_paths_desktop() -> Vec<String> {
    let mut v: Vec<String> = serialport::available_ports()
        .map(|ps| {
            ps.into_iter()
                .map(|p| p.port_name)
                .filter(|n| !n.starts_with("/dev/tty.") || !cfg!(target_os = "macos"))
                .collect()
        })
        .unwrap_or_default();
    v.retain(|p| !p.contains("Bluetooth") && !p.contains("debug-console"));
    v.sort();
    v
}

#[derive(Clone, Debug, PartialEq)]
pub struct RadioGuess {
    pub port: String,
    /// What the radio looks like, e.g. "Icom IC-7300" or "Yaesu (CP2105)".
    pub what: String,
    pub model: Option<HamlibModel>,
    /// Why we think so, for the operator.
    pub why: String,
}

/// Find a Hamlib model whose name appears in `text` (longest match wins, so
/// "IC-7300" beats "IC-730").
fn match_model<'a>(text: &str, models: &'a [HamlibModel]) -> Option<&'a HamlibModel> {
    let t = text.to_uppercase().replace(' ', "");
    models
        .iter()
        .filter(|m| {
            let name = m.model.to_uppercase().replace(' ', "");
            name.len() >= 4 && t.contains(&name)
        })
        .max_by_key(|m| m.model.len())
}

fn model_named<'a>(mfg: &str, name: &str, models: &'a [HamlibModel]) -> Option<&'a HamlibModel> {
    models
        .iter()
        .find(|m| m.mfg.eq_ignore_ascii_case(mfg) && m.model.eq_ignore_ascii_case(name))
}

/// Recognise radios among USB serial ports.
pub fn guess_radios(ports: &[UsbSerial], models: &[HamlibModel]) -> Vec<RadioGuess> {
    let mut out: Vec<RadioGuess> = vec![];
    for p in ports {
        let text = format!("{} {} {}", p.manufacturer, p.product, p.serial);
        let named = match_model(&text, models);
        let (what, model, why) = match (p.vid, p.pid) {
            // Icom's own USB (IC-705, IC-905).
            (0x0C26, _) => {
                let m = named.cloned();
                (
                    m.as_ref()
                        .map(|m| format!("Icom {}", m.model))
                        .unwrap_or("Icom".into()),
                    m,
                    "Icom USB device".to_string(),
                )
            }
            // Silicon Labs CP210x: Icom names the radio in the serial number.
            (0x10C4, 0xEA60) if named.is_some_and(|m| m.mfg.eq_ignore_ascii_case("Icom")) => {
                let m = named.cloned().unwrap();
                (
                    format!("Icom {}", m.model),
                    Some(m),
                    format!("serial number \"{}\"", p.serial),
                )
            }
            // CP2105 dual UART: Yaesu's radios. Two ports appear; CAT is the
            // first ("Enhanced"), the other carries RTS/DTR keying.
            (0x10C4, 0xEA70) => {
                if out.iter().any(|g| {
                    g.what.starts_with("Yaesu")
                        && g.port[..g.port.len().min(20)] == p.path[..p.path.len().min(20)]
                }) {
                    continue; // the second (Standard) port of the same radio
                }
                (
                    named
                        .map(|m| format!("Yaesu {}", m.model))
                        .unwrap_or("Yaesu radio (CP2105)".into()),
                    named.cloned(),
                    "CP2105 dual UART, as in Yaesu radios".into(),
                )
            }
            (0x10C4, 0xEA60) => (
                named
                    .map(|m| format!("{} {}", m.mfg, m.model))
                    .unwrap_or("Radio on a CP210x (Kenwood, Icom...)".into()),
                named.cloned(),
                "Silicon Labs CP210x, used by Kenwood and Icom radios".into(),
            ),
            _ if text.to_uppercase().contains("QMX") || text.to_uppercase().contains("QDX") => (
                "QRP Labs QMX/QDX".into(),
                model_named("Kenwood", "TS-480", models).cloned(),
                "QRP Labs radios speak Kenwood TS-480 CAT".into(),
            ),
            _ if named.is_some() => {
                let m = named.cloned().unwrap();
                (
                    format!("{} {}", m.mfg, m.model),
                    Some(m),
                    format!("USB name \"{}\"", text.trim()),
                )
            }
            // FTDI and CH340/CH9102 bridges: Elecraft, Xiegu, many cables.
            (0x0403, _) | (0x1A86, _) => (
                "USB serial (choose the model)".into(),
                None,
                format!("{} {}", p.manufacturer, p.product)
                    .trim()
                    .to_string(),
            ),
            // Anything else the phone's USB host found is a serial chip it
            // can drive; let the protocol probe work out the radio.
            _ if crate::cat::is_usb_host(&p.path) => (
                "USB serial (choose the model)".into(),
                None,
                format!("{} {}", p.manufacturer, p.product)
                    .trim()
                    .to_string(),
            ),
            _ => continue,
        };
        out.push(RadioGuess {
            port: p.path.clone(),
            what,
            model,
            why,
        });
    }
    out
}

pub fn detect() -> Vec<RadioGuess> {
    guess_radios(&usb_serial_ports(), hamlib_models())
}

/// How Whistlestop reaches the radio.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum RigMode {
    /// A rig control server that is already running (Hamlib rigctld, or
    /// RigHelm's built-in server) at `Config::rigctld`.
    #[default]
    External,
    /// Whistlestop runs rigctld itself for this radio.
    Managed,
    /// Whistlestop talks to the radio itself (its own CAT drivers, no
    /// Hamlib), and shares it with Mercury over a built-in rigctld server.
    Direct,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ManagedRig {
    /// Hamlib model number; 0 = detect at start.
    pub model: u32,
    /// Serial port; empty = detect at start.
    pub port: String,
    /// Serial speed; 0 = the radio's default in Hamlib.
    pub baud: u32,
    /// TCP port rigctld listens on (Mercury should use the same).
    pub listen_port: u16,
}

impl Default for ManagedRig {
    fn default() -> Self {
        Self {
            model: 0,
            port: String::new(),
            baud: 0,
            listen_port: 4532,
        }
    }
}

/// A rigctld that Whistlestop started; stopped when dropped.
pub struct RigctldProcess {
    child: Child,
    pub addr: String,
    pub description: String,
}

impl Drop for RigctldProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start rigctld for `m`. RTS and DTR are held off so opening the port can't
/// key the radio.
pub fn start_rigctld(m: &ManagedRig) -> Result<RigctldProcess> {
    let bin = hamlib_bin("rigctld")
        .ok_or_else(|| anyhow!("Hamlib's rigctld isn't installed (macOS: brew install hamlib)"))?;
    let (model, port, what) = if m.model == 0 || m.port.is_empty() {
        let g = detect()
            .into_iter()
            .find(|g| g.model.is_some())
            .ok_or_else(|| {
                anyhow!(
                    "no radio detected on USB: plug it in, or choose the model and port in Settings"
                )
            })?;
        let mm = g.model.clone().unwrap();
        (
            if m.model == 0 { mm.id } else { m.model },
            if m.port.is_empty() {
                g.port.clone()
            } else {
                m.port.clone()
            },
            g.what.clone(),
        )
    } else {
        let name = hamlib_models()
            .iter()
            .find(|x| x.id == m.model)
            .map(|x| x.label())
            .unwrap_or(format!("Hamlib model {}", m.model));
        (m.model, m.port.clone(), name)
    };
    let mut cmd = Command::new(&bin);
    cmd.args([
        "-m",
        &model.to_string(),
        "-r",
        &port,
        "-t",
        &m.listen_port.to_string(),
        "-T",
        "127.0.0.1",
    ])
    .args(["--set-conf=rts_state=OFF,dtr_state=OFF"]);
    if m.baud > 0 {
        cmd.args(["-s", &m.baud.to_string()]);
    }
    let child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {bin}"))?;
    Ok(RigctldProcess {
        child,
        addr: format!("127.0.0.1:{}", m.listen_port),
        description: format!("{what} on {port}"),
    })
}

/// Start a rigctld for Hamlib's Dummy radio (for tests and trying things).
pub fn start_dummy(listen_port: u16) -> Result<RigctldProcess> {
    let bin = hamlib_bin("rigctld").ok_or_else(|| anyhow!("rigctld not installed"))?;
    let child = Command::new(bin)
        .args(["-m", "1", "-t", &listen_port.to_string(), "-T", "127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(RigctldProcess {
        child,
        addr: format!("127.0.0.1:{listen_port}"),
        description: "Hamlib Dummy".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str =
        " Rig #  Mfg                    Model                   Version         Status      Macro
     1  Hamlib                 Dummy                   20240709.0      Stable      RIG_MODEL_DUMMY
  1035  Yaesu                  FT-991                  20241118.18     Stable      RIG_MODEL_FT991
  2028  Kenwood                TS-480                  20250515.6      Stable      RIG_MODEL_TS480
  3066  Icom                   IC-730                  20200101.0      Untested    RIG_MODEL_IC730
  3073  Icom                   IC-7300                 20250515.17     Stable      RIG_MODEL_IC7300
  3085  Icom                   IC-705                  20250515.13     Stable      RIG_MODEL_IC705
";

    fn port(path: &str, vid: u16, pid: u16, mfg: &str, product: &str, serial: &str) -> UsbSerial {
        UsbSerial {
            path: path.into(),
            vid,
            pid,
            manufacturer: mfg.into(),
            product: product.into(),
            serial: serial.into(),
        }
    }

    #[test]
    fn parses_hamlib_list() {
        let m = parse_model_list(LIST);
        assert_eq!(m.len(), 6);
        assert_eq!(
            m[4],
            HamlibModel {
                id: 3073,
                mfg: "Icom".into(),
                model: "IC-7300".into()
            }
        );
    }

    #[test]
    fn recognises_radios_without_opening_ports() {
        let models = parse_model_list(LIST);
        let ports = vec![
            port(
                "/dev/cu.SLAB_USBtoUART",
                0x10C4,
                0xEA60,
                "Silicon Labs",
                "CP2102 USB to UART Bridge Controller",
                "IC-7300 03001234",
            ),
            port(
                "/dev/cu.usbmodem14201",
                0x0C26,
                0x0036,
                "Icom Inc.",
                "IC-705",
                "12001234",
            ),
            port(
                "/dev/cu.SLAB_USBtoUART5",
                0x10C4,
                0xEA70,
                "Silicon Labs",
                "CP2105 Dual USB to UART Bridge Controller",
                "00A1B2C3",
            ),
            port(
                "/dev/cu.SLAB_USBtoUART6",
                0x10C4,
                0xEA70,
                "Silicon Labs",
                "CP2105 Dual USB to UART Bridge Controller",
                "00A1B2C3",
            ),
            port(
                "/dev/cu.usbmodem1",
                0x0483,
                0xA34C,
                "QRP Labs",
                "QMX Transceiver",
                "1",
            ),
            port(
                "/dev/cu.usbserial-A1",
                0x0403,
                0x6001,
                "FTDI",
                "FT232R USB UART",
                "A1",
            ),
            port(
                "/dev/cu.somethingelse",
                0x05AC,
                0x1234,
                "Apple",
                "Keyboard",
                "",
            ),
        ];
        let g = guess_radios(&ports, &models);
        assert_eq!(g[0].model.as_ref().unwrap().id, 3073, "{g:?}"); // IC-7300 beats IC-730
        assert_eq!(g[1].model.as_ref().unwrap().id, 3085);
        assert!(g[2].what.starts_with("Yaesu") && g[2].model.is_none());
        assert_eq!(
            g.iter().filter(|x| x.what.starts_with("Yaesu")).count(),
            1,
            "second CP2105 port is the same radio"
        );
        assert_eq!(g[3].model.as_ref().unwrap().id, 2028); // QMX speaks TS-480
        assert!(g[4].model.is_none() && g[4].what.contains("choose"));
        assert_eq!(g.len(), 5, "the keyboard isn't a radio");
    }
}
