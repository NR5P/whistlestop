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
use std::sync::{Mutex, MutexGuard, OnceLock};

use whistlestop::config::Config;
use whistlestop::engine::{self, Cmd, Handle, State};

pub mod bench;
#[cfg(feature = "mercury")]
pub mod mercury;

struct App {
    dir: PathBuf,
    h: Handle,
    cfg: Config,
    /// Waterfall rows already handed out.
    rows_seen: u64,
    /// The two-station bench operating this station (`bench.rs`).
    _bench: Option<whistlestop::remote::Server>,
}

static APP: Mutex<Option<App>> = Mutex::new(None);
static CHANGES: AtomicU64 = AtomicU64::new(0);
/// The settings folder, fixed by the first start (see `start`).
static SETTINGS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// A lock that a panic elsewhere can't poison for good: the data under it
/// is plain state, still usable after a panic.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    lock(&APP).as_mut().map(f)
}

/// Run an entry point the app calls, so a panic never unwinds into Swift,
/// Kotlin or the JVM: it's logged and the call returns `fallback`.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        eprintln!("whistlestop: internal error (see the panic above); the call was ignored");
        fallback
    })
}

/// Settings whose change needs the station restarted (`Cmd::Start`): how
/// it reaches the modem, the radio and the sound, and who it is. Anything
/// else applies to the running station (`Cmd::Update`) without dropping a
/// link.
const RESTART_KEYS: &[&str] = &[
    "callsign",
    "demo",
    "demo_real_mercury",
    "mercury_host",
    "mercury_port",
    "mercury_bcast_port",
    "mercury_bin",
    "rigctld",
    "rig_mode",
    "managed_rig",
    "cat",
    "audio_in",
    "audio_out",
    "monitor_device",
    "sdr",
    "download_dir",
    "depot_file",
];

fn needs_restart(old: &Config, new: &Config) -> bool {
    let (Ok(a), Ok(b)) = (serde_json::to_value(old), serde_json::to_value(new)) else {
        return true;
    };
    RESTART_KEYS.iter().any(|k| a.get(k) != b.get(k))
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
///
/// The settings folder is fixed by the first start in this process: a later
/// start (after `stop`) with another folder keeps using the first.
pub fn start(dir: &str) -> anyhow::Result<()> {
    let mut slot = lock(&APP);
    if slot.is_some() {
        return Ok(());
    }
    let asked = PathBuf::from(dir);
    // Settings live in the app's sandbox, not a desktop config folder.
    // Whistlestop's settings code (shared with the desktop) finds them
    // through WHISTLESTOP_CONFIG, so it's set here, exactly once.
    let dir = SETTINGS_DIR
        .get_or_init(|| {
            // SAFETY: setting the environment races with any other thread
            // reading it. This runs once per process, on the first start,
            // before Whistlestop has started any thread of its own (no
            // engine, no Mercury yet). The host app's own threads could in
            // principle read the environment at the same moment; that's
            // the remaining risk, kept as small as it can be by never
            // setting it again (a restart after `stop` doesn't, while the
            // old engine's threads may still be winding down).
            unsafe { std::env::set_var("WHISTLESTOP_CONFIG", asked.join("whistlestop.toml")) };
            asked.clone()
        })
        .clone();
    if dir != asked {
        eprintln!(
            "whistlestop: settings stay in {} (the folder of the first start), not {}",
            dir.display(),
            asked.display()
        );
    }
    std::fs::create_dir_all(&dir)?;
    let mut cfg = Config::load();
    if cfg.download_dir.as_os_str().is_empty() || !cfg.download_dir.starts_with(&dir) {
        cfg.download_dir = dir.join("Downloads");
    }
    let _ = std::fs::create_dir_all(&cfg.download_dir);
    if adopt_usb_radio(&mut cfg) {
        let _ = cfg.save();
    }
    let bench = bench::load(&dir);
    // An empty `mercury_audio` (the over-the-air test, `airtest`) keeps the
    // phone's own sound: only remote operation comes from the bench.
    if let Some(b) = bench.as_ref().filter(|b| !b.mercury_audio.is_empty()) {
        match bench::mercury_audio(&dir, b) {
            Ok(input) => set_radio_audio(&input, ""),
            Err(e) => eprintln!("whistlestop: bench: {e:#}"),
        }
    }
    ensure_mercury(&dir, &cfg);
    let h = engine::spawn(|| {
        CHANGES.fetch_add(1, Ordering::Relaxed);
    });
    h.send(Cmd::Start(cfg.clone()));
    let _bench = bench.and_then(|b| bench::serve(&b, &h, &cfg));
    *slot = Some(App {
        dir,
        h,
        cfg,
        rows_seen: 0,
        _bench,
    });
    Ok(())
}

/// Where Mercury keys the radio: Whistlestop's own rigctld server when
/// Whistlestop drives the radio directly (USB or a serial bridge) and shares
/// it; otherwise Mercury doesn't key (VOX, or a rigctld it isn't told of).
pub fn mercury_ptt(cfg: &Config) -> Option<String> {
    (!cfg.demo
        && cfg.rig_mode == whistlestop::rigsetup::RigMode::Direct
        && cfg.cat.share_port > 0)
        .then(|| format!("127.0.0.1:{}", cfg.cat.share_port))
}

/// The radio's sound card as the app found it ("dev:<id>" for Mercury's
/// input and output; empty = the phone's default audio route).
static RADIO_AUDIO: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));

/// Use this sound card for the radio (the app picks a USB audio device when
/// one is plugged in). Takes effect when Mercury starts.
pub fn set_radio_audio(input: &str, output: &str) {
    *RADIO_AUDIO.lock().unwrap() = (input.to_string(), output.to_string());
}

/// A radio on USB and no radio set up yet (a phone has no rigctld of its
/// own, so the desktop default `127.0.0.1:4532` means "not set up"): use the
/// radio on USB directly. True if the settings changed.
pub fn adopt_usb_radio(cfg: &mut Config) -> bool {
    use whistlestop::rigsetup::RigMode;
    let unset = cfg.rig_mode == RigMode::External
        && matches!(cfg.rigctld.trim(), "127.0.0.1:4532" | "localhost:4532");
    if cfg.demo || !unset || whistlestop::rigsetup::detect().is_empty() {
        return false;
    }
    cfg.rig_mode = RigMode::Direct;
    cfg.cat.port = String::new();
    true
}

/// Mercury is waiting for the radio before it starts.
#[cfg(feature = "mercury")]
static MERCURY_WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Mercury runs, but keys the radio differently from what the settings now
/// say. Its engine can't be restarted inside the app, so the app asks the
/// operator to close and reopen it.
static MERCURY_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A real station whose modem is this phone runs Mercury in the app. (The
/// demo station has its own simulated modem; a modem elsewhere on the
/// network needs nothing here.) When Mercury keys the radio through
/// Whistlestop's rigctld server, it starts once that server answers: the
/// engine opens the radio first (up to 30 s for the probe), and Mercury
/// connects to the server once, at start.
fn ensure_mercury(dir: &std::path::Path, cfg: &Config) {
    let local = matches!(
        cfg.mercury_host.trim(),
        "127.0.0.1" | "localhost" | "::1" | ""
    );
    #[cfg(feature = "mercury")]
    if !cfg.demo && local {
        let ptt = mercury_ptt(cfg);
        if mercury::running() {
            // Its engine can't be restarted inside the app: a change to how
            // it keys the radio or to its ports needs the app reopened.
            let ports = Some((cfg.mercury_port, cfg.mercury_bcast_port));
            MERCURY_STALE.store(mercury::ptt() != ptt || mercury::ports() != ports, Ordering::Relaxed);
            return;
        }
        if MERCURY_WAITING.swap(true, Ordering::SeqCst) {
            return;
        }
        let (dir, port, bcast) = (dir.to_path_buf(), cfg.mercury_port, cfg.mercury_bcast_port);
        let _ = std::thread::Builder::new().name("mercury-start".into()).spawn(move || {
            let mut ptt = ptt;
            if let Some(addr) = ptt.clone() {
                let t = std::time::Instant::now();
                while std::net::TcpStream::connect(&addr).is_err() {
                    if t.elapsed().as_secs() >= 30 {
                        eprintln!("whistlestop: no radio shared at {addr}; Mercury starts without keying it");
                        MERCURY_STALE.store(true, Ordering::Relaxed);
                        ptt = None;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
            let (input, output) = RADIO_AUDIO.lock().unwrap().clone();
            match mercury::start(&dir, port, bcast, &input, &output, ptt.as_deref()) {
                Ok(()) => connect_when_ready(),
                Err(e) => eprintln!("whistlestop: {e:#}"),
            }
            MERCURY_WAITING.store(false, Ordering::SeqCst);
            CHANGES.fetch_add(1, Ordering::Relaxed);
        });
    }
    #[cfg(not(feature = "mercury"))]
    let _ = (dir, cfg, local);
}

/// The engine starts alongside Mercury and connects to it once; Mercury
/// opens its ports a moment later (or, keying a USB radio, up to 30 s
/// later). Ask the engine to connect again until it has. Only the modem
/// connection is retried: restarting the station would drop the shared
/// rigctld port, and with it Mercury's one connection for keying the radio.
/// (Its ports aren't probed: Mercury serves one control client, and a probe
/// could unseat it.)
#[cfg(feature = "mercury")]
fn connect_when_ready() {
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let done = with_app(|a| {
            let ok = a.h.state.lock().unwrap().mercury_ok;
            if !ok {
                a.h.send(Cmd::ReconnectModem);
            }
            ok
        });
        if done != Some(false) {
            return;
        }
    }
}

/// Does the app need closing and reopening so Mercury keys the radio the
/// way the settings now say?
pub fn mercury_restart_needed() -> bool {
    MERCURY_STALE.load(Ordering::Relaxed)
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
        let g: MutexGuard<State> = lock(&a.h.state);
        serde_json::to_string(&*g).unwrap_or_else(|_| "{}".into())
    })
    .unwrap_or_else(|| "{}".into())
}

/// Waterfall rows (each a row of dB levels across the audio passband)
/// added since the last call.
pub fn rows_json() -> String {
    with_app(|a| {
        let g = lock(&a.h.state);
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
    with_app(|a| {
        // A restart (new radio settings) may change how Mercury keys.
        if let Cmd::Start(cfg) = &c {
            ensure_mercury(&a.dir, cfg);
        }
        a.h.send(c)
    })
    .ok_or_else(|| anyhow::anyhow!("the engine isn't running"))
}

pub fn config_json() -> String {
    with_app(|a| serde_json::to_string(&a.cfg).unwrap_or_default()).unwrap_or_else(|| {
        serde_json::to_string(&Config::load()).unwrap_or_default()
    })
}

/// New settings from the app: saved, then applied to the running engine.
/// A change to how the station connects (modem, radio, sound, callsign,
/// demo) restarts the station, as the desktop's Settings → Apply does;
/// anything else is applied without dropping a link. The embedded Mercury
/// starts if it should and isn't running; it can't restart inside the app,
/// so a change to its ports or keying asks for the app to be reopened.
pub fn set_config_json(json: &str) -> anyhow::Result<()> {
    let mut cfg: Config = serde_json::from_str(json)?;
    adopt_usb_radio(&mut cfg);
    cfg.save()?;
    with_app(|a| {
        let restart = needs_restart(&a.cfg, &cfg);
        ensure_mercury(&a.dir, &cfg);
        a.cfg = cfg.clone();
        a.h.send(if restart {
            Cmd::Start(cfg)
        } else {
            Cmd::Update(cfg)
        });
    });
    CHANGES.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn stop() {
    if let Some(a) = lock(&APP).take() {
        a.h.send(Cmd::Stop);
    }
    #[cfg(feature = "mercury")]
    mercury::stop();
}

// ------------------------------------------------------------ C ABI
//
// Every entry point runs inside `guard`: a panic in Rust is logged and the
// call returns its failure value instead of unwinding into the app.

/// 0 on success, -1 on failure (the reason goes to the log).
///
/// # Safety
/// `dir` is a NUL-terminated UTF-8 path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_start(dir: *const c_char) -> i32 {
    guard(-1, || match unsafe { arg(dir) }.map(|d| start(&d)) {
        Some(Ok(())) => 0,
        Some(Err(e)) => {
            eprintln!("whistlestop: can't start: {e:#}");
            -1
        }
        None => -1,
    })
}

/// 1 if Mercury runs inside the app.
#[unsafe(no_mangle)]
pub extern "C" fn ws_mercury_embedded() -> i32 {
    guard(0, || mercury_embedded() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_changes() -> u64 {
    guard(0, changes)
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_state() -> *mut c_char {
    guard(std::ptr::null_mut(), || out(state_json()))
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_rows() -> *mut c_char {
    guard(std::ptr::null_mut(), || out(rows_json()))
}

/// # Safety
/// `json` is a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_send(json: *const c_char) -> i32 {
    guard(-1, || match unsafe { arg(json) }.map(|j| send_json(&j)) {
        Some(Ok(())) => 0,
        Some(Err(e)) => {
            eprintln!("whistlestop: bad command: {e:#}");
            -1
        }
        None => -1,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_config() -> *mut c_char {
    guard(std::ptr::null_mut(), || out(config_json()))
}

/// # Safety
/// `json` is a NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ws_set_config(json: *const c_char) -> i32 {
    guard(-1, || match unsafe { arg(json) }.map(|j| set_config_json(&j)) {
        Some(Ok(())) => 0,
        Some(Err(e)) => {
            eprintln!("whistlestop: bad settings: {e:#}");
            -1
        }
        None => -1,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn ws_stop() {
    guard((), stop)
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
    //! `com.nr5p.whistlestop.Core`'s native methods, each inside `guard` so
    //! a panic never unwinds into the JVM.
    use super::guard;
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

    /// Hand the app's JavaVM and Context to `ndk_context`, once, before the
    /// engine starts. cpal's Android backend asks for them when it opens an
    /// output stream (AudioManager's mixer bursts); without them it panics,
    /// and the whistle's audio output never starts.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_initAndroid(
        env: JNIEnv,
        _: JClass,
        context: jni::objects::JObject,
    ) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        guard((), || {
            ONCE.call_once(|| {
                let (Ok(vm), Ok(ctx)) = (env.get_java_vm(), env.new_global_ref(&context)) else {
                    eprintln!("whistlestop: couldn't keep the Android context");
                    return;
                };
                let raw = ctx.as_obj().as_raw();
                // The application Context lives as long as the process.
                std::mem::forget(ctx);
                // SAFETY: a valid JavaVM and a global reference that is never
                // released; `ONCE` makes this the only initialisation.
                unsafe { ndk_context::initialize_android_context(vm.get_java_vm_pointer().cast(), raw.cast()) };
            })
        })
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_start(
        mut env: JNIEnv,
        _: JClass,
        dir: JString,
    ) -> jint {
        guard(-1, || {
            let d = s(&mut env, &dir);
            code(super::start(&d))
        })
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_changes(_: JNIEnv, _: JClass) -> jlong {
        guard(0, || super::changes() as jlong)
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_state(mut env: JNIEnv, _: JClass) -> jstring {
        guard(std::ptr::null_mut(), || js(&mut env, super::state_json()))
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_rows(mut env: JNIEnv, _: JClass) -> jstring {
        guard(std::ptr::null_mut(), || js(&mut env, super::rows_json()))
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_send(
        mut env: JNIEnv,
        _: JClass,
        json: JString,
    ) -> jint {
        guard(-1, || {
            let j = s(&mut env, &json);
            code(super::send_json(&j))
        })
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_config(mut env: JNIEnv, _: JClass) -> jstring {
        guard(std::ptr::null_mut(), || js(&mut env, super::config_json()))
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_setConfig(
        mut env: JNIEnv,
        _: JClass,
        json: JString,
    ) -> jint {
        guard(-1, || {
            let j = s(&mut env, &json);
            code(super::set_config_json(&j))
        })
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_stop(_: JNIEnv, _: JClass) {
        guard((), super::stop)
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_mercuryEmbedded(_: JNIEnv, _: JClass) -> jint {
        guard(0, || super::mercury_embedded() as jint)
    }
    /// The app's USB radio access (`UsbCat`) is ready: from now on the
    /// engine finds and opens USB radios through it.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_mercuryRestartNeeded(_: JNIEnv, _: JClass) -> jint {
        super::mercury_restart_needed() as jint
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_usbInit(mut env: JNIEnv, _: JClass) {
        if let Err(e) = super::usb::init(&mut env) {
            eprintln!("whistlestop: USB radios unavailable: {e}");
        }
    }
    /// The radio's sound card for Mercury ("dev:<id>"; "" = default route).
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_setRadioAudio(
        mut env: JNIEnv,
        _: JClass,
        input: JString,
        output: JString,
    ) {
        let (i, o) = (s(&mut env, &input), s(&mut env, &output));
        super::set_radio_audio(&i, &o);
    }
    /// The phone's sound devices for Mercury, "id\tin|out\tname" per line.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_nr5p_whistlestop_Core_setAudioDevices(mut env: JNIEnv, _: JClass, list: JString) {
        guard((), || {
            let _l = s(&mut env, &list);
            #[cfg(feature = "mercury")]
            super::mercury::set_android_audio_devices(&_l);
        })
    }
}

#[cfg(target_os = "android")]
mod usb {
    //! Radios on USB, through the app's `UsbCat` (Android's USB host API and
    //! usb-serial-for-android). Android apps can't open `/dev/ttyUSB0`, so
    //! Kotlin owns the USB connection and the CAT drivers' bytes cross JNI.
    use jni::objects::{GlobalRef, JByteArray, JClass, JString, JValue};
    use jni::{JNIEnv, JavaVM};
    use std::sync::OnceLock;
    use whistlestop::cat::{UsbHost, Wire};
    use whistlestop::rigsetup::UsbSerial;

    static VM: OnceLock<JavaVM> = OnceLock::new();
    /// `UsbCat`, looked up on an app thread (native threads only see the
    /// system class loader).
    static CLS: OnceLock<GlobalRef> = OnceLock::new();

    pub fn init(env: &mut JNIEnv) -> jni::errors::Result<()> {
        let _ = VM.set(env.get_java_vm()?);
        let c = env.find_class("com/nr5p/whistlestop/UsbCat")?;
        let _ = CLS.set(env.new_global_ref(c)?);
        whistlestop::cat::set_usb_host(Box::new(Host));
        Ok(())
    }

    /// Run `f` with an environment for this thread and `UsbCat`, in a local
    /// frame (the CAT thread stays attached, so references must not pile up).
    fn call<T>(f: impl FnOnce(&mut JNIEnv, &JClass) -> jni::errors::Result<T>) -> anyhow::Result<T> {
        let vm = VM.get().ok_or_else(|| anyhow::anyhow!("USB isn't set up"))?;
        let cls = CLS.get().ok_or_else(|| anyhow::anyhow!("USB isn't set up"))?;
        let mut env = vm.attach_current_thread_permanently()?;
        let r = env.with_local_frame(16, |env| f(env, <&JClass>::from(cls.as_obj())));
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        r.map_err(|e| anyhow::anyhow!("USB: {e}"))
    }

    fn string(env: &mut JNIEnv, o: jni::objects::JObject) -> String {
        if o.is_null() {
            return String::new();
        }
        let s = JString::from(o);
        env.get_string(&s).map(Into::into).unwrap_or_default()
    }

    struct Host;

    impl UsbHost for Host {
        fn list(&self) -> Vec<UsbSerial> {
            let text = call(|env, c| {
                let o = env.call_static_method(c, "list", "()Ljava/lang/String;", &[])?.l()?;
                Ok(string(env, o))
            })
            .unwrap_or_default();
            text.lines()
                .filter_map(|l| {
                    let f: Vec<&str> = l.split('\t').collect();
                    let hex = |s: &str| u16::from_str_radix(s, 16).ok();
                    Some(UsbSerial {
                        path: f.first()?.to_string(),
                        vid: hex(f.get(1)?)?,
                        pid: hex(f.get(2)?)?,
                        manufacturer: f.get(3).unwrap_or(&"").to_string(),
                        product: f.get(4).unwrap_or(&"").to_string(),
                        serial: f.get(5).unwrap_or(&"").to_string(),
                    })
                })
                .collect()
        }

        fn open(&self, path: &str, baud: u32) -> anyhow::Result<Box<dyn Wire>> {
            let (h, err) = call(|env, c| {
                let p = env.new_string(path)?;
                let h = env
                    .call_static_method(c, "open", "(Ljava/lang/String;I)I", &[JValue::Object(&p), JValue::Int(baud as i32)])?
                    .i()?;
                let err = if h > 0 {
                    String::new()
                } else {
                    let o = env.call_static_method(c, "lastError", "()Ljava/lang/String;", &[])?.l()?;
                    string(env, o)
                };
                Ok((h, err))
            })?;
            if h <= 0 {
                anyhow::bail!("{path}: {err}");
            }
            Ok(Box::new(UsbWire(h)))
        }
    }

    /// One open USB radio, by `UsbCat`'s handle.
    struct UsbWire(i32);

    fn io(e: anyhow::Error) -> std::io::Error {
        std::io::Error::other(format!("{e:#}"))
    }

    impl Wire for UsbWire {
        fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
            let ok = call(|env, c| {
                let a = env.byte_array_from_slice(b)?;
                env.call_static_method(c, "write", "(I[B)Z", &[JValue::Int(self.0), JValue::Object(&a)])?.z()
            })
            .map_err(io)?;
            if ok {
                Ok(())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the radio's USB connection closed"))
            }
        }

        fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let got = call(|env, c| {
                let o = env
                    .call_static_method(c, "read", "(III)[B", &[JValue::Int(self.0), JValue::Int(buf.len() as i32), JValue::Int(40)])?
                    .l()?;
                if o.is_null() {
                    return Ok(None);
                }
                Ok(Some(env.convert_byte_array(JByteArray::from(o))?))
            })
            .map_err(io)?;
            match got {
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the radio's USB connection closed",
                )),
                Some(v) => {
                    let n = v.len().min(buf.len());
                    buf[..n].copy_from_slice(&v[..n]);
                    Ok(n)
                }
            }
        }

        fn clear_input(&mut self) {
            let _ = call(|env, c| env.call_static_method(c, "purge", "(I)V", &[JValue::Int(self.0)]).map(|_| ()));
        }
    }

    impl Drop for UsbWire {
        fn drop(&mut self) {
            let _ = call(|env, c| env.call_static_method(c, "close", "(I)V", &[JValue::Int(self.0)]).map(|_| ()));
        }
    }
}

#[cfg(test)]
mod tests {
    use whistlestop::config::Config;

    #[test]
    fn what_needs_a_restart() {
        let a = Config::default();
        let mut b = a.clone();
        b.grid = "DM79".into();
        b.auto_answer = !a.auto_answer;
        assert!(!super::needs_restart(&a, &b));
        let changes: [fn(&mut Config); 6] = [
            |c: &mut Config| c.callsign = "N0NEW".into(),
            |c: &mut Config| c.mercury_port += 10,
            |c: &mut Config| c.mercury_host = "192.168.1.5".into(),
            |c: &mut Config| c.cat.port = "radio.local:4532".into(),
            |c: &mut Config| c.demo = !c.demo,
            |c: &mut Config| c.audio_in = "USB Audio".into(),
        ];
        for f in changes {
            let mut b = a.clone();
            f(&mut b);
            assert!(super::needs_restart(&a, &b));
        }
    }

    #[test]
    fn a_panic_doesnt_cross_the_boundary() {
        assert_eq!(super::guard(-1, || panic!("boom")), -1);
        assert_eq!(super::guard(-1, || 0), 0);
    }

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
        // A new callsign restarts the station, so it's in use at once.
        let mut cfg: serde_json::Value = serde_json::from_str(&super::config_json()).unwrap();
        cfg["callsign"] = "N0NEW".into();
        super::set_config_json(&cfg.to_string()).unwrap();
        let t = std::time::Instant::now();
        while serde_json::from_str::<serde_json::Value>(&super::state_json()).unwrap()["mycall"]
            != "N0NEW"
        {
            assert!(t.elapsed().as_secs() < 20, "the new callsign isn't in use");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        super::stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Mercury keys the radio through Whistlestop's rigctld only when
    /// Whistlestop drives the radio itself and shares it.
    #[test]
    fn mercury_keys_through_the_shared_radio() {
        use whistlestop::rigsetup::RigMode;
        let mut cfg = whistlestop::config::Config::default();
        cfg.demo = false;
        cfg.rig_mode = RigMode::Direct;
        assert_eq!(super::mercury_ptt(&cfg).as_deref(), Some("127.0.0.1:4532"));
        cfg.cat.share_port = 0;
        assert_eq!(super::mercury_ptt(&cfg), None);
        cfg.cat.share_port = 4532;
        cfg.rig_mode = RigMode::External;
        assert_eq!(super::mercury_ptt(&cfg), None);
        cfg.rig_mode = RigMode::Direct;
        cfg.demo = true;
        assert_eq!(super::mercury_ptt(&cfg), None);
    }

    /// An explicit choice is never overridden by a radio on USB (and with
    /// none attached, nothing changes).
    #[test]
    fn usb_radio_only_when_nothing_is_set_up() {
        use whistlestop::rigsetup::RigMode;
        let mut cfg = whistlestop::config::Config::default();
        cfg.demo = false;
        cfg.rig_mode = RigMode::External;
        cfg.rigctld = "192.168.1.20:4532".into();
        assert!(!super::adopt_usb_radio(&mut cfg));
        cfg.rig_mode = RigMode::Direct;
        cfg.cat.port = "192.168.1.30:4000".into();
        assert!(!super::adopt_usb_radio(&mut cfg));
        assert_eq!(cfg.cat.port, "192.168.1.30:4000");
    }
}
