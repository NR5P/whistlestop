//! The 🏤 Depot tab: Arrivals, Departures, In Transit and Delivered.

use super::help::{self, Help};
use super::{AMBER, BORDER, BRASS, CARD, CARD_HI, DIM, GREEN, TEAL, TEXT};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke, Ui};
use whistlestop::depot::Mail;
use whistlestop::engine::{Cmd, State};

#[derive(Default, PartialEq, Clone, Copy)]
pub enum Box_ {
    #[default]
    Arrivals,
    Departures,
    Transit,
    Delivered,
}

#[derive(Default)]
pub struct DepotTab {
    pub tab: Box_,
    to: String,
    via: String,
    text: String,
}

/// "2026-11-10 21:04Z" from unix seconds.
pub fn fmt_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}Z", rem / 3600, (rem % 3600) / 60)
}

impl DepotTab {
    /// A box tab. It's as wide as `widest` (its label with the largest count
    /// expected), so the row doesn't shift as mail comes and goes.
    fn tab_button(&mut self, ui: &mut Ui, b: Box_, label: String, widest: &str, topic: &help::Topic) {
        let on = self.tab == b;
        let font = egui::TextStyle::Button.resolve(ui.style());
        let w = ui.painter().layout_no_wrap(widest.to_string(), font, TEXT).size().x + 2.0 * ui.spacing().button_padding.x;
        let btn = egui::Button::new(RichText::new(label).color(if on { Color32::from_rgb(20, 16, 10) } else { TEXT }))
            .fill(if on { TEAL } else { CARD_HI })
            .min_size(egui::vec2(w, 0.0));
        if ui.add(btn).help(topic).clicked() {
            self.tab = b;
        }
    }

    fn mail_card(ui: &mut Ui, m: &Mail, header: RichText, unread: bool, buttons: impl FnOnce(&mut Ui)) {
        Frame::NONE
            .fill(if unread { BRASS.linear_multiply(0.12) } else { CARD })
            .stroke(Stroke::new(1.0, if unread { BRASS.linear_multiply(0.6) } else { BORDER }))
            .corner_radius(10.0)
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(header);
                    ui.label(RichText::new(fmt_utc(m.created)).monospace().small().color(DIM));
                    if unread {
                        ui.label(RichText::new("NEW").small().strong().color(Color32::from_rgb(20, 16, 10)).background_color(BRASS));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
                });
                ui.add(egui::Label::new(RichText::new(&m.text).size(15.0)).wrap());
            });
        ui.add_space(6.0);
    }

    pub fn show(&mut self, ui: &mut Ui, s: &State, send: &mut Vec<Cmd>, relay_on: bool) {
        let d = &s.depot;
        // Compose.
        Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10.0).inner_margin(Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("WRITE MAIL").size(11.0).color(DIM).strong());
            ui.horizontal(|ui| {
                ui.label("To");
                ui.add(egui::TextEdit::singleline(&mut self.to).hint_text("Callsign").desired_width(110.0)).help(&help::D_TO);
                ui.label("Via");
                ui.add(egui::TextEdit::singleline(&mut self.via).hint_text("optional relay").desired_width(110.0)).help(&help::D_VIA);
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .hint_text("Your message. It waits in Departures and goes across the next time you're linked with them (or the relay).")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
            .help(&help::D_TEXT);
            ui.horizontal(|ui| {
                let ok = !self.to.trim().is_empty() && !self.text.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new(RichText::new("📮  Post").strong()).fill(BRASS.linear_multiply(0.5))).help(&help::D_POST).clicked() {
                    send.push(Cmd::DepotPost { to: self.to.trim().to_string(), via: Some(self.via.trim().to_string()), text: self.text.trim().to_string() });
                    self.text.clear();
                    self.tab = Box_::Departures;
                }
                ui.add_sized(
                    egui::vec2(90.0, ui.spacing().interact_size.y),
                    egui::Label::new(RichText::new(format!("{:>5} bytes", self.text.len())).small().monospace().color(DIM)),
                );
            });
        });
        ui.add_space(8.0);
        // Boxes.
        ui.horizontal(|ui| {
            let unread = d.unread();
            self.tab_button(
                ui,
                Box_::Arrivals,
                if unread > 0 { format!("📬 Arrivals ({unread} new)") } else { format!("📬 Arrivals ({})", d.arrivals.len()) },
                "📬 Arrivals (88 new)",
                &help::D_ARRIVALS,
            );
            self.tab_button(ui, Box_::Departures, format!("📮 Departures ({})", d.departures.len()), "📮 Departures (888)", &help::D_DEPARTURES);
            self.tab_button(ui, Box_::Transit, format!("🚚 In Transit ({})", d.transit.len()), "🚚 In Transit (888)", &help::D_TRANSIT);
            self.tab_button(ui, Box_::Delivered, format!("✓ Delivered ({})", d.delivered.len()), "✓ Delivered (888)", &help::D_DELIVERED);
        });
        ui.add_space(6.0);
        let linked = s.link.as_ref().map(|l| l.peer.clone());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
            Box_::Arrivals => {
                if d.arrivals.is_empty() {
                    ui.label(RichText::new("No mail yet. Stations that connect while you're away, or that carry mail for you, leave it here.").color(DIM));
                }
                for m in d.arrivals.iter().rev() {
                    Self::mail_card(ui, m, RichText::new(format!("From {}", m.from)).strong().color(TEAL), !m.read, |ui| {
                        if ui.small_button("Delete").help(&help::D_DELETE).clicked() {
                            send.push(Cmd::DepotDelete(m.id));
                        }
                        if ui.small_button("Reply").help(&help::D_REPLY).clicked() {
                            self.to = m.from.clone();
                            self.via.clear();
                        }
                        if !m.read && ui.small_button("Mark read").help(&help::D_READ).clicked() {
                            send.push(Cmd::DepotRead(m.id));
                        }
                    });
                }
            }
            Box_::Departures => {
                if d.departures.is_empty() {
                    ui.label(RichText::new("Nothing waiting. Mail you write waits here until its station (or relay) is reached.").color(DIM));
                }
                for m in d.departures.iter().rev() {
                    let target = m.via.clone().unwrap_or(m.to.clone());
                    let head = match &m.via {
                        Some(v) => format!("To {} via {v}", m.to),
                        None => format!("To {}", m.to),
                    };
                    Self::mail_card(ui, m, RichText::new(head).strong().color(BRASS), false, |ui| {
                        if ui.small_button("Delete").help(&help::D_DELETE).clicked() {
                            send.push(Cmd::DepotDelete(m.id));
                        }
                        if linked.is_none() && ui.small_button(format!("Connect to {target}")).help(&help::D_CONNECT).clicked() {
                            send.push(Cmd::DepotConnect(target.clone()));
                        }
                        ui.label(RichText::new("waiting").small().color(AMBER));
                    });
                }
            }
            Box_::Transit => {
                if !relay_on {
                    ui.label(RichText::new("Carrying mail for other stations is off (Settings → Automation).").color(AMBER));
                }
                if d.transit.is_empty() {
                    ui.label(
                        RichText::new(
                            "You're not carrying anyone's mail. With relaying on, stations can hand you mail for others; it goes on when you reach them.",
                        )
                        .color(DIM),
                    );
                }
                for m in d.transit.iter().rev() {
                    let head = format!("{} → {}{}", m.from, m.to, m.handed.as_ref().map(|h| format!("  (handed over by {h})")).unwrap_or_default());
                    Self::mail_card(ui, m, RichText::new(head).strong().color(AMBER), false, |ui| {
                        if ui.small_button("Delete").help(&help::D_DELETE).clicked() {
                            send.push(Cmd::DepotDelete(m.id));
                        }
                        if linked.is_none() && ui.small_button(format!("Connect to {}", m.to)).help(&help::D_CONNECT).clicked() {
                            send.push(Cmd::DepotConnect(m.to.clone()));
                        }
                        ui.label(RichText::new(format!("hop {}", m.hops)).small().color(DIM));
                    });
                }
            }
            Box_::Delivered => {
                if d.delivered.is_empty() {
                    ui.label(RichText::new("Nothing delivered yet.").color(DIM));
                }
                for m in d.delivered.iter().rev() {
                    let by = m.handed.clone().unwrap_or_default();
                    let head = if by.is_empty() || by == m.to { format!("To {}", m.to) } else { format!("To {} (handed to {by})", m.to) };
                    Self::mail_card(ui, m, RichText::new(head).strong().color(GREEN), false, |ui| {
                        if ui.small_button("Delete").help(&help::D_DELETE).clicked() {
                            send.push(Cmd::DepotDelete(m.id));
                        }
                        if let Some(t) = m.delivered_at {
                            ui.label(RichText::new(format!("✓ {}", fmt_utc(t))).small().color(GREEN));
                        }
                    });
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_dates() {
        assert_eq!(super::fmt_utc(0), "1970-01-01 00:00Z");
        assert_eq!(super::fmt_utc(1_794_268_800), "2026-11-10 00:00Z");
        assert_eq!(super::fmt_utc(951_782_400 + 3_660), "2000-02-29 01:01Z");
    }
}
