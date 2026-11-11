//! Whistlestop's engine for the phone apps.
//!
//! The SwiftUI (iPhone) and Jetpack Compose (Android) apps are native
//! screens over the same Rust engine the desktop app runs, so a phone and a
//! desktop always speak exactly the same whistle and protocol. The interface
//! is deliberately small and JSON-shaped: the engine's `State`, `Cmd` and
//! `Config` are already serde types (remote operation sends them over the
//! LAN), so the apps decode them with their own JSON decoders.
//!
//! C interface (Swift calls it directly; see `whistlestop.h`):
//!
//! | call | does |
//! |---|---|
//! | `ws_start(dir)` | load settings from `dir`, start the engine |
//! | `ws_changes()` | a counter that moves whenever the state changes |
//! | `ws_state()` | the whole state, JSON |
//! | `ws_rows()` | waterfall rows since the last call, JSON `[[f32]]` |
//! | `ws_send(json)` | one engine command (`Cmd`), JSON |
//! | `ws_config()` / `ws_set_config(json)` | settings, JSON; setting saves and applies |
//! | `ws_stop()` | stop the engine (and the embedded Mercury) |
//! | `ws_free(s)` | free a string returned above |
//!
//! Android reaches the same calls through JNI (`android` module below).

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use whistlestop::config::Config;
use whistlestop::engine::{self, Cmd, Handle};

#[cfg(feature = "mercury")]
pub mod mercury;

struct App {
    dir: PathBuf,
    h: Handle,
    cfg: Config,
    /// Waterfall rows already handed out.
    rows_seen: u64,
}

static APP: Mutex<Option<App>> = Mutex::new(None);
static CHANGES: AtomicU64 = AtomicU64::new(0);

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.lock().ok()?.as_mut().map(f)
}

fn out(s: String) -> *mut c_char {
    CString::new(s)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn arg(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok().map(str::to_owned)
}

// ------------------------------------------------------------ the calls

/// Start the engine with the settings kept in `dir` (the app's own
/// documents/files folder). Received files go to `dir/Downloads` unless
/// the settings say otherwise. Starting twice does nothing.
pub fn start(dir: &str) -> anyhow::Result<()> {
    let mut slot = APP.lock().unwrap();
    if slot.is_some() {
        return Ok(());
    }
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    // Settings live in the app's sandbox, not a desktop config folder.
    // SAFETY: set once, before the engine's threads exist.
    unsafe { std::env::set_var("WHISTLESTOP_CONFIG", dir.join("whistlestop.toml")) };
    let mut cfg = Config::load();
    if cfg.download_dir.as_os_str().is_empty() || !cfg.download_dir.starts_with(&dir) {
        cfg.download_dir = dir.join("Downloads");
    }
    let _ = std::fs::create_dir_all(&cfg.download_dir);
    ensure_mercury(&dir, &cfg);
    let h = engine::spawn(|| {
        CHANGES.fetch_add(1, Ordering::Relaxed);
    });
    h.send(Cmd::Start(cfg.clone()));
    *slot = Some(App {
        dir,
        h,
        cfg,
        rows_seen: 0,
    });
    Ok(())
}

/// A real station whose modem is this phone runs Mercury in the app. (The
/// demo station has its own simulated modem; a modem elsewhere on the
/// network needs nothing here.)
fn ensure_mercury(dir: &std::path::Path, cfg: &Config) {
    let local = matches!(cfg.mercury_host.as_str(), "127.0.0.1" | "localhost" | "::1" | "");
    #[cfg(feature = "mercury")]
    if !cfg.demo && local && !mercury::running() {
        if let Err(e) = mercury::start(dir, cfg.mercury_port, cfg.mercury_bcast_port, "", "") {
            eprintln!("whistlestop: {e:#}");
        }
    }
    #[cfg(not(feature = "mercury"))]
    let _ = (dir, cfg, local);
}

/// Is Mercury running inside the app?
pub fn mercury_embedded() -> bool {
    #[cfg(feature = "mercury")]
    return mercury::running();
    #[cfg(not(feature = "mercury"))]
    false
}

pub fn changes() -> u64 {
    CHANGES.load(Ordering::Relaxed)
}

pub fn state_json() -> String {
    with_app(|a| {
        let g = a.h.state.lock().unwrap();
        serde_json::to_string(&*g).unwrap_or_else(|_| "{}".into())
    })
    .unwrap_or_else(|| "{}".into())
}

/// Waterfall rows (each a row of dB levels across the audio passband)
/// added since the last call.
pub fn rows_json() -> String {
    with_app(|a| {
        let g = a.h.state.lock().unwrap();
        let new = g.audio_seq.saturating_sub(a.rows_seen) as usize;
        a.rows_seen = g.audio_seq;
        let n = new.min(g.audio_rows.len());
        let rows: Vec<&Vec<f32>> = g.audio_rows.iter().skip(g.audio_rows.len() - n).collect();
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into())
    })
    .unwrap_or_else(|| "[]".into())
}

pub fn send_json(cmd: &str) -> anyhow::Result<()> {
    let c: Cmd = serde_json::from_str(cmd)?;
    with_app(|a| a.h.send(c)).ok_or_else(|| anyhow::anyhow!("the engine isn't running"))
}

pub fn config_json() -> String {
    with_app(|a| serde_json::to_string(&a.cfg).unwrap_or_default()).unwrap_or_else(|| {
        serde_json::to_string(&Config::load()).unwrap_or_default()
    })
}

/// New settings from the app: saved, then applied to the running engine.
pub fn set_config_json(json: &str) -> anyhow::Result<()> {
    let cfg: Config = serde_json::from_str(json)?;
    cfg.save()?;
    with_app(|a| {
        ensure_mercury(&a.dir, &cfg);
        a.cfg = cfg.clone();
        a.h.send(Cmd::Update(cfg));
    });
    CHANGES.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn stop() {
    if let Some(a) = APP.lock().unwrap().take() {
        a.h.send(Cmd::Stop);
    }
    #[cfg(feature = "mercury")]
    mercury::stop();
}

// ------------------------------------------------------------ C ABI

/// 0 on success, -1 on failure (the reason goes to the log).
///
/// # Safety
/// `dir` is a NUL-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_start(dir: *const c_char) -> i32 {
    match unsafe { arg(dir) }.map(|d| start(&d)) {
        Some(Ok(())) => 0,
        Some(Err(e)) => {
            eprintln!("whistlestop: can't start: {e:#}");
            -1
        }
        None => -1,
    }
}

/// 1 if Mercury runs inside the app.
#[unsafe(no_mangle)]
pub extern "C" fn ws_mercury_embedded() -> i32 {
    mercury_embedded() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_changes() -> u64 {
    changes()
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_state() -> *mut c_char {
    out(state_json())
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_rows() -> *mut c_char {
    out(rows_json())
}

/// # Safety
/// `json` is a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_send(json: *const c_char) -> i32 {
    match unsafe { arg(json) }.map(|j| send_json(&j)) {
        Some(Ok(())) => 0,
        Some(Err(e)) => {
            eprintln!("whistlestop: bad command: {e:#}");
            -1
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_config() -> *mut c_char {
    out(config_json())
}

/// # Safety
/// `json` is a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_set_config(json: *const c_char) -> i32 {
    match unsafe { arg(json) }.map(|j| set_config_json(&j)) {
        Some(Ok(())) => 0,
        Some(Err(e)) => {
            eprintln!("whistlestop: bad settings: {e:#}");
            -1
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_stop() {
    stop()
}

/// # Safety
/// `s` came from one of the calls above and is freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

// ------------------------------------------------------------ Android

#[cfg(target_os = "android")]
mod android {
    //! `com.nr5p.whistlestop.Core`'s native methods.
    use jni::JNIEnv;
    use jni::objects::{JClass, JString};
    use jni::sys::{jint, jlong, jstring};

    fn s(env: &mut JNIEnv, j: &JString) -> String {
        env.get_string(j).map(Into::into).unwrap_or_default()
    }
    fn js(env: &mut JNIEnv, v: String) -> jstring {
        env.new_string(v)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    fn code(r: anyhow::Result<()>) -> jint {
        match r {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("whistlestop: {e:#}");
                -1
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_start(
        mut env: JNIEnv,
        _: JClass,
        dir: JString,
    ) -> jint {
        let d = s(&mut env, &dir);
        code(super::start(&d))
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_changes(_: JNIEnv, _: JClass) -> jlong {
        super::changes() as jlong
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_state(mut env: JNIEnv, _: JClass) -> jstring {
        js(&mut env, super::state_json())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_rows(mut env: JNIEnv, _: JClass) -> jstring {
        js(&mut env, super::rows_json())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_send(
        mut env: JNIEnv,
        _: JClass,
        json: JString,
    ) -> jint {
        let j = s(&mut env, &json);
        code(super::send_json(&j))
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_config(mut env: JNIEnv, _: JClass) -> jstring {
        js(&mut env, super::config_json())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_setConfig(
        mut env: JNIEnv,
        _: JClass,
        json: JString,
    ) -> jint {
        let j = s(&mut env, &json);
        code(super::set_config_json(&j))
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_stop(_: JNIEnv, _: JClass) {
        super::stop()
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_mercuryEmbedded(_: JNIEnv, _: JClass) -> jint {
        super::mercury_embedded() as jint
    }
    /// The phone's sound devices for Mercury, "id\tin|out\tname" per line.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_setAudioDevices(mut env: JNIEnv, _: JClass, list: JString) {
        let _l = s(&mut env, &list);
        #[cfg(feature = "mercury")]
        super::mercury::set_android_audio_devices(&_l);
    }
}

#[cfg(test)]
mod tests {
    /// The whole round trip the apps make: start in a fresh folder (demo
    /// station), read state as JSON, send a command as JSON, change a
    /// setting.
    #[test]
    fn json_round_trip() {
        let dir = std::env::temp_dir().join(format!("ws-mobile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("whistlestop.toml"),
            "demo = true\ndemo_real_mercury = false\ncallsign = \"N0PHN\"\n",
        )
        .unwrap();
        super::start(dir.to_str().unwrap()).unwrap();
        let t = std::time::Instant::now();
        loop {
            let s: serde_json::Value = serde_json::from_str(&super::state_json()).unwrap();
            if s["mercury_ok"] == true {
                break;
            }
            assert!(t.elapsed().as_secs() < 20, "no Mercury: {s}");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(super::changes() > 0);
        super::send_json(r#"{"SetFreq":7103000.0}"#).unwrap();
        let t = std::time::Instant::now();
        while serde_json::from_str::<serde_json::Value>(&super::state_json()).unwrap()["rig"]["freq_hz"]
            != 7_103_000.0
        {
            assert!(t.elapsed().as_secs() < 10, "the dial didn't move");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(super::send_json(r#"{"NoSuchCommand":1}"#).is_err());
        let mut cfg: serde_json::Value = serde_json::from_str(&super::config_json()).unwrap();
        assert_eq!(cfg["callsign"], "N0PHN");
        cfg["grid"] = "DM79".into();
        super::set_config_json(&cfg.to_string()).unwrap();
        assert!(
            std::fs::read_to_string(dir.join("whistlestop.toml"))
                .unwrap()
                .contains("DM79")
        );
        super::stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
