//! Mercury's modes, bandwidth choices, speed selection, and time predictions.
//!
//! Figures come from Mercury `docs/MODES.md` (v1.9.23): payload bytes per
//! frame, frame duration, occupied bandwidth, and the SNR at which a mode
//! reliably delivers. They drive three things:
//!
//! * the bandwidth selector (narrow to wide),
//! * Roundtable speed ramp-up (pick the fastest mode the group's SNR supports),
//! * "how long will this take" predictions.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub name: &'static str,
    /// Index for Mercury's `MODE <n>` command / `-m`.
    pub index: u8,
    /// Code Mercury reports in `BITRATE (<code>) ...`.
    pub bitrate_code: u16,
    pub bandwidth_hz: u32,
    pub payload_bps: f64,
    pub frame_bytes: usize,
    pub frame_s: f64,
    /// SNR (dB, 2500 Hz) at which the mode delivers reliably (MODES.md MPP figures).
    pub min_snr_db: f64,
    /// e.g. "OFDM QPSK", "OFDM 16-QAM", "32-MFSK".
    pub modulation: &'static str,
    /// OFDM carriers, or MFSK tones.
    pub carriers: u16,
    /// Carrier (or tone) spacing, Hz.
    pub spacing_hz: f64,
    /// Forward error correction: code and rate.
    pub fec: &'static str,
}

impl Mode {
    /// Usable bytes in one broadcast frame (Mercury adds a 3-byte header).
    pub fn broadcast_bytes(&self) -> usize {
        self.frame_bytes - 3
    }
}

/// Runtime-switchable payload modes, slowest/most robust first.
///
/// Carriers, spacing and modulation are from Mercury's
/// `modem/freedv/ofdm_mode.c` (OFDM symbol 16 ms -> 62.5 Hz spacing; 2 bits
/// per symbol = QPSK unless set to 4 = 16-QAM) and its 32-MFSK floor; FEC and
/// rates from `docs/MODES.md` (v1.9.23).
pub const MODES: &[Mode] = &[
    Mode {
        name: "MFSK500",
        index: 12,
        bitrate_code: 100,
        bandwidth_hz: 500,
        payload_bps: 31.0,
        frame_bytes: 98,
        frame_s: 26.2,
        min_snr_db: -14.0,
        modulation: "32-MFSK",
        carriers: 32,
        spacing_hz: 15.6,
        fec: "LDPC 1/2 (N=1600)",
    },
    Mode {
        name: "MFSK",
        index: 11,
        bitrate_code: 100,
        bandwidth_hz: 1000,
        payload_bps: 60.0,
        frame_bytes: 98,
        frame_s: 13.5,
        min_snr_db: -11.0,
        modulation: "32-MFSK",
        carriers: 32,
        spacing_hz: 31.25,
        fec: "LDPC 1/2 (N=1600)",
    },
    Mode {
        name: "DATAC15",
        index: 7,
        bitrate_code: 15,
        bandwidth_hz: 200,
        payload_bps: 68.0,
        frame_bytes: 30,
        frame_s: 3.74,
        min_snr_db: -7.0,
        modulation: "OFDM QPSK",
        carriers: 3,
        spacing_hz: 62.5,
        fec: "LDPC 1/3 (768,256)",
    },
    Mode {
        name: "DATAC4",
        index: 3,
        bitrate_code: 4,
        bandwidth_hz: 250,
        payload_bps: 87.0,
        frame_bytes: 54,
        frame_s: 5.17,
        min_snr_db: -4.0,
        modulation: "OFDM QPSK",
        carriers: 4,
        spacing_hz: 62.5,
        fec: "LDPC 0.30 (1472,448)",
    },
    Mode {
        name: "DATAC3",
        index: 1,
        bitrate_code: 3,
        bandwidth_hz: 500,
        payload_bps: 321.0,
        frame_bytes: 126,
        frame_s: 3.19,
        min_snr_db: 0.0,
        modulation: "OFDM QPSK",
        carriers: 9,
        spacing_hz: 62.5,
        fec: "LDPC 1/2 (2048,1024)",
    },
    Mode {
        name: "DATAC1",
        index: 0,
        bitrate_code: 1,
        bandwidth_hz: 1700,
        payload_bps: 980.0,
        frame_bytes: 510,
        frame_s: 4.18,
        min_snr_db: 5.0,
        modulation: "OFDM QPSK",
        carriers: 27,
        spacing_hz: 62.5,
        fec: "LDPC 1/2 (8192,4096)",
    },
    Mode {
        name: "DATAC17",
        index: 9,
        bitrate_code: 17,
        bandwidth_hz: 2100,
        payload_bps: 1410.0,
        frame_bytes: 1180,
        frame_s: 6.71,
        min_snr_db: 10.0,
        modulation: "OFDM QPSK",
        carriers: 33,
        spacing_hz: 62.5,
        fec: "LDPC 0.59 (15936,9456)",
    },
    Mode {
        name: "QAM16C2",
        index: 10,
        bitrate_code: 25,
        bandwidth_hz: 2100,
        payload_bps: 3100.0,
        frame_bytes: 1213,
        frame_s: 3.2,
        min_snr_db: 15.7,
        modulation: "OFDM 16-QAM",
        carriers: 33,
        spacing_hz: 62.5,
        fec: "LDPC 0.60 (16200,9720)",
    },
];

/// Mercury's ARQ speed ladder for a session bandwidth (BW500 or wide),
/// slowest rung first.
pub fn arq_ladder(session_bw: u32) -> Vec<&'static Mode> {
    let names: &[&str] = if session_bw == 500 {
        &["MFSK500", "DATAC15", "DATAC4", "DATAC3"]
    } else {
        &[
            "MFSK", "DATAC15", "DATAC4", "DATAC3", "DATAC1", "DATAC17", "QAM16C2",
        ]
    };
    names.iter().filter_map(|n| mode_by_name(n)).collect()
}

pub fn mode_by_name(name: &str) -> Option<&'static Mode> {
    MODES.iter().find(|m| m.name.eq_ignore_ascii_case(name))
}

pub fn mode_by_index(index: u8) -> Option<&'static Mode> {
    MODES.iter().find(|m| m.index == index)
}

pub fn mode_by_bitrate_code(code: u16) -> Option<&'static Mode> {
    MODES
        .iter()
        .find(|m| m.bitrate_code == code && m.name != "MFSK500")
}

/// Bandwidth the operator selects, narrow to wide.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Bandwidth {
    Hz250,
    Hz500,
    Hz1000,
    Hz1700,
    Hz2300,
    Hz2750,
}

impl Bandwidth {
    pub const ALL: [Bandwidth; 6] = [
        Self::Hz250,
        Self::Hz500,
        Self::Hz1000,
        Self::Hz1700,
        Self::Hz2300,
        Self::Hz2750,
    ];

    pub fn hz(self) -> u32 {
        match self {
            Self::Hz250 => 250,
            Self::Hz500 => 500,
            Self::Hz1000 => 1000,
            Self::Hz1700 => 1700,
            Self::Hz2300 => 2300,
            Self::Hz2750 => 2750,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.trim_end_matches("hz");
        match s {
            "narrow" => Some(Self::Hz500),
            "wide" => Some(Self::Hz2300),
            "max" | "widest" => Some(Self::Hz2750),
            "narrowest" => Some(Self::Hz250),
            _ => {
                let v: u32 = s.parse().ok()?;
                Self::ALL.iter().copied().find(|b| b.hz() == v)
            }
        }
    }

    /// Mercury's ARQ bandwidth token for 1:1 links. Mercury has two ladders:
    /// BW500 (up to DATAC3) and BW2300/BW2750 (up to QAM16C2). Anything
    /// narrower than 2300 uses BW500 so the link never exceeds the selection
    /// (its MFSK floor is 1000 Hz, which is why 250 Hz stays on BW500 too).
    pub fn mercury_arq_bw(self) -> u32 {
        match self {
            Self::Hz2750 => 2750,
            Self::Hz2300 => 2300,
            _ => 500,
        }
    }

    /// The whistle profile that fits this bandwidth.
    pub fn whistle_profile(self) -> crate::whistle::WhistleParams {
        crate::whistle::profile(if self.hz() >= 1700 { "wide" } else { "narrow" })
    }

    /// Roundtable modes allowed inside this bandwidth, slowest first.
    pub fn modes(self) -> Vec<&'static Mode> {
        MODES
            .iter()
            .filter(|m| m.bandwidth_hz <= self.hz())
            .collect()
    }
}

impl fmt::Display for Bandwidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} Hz", self.hz())
    }
}

/// Speed ramp-up: the fastest mode within `bw` that `snr_db` supports, with
/// `margin_db` of headroom for fading. Falls back to the most robust mode.
pub fn best_mode(bw: Bandwidth, snr_db: f64, margin_db: f64) -> &'static Mode {
    let allowed = bw.modes();
    allowed
        .iter()
        .filter(|m| snr_db >= m.min_snr_db + margin_db)
        .max_by(|a, b| a.payload_bps.partial_cmp(&b.payload_bps).unwrap())
        .copied()
        .unwrap_or_else(|| {
            allowed
                .iter()
                .min_by(|a, b| a.min_snr_db.partial_cmp(&b.min_snr_db).unwrap())
                .copied()
                .unwrap_or(&MODES[0])
        })
}

// -------------------------------------------------------------- predictions

/// Fraction of a mode's raw payload rate a 1:1 ARQ session actually achieves
/// (turnarounds, acks, retries). A starting guess; the live estimator
/// replaces it with measured throughput.
pub const ARQ_EFFICIENCY: f64 = 0.6;

/// Predicted time to send `bytes` over a 1:1 link at `snr_db` within `bw`.
pub fn predict_arq_s(bytes: usize, bw: Bandwidth, snr_db: f64) -> f64 {
    // The ARQ ladder is capped by Mercury's token, not the finer selection.
    let ladder = match bw.mercury_arq_bw() {
        500 => Bandwidth::Hz500,
        _ => Bandwidth::Hz2750,
    };
    let m = best_mode(ladder, snr_db, 1.0);
    let setup_s = 8.0; // CALL/ACCEPT plus the first turnaround
    setup_s + bytes as f64 * 8.0 / (m.payload_bps * ARQ_EFFICIENCY)
}

/// Exact air time for a Roundtable message: whole frames, no ARQ.
pub fn predict_broadcast_s(bytes: usize, mode: &Mode) -> f64 {
    let frames = bytes.div_ceil(mode.broadcast_bytes()).max(1);
    frames as f64 * mode.frame_s
}

pub fn human_duration(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

pub fn human_bytes(b: usize) -> String {
    match b {
        0..=1023 => format!("{b} B"),
        1024..=1_048_575 => format!("{:.1} KB", b as f64 / 1024.0),
        _ => format!("{:.2} MB", b as f64 / 1_048_576.0),
    }
}

/// Live throughput estimate from Mercury's `BUFFER` reports during a 1:1
/// session: how fast the TX queue actually drains. Falls back to the
/// `BITRATE` report, then to the mode table.
#[derive(Default, Debug)]
pub struct ThroughputEstimator {
    last: Option<(std::time::Instant, usize)>,
    bps_ewma: Option<f64>,
    pub reported_bps: Option<f64>,
    pub reported_mode: Option<&'static Mode>,
    pub snr_db: Option<f64>,
    samples: u32,
}

impl ThroughputEstimator {
    pub fn on_buffer(&mut self, now: std::time::Instant, queued: usize) {
        if let Some((t0, q0)) = self.last {
            let dt = now.duration_since(t0).as_secs_f64();
            // A short message's drain is mostly turnaround time, and a long
            // gap includes idle time: neither says anything about speed.
            if queued < q0 && q0 - queued >= 512 && (0.5..=30.0).contains(&dt) {
                let bps = (q0 - queued) as f64 * 8.0 / dt;
                self.bps_ewma = Some(match self.bps_ewma {
                    Some(e) => 0.7 * e + 0.3 * bps,
                    None => bps,
                });
                self.samples += 1;
            }
        }
        self.last = Some((now, queued));
    }

    /// Parses `BITRATE (<code>) <bps> BPS`.
    pub fn on_bitrate_line(&mut self, line: &str) {
        let mut it = line.split_whitespace();
        if it.next() != Some("BITRATE") {
            return;
        }
        let code = it.next().and_then(|c| {
            c.trim_matches(|ch| ch == '(' || ch == ')')
                .parse::<u16>()
                .ok()
        });
        let bps = it.next().and_then(|b| b.parse::<f64>().ok());
        if let Some(c) = code {
            self.reported_mode = mode_by_bitrate_code(c);
        }
        if let Some(b) = bps {
            self.reported_bps = Some(b);
        }
    }

    pub fn on_snr_line(&mut self, line: &str) {
        if let Some(v) = line
            .strip_prefix("SN ")
            .and_then(|v| v.trim().parse::<f64>().ok())
        {
            self.snr_db = Some(v);
        }
    }

    /// Best current guess of goodput in bits/s.
    pub fn bps(&self, bw: Bandwidth) -> f64 {
        match (self.bps_ewma, self.reported_bps) {
            (Some(b), _) if self.samples >= 2 => return b,
            (_, Some(r)) => return r * ARQ_EFFICIENCY,
            (Some(b), None) => return b,
            _ => {}
        }
        let snr = self.snr_db.unwrap_or(0.0);
        8.0 / (predict_arq_s(1, bw, snr) - 8.0)
    }

    pub fn eta_s(&self, bytes: usize, bw: Bandwidth) -> f64 {
        bytes as f64 * 8.0 / self.bps(bw).max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_up_with_snr() {
        let bw = Bandwidth::Hz2750;
        assert_eq!(best_mode(bw, -12.0, 0.0).name, "MFSK500");
        assert_eq!(best_mode(bw, -10.0, 0.0).name, "MFSK");
        assert_eq!(best_mode(bw, 2.0, 1.0).name, "DATAC3");
        assert_eq!(best_mode(bw, 7.0, 1.0).name, "DATAC1");
        assert_eq!(best_mode(bw, 20.0, 1.0).name, "QAM16C2");
    }

    #[test]
    fn bandwidth_caps_the_mode() {
        assert_eq!(best_mode(Bandwidth::Hz500, 30.0, 1.0).name, "DATAC3");
        assert_eq!(best_mode(Bandwidth::Hz250, 30.0, 1.0).name, "DATAC4");
        assert_eq!(best_mode(Bandwidth::Hz1700, 30.0, 1.0).name, "DATAC1");
        assert!(
            Bandwidth::Hz250
                .modes()
                .iter()
                .all(|m| m.bandwidth_hz <= 250)
        );
        assert_eq!(Bandwidth::Hz1000.mercury_arq_bw(), 500);
        assert_eq!(Bandwidth::parse("narrow"), Some(Bandwidth::Hz500));
        assert_eq!(Bandwidth::parse("2300Hz"), Some(Bandwidth::Hz2300));
    }

    #[test]
    fn predictions_are_sane() {
        let m = mode_by_name("DATAC3").unwrap();
        assert_eq!(predict_broadcast_s(100, m), m.frame_s);
        assert_eq!(predict_broadcast_s(124, m), 2.0 * m.frame_s);
        // 10 KB at good SNR wide is much faster than narrow at poor SNR.
        assert!(
            predict_arq_s(10_000, Bandwidth::Hz2300, 20.0)
                < predict_arq_s(10_000, Bandwidth::Hz500, -3.0)
        );
        assert_eq!(human_duration(75.0), "1m15s");
    }

    #[test]
    fn estimator_learns_from_buffer() {
        let mut e = ThroughputEstimator::default();
        let t0 = std::time::Instant::now();
        e.on_buffer(t0, 10_000);
        e.on_buffer(t0 + std::time::Duration::from_secs(10), 9_000);
        e.on_buffer(t0 + std::time::Duration::from_secs(20), 8_000);
        assert!((e.bps(Bandwidth::Hz2300) - 800.0).abs() < 1e-6);
        // A 40-byte chat line draining over 9 s is not a speed measurement.
        let mut f = ThroughputEstimator::default();
        f.on_bitrate_line("BITRATE (25) 3100 BPS");
        f.on_buffer(t0, 40);
        f.on_buffer(t0 + std::time::Duration::from_secs(9), 0);
        assert!((f.bps(Bandwidth::Hz2300) - 3100.0 * ARQ_EFFICIENCY).abs() < 1e-6);
        e.on_bitrate_line("BITRATE (25) 3100 BPS");
        assert_eq!(e.reported_mode.unwrap().name, "QAM16C2");
    }
}
