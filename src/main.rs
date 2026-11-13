//! Whistlestop: HF chat, files and Roundtables over the Mercury modem,
//! with the CQ whistle that scanning stations stop for.

mod gui;

use anyhow::Context;
use clap::{Parser, Subcommand};
use whistlestop::whistle::{WhistleParams, profile, whistle};

#[derive(Parser)]
#[command(name = "whistlestop", version, about = "HF chat, files and Roundtables over the Mercury modem")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the app (the default).
    Gui,
    /// Write the CQ whistle to a WAV file, to hear what it sounds like.
    WhistleWav {
        #[arg(default_value = "whistle.wav")]
        path: String,
        /// 0.5 to 600 s.
        #[arg(long, default_value_t = 6.0, value_parser = seconds_0_5_to_600)]
        seconds: f64,
        #[arg(long, default_value = "wide", value_parser = ["wide", "narrow"])]
        profile: String,
    },
    /// List audio devices.
    Devices,
    /// Write the app icon as a PNG.
    IconPng {
        #[arg(default_value = "whistlestop-icon.png")]
        path: String,
        #[arg(long, default_value_t = 512)]
        size: u32,
        /// Phone icon: opaque, full-bleed, the navy square spanning this
        /// fraction of the image (1.0 for iOS, 0.667 for Android adaptive).
        #[arg(long)]
        square: Option<f32>,
    },
    /// Write demo sounds of a whole CQ (whistle + CQ burst), clean and as
    /// heard on the air, into a folder.
    CqDemo {
        #[arg(default_value = "demo-sounds")]
        dir: String,
        /// Whistle length in the demo, 0.5 to 600 s (the app's default is 10 s).
        #[arg(long, default_value_t = 10.0, value_parser = seconds_0_5_to_600)]
        whistle: f64,
    },
}

/// A length in seconds a sound can sensibly have (and memory can hold).
fn seconds_0_5_to_600(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("{s:?} isn't a number of seconds"))?;
    if !(0.5..=600.0).contains(&v) {
        return Err(format!("{s} s is out of range: use 0.5 to 600 s"));
    }
    Ok(v)
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd.unwrap_or(Command::Gui) {
        Command::Gui => {
            let options = eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_title("Whistlestop")
                    .with_app_id("whistlestop")
                    .with_icon(gui::icon::icon_data())
                    .with_inner_size([1440.0, 920.0])
                    .with_min_inner_size([1100.0, 700.0])
                    .with_drag_and_drop(true),
                ..Default::default()
            };
            eframe::run_native("Whistlestop", options, Box::new(|cc| Ok(Box::new(gui::App::new(cc))))).map_err(|e| anyhow::anyhow!("{e}"))
        }
        Command::WhistleWav { path, seconds, profile: name } => {
            let p: WhistleParams = profile(&name).with_fs(48_000);
            let audio = whistle(seconds, &p);
            let spec = hound::WavSpec { channels: 1, sample_rate: p.fs, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::create(&path, spec).with_context(|| format!("can't write {path}"))?;
            for s in audio {
                w.write_sample((s * i16::MAX as f32) as i16)?;
            }
            w.finalize()?;
            println!("Wrote {path}: {seconds} s {name} whistle, {:.0}–{:.0} Hz, {} s per chirp", p.f_lo, p.f_hi, p.chirp_s);
            Ok(())
        }
        Command::CqDemo { dir, whistle: wsecs } => cq_demo(&dir, wsecs),
        Command::IconPng { path, size, square } => {
            if !(16..=4096).contains(&size) {
                anyhow::bail!("--size must be 16 to 4096 pixels (got {size})");
            }
            if square.is_some_and(|s| !(s.is_finite() && s > 0.0 && s <= 4.0)) {
                anyhow::bail!("--square must be a number above 0, at most 4");
            }
            let rgba = match square {
                Some(span) => gui::icon::icon_rgba_square(size, span),
                None => gui::icon::icon_rgba(size),
            };
            let img = eframe::egui::ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &rgba);
            write_png(&path, size, size, img.as_raw())?;
            println!("Wrote {path}");
            Ok(())
        }
        Command::Devices => {
            let (ins, outs) = whistlestop::audio::list_devices();
            println!("Input devices:");
            ins.iter().for_each(|d| println!("  {d}"));
            println!("Output devices:");
            outs.iter().for_each(|d| println!("  {d}"));
            Ok(())
        }
    }
}

// ------------------------------------------------------------- demo sounds

const DEMO_FS: u32 = 16_000;

fn write_wav(path: &std::path::Path, x: &[f64]) -> anyhow::Result<()> {
    let peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-9);
    let spec = hound::WavSpec { channels: 1, sample_rate: DEMO_FS, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).with_context(|| format!("can't write {}", path.display()))?;
    for v in x {
        w.write_sample((v / peak * 0.8 * i16::MAX as f64) as i16)?;
    }
    w.finalize()?;
    Ok(())
}

/// An approximation of Mercury's CQFRAME on DATAC16: a ~3.1 s OFDM burst,
/// ~200 Hz wide around 1500 Hz (8 carriers, random QPSK each symbol, with
/// a two-symbol preamble). Mercury's real waveform differs in detail.
fn cqframe_like(fs: f64, seed: &mut u64) -> Vec<f64> {
    let mut rnd = move || {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 61) as usize // 0..7
    };
    let carriers = 8;
    let spacing = 25.0;
    let sym_s = 0.04;
    let n_sym = (3.1 / sym_s) as usize;
    let ns = (sym_s * fs) as usize;
    let mut out = Vec::with_capacity(n_sym * ns);
    let phases: Vec<Vec<f64>> = (0..n_sym)
        .map(|k| (0..carriers).map(|c| if k < 2 { (c % 2) as f64 * std::f64::consts::PI } else { (rnd() % 4) as f64 * std::f64::consts::FRAC_PI_2 }).collect())
        .collect();
    for (k, ph) in phases.iter().enumerate() {
        for i in 0..ns {
            let t = (k * ns + i) as f64 / fs;
            let mut v = 0.0;
            for (c, p) in ph.iter().enumerate() {
                let f = 1500.0 + (c as f64 - (carriers - 1) as f64 / 2.0) * spacing;
                v += (2.0 * std::f64::consts::PI * f * t + p).cos();
            }
            // Short ramps at symbol edges, as a cyclic prefix would smooth.
            let edge = (i.min(ns - 1 - i) as f64 / (0.002 * fs)).min(1.0);
            out.push(v / carriers as f64 * 2.2 * (0.6 + 0.4 * edge));
        }
    }
    out
}

/// What a USB receiver hears: shifted by `offset_hz`, band noise at `snr_db`,
/// slow fading, and the 300-2700 Hz passband.
fn on_air(x: &[f64], offset_hz: f64, snr_db: f64, seed: u64) -> Vec<f64> {
    use rand::SeedableRng;
    use rand_distr::{Distribution, Normal};
    let fs = DEMO_FS as f64;
    let faded: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let t = i as f64 / fs;
            v * (0.75 + 0.25 * (2.0 * std::f64::consts::PI * 0.23 * t).sin() * (2.0 * std::f64::consts::PI * 0.07 * t + 1.0).cos())
        })
        .collect();
    let shifted = whistlestop::whistle::ssb_channel(&faded, offset_hz, DEMO_FS, (0.0, fs / 2.0));
    let active: Vec<f64> = x.iter().copied().filter(|v| v.abs() > 1e-6).collect();
    let sig_p = active.iter().map(|v| v * v).sum::<f64>() / active.len().max(1) as f64;
    let noise_total = sig_p / 10f64.powf(snr_db / 10.0) * (fs / 2.0) / 2500.0;
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let nd = Normal::new(0.0, noise_total.sqrt()).unwrap();
    let noisy: Vec<f64> = shifted.iter().map(|v| v + nd.sample(&mut rng)).collect();
    whistlestop::whistle::ssb_channel(&noisy, 0.0, DEMO_FS, whistlestop::whistle::PASSBAND)
}

fn cq_demo(dir: &str, whistle_s: f64) -> anyhow::Result<()> {
    let dir = std::path::Path::new(dir);
    std::fs::create_dir_all(dir).with_context(|| format!("can't create the folder {}", dir.display()))?;
    let fs = DEMO_FS as f64;
    let silence = |s: f64| vec![0.0f64; (s * fs) as usize];
    let p = profile("wide").with_fs(DEMO_FS);
    let w: Vec<f64> = whistle(whistle_s, &p).iter().map(|&v| v as f64).collect();
    let mut seed = 0x5eed;
    let burst = cqframe_like(fs, &mut seed);

    // A whole CQ: key up, whistle, a beat, CQFRAME, then listening.
    let mut cq = silence(0.5);
    cq.extend(&w);
    cq.extend(silence(0.4));
    cq.extend(&burst);
    cq.extend(silence(1.5));

    let w6: Vec<f64> = whistle(6.0, &p).iter().map(|&v| v as f64).collect();
    let narrow: Vec<f64> = whistle(6.0, &profile("narrow").with_fs(DEMO_FS)).iter().map(|&v| v as f64).collect();
    let files: [(&str, Vec<f64>, &str); 5] = [
        ("1-whistle-clean.wav", w6.clone(), "the whistle alone, exactly as Whistlestop sends it"),
        ("2-cq-clean.wav", cq.clone(), "a whole CQ: whistle, then the CQ burst (approximation of Mercury's CQFRAME)"),
        ("3-cq-on-the-air.wav", on_air(&cq, 0.0, 6.0, 1), "the CQ as a station tuned right on it hears it: band noise and fading"),
        ("4-cq-heard-600hz-off.wav", on_air(&cq, 600.0, 6.0, 2), "as a scanning radio 600 Hz off hears it, before it retunes"),
        ("5-whistle-narrow.wav", narrow, "the narrow-filter whistle (1250-1750 Hz)"),
    ];
    for (name, x, what) in &files {
        write_wav(&dir.join(name), x)?;
        println!("{name:28} {:5.1} s  {what}", x.len() as f64 / fs);
    }
    Ok(())
}

/// A minimal PNG writer (stored, uncompressed deflate) so no image crate is needed.
fn write_png(path: &str, w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<()> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, ty: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend(data);
        out.extend(&c);
        out.extend(crc(&c).to_be_bytes());
    }
    let mut raw = vec![];
    for row in rgba.chunks((w * 4) as usize) {
        raw.push(0u8);
        raw.extend(row);
    }
    // zlib: header, stored blocks, adler32.
    let mut z = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend((block.len() as u16).to_le_bytes());
        z.extend((!(block.len() as u16)).to_le_bytes());
        z.extend(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out).with_context(|| format!("can't write {path}"))?;
    Ok(())
}
