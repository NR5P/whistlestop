//! The 📖 Guide window: renders docs/USER-GUIDE.md (built into the app) with a
//! table of contents and search. One source for the app and the repo docs.

use super::{BORDER, BRASS, CARD, CARD_HI, DIM, TEAL, TEXT};
use eframe::egui::{self, Align, Color32, FontId, Frame, Layout, Margin, RichText, Stroke, TextFormat, Ui, text::LayoutJob, vec2};

pub const GUIDE_MD: &str = include_str!("../../docs/USER-GUIDE.md");

struct Section {
    title: String,
    body: Vec<String>,
}

pub struct Guide {
    sections: Vec<Section>,
    selected: usize,
    query: String,
    to_top: bool,
}

impl Default for Guide {
    fn default() -> Self {
        Self::new(GUIDE_MD)
    }
}

impl Guide {
    pub fn new(md: &str) -> Self {
        let mut sections = vec![Section { title: "Introduction".into(), body: vec![] }];
        for line in md.lines() {
            if let Some(t) = line.strip_prefix("## ") {
                sections.push(Section { title: t.trim().into(), body: vec![] });
            } else if !line.starts_with("# ") {
                sections.last_mut().unwrap().body.push(line.to_string());
            }
        }
        sections.retain(|s| s.body.iter().any(|l| !l.trim().is_empty()));
        Self { sections, selected: 0, query: String::new(), to_top: false }
    }

    /// Jump to the first section whose title contains `name`.
    pub fn open(&mut self, name: &str) {
        if let Some(i) = self.sections.iter().position(|s| s.title.to_lowercase().contains(&name.to_lowercase())) {
            self.selected = i;
            self.query.clear();
            self.to_top = true;
        }
    }

    fn matches(&self, i: usize) -> bool {
        let q = self.query.trim().to_lowercase();
        q.is_empty() || {
            let s = &self.sections[i];
            s.title.to_lowercase().contains(&q) || s.body.iter().any(|l| l.to_lowercase().contains(&q))
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let h = ui.available_height();
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, h), Layout::left_to_right(Align::Min), |ui| {
            // Contents and search.
            ui.allocate_ui_with_layout(vec2(240.0, h), Layout::top_down(Align::Min), |ui| {
            Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10.0).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(218.0);
                ui.set_min_height(h - 22.0);
                ui.label(RichText::new("CONTENTS").size(11.0).color(DIM).strong());
                let r = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("🔍 Search the guide").desired_width(f32::INFINITY));
                if r.changed() {
                    if let Some(i) = (0..self.sections.len()).find(|&i| self.matches(i))
                        && !self.matches(self.selected) {
                            self.selected = i;
                        }
                    self.to_top = true;
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("guide-toc").auto_shrink([false, false]).show(ui, |ui| {
                    for i in 0..self.sections.len() {
                        if !self.matches(i) {
                            continue;
                        }
                        let on = i == self.selected;
                        let b = egui::Button::new(RichText::new(&self.sections[i].title).color(if on { Color32::from_rgb(20, 16, 10) } else { TEXT }))
                            .fill(if on { BRASS } else { Color32::TRANSPARENT })
                            .min_size(egui::vec2(ui.available_width(), 24.0));
                        if ui.add(b).clicked() {
                            self.selected = i;
                            self.to_top = true;
                        }
                    }
                    if !(0..self.sections.len()).any(|i| self.matches(i)) {
                        ui.label(RichText::new("Nothing found.").color(DIM));
                    }
                });
            });
            });
            // The section.
            let bw = ui.available_width();
            ui.allocate_ui_with_layout(vec2(bw, h), Layout::top_down(Align::Min), |ui| {
            Frame::NONE.fill(CARD).stroke(Stroke::new(1.0, BORDER)).corner_radius(10.0).inner_margin(Margin::symmetric(22, 14)).show(ui, |ui| {
                ui.set_width(bw - 46.0);
                ui.set_min_height(h - 30.0);
                let mut sa = egui::ScrollArea::vertical().id_salt("guide-body").auto_shrink([false, false]);
                if self.to_top {
                    sa = sa.vertical_scroll_offset(0.0);
                    self.to_top = false;
                }
                let sel = self.selected.min(self.sections.len().saturating_sub(1));
                let q = self.query.trim().to_lowercase();
                let text_w = (bw - 80.0).clamp(300.0, 900.0);
                sa.show(ui, |ui| {
                    ui.set_max_width(text_w);
                    let s = &self.sections[sel];
                    ui.label(RichText::new(&s.title).size(24.0).strong().color(BRASS));
                    ui.add_space(6.0);
                    render(ui, &s.body, &q);
                    ui.add_space(20.0);
                    // Next / previous.
                    ui.horizontal(|ui| {
                        if sel > 0 && ui.button(format!("← {}", self.sections[sel - 1].title)).clicked() {
                            self.selected = sel - 1;
                            self.to_top = true;
                        }
                        if sel + 1 < self.sections.len() && ui.button(format!("{} →", self.sections[sel + 1].title)).clicked() {
                            self.selected = sel + 1;
                            self.to_top = true;
                        }
                    });
                });
            });
            });
        });
    }
}

/// `**bold**` and `` `code` `` inline, plus search highlighting.
fn inline(text: &str, size: f32, color: Color32, query: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    let normal = TextFormat { font_id: FontId::proportional(size), color, ..Default::default() };
    let bold = TextFormat { font_id: FontId::proportional(size), color: Color32::WHITE, ..Default::default() };
    let code = TextFormat { font_id: FontId::monospace(size - 1.0), color: TEAL, background: CARD_HI, ..Default::default() };
    let mut rest = text;
    let mut in_bold = false;
    while !rest.is_empty() {
        let nb = rest.find("**");
        let nc = rest.find('`');
        let (cut, kind) = match (nb, nc) {
            (Some(b), Some(c)) if c < b => (c, '`'),
            (Some(b), _) => (b, '*'),
            (None, Some(c)) => (c, '`'),
            (None, None) => (rest.len(), ' '),
        };
        append_hl(&mut job, &rest[..cut], if in_bold { &bold } else { &normal }, query);
        rest = &rest[cut..];
        match kind {
            '*' => {
                in_bold = !in_bold;
                rest = &rest[2..];
            }
            '`' => {
                let end = rest[1..].find('`').map(|e| e + 1).unwrap_or(rest.len());
                append_hl(&mut job, &rest[1..end], &code, query);
                rest = if end < rest.len() { &rest[end + 1..] } else { "" };
            }
            _ => {}
        }
    }
    job
}

fn append_hl(job: &mut LayoutJob, s: &str, f: &TextFormat, query: &str) {
    if query.is_empty() {
        job.append(s, 0.0, f.clone());
        return;
    }
    let lower = s.to_lowercase();
    let mut i = 0;
    while let Some(p) = lower[i..].find(query) {
        let (a, b) = (i + p, i + p + query.len());
        if !s.is_char_boundary(a) || !s.is_char_boundary(b) {
            break;
        }
        job.append(&s[i..a], 0.0, f.clone());
        job.append(&s[a..b], 0.0, TextFormat { background: BRASS.linear_multiply(0.45), ..f.clone() });
        i = b;
    }
    job.append(&s[i..], 0.0, f.clone());
}

fn para(ui: &mut Ui, text: &str, q: &str) {
    let mut job = inline(text, 15.0, TEXT, q);
    job.wrap.max_width = ui.available_width();
    ui.label(job);
}

fn render(ui: &mut Ui, lines: &[String], q: &str) {
    let mut i = 0;
    let mut paragraph: Vec<&str> = vec![];
    let flush = |ui: &mut Ui, p: &mut Vec<&str>| {
        if !p.is_empty() {
            para(ui, &p.join(" "), q);
            ui.add_space(6.0);
            p.clear();
        }
    };
    while i < lines.len() {
        let l = lines[i].as_str();
        let t = l.trim();
        if t.is_empty() {
            flush(ui, &mut paragraph);
        } else if let Some(h) = t.strip_prefix("### ") {
            flush(ui, &mut paragraph);
            ui.add_space(8.0);
            ui.label(RichText::new(h).size(18.0).strong().color(TEXT));
            ui.add_space(2.0);
        } else if t.starts_with("```") {
            flush(ui, &mut paragraph);
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i].clone());
                i += 1;
            }
            Frame::NONE.fill(Color32::from_rgb(10, 13, 18)).corner_radius(6.0).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.label(RichText::new(code.join("\n")).monospace().color(TEAL));
            });
            ui.add_space(6.0);
        } else if t.starts_with('|') {
            flush(ui, &mut paragraph);
            let mut rows: Vec<Vec<String>> = vec![];
            while i < lines.len() && lines[i].trim().starts_with('|') {
                let row = lines[i].trim().trim_matches('|');
                if !row.chars().all(|c| c == '-' || c == '|' || c == ':' || c == ' ') {
                    rows.push(row.split('|').map(|c| c.trim().to_string()).collect());
                }
                i += 1;
            }
            Frame::NONE.stroke(Stroke::new(1.0, BORDER)).corner_radius(6.0).inner_margin(Margin::same(8)).show(ui, |ui| {
                egui::Grid::new(ui.next_auto_id()).striped(true).spacing(egui::vec2(16.0, 6.0)).show(ui, |ui| {
                    for (r, row) in rows.iter().enumerate() {
                        for cell in row {
                            let mut job = inline(cell, 13.5, if r == 0 { DIM } else { TEXT }, q);
                            job.wrap.max_width = 260.0;
                            ui.label(job);
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(8.0);
            continue;
        } else if let Some(n) = t.strip_prefix("> ") {
            flush(ui, &mut paragraph);
            Frame::NONE.fill(BRASS.linear_multiply(0.12)).stroke(Stroke::new(1.0, BRASS.linear_multiply(0.5))).corner_radius(8.0).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                para(ui, n, q);
            });
            ui.add_space(6.0);
        } else if t.starts_with("- ") || t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains(". ") && t.split(". ").next().is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())) {
            flush(ui, &mut paragraph);
            let indent = l.len() - l.trim_start().len();
            let (marker, text) = if let Some(x) = t.strip_prefix("- ") {
                ("•".to_string(), x)
            } else {
                let (n, x) = t.split_once(". ").unwrap();
                (format!("{n}."), x)
            };
            ui.horizontal_top(|ui| {
                ui.add_space(8.0 + indent as f32 * 6.0);
                ui.label(RichText::new(marker).color(BRASS).size(15.0));
                ui.vertical(|ui| para(ui, text, q));
            });
        } else {
            paragraph.push(t);
        }
        i += 1;
    }
    flush(ui, &mut paragraph);
}

#[cfg(test)]
mod tests {
    #[test]
    fn guide_has_its_sections() {
        let g = super::Guide::default();
        let titles: Vec<&str> = g.sections.iter().map(|s| s.title.as_str()).collect();
        for want in ["Welcome", "The whistle", "Calling CQ", "Finding stations", "Files", "Roundtables", "Encryption (optional)", "Troubleshooting", "Glossary"] {
            assert!(titles.contains(&want), "missing {want}: {titles:?}");
        }
    }
}
