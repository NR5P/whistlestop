//! The Whistlestop icon: a brass steam whistle blowing, with steam bursting
//! from its mouth and the chirp wave itself (faster and faster) blasting out.
//! Painted in code, pixel by pixel, so there are no image files: the same
//! picture is the app icon (Dock, window) and the logo in the top bar.

use eframe::egui;

type Rgb = (f32, f32, f32);

const BRASS_HI: Rgb = (255.0, 226.0, 150.0);
const BRASS: Rgb = (232.0, 160.0, 52.0);
const BRASS_LO: Rgb = (120.0, 66.0, 18.0);
const STEAM: Rgb = (236.0, 242.0, 250.0);

fn mix(a: Rgb, b: Rgb, m: f32) -> Rgb {
    let m = m.clamp(0.0, 1.0);
    (a.0 + (b.0 - a.0) * m, a.1 + (b.1 - a.1) * m, a.2 + (b.2 - a.2) * m)
}

// ------------------------------------------------- signed distance shapes

fn circle(p: (f32, f32), c: (f32, f32), r: f32) -> f32 {
    ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt() - r
}

/// Rounded box centred on `c` with half-size `h` and corner radius `r`.
fn rbox(p: (f32, f32), c: (f32, f32), h: (f32, f32), r: f32) -> f32 {
    let qx = (p.0 - c.0).abs() - (h.0 - r);
    let qy = (p.1 - c.1).abs() - (h.1 - r);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
}

fn segment(p: (f32, f32), a: (f32, f32), b: (f32, f32), r: f32) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    ((wx - t * vx).powi(2) + (wy - t * vy).powi(2)).sqrt() - r
}

/// An arc of radius `rad` around `c`, `w` thick, between angles `a0..a1`
/// (radians, 0 = right, y down), with round ends.
fn arc(p: (f32, f32), c: (f32, f32), rad: f32, w: f32, a0: f32, a1: f32) -> f32 {
    let (dx, dy) = (p.0 - c.0, p.1 - c.1);
    let ang = dy.atan2(dx);
    if ang >= a0 && ang <= a1 {
        ((dx * dx + dy * dy).sqrt() - rad).abs() - w
    } else {
        let e0 = (c.0 + rad * a0.cos(), c.1 + rad * a0.sin());
        let e1 = (c.0 + rad * a1.cos(), c.1 + rad * a1.sin());
        circle(p, e0, w).min(circle(p, e1, w))
    }
}

// ------------------------------------------------------------------ painting

/// The icon as RGBA pixels (`size` × `size`), with the transparent margin
/// macOS icons have.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let cov = |d: f32| (0.5 - d * s).clamp(0.0, 1.0);
    let mut px = vec![0u8; (size * size * 4) as usize];

    // The whistle leans back a little, for drama.
    let lean = -0.12f32;
    let pivot = (0.44f32, 0.72f32);
    let (sn, cs) = lean.sin_cos();
    let rot = |p: (f32, f32)| {
        let (dx, dy) = (p.0 - pivot.0, p.1 - pivot.1);
        (pivot.0 + dx * cs - dy * sn, pivot.1 + dx * sn + dy * cs)
    };
    // The chirp wave leaving the mouth, rightwards: faster cycles, an
    // amplitude that swells then tapers.
    let chirp: Vec<(f32, f32)> = (0..=220)
        .map(|i| {
            let t = i as f32 / 220.0;
            let phase = std::f32::consts::TAU * (1.0 * t + 2.2 * t * t);
            let amp = 0.085 * (std::f32::consts::PI * t).sin().powf(0.7);
            (0.56 + 0.31 * t, 0.43 - 0.06 * t - amp * phase.sin())
        })
        .collect();
    // Steam puffs behind the whistle: (centre, radius, opacity).
    let puffs: [((f32, f32), f32, f32); 12] = [
        ((0.31, 0.53), 0.055, 0.95),
        ((0.25, 0.46), 0.064, 0.9),
        ((0.21, 0.37), 0.068, 0.8),
        ((0.25, 0.28), 0.058, 0.62),
        ((0.33, 0.215), 0.048, 0.45),
        ((0.165, 0.30), 0.044, 0.5),
        ((0.58, 0.535), 0.044, 0.85),
        ((0.63, 0.47), 0.050, 0.65),
        ((0.62, 0.39), 0.040, 0.45),
        ((0.15, 0.43), 0.034, 0.5),
        ((0.40, 0.16), 0.030, 0.3),
        ((0.20, 0.52), 0.030, 0.6),
    ];

    for y in 0..size {
        for x in 0..size {
            let p = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let alpha = cov(rbox(p, (0.5, 0.5), (0.41, 0.41), 0.095));
            if alpha <= 0.0 {
                continue;
            }
            // Background: deep navy, lighter at the top, hot glow behind.
            let mut col = mix((30.0, 38.0, 56.0), (9.0, 11.0, 18.0), (p.1 - 0.09) / 0.82);
            let g = (1.0 - circle(p, (0.46, 0.44), 0.0) / 0.42).clamp(0.0, 1.0);
            col = mix(col, (255.0, 118.0, 28.0), g * g * 0.42);

            // The whistle blows out the chirp itself: a wave that speeds up
            // as it travels, with a hot glow around it.
            let d_w = chirp.windows(2).map(|w| segment(p, w[0], w[1], 0.0)).fold(f32::INFINITY, f32::min);
            let glow = (1.0 - d_w / 0.06).clamp(0.0, 1.0).powi(2);
            col = mix(col, (255.0, 150.0, 40.0), glow * 0.45);
            col = mix(col, mix(BRASS_HI, (255.0, 250.0, 235.0), 0.3), cov(d_w - 0.013));

            // Steam behind the whistle.
            let mut steam = 0.0f32;
            let mut shade = 0.0f32;
            for (c, r, o) in puffs.iter() {
                let d = circle(p, *c, *r);
                let soft = (0.5 - d * s * 0.09).clamp(0.0, 1.0);
                if soft * o > steam {
                    steam = soft * o;
                    // Lower part of each puff a touch greyer.
                    shade = ((p.1 - c.1) / r).clamp(0.0, 1.0) * 0.35;
                }
            }
            col = mix(col, mix(STEAM, (150.0, 162.0, 180.0), shade), steam * 0.88);

            // The whistle, drawn in its leaned frame.
            let q = rot(p);
            let body = rbox(q, (0.44, 0.40), (0.092, 0.14), 0.02);
            let dome = circle(q, (0.44, 0.27), 0.092).max(q.1 - 0.30);
            let bell = body.min(dome);
            let finial = circle(q, (0.44, 0.158), 0.026).min(rbox(q, (0.44, 0.188), (0.011, 0.022), 0.004));
            let cup = rbox(q, (0.44, 0.605), (0.112, 0.034), 0.018);
            let stem = rbox(q, (0.44, 0.71), (0.026, 0.085), 0.006);
            let flange = rbox(q, (0.44, 0.795), (0.058, 0.015), 0.006);
            let lever = segment(q, (0.47, 0.665), (0.665, 0.735), 0.012);
            let pull = arc(q, (0.695, 0.755), 0.029, 0.0075, -3.3, 3.3);
            let flat = lever.min(pull);
            let metal = bell.min(finial).min(cup).min(stem).min(flange).min(flat);
            let m = cov(metal);
            if m > 0.0 {
                // Cylindrical shading: a bright streak left of centre, dark
                // edges.
                let t = ((q.0 - 0.415) / 0.115).clamp(-1.0, 1.0);
                let mut c = mix(BRASS_LO, BRASS, (1.0 - t * t).max(0.0).sqrt());
                c = mix(c, BRASS_HI, (1.0 - ((q.0 - 0.405) / 0.022).powi(2)).max(0.0) * 0.9);
                // Ridges round the bell.
                if bell < 0.0 {
                    for ry in [0.29f32] {
                        let d = (q.1 - ry).abs();
                        if d < 0.008 {
                            c = mix(c, BRASS_LO, 0.6 * (1.0 - d / 0.008));
                        }
                    }
                }
                if flat <= metal + 1e-4 {
                    c = mix(BRASS, BRASS_HI, 0.3);
                }
                // Dark rim for punch.
                if metal > -2.2 / s {
                    c = mix(c, (64.0, 30.0, 8.0), 0.65);
                }
                col = mix(col, c, m);
            }
            // The mouth: a dark slot with a hot glow inside.
            col = mix(col, (70.0, 26.0, 6.0), cov(rbox(q, (0.44, 0.555), (0.108, 0.012), 0.004)));

            // Steam curling over the mouth, in front.
            let mut front = 0.0f32;
            for (c, r, o) in [((0.325, 0.565), 0.030, 1.0), ((0.555, 0.553), 0.027, 0.95), ((0.275, 0.505), 0.034, 0.8)] {
                front = front.max((0.5 - circle(p, c, r) * s * 0.12).clamp(0.0, 1.0) * o);
            }
            col = mix(col, STEAM, front);

            // Sparks.
            for (c, r) in [((0.80, 0.24), 0.007), ((0.84, 0.49), 0.006), ((0.70, 0.22), 0.005), ((0.89, 0.33), 0.007)] {
                col = mix(col, BRASS_HI, cov(circle(p, c, r)));
            }

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

/// The icon as a texture for the top bar (painted once).
pub fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    let size = 128usize;
    let img = egui::ColorImage::from_rgba_unmultiplied([size, size], &icon_rgba(size as u32));
    ctx.load_texture("whistlestop-icon", img, egui::TextureOptions::LINEAR)
}
