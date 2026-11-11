//! Whistlestop's graphical interface (egui).

mod depot;
mod guide;
pub mod icon;
mod rigwin;
mod help;
mod waterfall;

use help::Help;

use whistlestop::bandplan::{self, SearchScope};
use whistlestop::config::Config;
use whistlestop::e2e::{self, Policy};
use whistlestop::engine::{self, Cmd, Dir, E2eView, Handle, State, Who, XferState};
use whistlestop::modes::{self, Bandwidth};
use whistlestop::protocol;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2,
};
use std::time::{Duration, Instant};
use waterfall::Waterfall;

// ------------------------------------------------------------------ theme

pub const BG: Color32 = Color32::from_rgb(13, 17, 23);
pub const PANEL: Color32 = Color32::from_rgb(19, 25, 33);
pub const CARD: Color32 = Color32::from_rgb(26, 33, 43);
pub const CARD_HI: Color32 = Color32::from_rgb(34, 43, 56);
pub const BORDER: Color32 = Color32::from_rgb(44, 55, 70);
pub const TEXT: Color32 = Color32::from_rgb(222, 228, 236);
pub const DIM: Color32 = Color32::from_rgb(134, 148, 165);
pub const BRASS: Color32 = Color32::from_rgb(242, 169, 59);
pub const TEAL: Color32 = Color32::from_rgb(60, 200, 180);
pub const RED: Color32 = Color32::from_rgb(235, 87, 75);
pub const AMBER: Color32 = Color32::from_rgb(232, 168, 56);
pub const GREEN: Color32 = Color32::from_rgb(70, 190, 100);

fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| {
        use egui::{FontFamily as F, TextStyle as T};
        s.text_styles = [
            (T::Heading, FontId::new(19.0, F::Proportional)),
            (T::Body, FontId::new(14.0, F::Proportional)),
            (T::Monospace, FontId::new(13.5, F::Monospace)),
            (T::Button, FontId::new(14.0, F::Proportional)),
            (T::Small, FontId::new(11.5, F::Proportional)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 7.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        let v = &mut s.visuals;
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.extreme_bg_color = Color32::from_rgb(10, 13, 18);
        v.faint_bg_color = CARD;
        v.override_text_color = Some(TEXT);
        v.window_corner_radius = CornerRadius::same(12);
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.selection.bg_fill = BRASS.linear_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, BRASS);
        v.hyperlink_color = BRASS;
        for (w, fill) in [
            (&mut v.widgets.noninteractive, CARD),
            (&mut v.widgets.inactive, CARD_HI),
            (&mut v.widgets.hovered, Color32::from_rgb(46, 58, 75)),
            (&mut v.widgets.active, Color32::from_rgb(58, 72, 92)),
            (&mut v.widgets.open, CARD_HI),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.corner_radius = CornerRadius::same(7);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, BRASS.linear_multiply(0.6));
    });
}

fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::NONE
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(90) })
        .corner_radius(10.0)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title.to_uppercase()).size(11.0).color(DIM).strong());
            ui.add_space(2.0);
            add(ui)
        })
        .inner
}

/// Show a file in Finder / Explorer / the Linux file manager.
fn reveal(p: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(p).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(p.parent().unwrap_or(p)).spawn();
    let _ = r;
}

fn pill(ui: &mut Ui, label: &str, ok: bool) -> egui::Response {
    let color = if ok { GREEN } else { RED };
    Frame::NONE
        .fill(color.linear_multiply(0.14))
        .stroke(Stroke::new(1.0, color.linear_multiply(0.6)))
        .corner_radius(20.0)
        .inner_margin(Margin::symmetric(5, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, color);
                ui.label(RichText::new(label).size(11.5).color(TEXT));
            });
        })
        .response
}

fn big_button(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 34.0],
        egui::Button::new(RichText::new(text).strong().size(15.0).color(Color32::from_rgb(20, 16, 10))).fill(color).corner_radius(8.0),
    )
}

fn fmt_freq(hz: f64) -> String {
    let hz = hz.round().max(0.0) as u64;
    format!("{}.{:03}.{:03}", hz / 1_000_000, (hz / 1000) % 1000, hz % 1000)
}

pub fn guide_window_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("whistlestop-guide")
}

// -------------------------------------------------------------------- app

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Chat,
    Roundtable,
    Files,
    Log,
    Depot,
}

pub struct App {
    h: Handle,
    cfg: Config,
    tab: Tab,
    chat_input: String,
    table_input: String,
    connect_to: String,
    qsy_khz: String,
    join_name: String,
    cq_table: bool,
    cq_table_name: String,
    settings_open: bool,
    freq_input: String,
    search_band: String,
    power: f32,
    zip: bool,
    audio_wf: Waterfall,
    wide_wf: Waterfall,
    devices: (Vec<String>, Vec<String>),
    sdr_span: f64,
    toast: Option<(Instant, String)>,
    plan_bytes: Option<(String, u64)>,
    logo: Option<egui::TextureHandle>,
    last_err: Option<String>,
    guide: guide::Guide,
    /// The Guide's own window is open.
    guide_open: bool,
    depot_tab: depot::DepotTab,
    rigwin: rigwin::RigWindow,
    /// Settings: radio detection results and the model filter.
    detected: Vec<whistlestop::rigsetup::RadioGuess>,
    model_filter: String,
    wf_share: f32,
    call_cq_at: Option<Pos2>,
    call_cq_hovered: bool,
    hover_sent: Option<Pos2>,
    hover_at: Option<Pos2>,
    /// Encryption setting the operator picked, waiting for them to confirm
    /// the legal warning.
    e2e_confirm: Option<Policy>,
    /// WHISTLESTOP_SHOTS=<dir>: a scripted tour of the demo, with screenshots.
    tour: Option<Tour>,
}

struct Tour {
    dir: std::path::PathBuf,
    step: usize,
    at: Instant,
    started: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        style(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let h = engine::spawn(move || ctx.request_repaint());
        let cfg = Config::load();
        h.send(Cmd::Start(cfg.clone()));
        let first_run = !whistlestop::config::path().exists();
        Self {
            h,
            tab: Tab::Chat,
            chat_input: String::new(),
            table_input: String::new(),
            connect_to: String::new(),
            qsy_khz: String::new(),
            join_name: String::new(),
            cq_table: false,
            cq_table_name: "Evening Table".into(),
            settings_open: first_run,
            freq_input: String::new(),
            search_band: "20m".into(),
            power: 0.5,
            zip: true,
            audio_wf: Waterfall::new("audio-wf", 512, 160),
            wide_wf: Waterfall::new("wide-wf", whistlestop::sdr::COLS, 160),
            devices: whistlestop::audio::list_devices(),
            sdr_span: cfg.sdr.span_hz,
            cfg,
            toast: None,
            plan_bytes: None,
            logo: None,
            last_err: None,
            guide: guide::Guide::default(),
            guide_open: false,
            depot_tab: depot::DepotTab::default(),
            rigwin: rigwin::RigWindow::default(),
            detected: vec![],
            model_filter: String::new(),
            wf_share: 0.30,
            call_cq_at: None,
            call_cq_hovered: false,
            hover_sent: None,
            hover_at: None,
            e2e_confirm: None,
            tour: std::env::var_os("WHISTLESTOP_SHOTS").map(|d| Tour {
                dir: d.into(),
                step: 0,
                at: Instant::now(),
                started: Instant::now(),
            }),
        }
    }

    fn send(&self, c: Cmd) {
        self.h.send(c);
    }

    fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((Instant::now(), s.into()));
    }

    // ------------------------------------------------------------- top bar

    fn top_bar(&mut self, ui: &mut Ui, s: &State) {
        ui.horizontal(|ui| {
            self.wordmark(ui);
            ui.add_space(14.0);
            // Frequency, like a radio's display; it glows red on transmit.
            Frame::NONE
                .fill(Color32::from_rgb(8, 11, 16))
                .stroke(Stroke::new(if s.tx { 1.5 } else { 1.0 }, if s.tx { RED } else { BORDER }))
                .shadow(egui::epaint::Shadow { offset: [0, 0], blur: if s.tx { 18 } else { 0 }, spread: 0, color: RED.linear_multiply(0.45) })
                .corner_radius(10.0)
                .inner_margin(Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let lamp = if s.tx { RED } else { GREEN };
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 6.0, lamp);
                        ui.add_sized([24.0, 18.0], egui::Label::new(RichText::new(if s.tx { "TX" } else { "RX" }).monospace().strong().color(lamp)));
                        ui.add_space(4.0);
                        ui.label(RichText::new(fmt_freq(s.rig.freq_hz)).font(FontId::monospace(28.0)).color(BRASS));
                        ui.label(RichText::new("MHz").color(DIM));
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_width(58.0);
                            ui.label(
                                RichText::new(bandplan::band_of(s.rig.freq_hz).unwrap_or("—")).strong().color(TEXT),
                            );
                            ui.label(RichText::new(if s.rig.mode.is_empty() { "—" } else { &s.rig.mode }).small().color(DIM));
                        });
                    });
                })
                .response
                .help(&help::FREQ);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space(0.0);
                    self.meter(ui, "S", s.rig.strength_db.map(|d| ((d + 54) as f32 / 114.0).clamp(0.0, 1.0)), TEAL);
                    ui.add_sized(
                        [58.0, 16.0],
                        egui::Label::new(
                            RichText::new(s.rig.strength_db.map(|d| if d >= 0 { format!("S9+{d}") } else { format!("S{}", ((d + 54) / 6).clamp(0, 9)) }).unwrap_or("—".into()))
                                .monospace()
                                .color(DIM),
                        ),
                    );
                })
                .response
                .help(&help::S_METER);
                ui.horizontal(|ui| {
                    let snr = s.snr_db.map(|v| ((v + 15.0) / 40.0).clamp(0.0, 1.0) as f32);
                    self.meter(ui, "SNR", snr, BRASS);
                    ui.add_sized([58.0, 16.0], egui::Label::new(RichText::new(s.snr_db.map(|v| format!("{v:+.0} dB")).unwrap_or("—".into())).monospace().color(DIM)));
                })
                .response
                .help(&help::SNR_METER);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("⚙ Settings").size(14.0))).help(&help::SETTINGS).clicked() {
                    self.settings_open = true;
                }
                let lon = s.monitor_on;
                let lb = egui::Button::new(RichText::new(if lon { "🔊 Listen" } else { "🔈 Listen" }).size(14.0).strong().color(if lon { BG } else { TEXT }))
                    .fill(if lon { GREEN } else { CARD_HI });
                if ui.add(lb).help(&help::LISTEN).clicked() {
                    self.cfg.monitor = !lon;
                    let _ = self.cfg.save();
                    self.send(Cmd::Monitor(!lon));
                }
                let gb = egui::Button::new(RichText::new("📖 Guide").size(14.0).strong().color(if self.guide_open { BG } else { TEXT }))
                    .fill(if self.guide_open { TEAL } else { CARD_HI });
                if ui.add(gb).help(&help::GUIDE_BTN).clicked() {
                    self.guide_open = !self.guide_open;
                }
                let on = help::on();
                let hb = egui::Button::new(RichText::new("? Help").size(14.0).strong().color(if on { BG } else { TEXT })).fill(if on { BRASS } else { CARD_HI });
                if ui.add(hb).help(&help::HELP_TOGGLE).clicked() {
                    help::set(!on);
                }
                if s.demo {
                    ui.label(RichText::new(" DEMO ").strong().size(12.0).color(BG).background_color(BRASS)).help(&help::DEMO);
                }
                let away = s.away;
                let ab = egui::Button::new(RichText::new(if away { "🚪 Away" } else { "🙋 Here" }).size(14.0).strong().color(if away { BG } else { TEXT }))
                    .fill(if away { AMBER } else { CARD_HI });
                if ui.add(ab).help(&help::AWAY).clicked() {
                    self.send(Cmd::SetAway(!away));
                }
            });
        });
    }

    fn wordmark(&mut self, ui: &mut Ui) {
        let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        icon::paint(ui.painter(), r, BRASS);
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("WHISTLESTOP").size(21.0).strong().color(TEXT).extra_letter_spacing(2.5));
            if ui.ctx().content_rect().width() > 1560.0 {
                ui.label(RichText::new("HF chat · files · roundtables over Mercury").size(11.5).color(DIM));
            }
        });
    }

    fn meter(&self, ui: &mut Ui, label: &str, v: Option<f32>, color: Color32) {
        ui.label(RichText::new(label).size(10.5).color(DIM));
        let (r, _) = ui.allocate_exact_size(vec2(84.0, 9.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, 4.0, Color32::from_rgb(10, 13, 18));
        if let Some(v) = v {
            let mut f = r;
            f.set_width(r.width() * v);
            p.rect_filled(f, 4.0, color);
        }
    }

    // --------------------------------------------------------- modem strip

    /// What the modem is doing right now, spelled out.
    fn modem_strip(&mut self, ui: &mut Ui, s: &State) {
        let user_bw = s.bandwidth.unwrap_or(self.cfg.bandwidth);
        let (mode, context) = match &s.link {
            Some(l) => (l.mode, format!("1:1 link with {}", l.peer)),
            None if s.table.is_some() => (s.listen_mode, "Roundtable (broadcast)".to_string()),
            None => (s.listen_mode, "idle · listening".to_string()),
        };
        let seg = |ui: &mut Ui, label: &str, value: RichText, sub: Option<String>, topic: &help::Topic| {
            // Each readout has a fixed width, so changing values never push
            // their neighbours around.
            let w = match label {
                "MODEM" => 132.0,
                "MODE" => 104.0,
                "CARRIERS" => 92.0,
                "BANDWIDTH" => 184.0,
                "SPEED" => 128.0,
                "ERROR CORRECTION" => 160.0,
                _ => 104.0,
            };
            ui.allocate_ui_with_layout(vec2(w, 46.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(label).size(10.0).color(DIM).strong());
                ui.label(value);
                if let Some(sub) = sub {
                    ui.label(RichText::new(sub).size(10.5).color(DIM));
                }
            })
            .response
            .help(topic);
            // A fixed-height divider (a separator would stretch to fill).
            let (r, _) = ui.allocate_exact_size(vec2(9.0, 46.0), Sense::hover());
            ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, BORDER));
        };
        // Pills in a fixed column at the right; the readouts (fixed widths)
        // fill the rest and wrap to a second row only in a narrow window,
        // so nothing ever runs underneath anything else.
        let sdr_on = self.h.sdr.lock().unwrap().running;
        const PILLS_W: f32 = 270.0;
        let full = ui.available_width();
        let top = ui.cursor().min;
        {
            let left_w = (full - PILLS_W - 8.0).max(300.0);
            let left_rect = Rect::from_min_size(top, vec2(left_w, 400.0));
            let used = ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect).layout(Layout::left_to_right(Align::Min).with_main_wrap(true)), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let (state, col) = if s.tx { ("TRANSMITTING", RED) } else { ("RECEIVING", GREEN) };
            let state = RichText::new(state).monospace().strong();
            seg(ui, "MODEM", state.color(col), Some(context), &help::M_STATE);
            match mode {
                Some(m) => {
                    seg(ui, "MODE", RichText::new(m.name).strong().size(15.0).color(BRASS), Some(m.modulation.to_string()), &help::M_MODE);
                    let tones = if m.modulation.contains("MFSK") { "tones" } else { "carriers" };
                    seg(ui, "CARRIERS", RichText::new(format!("{} {tones}", m.carriers)).strong(), Some(format!("{:.1} Hz apart", m.spacing_hz)), &help::M_CARRIERS);
                    seg(
                        ui,
                        "BANDWIDTH",
                        RichText::new(format!("{} Hz used", m.bandwidth_hz)).strong(),
                        Some(match &s.link {
                            Some(l) => format!("limit {} · agreed {} Hz", user_bw, l.bw),
                            None => format!("your limit {user_bw}"),
                        }),
                        &help::M_BW,
                    );
                    let measured = if s.link.is_some() && s.est_bps > 0.0 { format!("measured {:.0} bps", s.est_bps) } else { "raw rate of this mode".into() };
                    seg(ui, "SPEED", RichText::new(format!("{:>5.0} bps", s.modem_bps.filter(|_| s.link.is_some()).unwrap_or(m.payload_bps))).monospace().strong().color(TEAL), Some(measured), &help::M_SPEED);
                    seg(ui, "ERROR CORRECTION", RichText::new(m.fec).strong(), Some(format!("{} B per {:.1} s frame", m.frame_bytes, m.frame_s)), &help::M_FEC);
                }
                None => seg(ui, "MODE", RichText::new("connecting…").color(DIM), None, &help::M_MODE),
            }
            seg(ui, "SNR", RichText::new(s.snr_db.map(|v| format!("{v:+5.1} dB")).unwrap_or("—".into())).monospace().strong(), Some("Mercury's measure".into()), &help::SNR_METER);
            // The speed ladder: where Mercury sits, and the rungs it can use.
            let session_bw = s.link.as_ref().map(|l| l.bw).unwrap_or(user_bw.mercury_arq_bw());
            let ladder = modes::arq_ladder(session_bw);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new("SPEED LADDER").size(10.0).color(DIM).strong());
                let (r, resp) = ui.allocate_exact_size(vec2(16.0 * ladder.len() as f32, 26.0), Sense::hover());
                let p = ui.painter();
                let mut tip = vec![];
                for (i, m) in ladder.iter().enumerate() {
                    let h = 6.0 + 20.0 * (i + 1) as f32 / ladder.len() as f32;
                    let x = r.left() + i as f32 * 16.0;
                    let bar = Rect::from_min_max(Pos2::new(x + 2.0, r.bottom() - h), Pos2::new(x + 13.0, r.bottom()));
                    let on = mode.is_some_and(|c| c.name == m.name) && s.link.is_some();
                    p.rect_filled(bar, 2.0, if on { BRASS } else { CARD_HI });
                    tip.push(format!("{}{}: {} · {:.0} bps · needs ≈{:+.0} dB", if on { "▶ " } else { "   " }, m.name, m.modulation, m.payload_bps, m.min_snr_db));
                }
                let resp = if help::on() { resp.help(&help::M_LADDER) } else { resp };
                resp.on_hover_text(format!(
                    "Mercury climbs and descends this ladder by itself during a 1:1 link,\nfrom what actually gets through. Session bandwidth {session_bw} Hz.\n\n{}",
                    tip.join("\n")
                ));
                ui.label(RichText::new(if s.link.is_some() { "automatic" } else { "used when linked" }).size(10.5).color(DIM));
            });
            }).response.rect;
            let pills_rect = Rect::from_min_size(Pos2::new(top.x + full - PILLS_W, top.y), vec2(PILLS_W, used.height().max(46.0)));
            ui.scope_builder(egui::UiBuilder::new().max_rect(pills_rect).layout(Layout::right_to_left(Align::Center)), |ui| {
                pill(ui, "SDR", sdr_on).help(&help::PILLS);
                pill(ui, "Audio", s.audio_ok).help(&help::PILLS);
                let mp = pill(ui, "Mercury", s.mercury_ok).help(&help::PILLS);
                if let Some(v) = &s.modem_version {
                    mp.on_hover_text(v.as_str());
                }
                pill(ui, "Radio", s.rig_ok).help(&help::PILLS);
            });
        }
    }

    // ---------------------------------------------------------- left panel

    fn left(&mut self, ui: &mut Ui, s: &State) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.cq_card(ui, s);
            ui.add_space(8.0);
            self.search_card(ui, s);
            ui.add_space(8.0);
            self.heard_card(ui, s);
            ui.add_space(8.0);
            self.bandwidth_card(ui, s);
            ui.add_space(8.0);
            self.radio_card(ui, s);
        });
    }

    fn radio_card(&mut self, ui: &mut Ui, s: &State) {
        card(ui, "Radio", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (band, hz) in [
                    ("80", 3_585_000.0),
                    ("40", 7_100_000.0),
                    ("30", 10_140_000.0),
                    ("20", 14_100_500.0),
                    ("17", 18_105_000.0),
                    ("15", 21_090_000.0),
                    ("12", 24_925_000.0),
                    ("10", 28_120_000.0),
                ] {
                    let on = bandplan::band_of(s.rig.freq_hz) == Some(&*format!("{band}m"));
                    let b = egui::Button::new(RichText::new(band).monospace().color(if on { BG } else { TEXT }))
                        .fill(if on { BRASS } else { CARD_HI })
                        .min_size(vec2(34.0, 26.0));
                    if ui.add(b).on_hover_text(format!("{band} m digital segment")).help(&help::R_BANDS).clicked() {
                        self.send(Cmd::SetFreq(hz));
                    }
                }
            });
            ui.horizontal(|ui| {
                for (lbl, d) in [("−1k", -1000.0), ("−100", -100.0), ("+100", 100.0), ("+1k", 1000.0)] {
                    if ui.small_button(lbl).help(&help::R_NUDGE).clicked() {
                        self.send(Cmd::SetFreq(s.rig.freq_hz + d));
                    }
                }
            });
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.freq_input).hint_text("kHz, e.g. 14103.15").desired_width(130.0)).help(&help::R_FREQ);
                let go = ui.button("Tune").help(&help::R_FREQ).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go {
                    if let Ok(k) = self.freq_input.trim().replace(',', ".").parse::<f64>() {
                        let hz = if k > 100_000.0 { k } else { k * 1000.0 };
                        self.send(Cmd::SetFreq(hz));
                        self.freq_input.clear();
                    } else {
                        self.toast("Type a frequency in kHz");
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Mode").color(DIM));
                for m in ["PKTUSB", "USB", "LSB"] {
                    if ui.selectable_label(s.rig.mode == m, m).help(&help::R_MODE).clicked() {
                        self.send(Cmd::SetMode(m.into()));
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Power").color(DIM));
                if let Some(p) = s.rig.rf_power
                    && !ui.ctx().egui_is_using_pointer() {
                        self.power = p;
                    }
                let r = ui.add(egui::Slider::new(&mut self.power, 0.0..=1.0).show_value(false)).help(&help::R_POWER);
                ui.label(RichText::new(format!("{:.0}%", self.power * 100.0)).monospace());
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    self.send(Cmd::SetPower(self.power));
                }
            });
            if let Some(n) = &s.schedule_now {
                ui.label(RichText::new(format!("🗓 Schedule: {n}")).color(TEAL)).help(&help::ST_SCHED_ON);
            }
            ui.horizontal(|ui| {
                let b = egui::Button::new(RichText::new("🎛  Radio controls").strong().color(if self.rigwin.open { BG } else { TEXT })).fill(if self.rigwin.open { TEAL } else { CARD_HI });
                if ui.add(b).help(&help::RC_OPEN).clicked() {
                    self.rigwin.open = !self.rigwin.open;
                    self.send(Cmd::RigWatch(self.rigwin.open));
                }
                if let Some(v) = s.rig_ext.swr.or(s.rig_ext.swr_last_tx) {
                    let bad = v > self.cfg.swr_limit;
                    ui.add_sized([86.0, 20.0], egui::Label::new(RichText::new(format!("SWR {v:.1}")).monospace().color(if bad { RED } else { DIM }))).help(&help::RC_SWR);
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Tune carrier").on_hover_text("Mercury sends a 1 kHz tone for your antenna tuner (60 s max)").help(&help::R_TUNE).clicked() {
                    self.send(Cmd::Tune(true));
                }
                if ui.button("Stop").help(&help::R_TUNE).clicked() {
                    self.send(Cmd::Tune(false));
                }
            });
        });
    }

    fn bandwidth_card(&mut self, ui: &mut Ui, s: &State) {
        card(ui, "Bandwidth  ·  narrow → wide", |ui| {
            let cur = s.bandwidth.unwrap_or(self.cfg.bandwidth);
            let w = (ui.available_width() - 5.0 * 4.0) / 6.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for b in Bandwidth::ALL {
                    let on = b == cur;
                    let txt = if b.hz() >= 1000 { format!("{:.1}k", b.hz() as f32 / 1000.0) } else { b.hz().to_string() };
                    let btn = egui::Button::new(RichText::new(txt).monospace().size(12.5).color(if on { BG } else { TEXT }))
                        .fill(if on { TEAL } else { CARD_HI })
                        .min_size(vec2(w, 28.0));
                    if ui.add(btn).on_hover_text(format!("{b}: Roundtable up to {}", b.modes().last().map(|m| m.name).unwrap_or("-"))).help(&help::BW_BUTTONS).clicked() {
                        self.cfg.bandwidth = b;
                        let _ = self.cfg.save();
                        self.send(Cmd::SetBandwidth(b));
                    }
                }
            });
            // A bar showing how much of the passband is used.
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 4.0, Color32::from_rgb(10, 13, 18));
            let frac = cur.hz() as f32 / 2750.0;
            let mut f = Rect::from_center_size(r.center(), vec2(r.width() * frac, r.height()));
            f = f.intersect(r);
            p.rect_filled(f, 4.0, TEAL.linear_multiply(0.7));
            let top = cur.modes().last().copied();
            ui.label(
                RichText::new(format!(
                    "1:1 link: Mercury BW{} · Roundtable max {} ({} bps)",
                    cur.mercury_arq_bw(),
                    top.map(|m| m.name).unwrap_or("-"),
                    top.map(|m| m.payload_bps).unwrap_or(0.0)
                ))
                .small()
                .color(DIM),
            );
        });
    }

    fn cq_card(&mut self, ui: &mut Ui, s: &State) {
        card(ui, "Call CQ", |ui| {
            if s.cq.active {
                ui.label(RichText::new(format!("Cycle {}/{} — {}", s.cq.cycle, s.cq.cycles, s.cq.phase)).color(BRASS).strong());
                ui.add(egui::ProgressBar::new(s.cq.cycle as f32 / s.cq.cycles.max(1) as f32).desired_height(6.0));
                if big_button(ui, "■  Stop CQ", RED).help(&help::CQ_STOP).clicked() {
                    self.send(Cmd::StopCq);
                }
                return;
            }
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.cq_table, false, "Chat (1:1)").help(&help::CQ_KIND);
                ui.selectable_value(&mut self.cq_table, true, "Roundtable").help(&help::CQ_KIND);
            });
            if self.cq_table {
                ui.add(egui::TextEdit::singleline(&mut self.cq_table_name).hint_text("Roundtable name").desired_width(f32::INFINITY)).help(&help::CQ_TABLE_NAME);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Whistle").color(DIM));
                if ui.add(egui::Slider::new(&mut self.cfg.whistle_s, 4.0..=60.0).suffix(" s").integer()).help(&help::CQ_WHISTLE).changed() {
                    let _ = self.cfg.save();
                }
            });
            ui.label(
                RichText::new("Whistle ≥ searchers' scan cycle. Then Mercury sends your CQFRAME (ID) and listens.")
                    .small()
                    .color(DIM),
            );
            let dis = s.link.is_some() || !s.mercury_ok;
            ui.add_enabled_ui(!dis, |ui| {
                let call = big_button(ui, "🚂  Call CQ", BRASS).help(&help::CQ_CALL);
                self.call_cq_at = Some(call.rect.center());
                self.call_cq_hovered = call.hovered();
                if call.clicked() {
                    let table = self.cq_table.then(|| self.cq_table_name.clone());
                    self.send(Cmd::Update(self.cfg.clone()));
                    self.send(Cmd::CallCq { table });
                }
            });
        });
    }

    fn search_card(&mut self, ui: &mut Ui, s: &State) {
        card(ui, "Search for whistles", |ui| {
            if s.search.active {
                ui.label(RichText::new(&s.search.scope).strong());
                ui.label(
                    RichText::new(format!("{} kHz  ·  channel {}/{}", fmt_freq(s.search.channel_hz), s.search.index + 1, s.search.total))
                        .monospace()
                        .color(TEAL),
                );
                ui.add(egui::ProgressBar::new((s.search.index + 1) as f32 / s.search.total.max(1) as f32).desired_height(6.0));
                ui.label(RichText::new(&s.search.note).color(BRASS));
                if big_button(ui, "■  Stop search", RED).help(&help::S_STOP).clicked() {
                    self.send(Cmd::StopSearch);
                }
                return;
            }
            ui.label(
                RichText::new("Sweeps the digital segment and stops on a CQ whistle. Optional: parked on a frequency, the whistle watch still listens.")
                    .small()
                    .color(DIM),
            );
            ui.horizontal(|ui| {
                if ui.button("Current band").help(&help::S_CURRENT).clicked() {
                    self.send(Cmd::StartSearch(SearchScope::CurrentBand));
                }
                if ui.button("All bands").help(&help::S_ALL).clicked() {
                    self.send(Cmd::StartSearch(SearchScope::AllBands));
                }
            });
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("searchband").width(80.0).selected_text(&self.search_band).show_ui(ui, |ui| {
                    let mut bands: Vec<String> = self.cfg.segments.iter().map(|s| s.band.clone()).collect();
                    bands.dedup();
                    for b in bands {
                        ui.selectable_value(&mut self.search_band, b.clone(), b);
                    }
                })
                .response
                .help(&help::S_BAND);
                if ui.button("Search band").help(&help::S_BAND).clicked() {
                    self.send(Cmd::StartSearch(SearchScope::Band(self.search_band.clone())));
                }
            });
            let sdr_on = self.h.sdr.lock().unwrap().running;
            ui.separator();
            ui.add_enabled_ui(sdr_on, |ui| {
                ui.horizontal(|ui| {
                    let mut on = s.wide.on;
                    if ui.checkbox(&mut on, RichText::new("Wide search (SDR)").strong()).on_hover_text("Watch the whole digital segment on the RTL-SDR at once").help(&help::S_WIDE).changed() {
                        self.cfg.sdr.wide_search = on;
                        let _ = self.cfg.save();
                        self.send(Cmd::WideSearch { on, auto: s.wide.auto });
                    }
                    let mut auto = s.wide.auto;
                    if ui.add_enabled(s.wide.on, egui::Checkbox::new(&mut auto, "auto-tune")).on_hover_text("Tune the radio to whistles the SDR finds").help(&help::S_AUTOTUNE).changed() {
                        self.cfg.sdr.wide_auto = auto;
                        let _ = self.cfg.save();
                        self.send(Cmd::WideSearch { on: s.wide.on, auto });
                    }
                });
                if !sdr_on {
                    ui.label(RichText::new("Needs the RTL-SDR (Settings → Wide receiver).").small().color(DIM));
                } else if s.wide.on && !s.wide.note.is_empty() {
                    ui.label(RichText::new(&s.wide.note).small().color(TEAL));
                }
            });
            if ui.checkbox(&mut self.cfg.auto_answer, "Answer automatically").help(&help::S_AUTOANSWER).changed() {
                let _ = self.cfg.save();
                self.send(Cmd::Update(self.cfg.clone()));
            }
        });
    }

    fn heard_card(&mut self, ui: &mut Ui, s: &State) {
        card(ui, "Heard", |ui| {
            for a in s.mail_alerts.iter().filter(|a| a.at.elapsed() < Duration::from_secs(1800)) {
                Frame::NONE.fill(if a.ours { BRASS.linear_multiply(0.15) } else { TEAL.linear_multiply(0.15) }).corner_radius(8.0).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if a.ours {
                            ui.label(RichText::new(format!("📮 {} message(s) for {}", a.count, a.call)).strong().color(BRASS)).help(&help::ALERT_OURS);
                        } else {
                            ui.label(RichText::new(format!("📬 {} has mail for you", a.call)).strong().color(TEAL)).help(&help::ALERT_THEIRS);
                        }
                        if s.link.is_none() {
                            let label = if a.ours { "Deliver now" } else { "Collect" };
                            if ui.small_button(label).help(if a.ours { &help::ALERT_OURS } else { &help::ALERT_THEIRS }).clicked() {
                                self.send(Cmd::DepotConnect(a.call.clone()));
                            }
                        }
                    });
                });
            }
            if let Some(w) = &s.whistle
                && w.at.elapsed() < Duration::from_secs(30) && !s.search.active {
                    Frame::NONE.fill(BRASS.linear_multiply(0.15)).corner_radius(8.0).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.label(RichText::new(format!("🚂 Whistle {:+.0} Hz from here", w.offset_hz)).strong().color(BRASS));
                        if ui.button("Tune to it").help(&help::H_TUNE_TO).clicked() {
                            self.send(Cmd::Nudge(w.offset_hz));
                        }
                    });
                }
            for (dial, score, age) in s.wide.hits.iter().filter(|h| h.2 < 30.0) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SDR").monospace().small().color(BG).background_color(TEAL));
                    ui.label(RichText::new(format!("🚂 {} kHz", fmt_freq(*dial))).strong().color(BRASS));
                    ui.label(RichText::new(format!("{score:.2} · {age:.0}s")).small().color(DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if s.link.is_none() && !s.cq.active && ui.small_button("Lock on").on_hover_text("Tune the radio there and fine-lock on its audio").help(&help::H_LOCK).clicked() {
                            self.send(Cmd::LockOn(*dial));
                        }
                    });
                });
            }
            if s.heard.is_empty() && s.wide.hits.is_empty() {
                ui.label(RichText::new("No CQs heard yet.").color(DIM));
            }
            for h in s.heard.iter().take(8) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(h.at.hhmm()).monospace().small().color(DIM));
                    ui.label(RichText::new(&h.call).strong().color(TEXT));
                    ui.label(RichText::new(&h.what).small().color(DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(t) = &h.table {
                            if ui.small_button("Join").help(&help::H_JOIN).clicked() {
                                self.send(Cmd::JoinTable(t.clone()));
                                self.tab = Tab::Roundtable;
                            }
                        } else if s.link.is_none() && ui.small_button("Answer").help(&help::H_ANSWER).clicked() {
                            self.send(Cmd::Connect(h.call.clone()));
                            self.tab = Tab::Chat;
                        }
                    });
                });
            }
        });
    }

    // -------------------------------------------------------------- centre

    fn centre(&mut self, ui: &mut Ui, s: &State) {
        ui.horizontal(|ui| {
            for (t, label) in [(Tab::Chat, "💬  Chat"), (Tab::Roundtable, "◎  Roundtable"), (Tab::Files, "📁  Files"), (Tab::Depot, "🏤  Depot"), (Tab::Log, "≡  Log")] {
                let on = self.tab == t;
                let badge = match t {
                    Tab::Files => s.transfers.iter().filter(|x| x.state == XferState::WaitingForYou).count(),
                    Tab::Depot => s.depot.unread(),
                    _ => 0,
                };
                let txt = if badge > 0 { format!("{label}  ({badge})") } else { label.to_string() };
                let b = egui::Button::new(RichText::new(txt).size(14.5).color(if on { BG } else { TEXT })).fill(if on { BRASS } else { CARD });
                if ui.add(b).help(if t == Tab::Depot { &help::DEPOT_TAB } else { &help::TABS }).clicked() {
                    self.tab = t;
                }
            }
        });
        ui.add_space(6.0);
        match self.tab {
            Tab::Chat => self.chat_tab(ui, s),
            Tab::Roundtable => self.table_tab(ui, s),
            Tab::Files => self.files_tab(ui, s),
            Tab::Log => self.log_tab(ui, s),
            Tab::Depot => {
                let mut cmds = vec![];
                self.depot_tab.show(ui, s, &mut cmds, self.cfg.relay);
                for c in cmds {
                    self.send(c);
                }
            }
        }
    }

    fn link_header(&mut self, ui: &mut Ui, s: &State) {
        Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10.0).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| match &s.link {
                Some(l) => {
                    ui.label(RichText::new(&l.peer).size(20.0).strong().color(BRASS));
                    let mut who = vec![];
                    if !l.peer_name.is_empty() {
                        who.push(l.peer_name.clone());
                    }
                    if !l.peer_grid.is_empty() {
                        who.push(l.peer_grid.clone());
                    }
                    ui.label(RichText::new(who.join(" · ")).color(TEXT));
                    ui.label(RichText::new(format!("{} Hz", l.bw)).color(DIM));
                    match &l.e2e {
                        E2eView::Off => {}
                        E2eView::Negotiating => {
                            ui.label(RichText::new("⏳ setting up encryption…").color(BRASS))
                                .on_hover_text("Exchanging keys. Chat and files wait until this is done.");
                        }
                        E2eView::Encrypted { sas } => {
                            Frame::NONE.fill(GREEN.linear_multiply(0.25)).stroke(Stroke::new(1.0, GREEN)).corner_radius(6.0).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                                ui.label(RichText::new("🔒 encrypted").strong().color(GREEN));
                                ui.label(RichText::new(format!("code {sas}")).size(18.0).monospace().strong().color(TEXT));
                            })
                            .response
                            .on_hover_text(format!(
                                "Chat and files on this link are encrypted with keys made for this link only, and forgotten when it ends.\n\n\
                                 Read the code {sas} to {} by voice (or any channel no one else controls). \
                                 If their screen shows the same code, no one is in the middle. If it differs, disconnect.",
                                l.peer
                            ));
                        }
                        E2eView::Plain { why } => {
                            ui.label(RichText::new("🔓 not encrypted").color(DIM)).on_hover_text(why.as_str());
                        }
                    }
                    if let Some(r) = l.their_report_db {
                        ui.label(RichText::new(format!("hears you {r:+} dB")).color(TEAL));
                    }
                    if let Some(since) = l.since {
                        ui.label(RichText::new(modes::human_duration(since.elapsed().as_secs_f64())).monospace().color(DIM));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(egui::Button::new("Disconnect (73)").fill(RED.linear_multiply(0.3))).help(&help::L_DISCONNECT).clicked() {
                            self.send(Cmd::Disconnect);
                        }
                    });
                }
                None => {
                    let status = if let Some(c) = &s.calling {
                        format!("Calling {c}…")
                    } else if s.incoming_call {
                        "Incoming call…".into()
                    } else {
                        "Not linked".into()
                    };
                    ui.label(RichText::new(status).size(16.0).color(DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let go = ui.add_enabled(s.mercury_ok, egui::Button::new("Connect").fill(TEAL.linear_multiply(0.35))).clicked();
                        let r = ui.add(egui::TextEdit::singleline(&mut self.connect_to).hint_text("Callsign").desired_width(120.0)).help(&help::L_CONNECT);
                        if (go || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !self.connect_to.trim().is_empty() {
                            self.send(Cmd::Connect(self.connect_to.trim().to_string()));
                        }
                    });
                }
            });
            // Moving the link (QSY).
            if let Some(l) = &s.link {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("↔ Move this link to").color(DIM)).help(&help::L_QSY);
                    let r = ui.add(egui::TextEdit::singleline(&mut self.qsy_khz).hint_text("kHz, e.g. 14106.5").desired_width(120.0)).help(&help::L_QSY);
                    let ask = ui.add_enabled(s.qsy_note.is_none(), egui::Button::new("Ask")).help(&help::L_QSY).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if ask && !self.qsy_khz.trim().is_empty() {
                        if let Ok(k) = self.qsy_khz.trim().replace(',', ".").parse::<f64>() {
                            let hz = if k > 100_000.0 { k } else { k * 1000.0 };
                            self.send(Cmd::Qsy(hz));
                            self.qsy_khz.clear();
                        } else {
                            self.toast("Type a frequency in kHz, e.g. 14106.5");
                        }
                    }
                    if let Some(q) = &s.qsy {
                        ui.label(RichText::new(format!("on {:.3} kHz with {} · back to {:.3} when the link ends", q.at_hz / 1e3, l.peer, q.home_hz / 1e3)).color(TEAL)).help(&help::L_QSY);
                    } else if let Some(n) = &s.qsy_note {
                        ui.label(RichText::new(n).color(BRASS));
                    }
                });
            }
        });
    }

    fn chat_tab(&mut self, ui: &mut Ui, s: &State) {
        self.link_header(ui, s);
        ui.add_space(6.0);
        let input_h = 74.0;
        let h = ui.available_height() - input_h;
        egui::ScrollArea::vertical().max_height(h).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if s.chat.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No messages yet").size(16.0).color(DIM));
                    ui.label(RichText::new("Call CQ, search for whistles, or connect to a callsign.").color(DIM));
                });
            }
            for c in &s.chat {
                match c.who {
                    Who::System => {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(format!("— {} · {} —", c.text, c.at.hhmm())).small().color(DIM));
                        });
                    }
                    Who::Me | Who::Them => {
                        let mine = c.who == Who::Me;
                        let layout = if mine { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) };
                        ui.with_layout(layout, |ui| {
                            let maxw = ui.available_width() * 0.72;
                            Frame::NONE
                                .fill(if mine { BRASS.linear_multiply(0.22) } else { CARD_HI })
                                .corner_radius(CornerRadius { nw: 12, ne: 12, sw: if mine { 12 } else { 3 }, se: if mine { 3 } else { 12 } })
                                .inner_margin(Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.set_max_width(maxw);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(&c.from).strong().color(if mine { BRASS } else { TEAL }));
                                            ui.label(RichText::new(c.at.hhmm()).small().color(DIM));
                                            if mine {
                                                let (t, col) = if c.delivered { ("✓ delivered", TEAL) } else { ("… sending", BRASS) };
                                                ui.label(RichText::new(t).small().color(col));
                                            }
                                        });
                                        ui.add(egui::Label::new(RichText::new(&c.text).size(15.0)).wrap());
                                    });
                                });
                        });
                    }
                }
                ui.add_space(4.0);
            }
        });
        // Input with a live air-time prediction.
        ui.add_space(4.0);
        let bytes = protocol::chat_wire_size(&self.chat_input);
        let bps = if s.link.is_some() { s.est_bps } else { s.est_bps.max(1.0) };
        let eta = bytes as f64 * 8.0 / bps.max(1.0) + 3.0;
        ui.horizontal(|ui| {
            let w = ui.available_width() - 90.0;
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.chat_input)
                    .hint_text(if s.link.is_some() { "Type a message, Enter to send" } else { "Connect to someone to chat" })
                    .desired_width(w)
                    .font(FontId::proportional(15.0))
                    .margin(Margin::symmetric(10, 8)),
            );
            let r = r.help(&help::C_INPUT);
            let send = ui.add_sized([80.0, 34.0], egui::Button::new(RichText::new("Send").strong()).fill(BRASS.linear_multiply(0.5))).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if send && !self.chat_input.trim().is_empty() {
                self.send(Cmd::Chat(self.chat_input.trim().to_string()));
                self.chat_input.clear();
                r.request_focus();
            }
        });
        ui.label(
            RichText::new(if self.chat_input.is_empty() {
                format!("Link speed ≈ {:.0} bps", s.est_bps)
            } else {
                format!("{bytes} bytes on air · ≈ {} at {:.0} bps", modes::human_duration(eta), s.est_bps)
            })
            .small()
            .color(DIM),
        );
    }

    fn table_tab(&mut self, ui: &mut Ui, s: &State) {
        let Some(t) = &s.table else {
            card(ui, "Roundtable", |ui| {
                ui.label("A Roundtable is a group conversation on one frequency. No fixed channel: it's wherever its host calls it.");
                ui.label(RichText::new("The host sets the speed: it ramps up to the fastest mode every member can copy, and drops back if someone fades.").color(DIM));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.join_name).hint_text("Roundtable name").desired_width(220.0));
                    if ui.button("Join here").help(&help::T_JOIN).clicked() && !self.join_name.trim().is_empty() {
                        self.send(Cmd::JoinTable(self.join_name.trim().into()));
                    }
                });
                ui.label(RichText::new("To host one, use Call CQ → Roundtable.").small().color(DIM));
            });
            return;
        };
        Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10.0).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&t.name).size(20.0).strong().color(BRASS));
                ui.label(RichText::new(if t.host { "you host" } else { "member" }).color(DIM));
                ui.label(RichText::new(format!("{} · {:.0} bps · {} B/frame", t.mode.name, t.mode.payload_bps, t.mode.broadcast_bytes())).color(TEAL));
                if let Some(f) = &t.floor {
                    ui.label(RichText::new(format!("floor: {f}")).color(BRASS));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Leave").help(&help::T_LEAVE).clicked() {
                        self.send(Cmd::LeaveTable);
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("At the table:").color(DIM));
                ui.label(RichText::new(format!("{} (you)", s.mycall)).strong());
                let mut members: Vec<_> = t.members.iter().collect();
                members.sort_by_key(|m| m.0.clone());
                for (call, m) in members {
                    let snr = m.snr_of_host.map(|v| format!(" {v:+}dB")).unwrap_or_default();
                    if ui.add(egui::Button::new(RichText::new(format!("{call}{snr}")).color(TEXT)).small()).on_hover_text("Pass the floor").help(&help::T_MEMBER).clicked() {
                        self.send(Cmd::PassFloor(call.clone()));
                    }
                }
            });
        });
        ui.add_space(6.0);
        let h = ui.available_height() - 74.0;
        egui::ScrollArea::vertical().max_height(h).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for (at, from, text) in &t.lines {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(at.hhmm()).monospace().small().color(DIM));
                    if from.is_empty() {
                        ui.label(RichText::new(text).italics().color(DIM));
                    } else {
                        let me = *from == s.mycall;
                        ui.label(RichText::new(from).strong().color(if me { BRASS } else { TEAL }));
                        ui.label(RichText::new(text).size(15.0));
                    }
                });
            }
        });
        let frames = protocol::table_text_frames(t.id, &s.mycall, 1, &self.table_input, t.mode.broadcast_bytes()).map(|f| f.len()).unwrap_or(0);
        ui.horizontal(|ui| {
            let w = ui.available_width() - 90.0;
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.table_input)
                    .hint_text("Say something to the table")
                    .desired_width(w)
                    .font(FontId::proportional(15.0))
                    .margin(Margin::symmetric(10, 8)),
            );
            let r = r.help(&help::T_INPUT);
            let send = ui.add_sized([80.0, 34.0], egui::Button::new(RichText::new("Send").strong()).fill(TEAL.linear_multiply(0.4))).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if send && !self.table_input.trim().is_empty() {
                self.send(Cmd::TableSay(self.table_input.trim().into()));
                self.table_input.clear();
                r.request_focus();
            }
        });
        ui.label(
            RichText::new(if t.sending_eta_s > 0.0 {
                format!("On air: {} left", modes::human_duration(t.sending_eta_s))
            } else if self.table_input.is_empty() {
                format!("Each frame takes {:.1} s on {}", t.mode.frame_s, t.mode.name)
            } else {
                format!("{frames} frame(s) · exactly {} on air", modes::human_duration(frames as f64 * t.mode.frame_s))
            })
            .small()
            .color(DIM),
        );
    }

    fn files_tab(&mut self, ui: &mut Ui, s: &State) {
        if let Some(l) = &s.link {
            let peer = l.peer.clone();
            card(ui, &format!("{peer}'s library"), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("📚  Browse").help(&help::F_LIB_BROWSE).clicked() {
                        self.send(Cmd::LibList(String::new()));
                    }
                    if let Some(v) = &s.library {
                        ui.label(RichText::new(if v.path.is_empty() { "/".to_string() } else { format!("/{}", v.path.trim_matches('/')) }).monospace().color(DIM));
                        if !v.path.trim_matches('/').is_empty() && ui.small_button("⬆ Up").help(&help::F_LIB_BROWSE).clicked() {
                            let p = v.path.trim_matches('/');
                            self.send(Cmd::LibList(p.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default()));
                        }
                    } else {
                        ui.label(RichText::new("See what files they offer, and download any.").color(DIM));
                    }
                });
                if let Some(v) = &s.library {
                    if !v.note.is_empty() {
                        ui.label(RichText::new(&v.note).color(AMBER));
                    } else if v.entries.is_empty() {
                        ui.label(RichText::new("Empty folder.").color(DIM));
                    }
                    let base = v.path.trim_matches('/').to_string();
                    egui::ScrollArea::vertical().id_salt("lib").max_height(170.0).show(ui, |ui| {
                        for e in &v.entries {
                            let full = if base.is_empty() { e.name.clone() } else { format!("{base}/{}", e.name) };
                            ui.horizontal(|ui| {
                                ui.label(if e.dir { "📁" } else { "📄" });
                                ui.add_sized([280.0, 18.0], egui::Label::new(RichText::new(&e.name).color(TEXT)).truncate());
                                if e.dir {
                                    if ui.small_button("Open").help(&help::F_LIB_BROWSE).clicked() {
                                        self.send(Cmd::LibList(full.clone()));
                                    }
                                } else {
                                    ui.add_sized([70.0, 18.0], egui::Label::new(RichText::new(modes::human_bytes(e.size as usize)).monospace().color(DIM)));
                                    if ui.small_button("Download").help(&help::F_LIB_DOWNLOAD).clicked() {
                                        self.send(Cmd::LibFetch(full.clone()));
                                    }
                                }
                            });
                        }
                    });
                }
            });
            ui.add_space(6.0);
        }
        card(ui, "Send files", |ui| {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("+  Files…").fill(BRASS.linear_multiply(0.4))).help(&help::F_FILES).clicked()
                    && let Some(p) = rfd::FileDialog::new().pick_files() {
                        self.send(Cmd::SendFiles { paths: p, zip: self.zip });
                    }
                if ui.add(egui::Button::new("+  Folder…").fill(BRASS.linear_multiply(0.4))).help(&help::F_FOLDER).clicked()
                    && let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.send(Cmd::SendFiles { paths: vec![p], zip: true });
                    }
                ui.checkbox(&mut self.zip, "Zip before sending").help(&help::F_ZIP);
                if ui.button("Estimate…").on_hover_text("How long would a file take?").help(&help::F_ESTIMATE).clicked()
                    && let Some(p) = rfd::FileDialog::new().pick_file() {
                        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        self.plan_bytes = Some((p.file_name().unwrap_or_default().to_string_lossy().into_owned(), size));
                    }
            });
            ui.label(RichText::new("Or drop files on the window. Folders are always zipped. Files are offered first; the other station sees the size and time and accepts.").small().color(DIM));
            if let Some((name, size)) = &self.plan_bytes {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("Estimate for {name} ({})", modes::human_bytes(*size as usize))).strong());
                let snr = s.snr_db.unwrap_or(5.0);
                egui::Grid::new("plan").striped(true).show(ui, |ui| {
                    ui.label(RichText::new("Bandwidth").color(DIM));
                    for v in [-5.0, 0.0, 5.0, 10.0, 20.0] {
                        ui.label(RichText::new(format!("{v:+} dB")).color(DIM));
                    }
                    ui.label(RichText::new("now").color(BRASS));
                    ui.end_row();
                    for b in Bandwidth::ALL {
                        ui.label(b.to_string());
                        for v in [-5.0, 0.0, 5.0, 10.0, 20.0] {
                            ui.label(RichText::new(modes::human_duration(modes::predict_arq_s(protocol::file_wire_size(name, *size), b, v))).monospace());
                        }
                        ui.label(RichText::new(modes::human_duration(modes::predict_arq_s(protocol::file_wire_size(name, *size), b, snr))).monospace().color(BRASS));
                        ui.end_row();
                    }
                });
                if ui.small_button("Close estimate").clicked() {
                    self.plan_bytes = None;
                }
            }
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if s.transfers.is_empty() {
                ui.label(RichText::new("No transfers yet.").color(DIM));
            }
            for t in s.transfers.iter().rev() {
                Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10.0).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if t.dir == Dir::Out { "↑" } else { "↓" }).size(18.0).color(if t.dir == Dir::Out { BRASS } else { TEAL }));
                        ui.label(RichText::new(&t.name).strong().size(15.0));
                        ui.label(RichText::new(modes::human_bytes(t.size as usize)).color(DIM));
                        if t.zipped && t.original > t.size {
                            ui.label(RichText::new(format!("zip of {} file(s), {:.0}% of {}", t.files, 100.0 * t.size as f64 / t.original as f64, modes::human_bytes(t.original as usize))).small().color(DIM));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &t.state {
                            XferState::WaitingForYou => {
                                if ui.add(egui::Button::new("Accept").fill(GREEN.linear_multiply(0.4))).help(&help::F_ACCEPT).clicked() {
                                    self.send(Cmd::AnswerOffer { fid: t.fid, accept: true });
                                }
                                if ui.button("Decline").help(&help::F_DECLINE).clicked() {
                                    self.send(Cmd::AnswerOffer { fid: t.fid, accept: false });
                                }
                                ui.label(RichText::new(format!("≈ {}", modes::human_duration(t.eta_s))).color(BRASS));
                            }
                            XferState::Sending | XferState::Receiving | XferState::Offered => {
                                if ui.small_button("Cancel").help(&help::F_CANCEL).clicked() {
                                    self.send(Cmd::Cancel(t.fid));
                                }
                            }
                            XferState::Done(Some(p)) => {
                                if ui.small_button("Show").clicked() {
                                    reveal(p);
                                }
                                ui.label(RichText::new("✓ received").color(GREEN));
                            }
                            XferState::Done(None) => {
                                ui.label(RichText::new("✓ delivered, checksum good").color(GREEN));
                            }
                            XferState::Failed(e) => {
                                ui.label(RichText::new(e).color(RED));
                            }
                            XferState::Declined => {
                                ui.label(RichText::new("declined").color(DIM));
                            }
                        });
                    });
                    let frac = if t.size == 0 { 1.0 } else { t.done as f32 / t.size as f32 };
                    let label = match &t.state {
                        XferState::Offered => "offered, waiting for an answer".to_string(),
                        XferState::Sending | XferState::Receiving => {
                            let rate = t.started.map(|s| t.done as f64 * 8.0 / s.elapsed().as_secs_f64().max(1.0)).unwrap_or(0.0);
                            format!(
                                "{} / {}  ·  {:.0} bps  ·  {} left",
                                modes::human_bytes(t.done as usize),
                                modes::human_bytes(t.size as usize),
                                rate,
                                modes::human_duration(t.eta_s)
                            )
                        }
                        _ => String::new(),
                    };
                    if !label.is_empty() || frac < 1.0 {
                        ui.add(egui::ProgressBar::new(frac).text(label).fill(if t.dir == Dir::Out { BRASS.linear_multiply(0.7) } else { TEAL.linear_multiply(0.7) }));
                    }
                });
                ui.add_space(6.0);
            }
        });
    }

    fn log_tab(&mut self, ui: &mut Ui, s: &State) {
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for (at, l) in &s.log {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(at.hhmm()).monospace().small().color(DIM));
                    let c = if l.starts_with('⚠') { RED } else if l.contains("Whistle") || l.contains("CQ") { BRASS } else { TEXT };
                    ui.label(RichText::new(l).monospace().color(c));
                });
            }
        });
    }

    // ---------------------------------------------------------- waterfalls

    fn waterfalls(&mut self, ui: &mut Ui, s: &mut State) {
        let ctx = ui.ctx().clone();
        for row in s.audio_rows.drain(..) {
            // 256 bins over 0..4 kHz -> 512 columns over 0..3 kHz.
            let cols: Vec<f32> = (0..512)
                .map(|c| {
                    let f = (c as f32 + 0.5) / 512.0 * 3000.0;
                    let bin = f / 4000.0 * row.len() as f32;
                    let i = (bin as usize).min(row.len() - 2);
                    let u = bin - i as f32;
                    row[i] * (1.0 - u) + row[i + 1] * u
                })
                .collect();
            self.audio_wf.push(&ctx, &cols);
        }
        let wide_rows: Vec<_> = self.h.sdr.lock().unwrap().rows.drain(..).collect();
        for r in wide_rows {
            self.wide_wf.start_hz = r.start_hz;
            self.wide_wf.span_hz = r.bin_hz * r.levels.len() as f64;
            self.wide_wf.push(&ctx, &r.levels);
        }
        let sdr_running = self.h.sdr.lock().unwrap().running;
        // Fill exactly the panel: allocating more would make a resizable
        // panel grow a little every frame.
        let h = ui.available_height();
        let total = ui.available_width();
        // Drag the top edge of the waterfalls to resize them.
        let edge = Rect::from_min_size(ui.max_rect().left_top() - vec2(0.0, 10.0), vec2(total, 8.0));
        let r = ui.interact(edge, ui.scope_id().with("wf-resize"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if r.dragged() {
            let screen_h = ui.ctx().content_rect().height();
            self.wf_share = (self.wf_share - r.drag_delta().y / screen_h).clamp(0.15, 0.6);
        }
        let aw = if sdr_running { (total * 0.32).max(260.0) } else { total };
        ui.allocate_ui_with_layout(vec2(total, h), Layout::left_to_right(Align::Min), |ui| {
            ui.allocate_ui_with_layout(vec2(aw, h), Layout::top_down(Align::Min), |ui| {
                self.audio_waterfall(ui, s, aw);
            });
            if sdr_running {
                let ww = ui.available_width();
                ui.allocate_ui_with_layout(vec2(ww, h), Layout::top_down(Align::Min), |ui| {
                    self.wide_waterfall(ui, s, ww);
                });
            }
        });
    }

    fn audio_waterfall(&mut self, ui: &mut Ui, s: &State, width: f32) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("RADIO AUDIO").size(11.0).strong().color(DIM));
            ui.label(RichText::new("0–3 kHz").size(11.0).color(DIM));
            if let Some(w) = &s.whistle
                && w.at.elapsed() < Duration::from_secs(8) {
                    ui.label(RichText::new(format!("🚂 whistle {:+.0} Hz", w.offset_hz)).size(11.5).strong().color(BRASS));
                }
        });
        let (rect, wr) = ui.allocate_exact_size(vec2(width, ui.available_height().max(40.0)), Sense::hover());
        wr.help(&help::W_AUDIO);
        let p = ui.painter_at(rect);
        self.audio_wf.paint(&p, rect);
        let x = |hz: f32| rect.left() + rect.width() * hz / 3000.0;
        // Whistle sweep band and the receive passband.
        let wp = s.bandwidth.unwrap_or(Bandwidth::Hz2300).whistle_profile();
        p.rect_filled(
            Rect::from_x_y_ranges(x(wp.f_lo as f32)..=x(wp.f_hi as f32), rect.top()..=rect.top() + 3.0),
            0.0,
            BRASS.linear_multiply(0.8),
        );
        for hz in [500.0, 1000.0, 1500.0, 2000.0, 2500.0] {
            p.line_segment([Pos2::new(x(hz), rect.bottom() - 6.0), Pos2::new(x(hz), rect.bottom())], Stroke::new(1.0, DIM));
            p.text(Pos2::new(x(hz), rect.bottom() - 8.0), Align2::CENTER_BOTTOM, format!("{}", hz as u32), FontId::proportional(10.0), DIM);
        }
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    }

    fn wide_waterfall(&mut self, ui: &mut Ui, s: &State, width: f32) {
        let status = self.h.sdr.lock().unwrap().status.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("WIDE RECEIVER").size(11.0).strong().color(DIM));
            ui.label(RichText::new(&status).size(11.0).color(DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for (lbl, span) in [("1.2 MHz", 1_200_000.0), ("400k", 400_000.0), ("100k", 100_000.0), ("25k", 25_000.0)] {
                    if ui.selectable_label((self.sdr_span - span).abs() < 1.0, RichText::new(lbl).size(11.0)).help(&help::W_SPAN).clicked() {
                        self.sdr_span = span;
                        self.send(Cmd::SetSdrSpan(span));
                    }
                }
                ui.label(RichText::new("span").size(11.0).color(DIM));
            });
        });
        let (rect, resp) = ui.allocate_exact_size(vec2(width, ui.available_height().max(40.0)), Sense::click());
        let resp = resp.help(&help::W_WIDE);
        let p = ui.painter_at(rect);
        self.wide_wf.paint(&p, rect);
        let (start, span) = (self.wide_wf.start_hz, self.wide_wf.span_hz.max(1.0));
        let x = |hz: f64| rect.left() + rect.width() * ((hz - start) / span) as f32;
        // Digital segments where whistles are searched for.
        for seg in &self.cfg.segments {
            if seg.hi_hz < start || seg.lo_hz > start + span {
                continue;
            }
            let r = Rect::from_x_y_ranges(x(seg.lo_hz).max(rect.left())..=x(seg.hi_hz).min(rect.right()), rect.top()..=rect.top() + 4.0);
            p.rect_filled(r, 0.0, TEAL.linear_multiply(0.8));
        }
        // Receive passband (USB: dial + 300..2700 Hz).
        let d = s.rig.freq_hz;
        let pb = Rect::from_x_y_ranges(x(d + 300.0)..=x(d + 2700.0).max(x(d + 300.0) + 2.0), rect.top()..=rect.bottom());
        p.rect_filled(pb, 0.0, BRASS.linear_multiply(0.12));
        p.line_segment([Pos2::new(x(d), rect.top()), Pos2::new(x(d), rect.bottom())], Stroke::new(1.5, BRASS));
        // Whistles found by the wide search: a box over the caller's
        // passband, labelled.
        for (dial, _score, age) in s.wide.hits.iter().filter(|h| h.2 < 30.0) {
            let (x0, x1) = (x(dial + 500.0), x(dial + 2500.0).max(x(dial + 500.0) + 3.0));
            if x1 < rect.left() || x0 > rect.right() {
                continue;
            }
            let fade = (1.0 - (*age as f32 / 30.0)).clamp(0.3, 1.0);
            let top = rect.top() + 6.0;
            p.rect_stroke(Rect::from_x_y_ranges(x0..=x1, top..=rect.bottom() - 14.0), 3.0, Stroke::new(1.5, BRASS.linear_multiply(fade)), StrokeKind::Inside);
            p.text(Pos2::new(x1 + 4.0, top), Align2::LEFT_TOP, format!("🚂 {:.2}", dial / 1e3), FontId::proportional(11.0), BRASS.linear_multiply(fade));
        }
        // Frequency scale.
        let step = [1e3, 2e3, 5e3, 10e3, 20e3, 50e3, 100e3, 200e3].into_iter().find(|s| span / s <= 10.0).unwrap_or(500e3);
        let mut f = (start / step).ceil() * step;
        while f < start + span {
            let xx = x(f);
            p.line_segment([Pos2::new(xx, rect.bottom() - 6.0), Pos2::new(xx, rect.bottom())], Stroke::new(1.0, DIM));
            p.text(Pos2::new(xx, rect.bottom() - 8.0), Align2::CENTER_BOTTOM, format!("{:.0}", f / 1e3), FontId::proportional(10.0), DIM);
            f += step;
        }
        if let Some(pos) = resp.hover_pos() {
            let hz = start + (pos.x - rect.left()) as f64 / rect.width() as f64 * span;
            p.line_segment([Pos2::new(pos.x, rect.top()), Pos2::new(pos.x, rect.bottom())], Stroke::new(1.0, Color32::WHITE.linear_multiply(0.4)));
            p.text(pos + vec2(8.0, -14.0), Align2::LEFT_BOTTOM, format!("{:.2} kHz — click to tune", hz / 1e3), FontId::proportional(11.0), TEXT);
        }
        if let (true, Some(pos)) = (resp.clicked(), resp.interact_pointer_pos()) {
            // Put the clicked signal in the middle of the audio passband.
            let hz = start + (pos.x - rect.left()) as f64 / rect.width() as f64 * span;
            self.send(Cmd::SetFreq((hz - 1500.0).round()));
        }
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    }

    fn tour_step(&mut self, ctx: &egui::Context, s: &State) {
        let Some(t) = self.tour.as_mut() else { return };
        let shoot = |path: std::path::PathBuf| {
            egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let mut out = format!("P6\n{} {}\n255\n", img.size[0], img.size[1]).into_bytes();
                for p in &img.pixels {
                    out.extend_from_slice(&[p.r(), p.g(), p.b()]);
                }
                let _ = std::fs::write(&path, out);
            }))
        };
        let since = t.at.elapsed();
        let linked = s.link.is_some();
        let mut next = false;
        let mut cmds = vec![];
        match t.step {
            0 if since > Duration::from_secs(3) => {
                self.settings_open = false;
                self.cfg.auto_answer = true;
                // Show encryption in the tour (never saved: the tour doesn't save).
                self.cfg.e2e = whistlestop::e2e::Policy::Prefer;
                cmds.push(Cmd::Update(self.cfg.clone()));
                cmds.push(Cmd::WideSearch { on: true, auto: true });
                next = true;
            }
            1 if since > Duration::from_secs(9) => {
                ctx.send_viewport_cmd(shoot(t.dir.join("1-search.ppm")));
                next = true;
            }
            2 if linked || t.started.elapsed() > Duration::from_secs(150) => next = true,
            3 if since > Duration::from_secs(3) => {
                cmds.push(Cmd::Chat("Hello from Whistlestop! Found you by your whistle.".into()));
                next = true;
            }
            4 if since > Duration::from_secs(12) => {
                cmds.push(Cmd::Chat("Can you send me a file?".into()));
                next = true;
            }
            5 if since > Duration::from_secs(10) => {
                ctx.send_viewport_cmd(shoot(t.dir.join("2-chat.ppm")));
                next = true;
            }
            6 if since > Duration::from_secs(1) => {
                let demo = std::env::temp_dir().join("whistlestop-tour/Field Day logs");
                let _ = std::fs::create_dir_all(&demo);
                for i in 1..=4 {
                    let _ = std::fs::write(demo.join(format!("log-{i}.adi")), "<call:5>K7BOT <band:3>20m <mode:7>MERCURY <eor>\n".repeat(300));
                }
                cmds.push(Cmd::SendFiles { paths: vec![demo], zip: true });
                self.tab = Tab::Files;
                next = true;
            }
            7 if since > Duration::from_secs(14) => {
                ctx.send_viewport_cmd(shoot(t.dir.join("3-files.ppm")));
                next = true;
            }
            8 if since > Duration::from_secs(1) => {
                cmds.push(Cmd::Disconnect);
                next = true;
            }
            9 if since > Duration::from_secs(4) => {
                cmds.push(Cmd::JoinTable("Evening Table".into()));
                self.tab = Tab::Roundtable;
                next = true;
            }
            10 if since > Duration::from_secs(4) => {
                cmds.push(Cmd::TableSay("Good evening all, N0CALL checking in from the demo.".into()));
                next = true;
            }
            11 if since > Duration::from_secs(24) => {
                ctx.send_viewport_cmd(shoot(t.dir.join("4-roundtable.ppm")));
                self.guide_open = true;
                self.guide.open("The whistle");
                next = true;
            }
            12 if since > Duration::from_secs(2) => {
                ctx.send_viewport_cmd_to(guide_window_id(), shoot(t.dir.join("5-guide.ppm")));
                ctx.send_viewport_cmd(shoot(t.dir.join("5-main-with-guide.ppm")));
                help::set(true);
                self.hover_at = self.call_cq_at;
                next = true;
            }
            13 if since > Duration::from_secs(3) => {
                self.guide_open = false;
                ctx.send_viewport_cmd(shoot(t.dir.join("6-help-hover.ppm")));
                let _ = std::fs::write(t.dir.join("hover.txt"), format!("target {:?} hovered {} pointer {:?}", self.call_cq_at, self.call_cq_hovered, ctx.input(|i| i.pointer.hover_pos())));
                next = true;
            }
            14 if since > Duration::from_secs(1) => {
                help::set(false);
                self.hover_at = None;
                self.rigwin.open = true;
                cmds.push(Cmd::RigWatch(true));
                next = true;
            }
            15 if since > Duration::from_secs(3) => {
                ctx.send_viewport_cmd(shoot(t.dir.join("7-radio-controls.ppm")));
                next = true;
            }
            16 if since > Duration::from_secs(1) => {
                self.rigwin.open = false;
                self.cfg.rig_mode = whistlestop::rigsetup::RigMode::Managed; // tour only, not saved
                self.settings_open = true;
                next = true;
            }
            17 if since > Duration::from_secs(2) => {
                ctx.send_viewport_cmd(shoot(t.dir.join("8-settings.ppm")));
                next = true;
            }
            18 if since > Duration::from_secs(2) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => {}
        }
        if next {
            t.step += 1;
            t.at = Instant::now();
            let log: Vec<String> = s.log.iter().map(|(at, l)| format!("{} {l}", at.hhmm())).collect();
            let _ = std::fs::write(t.dir.join("log.txt"), log.join("\n"));
        }
        for c in cmds {
            self.send(c);
        }
    }

    // ------------------------------------------------------------ settings

    fn settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut apply = false;
        egui::Window::new(RichText::new("Settings").strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                    let c = &mut self.cfg;
                    card(ui, "Station", |ui| {
                        egui::Grid::new("st").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                            ui.label("Callsign");
                            ui.text_edit_singleline(&mut c.callsign).help(&help::ST_CALL);
                            ui.end_row();
                            ui.label("Name");
                            ui.text_edit_singleline(&mut c.name).help(&help::ST_NAMEGRID);
                            ui.end_row();
                            ui.label("Grid");
                            ui.text_edit_singleline(&mut c.grid).help(&help::ST_NAMEGRID);
                            ui.end_row();
                        });
                    });
                    ui.add_space(6.0);
                    card(ui, "Mode", |ui| {
                        ui.checkbox(&mut c.demo, "Demo mode (simulated radio and band, and a demo station)").help(&help::ST_DEMO);
                        ui.add_enabled_ui(c.demo, |ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut c.demo_real_mercury, "Demo on real Mercury").help(&help::ST_DEMO_REAL);
                                let found = whistlestop::realdemo::find_mercury(&c.mercury_bin);
                                ui.label(RichText::new(match &found {
                                    Some(p) => format!("found {}", p.display()),
                                    None => "Mercury not found: install it, or set the path".into(),
                                }).small().color(if found.is_some() { GREEN } else { AMBER }));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Mercury engine");
                                ui.add(egui::TextEdit::singleline(&mut c.mercury_bin).hint_text("automatic").desired_width(300.0)).help(&help::ST_MERCURY_BIN);
                            });
                        });
                        ui.label(RichText::new("Turn off to use your radio: start Mercury, and rigctld (Hamlib) or RigHelm's rigctld server.").small().color(DIM));
                    });
                    ui.add_space(6.0);
                    card(ui, "Mercury modem", |ui| {
                        egui::Grid::new("mc").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                            ui.label("Host");
                            ui.text_edit_singleline(&mut c.mercury_host).help(&help::ST_MERCURY);
                            ui.end_row();
                            ui.label("TNC port");
                            ui.add(egui::DragValue::new(&mut c.mercury_port)).help(&help::ST_MERCURY);
                            ui.end_row();
                            ui.label("Broadcast port");
                            ui.add(egui::DragValue::new(&mut c.mercury_bcast_port)).help(&help::ST_MERCURY);
                            ui.end_row();
                        });
                        ui.label(RichText::new("Set Mercury's radio to Hamlib model 2 (NET rigctl) at the rigctld address below, so both share the radio.").small().color(DIM));
                    });
                    ui.add_space(6.0);
                    card(ui, "Radio connection", |ui| {
                        use whistlestop::rigsetup::{self as rs, RigMode};
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut c.rig_mode, RigMode::External, "Use a running rigctld / RigHelm").help(&help::ST_RIGMODE);
                            ui.selectable_value(&mut c.rig_mode, RigMode::Managed, "Let Whistlestop run rigctld").help(&help::ST_RIGMODE);
                            ui.selectable_value(&mut c.rig_mode, RigMode::Direct, "Direct CAT (built in)").help(&help::ST_CAT);
                        });
                        match c.rig_mode {
                            RigMode::External => {
                                ui.horizontal(|ui| {
                                    ui.label("rigctld address");
                                    ui.text_edit_singleline(&mut c.rigctld).help(&help::ST_RIGCTLD);
                                });
                            }
                            RigMode::Managed => {
                                if rs::hamlib_bin("rigctld").is_none() {
                                    ui.label(RichText::new("Hamlib isn't installed. On macOS: brew install hamlib").color(RED));
                                }
                                ui.horizontal(|ui| {
                                    if ui.button("🔍  Detect radios").help(&help::ST_DETECT).clicked() {
                                        self.detected = rs::detect();
                                    }
                                    ui.label(RichText::new(format!("{} found", self.detected.len())).color(DIM));
                                });
                                for g in self.detected.clone() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&g.what).strong());
                                        ui.label(RichText::new(&g.port).monospace().small().color(DIM));
                                        if ui.small_button("Use").help(&help::ST_DETECT).clicked() {
                                            c.managed_rig.port = g.port.clone();
                                            if let Some(m) = &g.model {
                                                c.managed_rig.model = m.id;
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(&g.why);
                                }
                                let models = rs::hamlib_models();
                                egui::Grid::new("mr").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                                    ui.label("Model");
                                    let cur = models.iter().find(|m| m.id == c.managed_rig.model).map(|m| m.label()).unwrap_or("Automatic (detect at start)".into());
                                    egui::ComboBox::from_id_salt("model").width(300.0).selected_text(cur).show_ui(ui, |ui| {
                                        ui.add(egui::TextEdit::singleline(&mut self.model_filter).hint_text("Type to filter, e.g. 7300"));
                                        ui.selectable_value(&mut c.managed_rig.model, 0, "Automatic (detect at start)");
                                        let f = self.model_filter.to_lowercase();
                                        for m in models.iter().filter(|m| f.is_empty() || m.label().to_lowercase().contains(&f)).take(200) {
                                            ui.selectable_value(&mut c.managed_rig.model, m.id, m.label());
                                        }
                                    })
                                    .response
                                    .help(&help::ST_MODEL);
                                    ui.end_row();
                                    ui.label("Port");
                                    egui::ComboBox::from_id_salt("port").width(300.0).selected_text(if c.managed_rig.port.is_empty() { "Automatic".to_string() } else { c.managed_rig.port.clone() }).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut c.managed_rig.port, String::new(), "Automatic");
                                        for p in rs::all_serial_paths() {
                                            ui.selectable_value(&mut c.managed_rig.port, p.clone(), p);
                                        }
                                    })
                                    .response
                                    .help(&help::ST_PORT);
                                    ui.end_row();
                                    ui.label("Speed");
                                    egui::ComboBox::from_id_salt("baud").selected_text(if c.managed_rig.baud == 0 { "Automatic".to_string() } else { c.managed_rig.baud.to_string() }).show_ui(ui, |ui| {
                                        for b in [0u32, 4800, 9600, 19200, 38400, 57600, 115200] {
                                            ui.selectable_value(&mut c.managed_rig.baud, b, if b == 0 { "Automatic".to_string() } else { b.to_string() });
                                        }
                                    })
                                    .response
                                    .help(&help::ST_BAUD);
                                    ui.end_row();
                                    ui.label("rigctld port");
                                    ui.add(egui::DragValue::new(&mut c.managed_rig.listen_port).range(1024..=65535)).help(&help::ST_RIGPORT);
                                    ui.end_row();
                                });
                                ui.label(RichText::new(format!("Mercury: Hamlib model 2 (NET rigctl) at 127.0.0.1:{}. RTS and DTR are held off so connecting can't key the radio.", c.managed_rig.listen_port)).small().color(DIM));
                            }
                            RigMode::Direct => {
                                use whistlestop::cat::Family;
                                ui.label(RichText::new("Whistlestop talks to the radio itself (Icom CI-V, Kenwood, Elecraft, Yaesu): no Hamlib needed.").small().color(DIM));
                                ui.horizontal(|ui| {
                                    if ui.button("🔍  Detect radios").help(&help::ST_DETECT).clicked() {
                                        self.detected = rs::detect();
                                    }
                                    ui.label(RichText::new(format!("{} found", self.detected.len())).color(DIM));
                                });
                                for g in self.detected.clone() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&g.what).strong());
                                        ui.label(RichText::new(&g.port).monospace().small().color(DIM));
                                        if ui.small_button("Use").help(&help::ST_DETECT).clicked() {
                                            c.cat.port = g.port.clone();
                                            let f = Family::from_description(&g.what);
                                            if f != Family::Auto {
                                                c.cat.family = f;
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(&g.why);
                                }
                                egui::Grid::new("cat").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                                    ui.label("Protocol");
                                    egui::ComboBox::from_id_salt("catfam").width(300.0).selected_text(c.cat.family.label()).show_ui(ui, |ui| {
                                        for f in Family::ALL {
                                            ui.selectable_value(&mut c.cat.family, f, f.label());
                                        }
                                    })
                                    .response
                                    .help(&help::ST_CAT_FAMILY);
                                    ui.end_row();
                                    ui.label("Port");
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt("catport").width(300.0).selected_text(if c.cat.port.is_empty() { "Automatic".to_string() } else { c.cat.port.clone() }).show_ui(ui, |ui| {
                                            ui.selectable_value(&mut c.cat.port, String::new(), "Automatic");
                                            for p in rs::all_serial_paths() {
                                                ui.selectable_value(&mut c.cat.port, p.clone(), p);
                                            }
                                        })
                                        .response
                                        .help(&help::ST_CAT_PORT);
                                    });
                                    ui.end_row();
                                    ui.label("or network address");
                                    ui.add(egui::TextEdit::singleline(&mut c.cat.port).hint_text("host:port, for a serial-over-network bridge").desired_width(300.0)).help(&help::ST_CAT_PORT);
                                    ui.end_row();
                                    ui.label("Speed");
                                    egui::ComboBox::from_id_salt("catbaud").selected_text(if c.cat.baud == 0 { "Automatic".to_string() } else { c.cat.baud.to_string() }).show_ui(ui, |ui| {
                                        for b in [0u32, 4800, 9600, 19200, 38400, 57600, 115200] {
                                            ui.selectable_value(&mut c.cat.baud, b, if b == 0 { "Automatic".to_string() } else { b.to_string() });
                                        }
                                    })
                                    .response
                                    .help(&help::ST_CAT_BAUD);
                                    ui.end_row();
                                    if matches!(c.cat.family, Family::Icom | Family::Auto) {
                                        ui.label("CI-V address");
                                        ui.horizontal(|ui| {
                                            let mut auto = c.cat.civ_addr == 0;
                                            if ui.checkbox(&mut auto, "Ask the radio").help(&help::ST_CAT_CIV).changed() {
                                                c.cat.civ_addr = if auto { 0 } else { 0x94 };
                                            }
                                            if !auto {
                                                let mut hex = format!("{:02X}", c.cat.civ_addr);
                                                if ui.add(egui::TextEdit::singleline(&mut hex).desired_width(36.0).char_limit(2)).help(&help::ST_CAT_CIV).changed()
                                                    && let Ok(v) = u8::from_str_radix(hex.trim(), 16)
                                                {
                                                    c.cat.civ_addr = v;
                                                }
                                                ui.label(RichText::new(format!("h  ({})", whistlestop::cat::icom::model_name(c.cat.civ_addr))).color(DIM));
                                            }
                                        });
                                        ui.end_row();
                                    }
                                    ui.label("Full power");
                                    ui.add(egui::DragValue::new(&mut c.cat.max_power_w).range(1.0..=2000.0).suffix(" W")).help(&help::ST_CAT_POWER);
                                    ui.end_row();
                                    ui.label("Share with Mercury on port");
                                    ui.horizontal(|ui| {
                                        let mut on = c.cat.share_port != 0;
                                        if ui.checkbox(&mut on, "").help(&help::ST_CAT_SHARE).changed() {
                                            c.cat.share_port = if on { 4532 } else { 0 };
                                        }
                                        if on {
                                            ui.add(egui::DragValue::new(&mut c.cat.share_port).range(1024..=65535)).help(&help::ST_CAT_SHARE);
                                        }
                                    });
                                    ui.end_row();
                                });
                                if c.cat.share_port != 0 {
                                    ui.label(RichText::new(format!("Mercury: Hamlib model 2 (NET rigctl) at 127.0.0.1:{}. Whistlestop owns the radio's port; Mercury keys it through Whistlestop. RTS and DTR are held low.", c.cat.share_port)).small().color(DIM));
                                } else {
                                    ui.label(RichText::new("Not shared: Mercury will need another way to key the radio (VOX, or a separate PTT port).").small().color(AMBER));
                                }
                            }
                        }
                    });
                    ui.add_space(6.0);
                    card(ui, "Audio", |ui| {
                        egui::Grid::new("ra").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                            ui.label("Listen on").help(&help::ST_MON_DEV);
                            egui::ComboBox::from_id_salt("mon").width(280.0).selected_text(if c.monitor_device.is_empty() { "System default" } else { &c.monitor_device }).show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.monitor_device, String::new(), "System default");
                                for d in &self.devices.1 {
                                    ui.selectable_value(&mut c.monitor_device, d.clone(), d);
                                }
                            })
                            .response
                            .help(&help::ST_MON_DEV);
                            ui.end_row();
                            ui.label("Listen volume").help(&help::ST_MON_VOL);
                            if ui.add(egui::Slider::new(&mut c.monitor_volume, 0.0..=1.5).show_value(false)).help(&help::ST_MON_VOL).changed() {
                                self.h.send(Cmd::MonitorVolume(c.monitor_volume));
                            }
                            ui.end_row();
                            ui.label("Audio in").help(&help::ST_AUDIO);
                            egui::ComboBox::from_id_salt("ain").width(280.0).selected_text(if c.audio_in.is_empty() { "System default" } else { &c.audio_in }).show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.audio_in, String::new(), "System default");
                                for d in &self.devices.0 {
                                    ui.selectable_value(&mut c.audio_in, d.clone(), d);
                                }
                            });
                            ui.end_row();
                            ui.label("Audio out").help(&help::ST_AUDIO);
                            egui::ComboBox::from_id_salt("aout").width(280.0).selected_text(if c.audio_out.is_empty() { "System default" } else { &c.audio_out }).show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.audio_out, String::new(), "System default");
                                for d in &self.devices.1 {
                                    ui.selectable_value(&mut c.audio_out, d.clone(), d);
                                }
                            });
                            ui.end_row();
                        });
                    });
                    ui.add_space(6.0);
                    card(ui, "Wide receiver (RTL-SDR)", |ui| {
                        ui.checkbox(&mut c.sdr.enabled, "Use an RTL-SDR dongle for the wide waterfall").help(&help::ST_SDR);
                        ui.horizontal(|ui| {
                            ui.label("Connected to").help(&help::ST_SDR);
                            ui.selectable_value(&mut c.sdr.feed, whistlestop::sdr::Feed::RxOut, "RX OUT (antenna)");
                            ui.selectable_value(&mut c.sdr.feed, whistlestop::sdr::Feed::IfOut, "IF OUT");
                        });
                        if c.sdr.feed == whistlestop::sdr::Feed::IfOut {
                            ui.horizontal(|ui| {
                                ui.label("IF Hz");
                                ui.add(egui::DragValue::new(&mut c.sdr.if_hz)).help(&help::ST_SDR);
                                ui.checkbox(&mut c.sdr.if_inverted, "inverted").help(&help::ST_SDR);
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.label("Crystal ppm");
                            ui.add(egui::DragValue::new(&mut c.sdr.ppm).range(-200..=200)).help(&help::ST_PPM);
                        });
                    });
                    ui.add_space(6.0);
                    card(ui, "Search & CQ", |ui| {
                        egui::Grid::new("sc").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                            ui.label("Dwell per channel");
                            ui.add(egui::Slider::new(&mut c.dwell_s, 0.3..=4.0).suffix(" s")).help(&help::ST_DWELL);
                            ui.end_row();
                            ui.label("Channel step");
                            ui.add(egui::Slider::new(&mut c.step_hz, 400.0..=2000.0).suffix(" Hz")).help(&help::ST_STEP);
                            ui.end_row();
                            ui.label("CQ cycles");
                            ui.add(egui::Slider::new(&mut c.cq_cycles, 1..=20)).help(&help::ST_CYCLES);
                            ui.end_row();
                            ui.label("Listen after CQ");
                            ui.add(egui::Slider::new(&mut c.cq_listen_s, 5.0..=60.0).suffix(" s")).help(&help::ST_LISTEN);
                            ui.end_row();
                        });
                        ui.checkbox(&mut c.whistle_watch, "Whistle watch while parked on a frequency").help(&help::ST_WATCH);
                        ui.checkbox(&mut c.auto_ramp, "Roundtable speed ramp-up (host)").help(&help::ST_RAMP);
                    });
                    ui.add_space(6.0);
                    card(ui, "Encryption (1:1 chat and files)", |ui| {
                        let mut want = c.e2e;
                        ui.horizontal(|ui| {
                            for p in [Policy::Off, Policy::Prefer, Policy::Require] {
                                ui.selectable_value(&mut want, p, p.label()).help(&help::ST_E2E);
                            }
                        });
                        if want != c.e2e {
                            if c.e2e == Policy::Off {
                                self.e2e_confirm = Some(want); // ask first
                            } else {
                                c.e2e = want;
                            }
                        }
                        ui.label(
                            RichText::new(match c.e2e {
                                Policy::Off => "Links are plain text (the default).",
                                Policy::Prefer => "Encrypt when the other station does too; otherwise plain text.",
                                Policy::Require => "Encrypt or hang up: never plain text.",
                            })
                            .small()
                            .color(DIM),
                        );
                        ui.label(RichText::new(format!("⚠ {}", e2e::LEGAL_WARNING)).small().color(RED));
                        ui.label(RichText::new("Roundtables are never encrypted. Keys are made per link and never saved.").small().color(DIM));
                    });
                    ui.add_space(6.0);
                    card(ui, "QSY requests (off by default)", |ui| {
                        ui.label(RichText::new("A linked station may ask to move the link to another frequency (for a long transfer, or off a busy channel). Both go back here when the link ends.").small().color(DIM));
                        ui.checkbox(&mut c.accept_qsy, "Accept QSY requests that meet these rules").help(&help::ST_QSY_ACCEPT);
                        ui.add_enabled_ui(c.accept_qsy, |ui| {
                            ui.checkbox(&mut c.qsy_same_band, "Only within the band I'm on").help(&help::ST_QSY_SAME);
                            if !c.qsy_same_band {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label("Bands:").help(&help::ST_QSY_BANDS);
                                    let mut bands: Vec<String> = c.segments.iter().map(|s| s.band.clone()).collect();
                                    bands.dedup();
                                    for b in bands {
                                        let mut on = c.qsy_bands.contains(&b);
                                        if ui.checkbox(&mut on, &b).help(&help::ST_QSY_BANDS).changed() {
                                            if on {
                                                c.qsy_bands.push(b.clone());
                                            } else {
                                                c.qsy_bands.retain(|x| x != &b);
                                            }
                                        }
                                    }
                                });
                            }
                            ui.horizontal(|ui| {
                                ui.label("At most").help(&help::ST_QSY_MAX);
                                ui.add(egui::DragValue::new(&mut c.qsy_max_khz).range(0.0..=30_000.0).suffix(" kHz")).help(&help::ST_QSY_MAX);
                                ui.label(RichText::new(if c.qsy_max_khz == 0.0 { "(anywhere allowed)" } else { "from where I was" }).color(DIM));
                            });
                            ui.checkbox(&mut c.qsy_segments_only, "Only inside the digital segments (band plan / legal)").help(&help::ST_QSY_SEG);
                            ui.checkbox(&mut c.qsy_swr_home, "If SWR stays high there, end the link so we both go home").help(&help::ST_QSY_SWR);
                            ui.label(RichText::new(if c.auto_tune { "On arrival, high SWR starts the antenna tuner once (auto-tune is on)." } else { "Turn on auto-tune (Radio Controls) to have the tuner run on arrival if SWR is high." }).small().color(DIM));
                        });
                    });
                    ui.add_space(6.0);
                    card(ui, "File library (off by default)", |ui| {
                        ui.label(RichText::new("A folder linked stations may browse and download from, even while you're away. Nothing outside it can be reached.").small().color(DIM));
                        ui.checkbox(&mut c.share_library, "Share a file library").help(&help::ST_LIB_SHARE);
                        ui.add_enabled_ui(c.share_library, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Folder").help(&help::ST_LIB_DIR);
                                ui.add(egui::TextEdit::singleline(&mut c.library_dir).hint_text("choose a folder").desired_width(300.0)).help(&help::ST_LIB_DIR);
                                if ui.button("Choose…").help(&help::ST_LIB_DIR).clicked()
                                    && let Some(p) = rfd::FileDialog::new().pick_folder()
                                {
                                    c.library_dir = p.to_string_lossy().to_string();
                                }
                            });
                            ui.checkbox(&mut c.library_only_away, "Only while I'm away").help(&help::ST_LIB_AWAY);
                            ui.horizontal(|ui| {
                                ui.label("Largest file").help(&help::ST_LIB_MAX);
                                ui.add(egui::DragValue::new(&mut c.library_max_mb).range(0.1..=200.0).speed(0.1).suffix(" MB")).help(&help::ST_LIB_MAX);
                            });
                        });
                        ui.label(RichText::new(format!("Files you download go to the Download folder ({}).", c.download_dir.display())).small().color(DIM));
                    });
                    ui.add_space(6.0);
                    card(ui, "Schedule (off by default)", |ui| {
                        let now = whistlestop::depot::now_s();
                        ui.label(RichText::new(format!("Listen (and send Station ID) on different frequencies at different times. Times are UTC; it's {} UTC now.", &depot::fmt_utc(now)[11..16])).small().color(DIM));
                        ui.checkbox(&mut c.schedule_on, "Follow the schedule").help(&help::ST_SCHED_ON);
                        ui.checkbox(&mut c.schedule_only_away, "Only while I'm away").help(&help::ST_SCHED_AWAY);
                        let mut remove = None;
                        for (i, sl) in c.schedule.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut sl.from).desired_width(48.0).char_limit(5)).help(&help::ST_SCHED_SLOT);
                                ui.label("–");
                                ui.add(egui::TextEdit::singleline(&mut sl.to).desired_width(48.0).char_limit(5)).help(&help::ST_SCHED_SLOT);
                                egui::ComboBox::from_id_salt(("days", i)).width(90.0).selected_text(sl.days.label()).show_ui(ui, |ui| {
                                    for d in whistlestop::schedule::Days::ALL {
                                        ui.selectable_value(&mut sl.days, d, d.label());
                                    }
                                })
                                .response
                                .help(&help::ST_SCHED_SLOT);
                                ui.add(egui::DragValue::new(&mut sl.khz).range(1800.0..=29_700.0).speed(0.5).max_decimals(3).suffix(" kHz")).help(&help::ST_SCHED_SLOT);
                                ui.checkbox(&mut sl.beacon, "Station ID").help(&help::ST_SCHED_SLOT);
                                if !sl.valid() {
                                    ui.label(RichText::new("⚠ HH:MM").color(RED));
                                }
                                if ui.small_button("✕").help(&help::ST_SCHED_SLOT).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            c.schedule.remove(i);
                        }
                        if ui.button("+  Add a time slot").help(&help::ST_SCHED_ADD).clicked() {
                            c.schedule.push(whistlestop::schedule::Slot::default());
                        }
                    });
                    ui.add_space(6.0);
                    card(ui, "Automation (all off by default)", |ui| {
                        ui.label(RichText::new("Automatic transmissions happen only when you're not linked, calling or searching, and (if the last switch is on) only inside the automatic-control digital segments. You remain responsible for your station.").small().color(DIM));
                        ui.add_space(4.0);
                        ui.label(RichText::new("Away").strong());
                        ui.horizontal(|ui| {
                            ui.label("Away message");
                            ui.add(egui::TextEdit::singleline(&mut c.away_message).desired_width(330.0)).help(&help::ST_AWAY_MSG);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Hang up quiet links after");
                            ui.add(egui::Slider::new(&mut c.away_idle_s, 60.0..=900.0).suffix(" s").integer()).help(&help::ST_AWAY_IDLE);
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new("Station ID").strong());
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut c.beacon, "Station ID beacon every").help(&help::ST_BEACON);
                            ui.add(egui::Slider::new(&mut c.beacon_minutes, 10.0..=120.0).suffix(" min").integer()).help(&help::ST_BEACON);
                            if ui.button("Send one now").help(&help::ST_BEACON_NOW).clicked() {
                                self.h.send(Cmd::BeaconNow);
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new("Mail").strong());
                        ui.checkbox(&mut c.notify_mail, "Tell stations I hear that I have mail for them").help(&help::ST_NOTIFY);
                        ui.checkbox(&mut c.auto_deliver, "While away: connect and deliver mail to stations I hear").help(&help::ST_AUTO_DELIVER);
                        ui.checkbox(&mut c.auto_collect, "While away: connect and collect mail that's waiting for me").help(&help::ST_AUTO_COLLECT);
                        ui.checkbox(&mut c.relay, "Carry mail for other stations (store and forward)").help(&help::ST_RELAY);
                        ui.add_space(4.0);
                        ui.checkbox(&mut c.auto_in_segments_only, "Automatic transmissions only inside the automatic-control segments").help(&help::ST_SEGMENTS);
                        if !c.auto_in_segments_only {
                            ui.label(RichText::new("⚠ Unattended automatic operation outside the automatic-control segments isn't allowed in the US (47 CFR 97.221). Check your rules.").small().color(RED));
                        }
                    });
                    ui.add_space(6.0);
                    card(ui, "Files", |ui| {
                        ui.horizontal(|ui| {
                            ui.label(c.download_dir.display().to_string());
                            if ui.button("Change…").help(&help::ST_DOWNLOAD).clicked()
                                && let Some(p) = rfd::FileDialog::new().pick_folder() {
                                    c.download_dir = p;
                                }
                        });
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Save & connect").strong()).fill(BRASS.linear_multiply(0.5))).help(&help::ST_SAVE).clicked() {
                        apply = true;
                    }
                    ui.label(RichText::new(format!("Saved to {}", whistlestop::config::path().display())).small().color(DIM));
                });
            });
        if let Some(want) = self.e2e_confirm {
            let mut decided = None;
            egui::Window::new(RichText::new("Turn on encryption?").strong().color(RED))
                .collapsible(false)
                .resizable(false)
                .default_width(440.0)
                .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.label(RichText::new(e2e::LEGAL_WARNING).color(TEXT));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("You are responsible for where you transmit. Whistlestop does not check which band you are on.")
                            .small()
                            .color(DIM),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let b = egui::Button::new(RichText::new(format!("I understand: set {}", want.label())).strong()).fill(RED.linear_multiply(0.4));
                        if ui.add(b).clicked() {
                            decided = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            decided = Some(false);
                        }
                    });
                });
            match decided {
                Some(true) => {
                    self.cfg.e2e = want;
                    self.e2e_confirm = None;
                }
                Some(false) => self.e2e_confirm = None,
                None => {}
            }
        }
        if apply {
            if let Err(e) = self.cfg.save() {
                self.toast(format!("Could not save settings: {e}"));
            }
            self.sdr_span = self.cfg.sdr.span_hz;
            self.send(Cmd::Start(self.cfg.clone()));
            open = false;
        }
        self.settings_open = open;
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let _ = &self.logo;
        // Files dropped on the window go to the current link.
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            self.send(Cmd::SendFiles { paths: dropped, zip: self.zip });
            self.tab = Tab::Files;
        }
        // Take a snapshot of the engine state for this frame.
        let mut snap = {
            let mut g = self.h.state.lock().unwrap();
            let rows: std::collections::VecDeque<_> = std::mem::take(&mut g.audio_rows);
            State {
                running: g.running,
                demo: g.demo,
                mycall: g.mycall.clone(),
                rig_ok: g.rig_ok,
                mercury_ok: g.mercury_ok,
                audio_ok: g.audio_ok,
                rig: g.rig.clone(),
                tx: g.tx,
                snr_db: g.snr_db,
                bandwidth: g.bandwidth,
                link: g.link.clone(),
                incoming_call: g.incoming_call,
                calling: g.calling.clone(),
                chat: g.chat.clone(),
                transfers: g.transfers.clone(),
                heard: g.heard.clone(),
                whistle: g.whistle.clone(),
                search: g.search.clone(),
                cq: g.cq.clone(),
                table: g.table.clone(),
                log: g.log.clone(),
                audio_rows: rows,
                listen_mode: g.listen_mode,
                est_bps: g.est_bps,
                wide: g.wide.clone(),
                modem_version: g.modem_version.clone(),
                modem_bps: g.modem_bps,
                rig_desc: g.rig_desc.clone(),
                rig_caps: g.rig_caps.clone(),
                rig_ext: g.rig_ext.clone(),
                monitor_on: g.monitor_on,
                depot: g.depot.clone(),
                away: g.away,
                mail_alerts: g.mail_alerts.clone(),
                qsy: g.qsy.clone(),
                qsy_note: g.qsy_note.clone(),
                library: g.library.clone(),
                schedule_now: g.schedule_now.clone(),
            }
        };

        egui::Panel::top("top")
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(14, 10)))
            .show(ui, |ui| {
                self.top_bar(ui, &snap);
                let r = ui.max_rect();
                let y = r.bottom() + 9.0;
                ui.painter().line_segment([Pos2::new(r.left() - 14.0, y), Pos2::new(r.right() + 14.0, y)], Stroke::new(1.0, BRASS.linear_multiply(0.35)));
            });
        let screen_h = ctx.content_rect().height();
        if help::on() {
            egui::Panel::top("help-banner").frame(Frame::NONE.fill(BRASS.linear_multiply(0.2)).inner_margin(Margin::symmetric(14, 5))).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("?  HELP MODE").strong().color(BRASS));
                    ui.label(RichText::new("Hover over anything for an explanation. The full manual: 📖 Guide (top right).").color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Turn off").clicked() {
                            help::set(false);
                        }
                        if ui.button("Open the Guide").clicked() {
                            self.guide_open = true;
                            self.guide.open("A tour of the screen");
                        }
                    });
                });
            });
        }
        egui::Panel::top("modem")
            .frame(Frame::NONE.fill(PANEL).inner_margin(Margin::symmetric(14, 6)).stroke(Stroke::new(1.0, BORDER)))
            .show(ui, |ui| self.modem_strip(ui, &snap));
        egui::Panel::bottom("wf")
            .resizable(false)
            .exact_size((screen_h * self.wf_share).clamp(140.0, screen_h * 0.6))
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(14, 8)))
            .show(ui, |ui| self.waterfalls(ui, &mut snap));
        egui::Panel::left("left")
            .resizable(true)
            .default_size(318.0)
            .size_range(280.0..=420.0)
            .frame(Frame::NONE.fill(PANEL).inner_margin(Margin::same(12)))
            .show(ui, |ui| self.left(ui, &snap));
        egui::CentralPanel::default().frame(Frame::NONE.fill(BG).inner_margin(Margin::same(14))).show(ui, |ui| self.centre(ui, &snap));

        if self.settings_open {
            self.settings(&ctx);
        }
        if self.rigwin.open {
            let (cmds, changed) = self.rigwin.show(&ctx, &snap, &mut self.cfg);
            for c in cmds {
                self.send(c);
            }
            if changed {
                let _ = self.cfg.save();
                self.send(Cmd::Update(self.cfg.clone()));
            }
        }
        if self.guide_open {
            let builder = egui::ViewportBuilder::default()
                .with_title("Whistlestop Guide")
                .with_app_id("whistlestop")
                .with_inner_size([1080.0, 820.0])
                .with_min_inner_size([640.0, 420.0]);
            ctx.show_viewport_immediate(guide_window_id(), builder, |ui, _| {
                egui::CentralPanel::default()
                    .frame(Frame::NONE.fill(BG).inner_margin(Margin::same(14)))
                    .show(ui, |ui| self.guide.show(ui));
                if ui.input(|i| i.viewport().close_requested()) {
                    self.guide_open = false;
                }
            });
        }
        self.tour_step(&ctx, &snap);
        if let Some((at, msg)) = &self.toast {
            if at.elapsed() < Duration::from_secs(4) {
                egui::Area::new(egui::Id::unique("toast")).anchor(Align2::CENTER_TOP, vec2(0.0, 70.0)).show(&ctx, |ui| {
                    Frame::NONE.fill(CARD_HI).stroke(Stroke::new(1.0, BRASS)).corner_radius(10.0).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                        ui.label(RichText::new(msg).color(TEXT));
                    });
                });
            } else {
                self.toast = None;
            }
        }
        // Errors from the engine show briefly as a toast too.
        if let Some((_, l)) = snap.log.back()
            && l.starts_with('⚠') && self.last_err.as_deref() != Some(l.as_str()) {
                self.last_err = Some(l.clone());
                let l = l.clone();
                self.toast(l);
            }
        ctx.request_repaint_after(Duration::from_millis(66));
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Tour only: hold a simulated pointer still over a control (the real
        // mouse is ignored meanwhile) so the help card can be captured.
        if let (Some(_), Some(p)) = (self.hover_at, self.call_cq_at) {
            raw.events.retain(|e| !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerGone));
            if self.hover_sent != Some(p) {
                raw.events.push(egui::Event::PointerMoved(p));
                self.hover_sent = Some(p);
            }
        } else {
            self.hover_sent = None;
        }
    }

    fn on_exit(&mut self) {
        let _ = self.cfg.save();
        self.send(Cmd::Stop);
    }
}

#[cfg(test)]
mod tests;
