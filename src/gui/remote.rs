//! The Remote window: operate the Whistlestop on the radio computer from
//! this one, or let other computers on the LAN operate this one.

use super::help::{self, Help};
use super::{AMBER, DIM, GREEN, RED, TEAL, TEXT, card};
use eframe::egui::{self, RichText, vec2};
use whistlestop::config::Config;

#[derive(Default)]
pub struct RemoteWin {
    pub open: bool,
    pub addr: String,
    pub password: String,
    pub error: Option<String>,
}

/// What the window asks the app to do.
pub enum RemoteAction {
    Connect {
        addr: String,
        password: String,
    },
    Disconnect,
    /// Serving settings changed (start, stop or restart the server).
    Serve,
}

pub struct View<'a> {
    pub connected: Option<&'a str>,
    pub serving: Option<(u16, Vec<String>)>,
    pub cfg: &'a mut Config,
}

impl RemoteWin {
    pub fn show(&mut self, ctx: &egui::Context, v: View) -> Option<RemoteAction> {
        let mut act = None;
        let mut open = self.open;
        egui::Window::new(RichText::new("🛰  Remote operation").strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Run Whistlestop on the computer wired to the radio, and operate it from another Whistlestop anywhere on your home network.").small().color(DIM));
                ui.add_space(6.0);
                card(ui, "Operate a radio computer from here", |ui| {
                    if let Some(desc) = v.connected {
                        ui.label(RichText::new(format!("● Operating {desc}")).strong().color(GREEN));
                        ui.label(RichText::new("Everything you do here happens on the radio computer. Listen plays its radio's audio on this computer's speakers; files you receive are copied to this computer's Download folder.").small().color(DIM));
                        if ui.button("Disconnect (back to this computer)").help(&help::RM_DISCONNECT).clicked() {
                            act = Some(RemoteAction::Disconnect);
                        }
                    } else {
                        egui::Grid::new("rm").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                            ui.label("Radio computer").help(&help::RM_ADDR);
                            ui.add(egui::TextEdit::singleline(&mut self.addr).hint_text("e.g. 192.168.1.20 or shack.local").desired_width(260.0)).help(&help::RM_ADDR);
                            ui.end_row();
                            ui.label("Password").help(&help::RM_PASSWORD);
                            ui.add(egui::TextEdit::singleline(&mut self.password).password(true).hint_text("its remote password").desired_width(260.0)).help(&help::RM_PASSWORD);
                            ui.end_row();
                        });
                        let ok = !self.addr.trim().is_empty() && !self.password.is_empty();
                        if ui.add_enabled(ok, egui::Button::new(RichText::new("Connect to the radio computer").strong()).fill(TEAL.linear_multiply(0.4))).help(&help::RM_CONNECT).clicked() {
                            act = Some(RemoteAction::Connect { addr: self.addr.trim().to_string(), password: self.password.clone() });
                        }
                        if let Some(e) = &self.error {
                            ui.label(RichText::new(e).color(RED));
                        }
                    }
                });
                ui.add_space(6.0);
                card(ui, "Let other computers operate this one", |ui| {
                    let c = &mut *v.cfg;
                    let mut changed = ui.checkbox(&mut c.remote_serve, "Share this station on my LAN (off by default)").help(&help::RM_SERVE).changed();
                    egui::Grid::new("rs").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                        ui.label("Password").help(&help::RM_SERVE_PW);
                        changed |= ui.add(egui::TextEdit::singleline(&mut c.remote_password).password(true).hint_text("at least 4 characters").desired_width(200.0)).help(&help::RM_SERVE_PW).lost_focus();
                        ui.end_row();
                        ui.label("Port").help(&help::RM_SERVE);
                        changed |= ui.add(egui::DragValue::new(&mut c.remote_port).range(1024..=65535)).help(&help::RM_SERVE).changed();
                        ui.end_row();
                    });
                    if c.remote_serve && c.remote_password.trim().len() < 4 {
                        ui.label(RichText::new("Set a password of at least 4 characters to share.").color(AMBER));
                    }
                    match &v.serving {
                        Some((port, clients)) => {
                            let here = whistlestop::remote::lan_address().map(|a| a.to_string()).unwrap_or("this computer's address".into());
                            ui.label(RichText::new(format!("● Sharing: connect from another computer to {here}:{port}")).color(GREEN));
                            if clients.is_empty() {
                                ui.label(RichText::new("No one connected.").color(DIM));
                            } else {
                                ui.label(RichText::new(format!("Operated from: {}", clients.join(", "))).color(TEXT));
                            }
                        }
                        None if c.remote_serve => {
                            ui.label(RichText::new("Not sharing yet.").color(DIM));
                        }
                        None => {}
                    }
                    ui.label(RichText::new("Only computers on your home network (private addresses) can connect, and only with the password. The connection isn't encrypted: don't open this port to the internet.").small().color(DIM));
                    if changed {
                        act = Some(RemoteAction::Serve);
                    }
                });
            });
        self.open = open;
        act
    }
}
