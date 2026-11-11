//! The Radio Controls window: everything the radio offers through Hamlib,
//! kept out of the main screen until you open it. Only controls the radio's
//! capability report lists are shown.

use super::help::{self, Help};
use super::{AMBER, BORDER, BRASS, CARD, CARD_HI, DIM, GREEN, RED, TEAL, TEXT, card};
use eframe::egui::{self, Align2, Color32, Margin, RichText, Sense, Stroke, Ui, vec2};
use std::collections::HashMap;
use whistlestop::config::Config;
use whistlestop::engine::{Cmd, State};
use whistlestop::rig::RigCaps;

#[derive(Default)]
pub struct RigWindow {
    pub open: bool,
    /// Slider values being dragged (so polling doesn't fight the mouse).
    edit: HashMap<String, f32>,
    rit: i32,
    xit: i32,
    passband: i64,
}

/// Hamlib AGC values.
const AGC: [(&str, f32); 5] = [("Off", 0.0), ("Fast", 2.0), ("Medium", 5.0), ("Slow", 3.0), ("Auto", 6.0)];

fn meter(ui: &mut Ui, label: &str, value: Option<f32>, frac: Option<f32>, color: Color32, text: String) -> egui::Response {
    ui.vertical(|ui| {
        ui.set_width(150.0);
        ui.label(RichText::new(label).size(10.5).color(DIM).strong());
        let (r, _) = ui.allocate_exact_size(vec2(140.0, 10.0), Sense::hover());
        ui.painter().rect_filled(r, 4.0, Color32::from_rgb(10, 13, 18));
        if let Some(f) = frac {
            let mut b = r;
            b.set_width(r.width() * f.clamp(0.0, 1.0));
            ui.painter().rect_filled(b, 4.0, color);
        }
        ui.add_sized(
            [140.0, 18.0],
            egui::Label::new(RichText::new(if value.is_some() { text } else { "—".into() }).monospace().color(if value.is_some() { TEXT } else { DIM })),
        );
    })
    .response
}

impl RigWindow {
    fn slider(&mut self, ui: &mut Ui, caps: &RigCaps, s: &State, name: &str, label: &str, topic: &help::Topic, send: &mut Vec<Cmd>) {
        if !caps.can_set_level(name) {
            return;
        }
        let live = *s.rig_ext.levels.get(name).unwrap_or(&0.0);
        let mut v = *self.edit.get(name).unwrap_or(&live);
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new(label).color(TEXT)));
            let r = ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false)).help(topic);
            ui.add_sized([44.0, 20.0], egui::Label::new(RichText::new(format!("{:>3.0}%", v * 100.0)).monospace().color(DIM)));
            if r.dragged() || r.has_focus() {
                self.edit.insert(name.into(), v);
            } else {
                self.edit.remove(name);
            }
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                send.push(Cmd::RigLevel(name.into(), v));
            }
        });
    }

    fn toggle(ui: &mut Ui, caps: &RigCaps, s: &State, name: &str, label: &str, topic: &help::Topic, send: &mut Vec<Cmd>) {
        if !caps.can_func(name) {
            return;
        }
        let on = *s.rig_ext.funcs.get(name).unwrap_or(&false);
        let b = egui::Button::new(RichText::new(label).color(if on { Color32::from_rgb(20, 16, 10) } else { TEXT }))
            .fill(if on { TEAL } else { CARD_HI })
            .min_size(vec2(64.0, 26.0));
        if ui.add(b).help(topic).clicked() {
            send.push(Cmd::RigFunc(name.into(), !on));
        }
    }

    fn steps(ui: &mut Ui, caps: &RigCaps, s: &State, name: &str, label: &str, steps: &[i32], topic: &help::Topic, send: &mut Vec<Cmd>) {
        if !caps.can_set_level(name) || steps.is_empty() {
            return;
        }
        let cur = s.rig_ext.levels.get(name).map(|v| v.round() as i32).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new(label).color(TEXT)));
            for v in std::iter::once(0).chain(steps.iter().copied()) {
                let txt = if v == 0 { "Off".to_string() } else { format!("{v} dB") };
                if ui.selectable_label(cur == v, txt).help(topic).clicked() {
                    send.push(Cmd::RigLevel(name.into(), v as f32));
                }
            }
        });
    }

    /// Returns commands to send.
    pub fn show(&mut self, ctx: &egui::Context, s: &State, cfg: &mut Config) -> (Vec<Cmd>, bool) {
        let mut send = vec![];
        let mut cfg_changed = false;
        let mut open = self.open;
        let caps = s.rig_caps.clone().unwrap_or_default();
        egui::Window::new(RichText::new("🎛  Radio controls").strong())
            .open(&mut open)
            .resizable(true)
            .default_width(560.0)
            .default_height((ctx.content_rect().height() - 140.0).max(420.0))
            .default_pos(ctx.content_rect().center() - vec2(280.0, 300.0))
            .frame(egui::Frame::window(&ctx.global_style()).fill(super::PANEL))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 160.0).show(ui, |ui| {
                    // Who and what.
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if caps.model.is_empty() { "Radio" } else { &caps.model }).size(18.0).strong().color(BRASS));
                        ui.label(RichText::new(&s.rig_desc).color(DIM));
                    })
                    .response
                    .help(&help::RC_RADIO);
                    if !s.rig_ok {
                        ui.label(RichText::new("Not connected to a radio. See Settings → Radio connection.").color(RED));
                    }
                    ui.add_space(4.0);

                    // Transmit meters.
                    card(ui, "Transmit meters", |ui| {
                        if s.rig_ext.swr.is_none() && s.rig_ext.alc.is_none() && s.rig_ext.power_out.is_none() {
                            ui.label(RichText::new("Read while transmitting.").small().color(DIM));
                        }
                        ui.horizontal(|ui| {
                            let limit = cfg.swr_limit;
                            let swr = s.rig_ext.swr;
                            let col = match swr {
                                Some(v) if v >= limit => RED,
                                Some(v) if v >= 1.5 => AMBER,
                                _ => GREEN,
                            };
                            meter(ui, "SWR", swr, swr.map(|v| (v - 1.0) / 2.0), col, swr.map(|v| format!("{v:.2} : 1")).unwrap_or_default())
                                .help(&help::RC_SWR);
                            let alc = s.rig_ext.alc;
                            meter(
                                ui,
                                "ALC",
                                alc,
                                alc,
                                if alc.unwrap_or(0.0) > 0.6 { AMBER } else { TEAL },
                                alc.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_default(),
                            )
                            .help(&help::RC_ALC);
                            let po = s.rig_ext.power_out;
                            meter(ui, "POWER OUT", po, po, BRASS, po.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_default()).help(&help::RC_PO);
                        });
                        if let Some(v) = s.rig_ext.swr_last_tx {
                            let bad = v > cfg.swr_limit;
                            ui.label(RichText::new(format!("Last transmission: SWR {v:.2}{}", if bad { "  ⚠ above your limit" } else { "" })).color(if bad {
                                RED
                            } else {
                                DIM
                            }));
                        }
                    });
                    ui.add_space(6.0);

                    // Antenna tuner.
                    card(ui, "Antenna tuner", |ui| {
                        ui.horizontal(|ui| {
                            Self::toggle(ui, &caps, s, "TUNER", "ATU", &help::RC_ATU, &mut send);
                            if caps.has_tuner() {
                                if ui
                                    .add(egui::Button::new(RichText::new("⟳  Tune now").strong()).fill(BRASS.linear_multiply(0.45)))
                                    .help(&help::RC_TUNE_NOW)
                                    .clicked()
                                {
                                    send.push(Cmd::TuneAntenna);
                                }
                            } else {
                                ui.label(RichText::new("This radio's tuner isn't controllable through Hamlib.").color(DIM));
                            }
                            if ui.button("Tune carrier").help(&help::R_TUNE).clicked() {
                                send.push(Cmd::Tune(true));
                            }
                            if ui.button("Stop").help(&help::R_TUNE).clicked() {
                                send.push(Cmd::Tune(false));
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(caps.has_tuner(), egui::Checkbox::new(&mut cfg.auto_tune, "Auto-tune when SWR is above"))
                                .help(&help::RC_AUTOTUNE)
                                .changed()
                            {
                                cfg_changed = true;
                            }
                            if ui.add(egui::Slider::new(&mut cfg.swr_limit, 1.3..=3.0).fixed_decimals(1).suffix(" : 1")).help(&help::RC_SWR_LIMIT).changed() {
                                cfg_changed = true;
                            }
                        });
                        if s.rig_ext.tune_pending {
                            ui.label(RichText::new("High SWR: will tune as soon as the band is quiet (no link, CQ or Roundtable traffic).").color(AMBER));
                        } else if !s.rig_ext.tuner_note.is_empty() {
                            ui.label(RichText::new(&s.rig_ext.tuner_note).color(DIM));
                        }
                    });
                    ui.add_space(6.0);

                    // Receive.
                    card(ui, "Receive", |ui| {
                        self.slider(ui, &caps, s, "AF", "Volume (AF)", &help::RC_AF, &mut send);
                        self.slider(ui, &caps, s, "RF", "RF gain", &help::RC_RFG, &mut send);
                        self.slider(ui, &caps, s, "SQL", "Squelch", &help::RC_SQL, &mut send);
                        if caps.can_set_level("AGC") {
                            let cur = *s.rig_ext.levels.get("AGC").unwrap_or(&-1.0);
                            ui.horizontal(|ui| {
                                ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new("AGC").color(TEXT)));
                                for (n, v) in AGC {
                                    if ui.selectable_label((cur - v).abs() < 0.5, n).help(&help::RC_AGC).clicked() {
                                        send.push(Cmd::RigLevel("AGC".into(), v));
                                    }
                                }
                            });
                        }
                        Self::steps(ui, &caps, s, "PREAMP", "Preamp", &caps.preamp_db, &help::RC_PREAMP, &mut send);
                        Self::steps(ui, &caps, s, "ATT", "Attenuator", &caps.att_db, &help::RC_ATT, &mut send);
                        ui.horizontal(|ui| {
                            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new("Noise").color(TEXT)));
                            Self::toggle(ui, &caps, s, "NB", "NB", &help::RC_NB, &mut send);
                            Self::toggle(ui, &caps, s, "NR", "NR", &help::RC_NR, &mut send);
                            Self::toggle(ui, &caps, s, "ANF", "Notch", &help::RC_ANF, &mut send);
                        });
                        self.slider(ui, &caps, s, "NR", "NR level", &help::RC_NR, &mut send);
                        ui.horizontal(|ui| {
                            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new("Filter width").color(TEXT)));
                            if self.passband == 0 {
                                self.passband = if s.rig.passband_hz > 0 { s.rig.passband_hz } else { 3000 };
                            }
                            let r = ui.add(egui::Slider::new(&mut self.passband, 300..=4000).step_by(50.0).suffix(" Hz")).help(&help::RC_FILTER);
                            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                send.push(Cmd::RigPassband(self.passband));
                            }
                        });
                        ui.label(RichText::new("For data, keep NB, NR and Notch off: they smear OFDM and the whistle.").small().color(DIM));
                    });
                    ui.add_space(6.0);

                    // Transmit.
                    card(ui, "Transmit", |ui| {
                        self.slider(ui, &caps, s, "RFPOWER", "Power", &help::R_POWER, &mut send);
                        self.slider(ui, &caps, s, "MICGAIN", "Mic gain", &help::RC_MIC, &mut send);
                        self.slider(ui, &caps, s, "COMP", "Compression", &help::RC_COMP, &mut send);
                        ui.horizontal(|ui| {
                            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new("Switches").color(TEXT)));
                            Self::toggle(ui, &caps, s, "COMP", "Comp", &help::RC_COMP, &mut send);
                            Self::toggle(ui, &caps, s, "VOX", "VOX", &help::RC_VOX, &mut send);
                            Self::toggle(ui, &caps, s, "MON", "Monitor", &help::RC_MON, &mut send);
                        });
                        ui.label(RichText::new("For data, keep compression and VOX off.").small().color(DIM));
                    });
                    ui.add_space(6.0);

                    // VFO and offsets.
                    card(ui, "VFO, split and offsets", |ui| {
                        ui.horizontal(|ui| {
                            ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new("VFO").color(TEXT)));
                            for v in ["VFOA", "VFOB"] {
                                if ui.selectable_label(s.rig_ext.vfo == v, if v == "VFOA" { "A" } else { "B" }).help(&help::RC_VFO).clicked() {
                                    send.push(Cmd::RigVfo(v.into()));
                                }
                            }
                            if caps.can_op("CPY") && ui.button("A = B").help(&help::RC_CPY).clicked() {
                                send.push(Cmd::RigOp("CPY".into()));
                            }
                            if caps.can_op("XCHG") && ui.button("A ⇄ B").help(&help::RC_XCHG).clicked() {
                                send.push(Cmd::RigOp("XCHG".into()));
                            }
                            let sp = s.rig_ext.split;
                            if ui.selectable_label(sp, "Split").help(&help::RC_SPLIT).clicked() {
                                send.push(Cmd::RigSplit(!sp));
                            }
                        });
                        for (xit, label, topic, cur) in [(false, "RIT", &help::RC_RIT, s.rig_ext.rit), (true, "XIT", &help::RC_XIT, s.rig_ext.xit)] {
                            ui.horizontal(|ui| {
                                ui.add_sized([110.0, 20.0], egui::Label::new(RichText::new(label).color(TEXT)));
                                let v = if xit { &mut self.xit } else { &mut self.rit };
                                ui.add(egui::DragValue::new(v).range(-9999..=9999).suffix(" Hz")).help(topic);
                                if ui.button("Set").help(topic).clicked() {
                                    send.push(Cmd::RigOffset { xit, hz: *v });
                                }
                                if ui.button("Clear").help(topic).clicked() {
                                    *v = 0;
                                    send.push(Cmd::RigOffset { xit, hz: 0 });
                                }
                                ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new(format!("{cur:+} Hz")).monospace().color(DIM)));
                            });
                        }
                        ui.label(RichText::new("Keep RIT and XIT at 0 for data: an offset moves you off the other station.").small().color(DIM));
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("Only the controls your radio reports through Hamlib are shown.").small().color(DIM));
                });
            });
        let _ = (BORDER, CARD, Align2::CENTER_CENTER, Margin::same(0), Stroke::NONE);
        let closed = self.open && !open;
        self.open = open;
        if closed {
            send.push(Cmd::RigWatch(false));
        }
        (send, cfg_changed)
    }
}
