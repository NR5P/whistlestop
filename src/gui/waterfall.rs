//! Scrolling waterfall drawn from a ring texture (newest row on top).

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions};

pub struct Waterfall {
    name: &'static str,
    tex: Option<TextureHandle>,
    row: usize,
    pub cols: usize,
    rows: usize,
    lut: Vec<Color32>,
    noise: f32,
    lines: u64,
    /// Last row's frequency range (for overlays).
    pub start_hz: f64,
    pub span_hz: f64,
    pub range_db: f32,
}

/// Dark blue -> violet -> amber -> white, readable on a dark UI.
fn lut() -> Vec<Color32> {
    let stops: [(f32, [f32; 3]); 6] = [
        (0.00, [6.0, 10.0, 22.0]),
        (0.25, [22.0, 38.0, 92.0]),
        (0.50, [110.0, 44.0, 140.0]),
        (0.72, [226.0, 96.0, 60.0]),
        (0.88, [246.0, 186.0, 70.0]),
        (1.00, [255.0, 250.0, 225.0]),
    ];
    (0..256)
        .map(|i| {
            let t = i as f32 / 255.0;
            let k = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0).min(stops.len() - 2);
            let (a, b) = (stops[k], stops[k + 1]);
            let u = ((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
            let c = |j: usize| (a.1[j] + (b.1[j] - a.1[j]) * u) as u8;
            Color32::from_rgb(c(0), c(1), c(2))
        })
        .collect()
}

impl Waterfall {
    pub fn new(name: &'static str, cols: usize, rows: usize) -> Self {
        Self { name, tex: None, row: 0, cols, rows, lut: lut(), noise: -100.0, lines: 0, start_hz: 0.0, span_hz: 1.0, range_db: 45.0 }
    }

    /// Add one row of `cols` dB levels.
    pub fn push(&mut self, ctx: &egui::Context, levels: &[f32]) {
        let (cols, rows) = (self.cols, self.rows);
        let tex = self.tex.get_or_insert_with(|| ctx.load_texture(self.name, ColorImage::filled([cols, rows], Color32::BLACK), TextureOptions::LINEAR_REPEAT));
        let mut sorted: Vec<f32> = levels.iter().copied().filter(|v| v.is_finite()).collect();
        if sorted.is_empty() {
            return;
        }
        sorted.sort_by(|a, b| a.total_cmp(b));
        let p = sorted[sorted.len() * 3 / 10];
        self.noise = if self.lines == 0 { p } else { 0.93 * self.noise + 0.07 * p };
        self.lines += 1;
        let floor = self.noise - 4.0;
        let px: Vec<Color32> = levels
            .iter()
            .take(cols)
            .map(|db| self.lut[(((db - floor) / self.range_db).clamp(0.0, 1.0) * 255.0) as usize])
            .chain(std::iter::repeat(Color32::BLACK))
            .take(cols)
            .collect();
        tex.set_partial([0, self.row], ColorImage::new([cols, 1], px), TextureOptions::LINEAR_REPEAT);
        self.row = (self.row + rows - 1) % rows;
    }

    pub fn paint(&self, painter: &egui::Painter, rect: Rect) {
        painter.rect_filled(rect, 6.0, Color32::from_rgb(6, 10, 22));
        if let Some(t) = &self.tex {
            let v0 = (self.row + 1) as f32 / self.rows as f32;
            let uv = Rect::from_min_max(Pos2::new(0.0, v0), Pos2::new(1.0, v0 + 1.0));
            painter.image(t.id(), rect, uv, Color32::WHITE);
        }
    }
}
