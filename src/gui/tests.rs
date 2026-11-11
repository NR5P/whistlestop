//! Click tests: the real Whistlestop window and engine, driven through the
//! accessibility tree (egui_kittest) the way a person would: clicking
//! buttons and tabs, typing into fields, opening windows. Each test checks
//! what the engine did, not just what's drawn.
//!
//! The tests share one settings file and Depot (temp files), so they run
//! one at a time.

use super::*;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ws-gui-{}-{name}", std::process::id()))
}

/// Start the app with these settings (written where it will load them).
fn app(cfg: Config, w: f32, h: f32) -> Harness<'static, App> {
    // SAFETY: tests hold SERIAL, so no other thread reads these meanwhile.
    unsafe {
        std::env::set_var("WHISTLESTOP_CONFIG", temp("config.toml"));
        std::env::set_var("WHISTLESTOP_DEPOT", temp("depot.json"));
    }
    let _ = std::fs::remove_file(temp("depot.json"));
    cfg.save().unwrap();
    Harness::builder().allow_missing_glyphs().with_size(vec2(w, h)).with_max_steps(1_000_000).build_eframe(|cc| App::new(cc))
}

fn demo_cfg(call: &str) -> Config {
    let mut c = Config { demo: true, demo_real_mercury: false, callsign: call.into(), ..Default::default() };
    c.sdr.enabled = false;
    c.download_dir = temp("downloads");
    c
}

/// Run frames until `f` is true (or fail with the log).
fn wait(h: &mut Harness<App>, secs: f32, what: &str, f: impl Fn(&App, &State) -> bool) {
    let t = Instant::now();
    while t.elapsed().as_secs_f32() < secs {
        h.step();
        {
            let a = h.state();
            let g = a.h.state.lock().unwrap();
            if f(a, &g) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let a = h.state();
    let g = a.h.state.lock().unwrap();
    let log: Vec<_> = g.log.iter().rev().take(40).rev().map(|l| l.1.clone()).collect();
    panic!("timed out waiting for {what}; log:\n{}", log.join("\n"));
}

fn frames(h: &mut Harness<App>, n: usize) {
    for _ in 0..n {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Click the (first) button whose label contains `text`, scrolling it into
/// view first as a person would.
fn click(h: &mut Harness<App>, text: &str) {
    let find = |h: &Harness<App>| {
        h.query_all_by(|n| matches!(n.role(), Role::Button | Role::Tab | Role::CheckBox | Role::RadioButton) && n.label().is_some_and(|l| l.contains(text)))
            .next()
            .is_some()
    };
    if find(h) {
        h.query_all_by(|n| matches!(n.role(), Role::Button | Role::Tab | Role::CheckBox | Role::RadioButton) && n.label().is_some_and(|l| l.contains(text)))
            .next()
            .unwrap()
            .scroll_to_me();
        frames(h, 3);
    }
    {
        let b = h
            .query_all_by(|n| matches!(n.role(), Role::Button | Role::Tab | Role::CheckBox | Role::RadioButton) && n.label().is_some_and(|l| l.contains(text)))
            .next()
            .unwrap_or_else(|| panic!("no button labelled “{text}”; buttons: {:?}", buttons(h)));
        b.click();
    }
    frames(h, 3);
}

/// Click the button labelled exactly `text`.
fn click_exact(h: &mut Harness<App>, text: &str) {
    let q = |n: &egui_kittest::kittest::AccessKitNode<'_>| n.role() == Role::Button && n.label().is_some_and(|l| l == text);
    h.query_all_by(q).next().unwrap_or_else(|| panic!("no button “{text}”")).scroll_to_me();
    frames(h, 3);
    h.query_all_by(q).next().unwrap_or_else(|| panic!("no button “{text}”")).click();
    frames(h, 3);
}

fn has_exact_button(h: &Harness<App>, text: &str) -> bool {
    h.query_all_by(|n| n.role() == Role::Button && n.label().is_some_and(|l| l == text)).next().is_some()
}

fn has_button(h: &Harness<App>, text: &str) -> bool {
    h.query_all_by(|n| matches!(n.role(), Role::Button | Role::CheckBox) && n.label().is_some_and(|l| l.contains(text))).next().is_some()
}

fn buttons(h: &Harness<App>) -> Vec<String> {
    h.query_all_by(|n| matches!(n.role(), Role::Button | Role::CheckBox)).filter_map(|n| n.accesskit_node().label()).collect()
}

fn has_text(h: &Harness<App>, text: &str) -> bool {
    h.query_all_by(|n| n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))).next().is_some()
}

/// Type into the text field showing `placeholder`.
fn type_into(h: &mut Harness<App>, placeholder: &str, text: &str) {
    {
        let f = h
            .query_all_by(|n| matches!(n.role(), Role::TextInput | Role::MultilineTextInput | Role::PasswordInput))
            .find(|n| n.accesskit_node().placeholder().is_some_and(|p| p.contains(placeholder)))
            .unwrap_or_else(|| panic!("no text field “{placeholder}”"));
        f.click();
    }
    frames(h, 2);
    {
        let f = h
            .query_all_by(|n| matches!(n.role(), Role::TextInput | Role::MultilineTextInput | Role::PasswordInput))
            .find(|n| n.accesskit_node().is_focused())
            .expect("field focused after click");
        f.type_text(text);
    }
    frames(h, 2);
}

fn rect_of(h: &Harness<App>, text: &str) -> egui::Rect {
    h.query_all_by(|n| {
        n.bounding_box().is_some() && (n.label().is_some_and(|l| l == text) || (n.role() == Role::Label && n.value().is_some_and(|v| v == text)))
    })
    .next()
    .unwrap_or_else(|| panic!("no “{text}”"))
    .rect()
}

/// Depot by clicks: write mail to K7BOT, go Away, connect from the
/// Departures card; it's delivered, and what K7BOT types lands in Arrivals
/// with an unread badge on the tab.
#[test]
fn click_depot_away_and_delivery() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = app(demo_cfg("N0GUI"), 1400.0, 900.0);
    wait(&mut h, 15.0, "Mercury", |_, s| s.mercury_ok);
    click(&mut h, "Depot");
    assert!(has_text(&h, "WRITE MAIL"));
    type_into(&mut h, "Callsign", "k7bot");
    type_into(&mut h, "Your message", "Meet on 40 m tonight");
    click(&mut h, "Post");
    wait(&mut h, 5.0, "mail in Departures", |_, s| s.depot.departures.len() == 1 && s.depot.departures[0].to == "K7BOT");
    assert!(has_button(&h, "Departures (1)"), "{:?}", buttons(&h));
    click(&mut h, "Here");
    wait(&mut h, 5.0, "Away", |_, s| s.away);
    assert!(has_button(&h, "Away"));
    click(&mut h, "Connect to K7BOT");
    wait(&mut h, 60.0, "delivered", |_, s| s.depot.departures.is_empty() && s.depot.delivered.len() == 1);
    wait(&mut h, 60.0, "K7BOT's words in Arrivals", |_, s| s.depot.arrivals.iter().any(|m| m.from == "K7BOT"));
    frames(&mut h, 3);
    assert!(has_button(&h, "Depot  ("), "unread badge on the tab: {:?}", buttons(&h));
    click(&mut h, "Arrivals");
    assert!(has_text(&h, "From K7BOT"));
    click(&mut h, "Mark read");
    wait(&mut h, 5.0, "read", |_, s| s.depot.arrivals.iter().filter(|m| !m.read).count() < s.depot.arrivals.len());
    click(&mut h, "Delivered");
    assert!(has_text(&h, "Meet on 40 m tonight"));
    click(&mut h, "Away");
    wait(&mut h, 5.0, "back Here", |_, s| !s.away);
}

/// Search by clicks, lock onto K7BOT's whistle, link, chat (typed and sent
/// with the Send button), and the modem strip shows the link.
#[test]
fn click_search_link_chat() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = demo_cfg("N0GUI");
    cfg.auto_answer = true;
    let mut h = app(cfg, 1400.0, 900.0);
    wait(&mut h, 15.0, "Mercury", |_, s| s.mercury_ok);
    click(&mut h, "Current band");
    wait(&mut h, 150.0, "link with K7BOT", |_, s| s.link.as_ref().is_some_and(|l| l.peer == "K7BOT"));
    wait(&mut h, 30.0, "greeting", |_, s| s.chat.iter().any(|c| c.who == Who::Them));
    click_exact(&mut h, "💬  Chat");
    type_into(&mut h, "Type a message", "hello from the click test");
    click(&mut h, "Send");
    wait(&mut h, 40.0, "reply", |_, s| s.chat.iter().any(|c| c.who == Who::Them && c.text.contains("hello from the click test")));
    wait(&mut h, 40.0, "delivered tick", |_, s| s.chat.iter().any(|c| c.who == Who::Me && c.delivered));
    assert!(has_text(&h, "1:1 link with K7BOT"));
    click(&mut h, "Disconnect");
    wait(&mut h, 30.0, "link down", |_, s| s.link.is_none());
}

/// Help mode, the Guide window, Settings (Automation and Direct CAT
/// controls appear and stay off by default), and the Radio Controls window.
#[test]
fn click_help_guide_settings_radio() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = app(demo_cfg("N0GUI"), 1400.0, 900.0);
    wait(&mut h, 15.0, "radio", |_, s| s.rig_ok);
    // Help mode: off at start, on with a click, banner shows.
    assert!(!help::on());
    click(&mut h, "? Help");
    assert!(help::on());
    assert!(has_button(&h, "Turn off"));
    click(&mut h, "Turn off");
    assert!(!help::on());
    // Guide window.
    click(&mut h, "Guide");
    assert!(h.state().guide_open);
    click(&mut h, "Guide");
    assert!(!h.state().guide_open);
    // Settings.
    click(&mut h, "Settings");
    assert!(h.state().settings_open);
    frames(&mut h, 3);
    for label in [
        "Station ID beacon every",
        "Tell stations I hear that I have mail for them",
        "While away: connect and deliver",
        "While away: connect and collect",
        "Carry mail for other stations",
        "Automatic transmissions only inside",
    ] {
        assert!(has_button(&h, label), "missing {label}: {:?}", buttons(&h));
    }
    let c = &h.state().cfg;
    assert!(!c.beacon && !c.notify_mail && !c.auto_deliver && !c.auto_collect && !c.relay, "automation is off by default");
    assert!(c.auto_in_segments_only);
    // The window edits a draft; the settings change only on Save & connect.
    click(&mut h, "Carry mail for other stations");
    assert!(h.state().settings_draft.as_ref().unwrap().relay, "checkbox toggles the setting");
    assert!(!h.state().cfg.relay, "not applied before Save");
    click(&mut h, "Direct CAT");
    assert_eq!(h.state().settings_draft.as_ref().unwrap().rig_mode, whistlestop::rigsetup::RigMode::Direct);
    assert!(has_text(&h, "Share with Mercury on port"));
    assert!(has_button(&h, "Ask the radio"));
    // New cards: QSY requests, File library, Schedule, all off by default.
    for label in ["Accept QSY requests", "Share a file library", "Follow the schedule"] {
        assert!(has_button(&h, label), "missing {label}: {:?}", buttons(&h));
    }
    let c = &h.state().cfg;
    assert!(!c.accept_qsy && !c.share_library && !c.schedule_on, "new automation is off by default");
    click(&mut h, "Add a time slot");
    assert_eq!(h.state().settings_draft.as_ref().unwrap().schedule.len(), 1);
    // Closing without saving throws the edits away.
    h.state_mut().settings_open = false;
    frames(&mut h, 2);
    assert!(h.state().settings_draft.is_none());
    assert!(!h.state().cfg.relay && h.state().cfg.schedule.is_empty(), "closed without saving: nothing applied");
    assert_ne!(h.state().cfg.rig_mode, whistlestop::rigsetup::RigMode::Direct);
    // Radio controls window (demo radio has every control).
    click(&mut h, "Radio controls");
    assert!(h.state().rigwin.open);
    frames(&mut h, 5);
}

/// The modem strip never overlaps: readouts, speed ladder and status pills
/// keep apart at every window width (they wrap instead).
#[test]
fn modem_strip_never_overlaps() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = app(demo_cfg("N0GUI"), 1700.0, 900.0);
    wait(&mut h, 15.0, "Mercury", |_, s| s.mercury_ok);
    for w in [1700.0, 1400.0, 1200.0, 1100.0, 1000.0, 900.0] {
        h.set_size(vec2(w, 900.0));
        frames(&mut h, 6);
        let pills: Vec<egui::Rect> = ["Radio", "Mercury", "Audio", "SDR"].iter().map(|p| rect_of(&h, p)).collect();
        let labels = ["MODEM", "MODE", "CARRIERS", "BANDWIDTH", "SPEED", "ERROR CORRECTION", "SNR", "SPEED LADDER"];
        for l in labels {
            let r = rect_of(&h, l);
            for (i, p) in pills.iter().enumerate() {
                assert!(!r.intersects(*p), "at {w} px, {l} {r:?} overlaps pill {i} {p:?}");
            }
            assert!(r.right() <= w + 0.5, "at {w} px, {l} runs off the window: {r:?}");
        }
        for p in &pills {
            assert!(p.right() <= w + 0.5 && p.left() >= 0.0, "at {w} px a pill is off screen: {p:?}");
        }
        // The top bar's buttons stay clear of the frequency display and
        // meters at every width the window allows (1100 px and up).
        if w >= 1100.0 {
            let meters = [rect_of(&h, "SNR"), rect_of(&h, "MHz")];
            for b in ["🙋 Here", "? Help", "📖 Guide", "🛰 Remote", "🔈 Listen", "⚙ Settings"] {
                let r = rect_of(&h, b);
                for m in &meters {
                    assert!(!r.intersects(*m), "at {w} px, {b} {r:?} overlaps {m:?}");
                }
                assert!(r.right() <= w + 0.5, "at {w} px, {b} runs off the window");
            }
        }
        // Readouts don't overlap each other either.
        let rs: Vec<(&str, egui::Rect)> = labels.iter().map(|l| (*l, rect_of(&h, l))).collect();
        for (i, (a, ra)) in rs.iter().enumerate() {
            for (b, rb) in &rs[i + 1..] {
                assert!(!ra.intersects(*rb), "at {w} px, {a} overlaps {b}");
            }
        }
    }
}

/// The whole thing over real Mercury (when it's installed): search, lock,
/// link and chat by clicks, with two real Mercury modems on the simulated
/// band.
#[test]
fn click_real_mercury_demo() {
    if whistlestop::realdemo::find_mercury("").is_none() {
        eprintln!("Mercury isn't installed; skipping");
        return;
    }
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut cfg = demo_cfg("N0GUI");
    cfg.demo_real_mercury = true;
    let mut h = app(cfg, 1400.0, 900.0);
    wait(&mut h, 30.0, "real Mercury and its version", |_, s| s.mercury_ok && s.modem_version.as_deref().is_some_and(|v| v.starts_with("Mercury")));
    click(&mut h, "Current band");
    // Answering is up to the operator: when K7BOT's CQ is heard, click
    // Answer in the Heard card.
    let t = Instant::now();
    while !has_exact_button(&h, "Answer") && t.elapsed() < Duration::from_secs(240) {
        frames(&mut h, 5);
    }
    click_exact(&mut h, "Answer");
    wait(&mut h, 120.0, "link with K7BOT over real Mercury", |_, s| s.link.as_ref().is_some_and(|l| l.peer == "K7BOT"));
    wait(&mut h, 120.0, "greeting", |_, s| s.chat.iter().any(|c| c.who == Who::Them));
    click_exact(&mut h, "💬  Chat");
    type_into(&mut h, "Type a message", "real Mercury click test");
    click(&mut h, "Send");
    wait(&mut h, 150.0, "reply over real Mercury", |_, s| s.chat.iter().any(|c| c.who == Who::Them && c.text.contains("real Mercury click test")));
    click(&mut h, "Disconnect");
    wait(&mut h, 60.0, "link down", |_, s| s.link.is_none());
}

/// Linked to K7BOT: the QSY row is there, and the Files tab browses its
/// library and downloads a file, by clicks.
#[test]
fn click_library_and_qsy_row() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = app(demo_cfg("N0GUI"), 1400.0, 900.0);
    wait(&mut h, 15.0, "Mercury", |_, s| s.mercury_ok);
    type_into(&mut h, "Callsign", "K7BOT");
    click_exact(&mut h, "Connect");
    wait(&mut h, 40.0, "link", |_, s| s.link.is_some());
    assert!(has_text(&h, "Move this link to"));
    click(&mut h, "Files");
    click(&mut h, "Browse");
    wait(&mut h, 30.0, "listing", |_, s| s.library.is_some());
    frames(&mut h, 3);
    assert!(has_text(&h, "k7bot-qsl.txt"));
    click_exact(&mut h, "Download");
    wait(&mut h, 60.0, "downloaded", |_, s| s.transfers.iter().any(|t| t.name == "k7bot-qsl.txt" && matches!(t.state, XferState::Done(_))));
    click_exact(&mut h, "💬  Chat");
    type_into(&mut h, "kHz, e.g. 14106.5", "14106.5");
    click_exact(&mut h, "Ask");
    wait(&mut h, 40.0, "moved", |_, s| s.rig.freq_hz == 14_106_500.0 && s.qsy.is_some());
    assert!(has_text(&h, "back to"));
    click(&mut h, "Disconnect");
    wait(&mut h, 60.0, "home", |_, s| s.link.is_none() && s.qsy.is_none());
}

/// Remote operation by clicks: a station shared on this computer's loopback,
/// operated through the Remote window, then back to the local one.
#[test]
fn click_remote_connect_and_back() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // The "radio computer": a demo station, shared.
    let mut rc = demo_cfg("N0SHACK");
    rc.callsign = "N0SHACK".into();
    let radio = engine::spawn(|| {});
    radio.send(Cmd::Start(rc.clone()));
    let shared = whistlestop::remote::Shared { handle: radio.clone(), cfg: std::sync::Arc::new(std::sync::Mutex::new((0, rc))), devices: (vec![], vec![]) };
    let srv = whistlestop::remote::serve(0, "shack123", shared).unwrap();
    // This computer.
    let mut h = app(demo_cfg("N0DEN"), 1400.0, 900.0);
    wait(&mut h, 15.0, "local station", |_, s| s.mycall == "N0DEN");
    click(&mut h, "Remote");
    assert!(h.state().rm.open);
    type_into(&mut h, "e.g. 192.168.1.20", &format!("127.0.0.1:{}", srv.port));
    type_into(&mut h, "its remote password", "shack123");
    click_exact(&mut h, "Connect to the radio computer");
    wait(&mut h, 15.0, "operating the radio computer", |a, s| a.client.is_some() && s.mycall == "N0SHACK");
    assert!(has_button(&h, "Remote ●"));
    assert_eq!(srv.clients.lock().unwrap().len(), 1);
    // A command from here acts there.
    h.state().h.send(Cmd::SetFreq(14_104_000.0));
    wait(&mut h, 10.0, "tuned remotely", |_, s| s.rig.freq_hz == 14_104_000.0);
    assert_eq!(radio.state.lock().unwrap().rig.freq_hz, 14_104_000.0);
    click(&mut h, "Remote");
    click(&mut h, "Disconnect (back to this computer)");
    wait(&mut h, 15.0, "back to the local station", |a, s| a.client.is_none() && s.mycall == "N0DEN");
    radio.send(Cmd::Stop);
}

#[test]
fn typed_frequencies_in_mhz_khz_or_hz() {
    use super::parse_freq;
    assert_eq!(parse_freq("14103.15"), Some(14_103_150.0)); // kHz
    assert_eq!(parse_freq("14.10315"), Some(14_103_150.0)); // MHz
    assert_eq!(parse_freq("14.1"), Some(14_100_000.0)); // not 14.1 kHz
    assert_eq!(parse_freq("7,1"), Some(7_100_000.0)); // decimal comma
    assert_eq!(parse_freq(" 3585 "), Some(3_585_000.0));
    assert_eq!(parse_freq("14103150"), Some(14_103_150.0)); // Hz
    assert_eq!(parse_freq("50.313"), Some(50_313_000.0)); // 6 m
    assert_eq!(parse_freq("abc"), None);
    assert_eq!(parse_freq("-14100"), None);
    assert_eq!(parse_freq(""), None);
}
