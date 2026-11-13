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

/// How to install Hamlib on this computer, for messages.
pub fn hamlib_install_hint() -> &'static str {
    if cfg!(windows) {
        "install Hamlib for Windows (hamlib-w64 from github.com/Hamlib/Hamlib/releases)"
    } else if cfg!(target_os = "macos") {
        "brew install hamlib"
    } else {
        "install your distribution's hamlib (libhamlib-utils) package"
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
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    // Fixed-width columns, placed by the header: a maker's name can fill
    // its column with no gap before the model ("DTTS Microwave Society
    // DttSP IPC"), and a model can be empty ("FLRig").
    let (mfg_at, model_at) = match (header.find("Mfg"), header.find("Model")) {
        (Some(a), Some(b)) if a < b => (a, b),
        _ => (0, 0),
    };
    let mut out = vec![];
    for line in lines {
        let mut t = line.split_whitespace();
        let Some(Ok(id)) = t.next().map(str::parse::<u32>) else { continue };
        // The version (a date, "20200319.0") ends the model.
        let is_version = |w: &str| {
            let (d, rest) = w.split_once('.').unwrap_or((w, ""));
            d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()) && rest.bytes().all(|b| b.is_ascii_digit())
        };
        let version_at = line
            .match_indices(|c: char| c.is_ascii_digit())
            .map(|(i, _)| i)
            .find(|&i| {
                (i == 0 || line.as_bytes()[i - 1] == b' ')
                    && is_version(line[i..].split_whitespace().next().unwrap_or(""))
                    && i > mfg_at
            })
            .unwrap_or(line.len());
        let (mfg, model) = if model_at > 0 && line.len() > mfg_at && line.is_char_boundary(mfg_at) && line.is_char_boundary(model_at.min(line.len())) {
            let end = model_at.min(version_at);
            let mfg = line.get(mfg_at..end).unwrap_or("").trim();
            let model = line.get(model_at.min(version_at)..version_at).unwrap_or("").trim();
            (mfg.to_string(), model.to_string())
        } else {
            // No header: columns separated by runs of 2+ spaces.
            let cols: Vec<&str> = line.split("  ").map(str::trim).filter(|c| !c.is_empty()).collect();
            if cols.len() < 3 {
                continue;
            }
            (cols[1].to_string(), cols[2].to_string())
        };
        if mfg.is_empty() {
            continue;
        }
        out.push(HamlibModel { id, mfg, model });
    }
    out
}

/// Hamlib's radio list (cached after the first call). Empty if Hamlib isn't
/// installed.
pub fn hamlib_models() -> &'static [HamlibModel] {
    // Kept once read; an empty list (Hamlib not installed yet) isn't kept,
    // so installing it while Whistlestop runs is noticed.
    static LIST: OnceLock<Vec<HamlibModel>> = OnceLock::new();
    if let Some(l) = LIST.get() {
        return l;
    }
    let read = hamlib_bin("rigctl")
        .and_then(|bin| Command::new(bin).arg("-l").output().ok())
        .map(|o| parse_model_list(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    if read.is_empty() {
        return &[];
    }
    LIST.get_or_init(|| read)
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsbSerial {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    /// The USB interface the port belongs to, where the platform says
    /// (a CP2105's two ports are interfaces 0 and 1).
    pub interface: Option<u8>,
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
                interface: u.interface,
            }),
            _ => None,
        })
        // macOS lists each port twice; the call-out device (cu.) is the one
        // to use.
        .filter(|u| !u.path.starts_with("/dev/tty.") || !cfg!(target_os = "macos"))
        .collect();
    // Linux numbers ttyUSB/ttyACM in the order devices appear, so after a
    // reboot or a replug two can swap (and CAT would open the other one,
    // maybe its keying port): use the stable /dev/serial/by-id name.
    #[cfg(target_os = "linux")]
    for u in out.iter_mut() {
        if let Some(stable) = by_id_name(&u.path) {
            u.path = stable;
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The /dev/serial/by-id name of a Linux serial device, if udev made one.
#[cfg(target_os = "linux")]
fn by_id_name(dev: &str) -> Option<String> {
    let want = std::fs::canonicalize(dev).ok()?;
    std::fs::read_dir("/dev/serial/by-id")
        .ok()?
        .flatten()
        .find(|e| std::fs::canonicalize(e.path()).ok().as_ref() == Some(&want))
        .map(|e| e.path().to_string_lossy().into_owned())
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
    // CP2105 ports kept so far (path, USB serial number).
    let mut cp2105: Vec<(&str, &str)> = vec![];
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
                // The second (Standard) port of the radio carries RTS/DTR
                // keying, not CAT: by its interface number where the
                // platform gives it; else the same USB serial number, or
                // (macOS, no serial) the same path prefix as one kept.
                let second = match p.interface {
                    Some(i) => i != 0,
                    None => cp2105.iter().any(|(path, serial)| {
                        if !p.serial.is_empty() {
                            *serial == p.serial
                        } else {
                            path[..path.len().min(20)] == p.path[..p.path.len().min(20)]
                        }
                    }),
                };
                if second {
                    continue;
                }
                cp2105.push((&p.path, &p.serial));
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
        let _ = std::fs::remove_file(rigctld_pid_file());
    }
}

/// Where the pid of the rigctld Whistlestop runs is kept, so the next start
/// can stop one left running by a Whistlestop that ended without quitting
/// (a crash, a force-quit, or a quit that didn't wait for it).
fn rigctld_pid_file() -> std::path::PathBuf {
    crate::config::path().with_file_name("rigctld.pid")
}

/// Stop the rigctld a previous Whistlestop started and left running (two
/// on one radio interleave their CAT traffic).
fn stop_leftover_rigctld() {
    let f = rigctld_pid_file();
    let Some(pid) = std::fs::read_to_string(&f).ok().and_then(|s| s.trim().parse::<u32>().ok()) else {
        return;
    };
    let _ = std::fs::remove_file(&f);
    #[cfg(unix)]
    {
        let cmd = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if cmd.contains("rigctld") {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Model and port for a managed rigctld: what's set, completed from the
/// *same* detected radio (never the model of one and the port of another).
pub fn pick_managed(m: &ManagedRig, found: &[RadioGuess]) -> Result<(u32, String, String)> {
    let label = |id: u32| {
        hamlib_models()
            .iter()
            .find(|x| x.id == id)
            .map(|x| x.label())
            .unwrap_or(format!("Hamlib model {id}"))
    };
    match (m.model, m.port.trim()) {
        (0, "") => {
            let g = found.iter().find(|g| g.model.is_some()).ok_or_else(|| {
                anyhow!("no radio detected on USB: plug it in, or choose the model and port in Settings")
            })?;
            Ok((g.model.as_ref().unwrap().id, g.port.clone(), g.what.clone()))
        }
        (0, port) => {
            let g = found.iter().find(|g| g.port == port && g.model.is_some()).ok_or_else(|| {
                anyhow!("can't tell which radio is on {port}: choose its model in Settings → Radio connection")
            })?;
            Ok((g.model.as_ref().unwrap().id, port.to_string(), g.what.clone()))
        }
        (id, "") => {
            // The radio of that model, or the only radio there is (a Yaesu
            // on its CP2105 isn't named by USB).
            let g = found
                .iter()
                .find(|g| g.model.as_ref().is_some_and(|x| x.id == id))
                .or(if found.len() == 1 { found.first() } else { None })
                .ok_or_else(|| {
                    anyhow!("can't tell which port the {} is on: choose the port in Settings → Radio connection", label(id))
                })?;
            Ok((id, g.port.clone(), label(id)))
        }
        (id, port) => Ok((id, port.to_string(), label(id))),
    }
}

/// Start rigctld for `m`. RTS and DTR are held off so opening the port can't
/// key the radio.
pub fn start_rigctld(m: &ManagedRig) -> Result<RigctldProcess> {
    let bin = hamlib_bin("rigctld")
        .ok_or_else(|| anyhow!("Hamlib's rigctld isn't installed ({})", hamlib_install_hint()))?;
    let found = if m.model == 0 || m.port.trim().is_empty() { detect() } else { vec![] };
    let (model, port, what) = pick_managed(m, &found)?;
    stop_leftover_rigctld();
    // Something already serves this port (another rigctld, RigHelm's
    // server): a second would bind beside it and the two would share the
    // radio unseen.
    if std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], m.listen_port)),
        std::time::Duration::from_millis(300),
    )
    .is_ok()
    {
        anyhow::bail!(
            "port {} is already in use (another rigctld, or RigHelm's server?): stop it, choose another port, or pick \"An existing rigctld\" in Settings → Radio connection",
            m.listen_port
        );
    }
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
    // Its messages go to a log beside the settings, to say why if it stops.
    let log_path = crate::config::path().with_file_name("rigctld.log");
    let err = std::fs::File::create(&log_path).map(Stdio::from).unwrap_or(Stdio::null());
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(err)
        .spawn()
        .with_context(|| format!("starting {bin}"))?;
    let _ = std::fs::write(rigctld_pid_file(), child.id().to_string());
    // A wrong or busy port makes it exit at once: say so, with its reason.
    std::thread::sleep(std::time::Duration::from_millis(400));
    if let Ok(Some(status)) = child.try_wait() {
        let _ = std::fs::remove_file(rigctld_pid_file());
        let why = std::fs::read_to_string(&log_path).unwrap_or_default();
        let why = why.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no message");
        anyhow::bail!("rigctld for {what} on {port} stopped at once ({status}): {why}");
    }
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
            interface: None,
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

    /// A Yaesu's two CP2105 ports are one radio, on its CAT (Enhanced,
    /// interface 0) port, whatever the paths look like: by interface where
    /// the platform gives it, else by the USB serial number (Linux paths
    /// share no prefix, two radios' macOS paths may).
    #[test]
    fn cp2105_pairs_are_one_radio_on_its_cat_port() {
        let models = parse_model_list(LIST);
        let cp = |path: &str, serial: &str, interface: Option<u8>| UsbSerial {
            interface,
            ..port(path, 0x10C4, 0xEA70, "Silicon Labs", "CP2105 Dual USB to UART Bridge Controller", serial)
        };
        // By interface: the Standard port (1) listed first is still skipped.
        let g = guess_radios(&[cp("/dev/ttyUSB0", "X1", Some(1)), cp("/dev/ttyUSB1", "X1", Some(0))], &models);
        assert_eq!(g.len(), 1, "{g:?}");
        assert_eq!(g[0].port, "/dev/ttyUSB1");
        // Linux without interface numbers: same serial, one radio.
        let g = guess_radios(&[cp("/dev/ttyUSB0", "X1", None), cp("/dev/ttyUSB1", "X1", None)], &models);
        assert_eq!(g.len(), 1, "{g:?}");
        // Two radios: two serials, two radios.
        let g = guess_radios(
            &[cp("/dev/ttyUSB0", "X1", None), cp("/dev/ttyUSB1", "X1", None), cp("/dev/ttyUSB2", "Y2", None), cp("/dev/ttyUSB3", "Y2", None)],
            &models,
        );
        assert_eq!(g.len(), 2, "{g:?}");
    }

    /// A managed rigctld gets the model and the port of the same radio:
    /// never the model you chose with another radio's port.
    #[test]
    fn managed_model_and_port_come_from_one_radio() {
        let models = parse_model_list(LIST);
        let found = guess_radios(
            &[
                port("/dev/cu.SLAB_USBtoUART", 0x10C4, 0xEA60, "Silicon Labs", "CP2102", "IC-7300 03001234"),
                port("/dev/cu.SLAB_USBtoUART5", 0x10C4, 0xEA70, "Silicon Labs", "CP2105 Dual USB to UART Bridge Controller", "00A1"),
            ],
            &models,
        );
        let m = |model: u32, port: &str| ManagedRig { model, port: port.into(), ..Default::default() };
        // A model no detected radio is named as, with two radios: ask.
        assert!(pick_managed(&m(1036, ""), &found).is_err());
        // The IC-7300's own port for the IC-7300.
        assert_eq!(pick_managed(&m(3073, ""), &found).unwrap().1, "/dev/cu.SLAB_USBtoUART");
        // A port set, model automatic: that port's radio, or ask.
        assert_eq!(pick_managed(&m(0, "/dev/cu.SLAB_USBtoUART"), &found).unwrap().0, 3073);
        assert!(pick_managed(&m(0, "/dev/cu.SLAB_USBtoUART5"), &found).is_err(), "the Yaesu isn't named by USB");
        // Only one radio: a chosen model goes to its port.
        assert_eq!(pick_managed(&m(1036, ""), &found[1..]).unwrap().1, "/dev/cu.SLAB_USBtoUART5");
    }

    /// Rows of Hamlib 4.6's real list that split on spaces got wrong: an
    /// empty model, and a maker filling its column.
    #[test]
    fn parses_awkward_hamlib_rows() {
        let list = r" Rig #  Mfg                    Model                   Version         Status      Macro
     4  FLRig                                          20250107.0      Stable      
 23003  DTTS Microwave Society DttSP IPC               20200319.0      Stable      RIG_MODEL_DTTSP
  3073  Icom                   IC-7300                 20240916.0      Stable      RIG_MODEL_IC7300
";
        let m = parse_model_list(list);
        assert_eq!((m[0].id, m[0].mfg.as_str(), m[0].model.as_str()), (4, "FLRig", ""));
        assert_eq!((m[1].mfg.as_str(), m[1].model.as_str()), ("DTTS Microwave Society", "DttSP IPC"));
        assert_eq!((m[2].id, m[2].mfg.as_str(), m[2].model.as_str()), (3073, "Icom", "IC-7300"));
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

/// Is `port` a Yaesu radio's CAT port (a CP2105's Enhanced port, USB
/// interface 0)? Direct CAT then holds RTS high, as the radio's "CAT RTS"
/// handshake wants (RigHelm does the same). Never for a port the platform
/// can't place: on the other port of the pair, RTS keys the transmitter.
pub fn is_yaesu_cat_port(port: &str) -> bool {
    usb_serial_ports()
        .iter()
        .any(|u| u.path == port && (u.vid, u.pid) == (0x10C4, 0xEA70) && u.interface == Some(0))
}
