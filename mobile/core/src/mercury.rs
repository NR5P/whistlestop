//! Rhizomatica's Mercury modem, running inside the app (GPL-3.0).
//!
//! Mercury is unchanged: its engine starts through its own embedding bridge
//! (`mercury_bridge.h`, the one its own phone apps use) with the same
//! command-line options as the desktop daemon, and serves the same TNC
//! ports on this phone. Whistlestop's engine then talks to it exactly as it
//! does to a Mercury on a desktop.

use std::ffi::{CString, c_char, c_int};
use std::path::Path;
use std::sync::Mutex;

unsafe extern "C" {
    fn mercury_init(
        argc: c_int,
        argv: *mut *mut c_char,
        default_config: *const c_char,
        log_path: *const c_char,
    ) -> c_int;
    fn mercury_request_shutdown();
    fn mercury_shutdown();
    #[cfg(target_os = "android")]
    fn mercury_ui_set_android_audio_devices(list: *const c_char);
}

/// A running Mercury, and everything `mercury_init` was handed: Mercury
/// may keep any of those pointers (argv, its strings, the config and log
/// paths) for as long as it runs, so they live here until it stops.
struct Running {
    _args: Vec<CString>,
    _argv: Vec<*mut c_char>,
    _cfg: CString,
    _log: CString,
    port: u16,
    bcast: u16,
    /// Where it keys the radio (a rigctld), if anywhere.
    ptt: Option<String>,
}

// SAFETY: the raw pointers in `_argv` point into `_args`, which this struct
// owns; Rust never reads or writes through them, so moving the struct to
// another thread (behind RUNNING's mutex) is fine.
unsafe impl Send for Running {}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

fn running_slot() -> std::sync::MutexGuard<'static, Option<Running>> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

/// The sound system Mercury uses on this phone.
pub const SOUND: &str = if cfg!(target_os = "android") { "aaudio" } else { "coreaudio" };

/// Start Mercury with its TNC on `port` (control; data is port + 1) and
/// broadcast on `bcast`, its settings and log in `dir`. `input`/`output`
/// pick sound devices ("" = the phone's default route). With `ptt`
/// (`host:port` of a rigctld, normally Whistlestop's own), Mercury keys the
/// radio there (Hamlib model 2, NET rigctl); without it, it doesn't key.
pub fn start(
    dir: &Path,
    port: u16,
    bcast: u16,
    input: &str,
    output: &str,
    ptt: Option<&str>,
) -> anyhow::Result<()> {
    let mut g = running_slot();
    if g.is_some() {
        return Ok(());
    }
    let mut args: Vec<String> = vec![
        "mercury".into(),
        "-x".into(),
        SOUND.into(),
        "-p".into(),
        port.to_string(),
        "-b".into(),
        bcast.to_string(),
        // The phone's own UI is Whistlestop; Mercury's spectrum isn't needed.
        "-W".into(),
    ];
    if let Some(addr) = ptt {
        args.extend(["-R".into(), "2".into(), "-A".into(), addr.into()]);
    }
    if !input.is_empty() {
        args.extend(["-i".into(), input.into()]);
    }
    if !output.is_empty() {
        args.extend(["-o".into(), output.into()]);
    }
    let ini = dir.join("mercury.ini");
    if !ini.exists() {
        // Mercury reads its settings from here; it writes what it changes.
        let _ = std::fs::write(&ini, "");
    }
    args.extend(["-C".into(), ini.to_string_lossy().into_owned()]);
    let c: Vec<CString> = args
        .iter()
        .map(|a| CString::new(a.as_str()))
        .collect::<Result<_, _>>()?;
    let mut argv: Vec<*mut c_char> = c.iter().map(|s| s.as_ptr() as *mut c_char).collect();
    argv.push(std::ptr::null_mut());
    let cfg = CString::new(ini.to_string_lossy().as_bytes())?;
    let log = CString::new(dir.join("mercury.log").to_string_lossy().as_bytes())?;
    let rc = unsafe { mercury_init(c.len() as c_int, argv.as_mut_ptr(), cfg.as_ptr(), log.as_ptr()) };
    if rc != 0 {
        anyhow::bail!("Mercury didn't start ({rc}); see mercury.log");
    }
    *g = Some(Running {
        _args: c,
        _argv: argv,
        _cfg: cfg,
        _log: log,
        port,
        bcast,
        ptt: ptt.map(str::to_owned),
    });
    Ok(())
}

pub fn running() -> bool {
    running_slot().is_some()
}

/// The (TNC, broadcast) ports of the Mercury running in the app.
pub fn ports() -> Option<(u16, u16)> {
    running_slot().as_ref().map(|r| (r.port, r.bcast))
}

/// Where the running Mercury keys the radio.
pub fn ptt() -> Option<String> {
    running_slot().as_ref().and_then(|r| r.ptt.clone())
}

pub fn stop() {
    let mut g = running_slot();
    if g.is_some() {
        unsafe {
            mercury_request_shutdown();
            mercury_shutdown();
        }
        // Only now that Mercury has shut down may its arguments go.
        *g = None;
    }
}

/// Android: the phone's sound devices (from AudioManager), one per line,
/// "id\tin|out\tname"; Mercury then offers them as "dev:<id>".
#[cfg(target_os = "android")]
pub fn set_android_audio_devices(list: &str) {
    if let Ok(c) = CString::new(list) {
        unsafe { mercury_ui_set_android_audio_devices(c.as_ptr()) }
    }
}
