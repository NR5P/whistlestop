//! The Whistlestop mark: a chirp wave (the whistle: cycles speeding up,
//! tapered ends) in a brass ring. Drawn in code, both as the
//! app icon (Dock, window) and in the top bar, so there are no image files.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};

/// The chirp wave, in a unit square (0..1, y down): a wave whose cycles
/// get faster from left to right, tapered at both ends like the whistle's
/// chirps.
pub fn curve() -> Vec<(f32, f32)> {
    let n = 160;
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let phase = std::f32::consts::TAU * (1.2 * t + 1.6 * t * t);
            let taper = (std::f32::consts::PI * t).sin().powf(0.6);
            (0.25 + 0.5 * t, 0.5 - 0.15 * taper * phase.sin())
        })
        .collect()
}

/// Draw the mark into `rect` (top bar and anywhere else in the UI).
pub fn paint(p: &egui::Painter, rect: Rect, brass: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    p.circle_filled(c, r, brass.linear_multiply(0.14));
    p.circle_stroke(c, r - 1.0, Stroke::new((r * 0.08).max(1.5), brass));
    let pts: Vec<Pos2> = curve().iter().map(|&(x, y)| Pos2::new(rect.left() + x * rect.width(), rect.top() + y * rect.height())).collect();
    p.add(egui::Shape::line(pts, Stroke::new((r * 0.11).max(2.0), brass)));
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// Signed distance to a rounded square of half-size `h`, corner radius `rad`.
fn rounded_box(p: (f32, f32), h: f32, rad: f32) -> f32 {
    let qx = p.0.abs() - (h - rad);
    let qy = p.1.abs() - (h - rad);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - rad
}

/// The app icon as RGBA pixels: a deep navy rounded square (with the
/// transparent margin macOS icons have) holding the mark.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    render(size, None)
}

/// The icon for phones: opaque and full-bleed (iOS and Android round the
/// corners themselves), the navy square scaled to span `span` of the image
/// (1.0 for iOS; 72/108 for an Android adaptive icon's visible area).
pub fn icon_rgba_square(size: u32, span: f32) -> Vec<u8> {
    render(size, Some(span))
}

fn render(size: u32, square: Option<f32>) -> Vec<u8> {
    // The macOS square spans 0.82 of its image; a phone icon maps it to `span`.
    let zoom = square.map_or(1.0, |span| 0.82 / span);
    let s = size as f32 / zoom;
    let brass = (242.0f32, 169.0f32, 59.0f32);
    let wave = curve();
    let mut px = vec![0u8; (size * size * 4) as usize];
    // Coverage of a shape at signed distance `d` (unit coords), ~1 px edge.
    let cov = |d: f32| (0.5 - d * s).clamp(0.0, 1.0);
    let mix = |a: (f32, f32, f32), b: (f32, f32, f32), m: f32| (a.0 + (b.0 - a.0) * m, a.1 + (b.1 - a.1) * m, a.2 + (b.2 - a.2) * m);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
            let (u, v) = (0.5 + (u - 0.5) * zoom, 0.5 + (v - 0.5) * zoom);
            let alpha = if square.is_some() { 1.0 } else { cov(rounded_box((u - 0.5, v - 0.5), 0.41, 0.095)) };
            if alpha <= 0.0 {
                continue;
            }
            // Navy, a little lighter at the top.
            let mut col = mix((28.0, 36.0, 50.0), (11.0, 14.0, 20.0), ((v - 0.09) / 0.82).clamp(0.0, 1.0));
            let d_c = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
            col = mix(col, brass, (1.0 - d_c / 0.30).clamp(0.0, 1.0) * 0.13); // glow
            let ring = cov((d_c - 0.30).abs() - 0.021);
            let d_w = wave.windows(2).map(|w| dist_to_segment((u, v), w[0], w[1])).fold(f32::INFINITY, f32::min);
            let wavec = cov(d_w - 0.024);
            col = mix(col, brass, ring.max(wavec));
            let i = ((y * size + x) * 4) as usize;
            px[i] = col.0 as u8;
            px[i + 1] = col.1 as u8;
            px[i + 2] = col.2 as u8;
            px[i + 3] = (alpha * 255.0) as u8;
        }
    }
    px
}

pub fn icon_data() -> egui::IconData {
    let size = 512;
    egui::IconData { rgba: icon_rgba(size), width: size, height: size }
}
