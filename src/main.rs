//! Whistlestop: HF chat, files and Roundtables over the Mercury modem,
//! with the CQ whistle that scanning stations stop for.

mod gui;

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
        #[arg(long, default_value_t = 6.0)]
        seconds: f64,
        /// wide or narrow
        #[arg(long, default_value = "wide")]
        profile: String,
    },
    /// List audio devices.
    Devices,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd.unwrap_or(Command::Gui) {
        Command::Gui => {
            let options = eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_title("Whistlestop")
                    .with_app_id("whistlestop")
                    .with_inner_size([1440.0, 920.0])
                    .with_min_inner_size([1100.0, 700.0])
                    .with_drag_and_drop(true),
                ..Default::default()
            };
            eframe::run_native("Whistlestop", options, Box::new(|cc| Ok(Box::new(gui::App::new(cc)))))
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        Command::WhistleWav { path, seconds, profile: name } => {
            let p: WhistleParams = profile(&name).with_fs(48_000);
            let audio = whistle(seconds, &p);
            let spec = hound::WavSpec { channels: 1, sample_rate: p.fs, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::create(&path, spec)?;
            for s in audio {
                w.write_sample((s * i16::MAX as f32) as i16)?;
            }
            w.finalize()?;
            println!("Wrote {path}: {seconds} s {name} whistle, {:.0}–{:.0} Hz, {} s per chirp", p.f_lo, p.f_hi, p.chirp_s);
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
