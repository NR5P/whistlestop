//! Help mode: when switched on (the "? Help" toggle in the top bar),
//! hovering over a control shows what it does. Off by default and not saved,
//! so it never gets in the way.

use super::{BRASS, DIM, TEXT};
use eframe::egui::{Response, RichText};
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set(v: bool) {
    ON.store(v, Ordering::Relaxed);
}

pub trait Help {
    /// In help mode, explain this control on hover: a title, the
    /// explanation, and where to read more in the Guide.
    fn help(self, topic: &(&str, &str, &str)) -> Self;
}

impl Help for Response {
    fn help(self, topic: &(&str, &str, &str)) -> Self {
        let (title, body, guide) = *topic;
        // Sliders, number boxes and dropdowns sit beside a separate text
        // label; give them the topic's name so screen readers can say what
        // they are.
        self.ctx.accesskit_node_builder(self.id, |n| {
            if n.label().is_none() {
                n.set_label(title);
            }
        });
        if !on() {
            return self;
        }
        self.on_hover_ui(|ui| {
            ui.set_max_width(400.0);
            ui.label(RichText::new(title).strong().size(15.0).color(BRASS));
            ui.add_space(2.0);
            ui.label(RichText::new(body).size(13.5).color(TEXT));
            if !guide.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("📖 Guide: {guide}")).size(11.5).color(DIM));
            }
        })
    }
}

/// (title, explanation, Guide section)
pub type Topic = (&'static str, &'static str, &'static str);

pub const HELP_TOGGLE: Topic = (
    "Help mode",
    "When on, hovering over any control explains it, like this. Turn it off when you know your way around: it's off every time Whistlestop starts.",
    "Help mode and the Guide",
);
pub const FREQ: Topic = (
    "Frequency display",
    "The radio's dial frequency (USB), band and mode, read from the radio about once a second, including changes you make on the radio itself. The lamp is green when receiving and red when transmitting.",
    "Rig controls",
);
pub const S_METER: Topic = ("S-meter", "The radio's signal strength reading (rigctld STRENGTH), shown as an S-unit. Not all radios report it.", "Rig controls");
pub const SNR_METER: Topic = (
    "SNR",
    "Mercury's signal-to-noise measurement of the last signal it decoded, in dB. Mercury uses it to choose its speed. Above about +15 dB it can reach its fastest mode; near 0 dB it slows to DATAC3 or below.",
    "Bandwidth and speed",
);
pub const PILLS: Topic = (
    "Connections",
    "Green means connected. Radio: the rigctld server (Hamlib or RigHelm). Mercury: the modem's TNC port. Audio: the radio's sound card input. SDR: the optional wide receiver. Red: open Settings, or see Troubleshooting.",
    "Troubleshooting",
);
pub const SETTINGS: Topic = (
    "Settings",
    "Callsign, Demo mode, Mercury and radio addresses, audio devices, the wide receiver, search and CQ timing, encryption and the download folder. Save & connect applies them.",
    "Settings reference",
);
pub const DEMO: Topic = (
    "Demo mode",
    "Everything is simulated: radio, band, Mercury and the K7BOT demo station. Nothing is transmitted. Untick Demo mode in Settings to use your radio.",
    "Quick start: Demo mode",
);

pub const M_STATE: Topic = (
    "Modem state",
    "Whether Mercury is transmitting (red) or receiving (green), and what for: a 1:1 link, a Roundtable, or idle and listening for calls.",
    "The modem strip",
);
pub const M_MODE: Topic = (
    "Mode and modulation",
    "The mode on the air now. During a link it's the step Mercury reports on its speed ladder; otherwise the mode it listens on (DATAC3 by default; a Roundtable's mode at a table). OFDM QPSK and 16-QAM use many carriers at once; 32-MFSK sends one of 32 tones at a time for the weakest signals.",
    "Bandwidth and speed",
);
pub const M_CARRIERS: Topic = (
    "Carriers",
    "How many OFDM carriers (or MFSK tones) the mode uses, and their spacing. More carriers in the same spacing means a wider, faster signal.",
    "Bandwidth and speed",
);
pub const M_BW: Topic = (
    "Bandwidth",
    "How wide the signal on the air is now; your limit (set in the Bandwidth card); and the width both stations agreed when the link came up (the narrower of the two limits wins).",
    "Bandwidth and speed",
);
pub const M_SPEED: Topic = (
    "Speed",
    "The current mode's raw data rate (from Mercury during a link) and the speed actually measured from what got through. Measured speed is lower because of acknowledgements and turnarounds.",
    "Bandwidth and speed",
);
pub const M_FEC: Topic = (
    "Error correction",
    "The LDPC code Mercury uses in this mode, and the size and duration of one frame. Lower code rates (1/3, 1/2) add more protection; 0.6 carries more data per frame.",
    "Bandwidth and speed",
);
pub const M_LADDER: Topic = (
    "Speed ladder",
    "The steps Mercury can use on this link, slowest (most robust) on the left. Mercury climbs and descends by itself from what actually gets through. You control the limit (Bandwidth), not the step.",
    "Bandwidth and speed",
);

pub const CQ_KIND: Topic = (
    "What the CQ is for",
    "Chat (1:1): someone answers and you have a private link for chat and files. Roundtable: a group conversation on your frequency; give it a name, and others join.",
    "Calling CQ",
);
pub const CQ_TABLE_NAME: Topic = (
    "Roundtable name",
    "Sent in your CALL packet so others can join the right table. Stations at a table with a different name ignore each other.",
    "Roundtables",
);
pub const CQ_WHISTLE: Topic = (
    "Whistle length",
    "How long the chirp whistle lasts before your digital CQ. Searching stations need to hear it during their pass, so make it at least one search pass: about 6–8 s for the 20 m segment (default 10 s). Longer reaches more searchers but holds the frequency longer.",
    "The whistle",
);
pub const CQ_CALL: Topic = (
    "Call CQ",
    "Starts the cycle: key up and whistle, then Mercury's CQFRAME with your callsign, a CALL packet, then listen for answers. Repeats (5 cycles by default) until someone answers or you press Stop. When a station calls in, the CQ goes on hold so nothing is sent over Mercury's answer.",
    "Calling CQ",
);
pub const CQ_STOP: Topic = ("Stop CQ", "Stops calling at once, mid-whistle if necessary, and unkeys.", "Calling CQ");

pub const S_CURRENT: Topic = (
    "Search current band",
    "Steps your radio through the digital segment(s) of the band you're on: half a second per channel (up to 2 s where something looks whistle-like), 2 kHz apart, about 6–8 s per pass on 20 m. Stops on a whistle, retunes onto it to within a few Hz, and waits for the CQ.",
    "Finding stations",
);
pub const S_ALL: Topic = (
    "Search all bands",
    "Every configured segment, band after band. A pass is long, so callers would need very long whistles; mostly for monitoring.",
    "Finding stations",
);
pub const S_BAND: Topic = ("Search one band", "Pick a band and search only its digital segment, without changing band first.", "Finding stations");
pub const S_STOP: Topic = ("Stop search", "Stops stepping; the radio stays where it is.", "Finding stations");
pub const S_WIDE: Topic = (
    "Wide search (SDR)",
    "With an RTL-SDR second receiver, watch the whole digital segment at once: the SDR's view is split into virtual receivers every 1.8 kHz and every one is checked each second. Finds a whistle within about 1.5 s without moving your radio. While a stepping search runs, an SDR hit pauses it, the radio goes straight to the hit, and the search picks up where it left off if no CQ follows.",
    "Finding stations",
);
pub const S_AUTOTUNE: Topic = (
    "Auto-tune",
    "Tune the radio to whistles the SDR finds, and fine-lock on the radio's own audio (SDR crystals are often hundreds of Hz off). Only when you're not linked, calling or searching. (A running search always takes SDR hits, ticked or not.)",
    "Finding stations",
);
pub const S_AUTOANSWER: Topic = (
    "Answer automatically",
    "When a search locks onto a whistle and Mercury decodes the caller's CQ, connect at once instead of waiting for you to press Answer.",
    "Finding stations",
);

pub const H_TUNE_TO: Topic = (
    "Tune to the whistle",
    "A whistle was heard near your frequency (whistle watch). This retunes by the measured offset so you're right on the caller.",
    "Finding stations",
);
pub const H_ANSWER: Topic = ("Answer", "Connect to this station: Mercury places the call, and the Chat tab opens when the link is up.", "Chat");
pub const H_JOIN: Topic =
    ("Join", "Join this Roundtable on the current frequency. The host hears your report and may adjust the table's speed. Not offered during a 1:1 link (joining would cut it).", "Roundtables");
pub const H_LOCK: Topic = ("Lock on", "Tune the radio to this SDR hit and fine-lock using the radio's own audio, then wait for the CQ.", "Finding stations");

pub const BW_BUTTONS: Topic = (
    "Bandwidth limit",
    "How wide Whistlestop may go, from 250 Hz to 2750 Hz. 1:1 links use Mercury's narrow ladder below 2300 Hz and the full ladder at 2300/2750; the narrower of the two stations' limits wins. Roundtables use any mode inside the limit. Below 1700 Hz the whistle switches to its narrow profile.",
    "Bandwidth and speed",
);

pub const R_BANDS: Topic = ("Band buttons", "Jump to that band's digital segment, where whistles and Whistlestop stations are searched for. Locked during a link: moving the dial would break it.", "Band plan");
pub const R_NUDGE: Topic = ("Nudge", "Move the dial down or up by 1 kHz or 100 Hz. Locked during a link.", "Rig controls");
pub const R_FREQ: Topic = ("Frequency entry", "Type a frequency in kHz (14103.15), MHz (14.10315) or Hz (14103150) and press Enter or Tune. Locked during a link: use QSY to move one.", "Rig controls");
pub const R_MODE: Topic =
    ("Radio mode", "PKTUSB (data USB) or USB for Whistlestop. LSB is there for completeness: Whistlestop's signals are upper sideband.", "Rig controls");
pub const R_POWER: Topic = (
    "RF power",
    "The radio's transmit power, if it supports remote setting. Data and the whistle are full duty cycle while transmitting: run the radio cool.",
    "Rig controls",
);
pub const R_TUNE: Topic =
    ("Tune carrier", "Mercury sends a steady 1 kHz tone for your antenna tuner. It always stops by itself after 60 s; Stop ends it sooner.", "Rig controls");

pub const TABS: Topic =
    ("Tabs", "Chat: 1:1 conversation. Roundtable: group conversation. Files: send and receive files. Log: everything that happened.", "A tour of the screen");
pub const GUIDE_BTN: Topic = (
    "Guide",
    "Opens the full manual in its own window, so you can keep it beside the main one. Contents and search on the left. Click again (or close its window) to put it away.",
    "Help mode and the Guide",
);
pub const L_CONNECT: Topic =
    ("Connect", "Call a station by callsign. Mercury places the call; when it answers, the link header shows their name, grid and signal report.", "Chat");
pub const L_DISCONNECT: Topic = ("Disconnect (73)", "Says goodbye, lets Mercury deliver anything still queued, then hangs up.", "Chat");
pub const C_INPUT: Topic = (
    "Message",
    "Type and press Enter. The line below shows how many bytes go on the air and about how long that takes at the current speed. Messages show '… sending' until the other station confirms, then '✓ delivered'.",
    "Chat",
);

pub const F_FILES: Topic = (
    "Send files",
    "Pick files to send. They're offered first: the other station sees the name, size and time and accepts. You can also drop files on the window.",
    "Files",
);
pub const F_FOLDER: Topic = ("Send a folder", "Pick a folder: it's zipped with everything inside and offered as one file.", "Files");
pub const F_ZIP: Topic =
    ("Zip before sending", "Pack files into one zip at maximum compression. Text and logs often shrink to a few percent; photos don't shrink.", "Files");
pub const F_ESTIMATE: Topic =
    ("Estimate", "Pick any file to see how long it would take at every bandwidth and several signal levels, plus at the current SNR.", "Files");
pub const F_ACCEPT: Topic = ("Accept", "Receive this file. If part of it arrived before (a dropped link), the transfer resumes from there.", "Files");
pub const F_DECLINE: Topic = ("Decline", "Refuse this file; the sender is told.", "Files");
pub const F_CANCEL: Topic = ("Cancel", "Stop this transfer on both sides. A partly received file is kept so it can resume later.", "Files");

pub const T_JOIN: Topic = ("Join here", "Join the named Roundtable on the frequency you're on (the host's frequency). Off during a 1:1 link: disconnect first.", "Roundtables");
pub const T_LEAVE: Topic = ("Leave", "Say goodbye to the table and stop listening to it.", "Roundtables");
pub const T_MEMBER: Topic = (
    "Pass the floor",
    "Click a member to pass them the floor (the talking stick): a courtesy so people take turns. The number is how well they hear the host.",
    "Roundtables",
);
pub const T_INPUT: Topic = (
    "Roundtable message",
    "Goes to everyone at the table as broadcast frames. The line below shows exactly how many frames and how long on the air.",
    "Roundtables",
);

pub const W_AUDIO: Topic = (
    "Radio audio waterfall",
    "What your radio hears, 0 to 3 kHz, newest at the top. The brass bar on top marks where the whistle sweeps. A whistle shows as slanted strokes.",
    "A tour of the screen",
);
pub const W_WIDE: Topic = (
    "Wide receiver waterfall",
    "The band around your dial from the RTL-SDR. Teal bars: digital segments. Brass line: your dial; shaded: your receive passband. 🚂 boxes: whistles the wide search found. Hover to read a frequency, click to tune there.",
    "Wide receiver (RTL-SDR)",
);
pub const W_SPAN: Topic = ("Span", "Zoom the wide waterfall: 25 kHz to 1.2 MHz across.", "Wide receiver (RTL-SDR)");

// ----------------------------------------------------------------- Settings
pub const ST_CALL: Topic = (
    "Callsign",
    "Your call. Mercury sends it in every CQFRAME (your identification) and in connection requests; Whistlestop puts it in CALL and Roundtable frames.",
    "Settings reference",
);
pub const ST_NAMEGRID: Topic =
    ("Name and grid", "Sent to the other station when a link comes up (HELLO), and shown in their link header. Optional.", "Settings reference");
pub const ST_DEMO: Topic = (
    "Demo mode",
    "Simulated radio, band, Mercury and the K7BOT demo station. Nothing is transmitted. Untick to use your radio, Mercury and rigctld.",
    "Quick start: Demo mode",
);
pub const ST_MERCURY: Topic = (
    "Mercury address and ports",
    "Where Mercury listens: TNC control port (data is the next port up) and the broadcast port used by Roundtables. Defaults 8300 and 8100 match Mercury's own.",
    "Setting up your station",
);
pub const ST_RIGCTLD: Topic = (
    "rigctld address",
    "The rig control server that owns your radio's CAT port: Hamlib rigctld or RigHelm's built-in server. Mercury should use the same one (Hamlib model 2, NET rigctl).",
    "Setting up your station",
);
pub const ST_AUDIO: Topic = (
    "Audio devices",
    "The radio's sound card. Whistlestop listens (whistle detector, waterfall) and plays the whistle through it; Mercury uses the same device for data.",
    "Setting up your station",
);
pub const ST_SDR: Topic = (
    "Wide receiver",
    "Use an RTL-SDR dongle for the wide waterfall and the wide whistle search. RX OUT: the antenna signal (the dongle follows your VFO). IF OUT: your radio's first IF (set its frequency and whether it is inverted).",
    "Wide receiver (RTL-SDR)",
);
pub const ST_PPM: Topic = (
    "Crystal ppm",
    "Corrects the dongle's frequency error for the waterfall picture. The wide search doesn't depend on it: hits are fine-locked with the main radio.",
    "Wide receiver (RTL-SDR)",
);
pub const ST_DWELL: Topic = (
    "Dwell per channel",
    "How long a search first listens on each channel. 0.5 s holds a whole whistle pattern (0.4 s); a promising channel gets up to 2 s anyway. Longer is more sensitive to weak callers but makes each pass slower, so callers need longer whistles.",
    "Finding stations",
);
pub const ST_STEP: Topic = (
    "Channel step",
    "Distance between search channels. It is capped so every caller is within the whistle's capture range of some channel (2000 Hz for the wide whistle).",
    "Finding stations",
);
pub const ST_CYCLES: Topic = ("CQ cycles", "How many whistle → CQFRAME → listen cycles before giving up.", "Calling CQ");
pub const ST_LISTEN: Topic = ("Listen after CQ", "How long to wait for answers after each CQ before whistling again.", "Calling CQ");
pub const ST_WATCH: Topic = (
    "Whistle watch",
    "Report whistles heard near the frequency you're parked on (with a Tune to it button). Paused while transmitting and during a link.",
    "Finding stations",
);
pub const ST_RAMP: Topic = (
    "Roundtable speed ramp-up",
    "As host, speed the table up to the fastest mode every member can copy (3 dB margin) and slow it down when someone fades.",
    "Roundtables",
);
pub const ST_E2E: Topic = (
    "Encryption",
    "Off (default), Prefer (encrypt when the other station also wants to) or Require (encrypt or hang up). Fresh keys every link, never saved. Not legal on amateur bands.",
    "Encryption (optional)",
);
pub const ST_DOWNLOAD: Topic =
    ("Download folder", "Where received files are saved. Partial files (for resuming) are kept in a .partial folder inside it.", "Files");
pub const ST_SAVE: Topic =
    ("Save & connect", "Save settings to whistlestop.toml and reconnect to the radio, Mercury and audio with them.", "Settings reference");

// ------------------------------------------------------------ Radio controls
pub const RC_OPEN: Topic = (
    "Radio controls",
    "Opens a window with everything your radio offers through Hamlib: transmit meters (SWR, ALC, power), the antenna tuner, receive and transmit settings, VFO, split and RIT/XIT. It closes again so the main screen stays uncluttered.",
    "Rig controls",
);
pub const RC_RADIO: Topic =
    ("Radio", "The model Hamlib reports and how Whistlestop reaches it (a rigctld it started, or one already running).", "Rig controls");
pub const RC_SWR: Topic = (
    "SWR",
    "Standing-wave ratio while transmitting: how well the antenna matches. 1.0 is perfect, under 1.5 is good; above your limit the line is red. Read only while transmitting.",
    "Antenna tuner and SWR",
);
pub const RC_ALC: Topic = (
    "ALC",
    "The radio's automatic level control. For data it should barely move: if it climbs, lower the audio drive, or the signal distorts.",
    "Antenna tuner and SWR",
);
pub const RC_PO: Topic = ("Power out", "Transmit power as the radio measures it, as a fraction of full scale.", "Antenna tuner and SWR");
pub const RC_ATU: Topic = ("ATU", "Switches the radio's built-in antenna tuner in or out of the antenna line.", "Antenna tuner and SWR");
pub const RC_TUNE_NOW: Topic = (
    "Tune now",
    "Starts the radio's antenna tuner: it switches the tuner in and sends its own short carrier while it finds a match. Listen first: tuning transmits.",
    "Antenna tuner and SWR",
);
pub const RC_AUTOTUNE: Topic = (
    "Auto-tune on high SWR",
    "Off by default. When on, a transmission whose SWR goes above the limit makes Whistlestop start the tuner, but only once the band is quiet: never during a link, CQ or Roundtable traffic, and at most once every 2 minutes per band.",
    "Antenna tuner and SWR",
);
pub const RC_SWR_LIMIT: Topic = (
    "SWR limit",
    "Above this the SWR meter turns red, the log warns, and (if auto-tune is on) the tuner is started. 2.0 : 1 is a common safe limit for solid-state radios.",
    "Antenna tuner and SWR",
);
pub const RC_AF: Topic =
    ("Volume (AF)", "The radio's audio volume to the speaker. It doesn't change what Whistlestop and Mercury hear over USB on most radios.", "Rig controls");
pub const RC_RFG: Topic = ("RF gain", "Receiver gain. Leave it at maximum for data unless a very strong signal overloads the receiver.", "Rig controls");
pub const RC_SQL: Topic =
    ("Squelch", "Mutes weak signals. Keep it fully open (0) for data: squelch cuts off the start of frames and whistles.", "Rig controls");
pub const RC_AGC: Topic = ("AGC", "Automatic gain control speed. Medium or slow suits data; fast can pump on OFDM signals.", "Rig controls");
pub const RC_PREAMP: Topic = ("Preamp", "Extra receive gain for weak signals on quiet higher bands. Usually off on 80 and 40 m.", "Rig controls");
pub const RC_ATT: Topic = ("Attenuator", "Cuts receive signal strength when strong stations overload the receiver.", "Rig controls");
pub const RC_NB: Topic =
    ("Noise blanker", "Removes impulse noise (ignition, power lines). Leave it off for data: it distorts OFDM and the whistle.", "Rig controls");
pub const RC_NR: Topic = (
    "Noise reduction",
    "The radio's DSP noise reduction (on/off and level). Off for data: it smears the signals Mercury and the whistle detector need.",
    "Rig controls",
);
pub const RC_ANF: Topic = ("Auto notch", "Removes steady tones. Off for data: it can notch out OFDM carriers.", "Rig controls");
pub const RC_FILTER: Topic =
    ("Filter width", "The receive filter (passband) width. At least 2.4 kHz for wide bandwidths; 500 Hz settings work with narrow filters.", "Rig controls");
pub const RC_MIC: Topic = ("Mic gain", "Audio drive on transmit for radios that use mic gain for USB audio. Set so ALC barely moves.", "Rig controls");
pub const RC_COMP: Topic = ("Speech compressor", "For voice only. Off for data: it distorts the signal.", "Rig controls");
pub const RC_VOX: Topic = ("VOX", "Voice-operated transmit. Off: Whistlestop and Mercury key the radio through rig control.", "Rig controls");
pub const RC_MON: Topic = ("Monitor", "Hear your own transmitted audio.", "Rig controls");
pub const RC_VFO: Topic = ("VFO A / B", "Which VFO is active.", "Rig controls");
pub const RC_CPY: Topic = ("A = B", "Copy VFO A's frequency and mode to VFO B.", "Rig controls");
pub const RC_XCHG: Topic = ("A ⇄ B", "Swap VFOs A and B.", "Rig controls");
pub const RC_SPLIT: Topic =
    ("Split", "Transmit on VFO B while receiving on A. Keep it off for Whistlestop: both stations must transmit where they listen.", "Rig controls");
pub const RC_RIT: Topic = ("RIT", "Receive offset. Keep it at 0 for data, or you'll listen beside the other station.", "Rig controls");
pub const RC_XIT: Topic = ("XIT", "Transmit offset. Keep it at 0 for data, or you'll transmit beside the other station.", "Rig controls");

// ---------------------------------------------------------- Radio connection
pub const ST_RIGMODE: Topic = (
    "Radio connection",
    "Use a rig control server that's already running (Hamlib rigctld, or RigHelm's server), let Whistlestop run rigctld for your radio, or use Whistlestop's own built-in CAT (no Hamlib). Mercury shares the radio through the same address.",
    "Setting up your station",
);
pub const ST_DETECT: Topic = (
    "Detect radios",
    "Looks at the USB devices your computer already knows about (it never opens a port, which could key a radio) and recognises Icom, Yaesu, Kenwood, QRP Labs and others. Use fills in the model and port.",
    "Setting up your station",
);
pub const ST_MODEL: Topic =
    ("Radio model", "Hamlib's model for your radio. Automatic detects it at start from the USB description.", "Setting up your station");
pub const ST_PORT: Topic = (
    "Serial port",
    "The radio's CAT port. On Yaesu radios with two ports, the first (Enhanced) carries CAT. Automatic uses the detected radio.",
    "Setting up your station",
);
pub const ST_BAUD: Topic =
    ("Speed", "Serial speed, matching the radio's CAT setting. Automatic uses Hamlib's default for the model.", "Setting up your station");
pub const ST_RIGPORT: Topic = (
    "rigctld port",
    "The TCP port Whistlestop's rigctld listens on (127.0.0.1). Point Mercury's Hamlib NET rigctl at the same address.",
    "Setting up your station",
);

// ------------------------------------------------------------ Speakers
pub const LISTEN: Topic = (
    "Listen on the speakers",
    "Plays what the radio hears through the computer's speakers (it stays live, under half a second behind). In Demo mode you also hear your own CQ: the whistle, then the CQ burst. Off by default; volume and speaker are in Settings → Audio.",
    "Listening on your speakers",
);
pub const ST_MON_DEV: Topic = (
    "Speaker",
    "Where Listen plays: the computer's speakers or headphones (System default). Don't pick the radio's own USB audio here, or you'd send the band back into the radio.",
    "Listening on your speakers",
);
pub const ST_MON_VOL: Topic = ("Listen volume", "How loud Listen plays. Changes take effect at once.", "Listening on your speakers");
pub const ST_DEMO_REAL: Topic = (
    "Demo on real Mercury",
    "When Mercury is installed, Demo mode runs two real Mercury modems on a simulated band: one for you, one for K7BOT. Everything you see and hear is then real Mercury. Off: everything simulated, including the modem.",
    "Quick start: Demo mode",
);
pub const ST_MERCURY_BIN: Topic = (
    "Mercury engine",
    "Where the Mercury engine program is. Empty looks in the usual install places (~/.local/bin, /opt/homebrew/bin, /usr/local/bin, Program Files\\Mercury on Windows) and on PATH.",
    "Quick start: Demo mode",
);

// ------------------------------------------------------------------- Depot
pub const DEPOT_TAB: Topic = (
    "Depot",
    "Whistlestop's mailbox: Arrivals (mail for you), Departures (mail you wrote, waiting for its station), In Transit (mail you're carrying for others) and Delivered.",
    "The Depot",
);
pub const D_TO: Topic = ("To", "The station this mail is for. It's delivered the next time you're linked with them.", "The Depot");
pub const D_VIA: Topic = (
    "Via (relay)",
    "Optional: hand this mail to another station, which carries it the rest of the way (if they've turned on carrying mail). Leave empty to wait for the station itself.",
    "The Depot",
);
pub const D_TEXT: Topic = ("Message", "Plain text. Short is kind on HF: every byte is airtime.", "The Depot");
pub const D_POST: Topic = ("Post", "Puts the message in Departures. If you're linked with its station (or the relay) right now, it goes at once.", "The Depot");
pub const D_ARRIVALS: Topic =
    ("Arrivals", "Mail for you: left while you were away, handed over by a station that held it, or relayed. NEW marks unread mail.", "The Depot");
pub const D_DEPARTURES: Topic = ("Departures", "Mail you wrote, waiting for its station or relay. It moves to Delivered when they confirm.", "The Depot");
pub const D_TRANSIT: Topic = (
    "In Transit",
    "Store and forward: mail other stations handed you for someone else. It goes on when you're next linked with that station (or automatically when away, if turned on).",
    "The Depot",
);
pub const D_DELIVERED: Topic = ("Delivered", "Mail the next station confirmed: the addressee, or the relay that took it.", "The Depot");
pub const D_DELETE: Topic = ("Delete", "Removes this message from your Depot for good.", "The Depot");
pub const D_REPLY: Topic = ("Reply", "Starts a new message to the sender.", "The Depot");
pub const D_READ: Topic = ("Mark read", "Clears the NEW mark.", "The Depot");
pub const D_CONNECT: Topic = ("Connect", "Calls this station now; waiting mail goes across as soon as you're linked.", "The Depot");
pub const AWAY: Topic = (
    "Away",
    "You're away from the radio: stations that connect get your away message, what they type is saved to your Arrivals, and quiet links hang up so others can get through. Automatic delivery and collection (if turned on in Settings → Automation) only happen while away.",
    "Away",
);
pub const ALERT_OURS: Topic =
    ("Mail waiting for them", "You heard a station you hold Depot mail for. Deliver now connects and hands it over.", "Automatic mail handling");
pub const ALERT_THEIRS: Topic =
    ("Mail waiting for you", "A station says it holds mail for you. Collect connects to it; the mail arrives in your Arrivals.", "Automatic mail handling");
pub const ST_AWAY_MSG: Topic = ("Away message", "What stations see when they connect while you're away. Your callsign is added in front.", "Away");
pub const ST_AWAY_IDLE: Topic =
    ("Hang up after", "While away, a link with nothing happening for this long is ended, so the next station can get through.", "Away");
pub const ST_BEACON: Topic = (
    "Station ID beacon",
    "Every N minutes, when the band is quiet, send one short frame with your callsign, grid, whether you're away, and the callsigns you hold mail for. No whistle. Only stations on your frequency (in the calling mode) hear it. Off by default.",
    "Station ID beacon",
);
pub const ST_BEACON_NOW: Topic = ("Send Station ID now", "Sends one Station ID immediately.", "Station ID beacon");
pub const ST_NOTIFY: Topic = (
    "Tell stations about their mail",
    "When you hear a station you hold mail for, send it a short 'mail waiting, connect to me' notice (at most every 15 minutes per station). Off by default.",
    "Automatic mail handling",
);
pub const ST_AUTO_DELIVER: Topic = (
    "Deliver automatically while away",
    "While away: when you hear a station you hold mail for, connect to it, hand the mail over, and hang up. At most every 15 minutes per station. Off by default.",
    "Automatic mail handling",
);
pub const ST_AUTO_COLLECT: Topic = (
    "Collect automatically while away",
    "While away: when a station says it holds mail for you, connect and collect it. Off by default.",
    "Automatic mail handling",
);
pub const ST_RELAY: Topic = (
    "Carry mail for other stations",
    "Store and forward: accept Depot mail addressed to someone else and carry it to them (In Transit). Mail is relayed at most 3 times, and never twice. Off by default.",
    "Store and forward (relaying)",
);
pub const ST_SEGMENTS: Topic = (
    "Automatic only in the automatic-control segments",
    "Automatic transmissions (beacon, notices, automatic connects) happen only when your radio is inside a configured digital segment: by default the US automatic-control segments of 47 CFR 97.221(b), where unattended data stations may operate. On by default; turn off only where your rules allow it.",
    "Legal and good practice",
);
pub const ST_CAT: Topic = (
    "Direct CAT",
    "Whistlestop talks to the radio itself, in its own protocol (Icom CI-V, Kenwood, Elecraft, Yaesu): nothing else to install. It also shares the radio with Mercury through a built-in rigctld server.",
    "Direct CAT (built in)",
);
pub const ST_CAT_FAMILY: Topic = (
    "Protocol",
    "Which control language the radio speaks. Automatic works it out from the USB description, then by asking the radio (ID; or a CI-V broadcast). Xiegu radios speak Icom CI-V; QRP Labs, truSDX and Flex speak Kenwood.",
    "Direct CAT (built in)",
);
pub const ST_CAT_PORT: Topic = (
    "CAT port",
    "The radio's serial port (on Yaesu radios with two, the first). Automatic uses the first radio found on USB. A host:port address reaches a radio through a serial-over-network bridge or a Flex CAT port.",
    "Direct CAT (built in)",
);
pub const ST_CAT_BAUD: Topic =
    ("Speed", "The radio's CAT speed (its menu setting). Automatic tries the usual speeds for the protocol until the radio answers.", "Direct CAT (built in)");
pub const ST_CAT_CIV: Topic = (
    "CI-V address",
    "Icom radios have an address on the CI-V bus (IC-7300 94h, IC-705 A4h...). Ask the radio finds it with a broadcast, even if you've changed it.",
    "Direct CAT (built in)",
);
pub const ST_CAT_POWER: Topic = (
    "Full power",
    "The radio's maximum output, so power reads and sets correctly in watts (Kenwood, Elecraft and Yaesu set power in watts).",
    "Direct CAT (built in)",
);
pub const ST_CAT_SHARE: Topic = (
    "Share with Mercury",
    "Only one program can open the radio's serial port. Whistlestop opens it and serves the rigctld protocol on this port, so Mercury (Hamlib model 2, NET rigctl), loggers and other programs can share the radio. On by default (4532).",
    "Direct CAT (built in)",
);
pub const L_QSY: Topic = (
    "Move this link (QSY)",
    "Ask the station you're linked with to move the link to another frequency: for a long transfer, or off a busy channel. If they agree (by their own rules), both stations retune and the link carries on there; both go back when the link ends.",
    "QSY (moving a link)",
);
pub const ST_QSY_ACCEPT: Topic = (
    "Accept QSY requests",
    "Let a linked station move the link to another frequency, within the rules below; useful while you're away. Both stations return here when the link ends. Off by default.",
    "QSY (moving a link)",
);
pub const ST_QSY_SAME: Topic =
    ("Only within the band I'm on", "Agree only to frequencies on the band you're already on, where you know your antenna works.", "QSY (moving a link)");
pub const ST_QSY_BANDS: Topic =
    ("QSY bands", "With 'only within this band' off: the bands a QSY may go to. Pick only bands your antenna and tuner can handle.", "QSY (moving a link)");
pub const ST_QSY_MAX: Topic = (
    "Largest move",
    "How far from where you were a QSY may go, in kHz (0 = anywhere in the allowed bands). Small moves keep the antenna's SWR close to where it was.",
    "QSY (moving a link)",
);
pub const ST_QSY_SEG: Topic = (
    "Only in the digital segments",
    "Agree only to frequencies inside your configured digital segments (by default the US automatic-control segments), so an unattended station stays where data is allowed. On by default.",
    "QSY (moving a link)",
);
pub const ST_QSY_SWR: Topic = (
    "Go home on high SWR",
    "If the SWR is high on the new frequency (still high after one tuner cycle, when auto-tune is on), end the link so both stations go back. On by default.",
    "QSY (moving a link)",
);
pub const F_LIB_BROWSE: Topic = ("Browse their library", "Ask the linked station what files its library holds; folders open with a click.", "File library");
pub const F_LIB_DOWNLOAD: Topic = (
    "Download",
    "Ask for this file. It's sent like any file (with a time estimate and resume), accepted automatically because you asked, and saved in your Download folder.",
    "File library",
);
pub const ST_LIB_SHARE: Topic = (
    "Share a file library",
    "Let linked stations see and download the files in one folder, even while you're away. Nothing outside the folder (and no hidden files) can be reached. Off by default.",
    "File library",
);
pub const ST_LIB_DIR: Topic = ("Library folder", "The folder others may browse. Put only files you're happy to share in it.", "File library");
pub const ST_LIB_AWAY: Topic = ("Only while I'm away", "Open the library only while you're Away.", "File library");
pub const ST_LIB_MAX: Topic = (
    "Largest file",
    "Files bigger than this are listed but refused: at a few hundred bytes a second, big files tie up the frequency for a long time.",
    "File library",
);
pub const ST_SCHED_ON: Topic = (
    "Follow the schedule",
    "At each slot's start, tune the radio to its frequency (when not linked, calling or searching), and send Station ID there if the slot says so. Off by default.",
    "Schedule",
);
pub const ST_SCHED_AWAY: Topic =
    ("Only while I'm away", "Follow the schedule only while you're Away, so it never moves the radio under you. On by default.", "Schedule");
pub const ST_SCHED_SLOT: Topic = (
    "Time slot",
    "From–to (HH:MM, UTC; may cross midnight), which days, the dial frequency, and whether to send Station ID while it's on. The first slot listed that is on wins.",
    "Schedule",
);
pub const ST_SCHED_ADD: Topic = ("Add a time slot", "Adds a slot you can then edit.", "Schedule");
pub const REMOTE_BTN: Topic = (
    "Remote",
    "Operate the Whistlestop on the computer wired to your radio from this one, anywhere on your home network, or share this station so another computer can. Lit while you're operating remotely; 'Shared' while this station is shared.",
    "Remote operation",
);
pub const RM_ADDR: Topic = (
    "Radio computer",
    "The address of the computer running Whistlestop with the radio: its LAN address (shown in its Remote window) or its name (e.g. shack.local). Add :port if it isn't 7373.",
    "Remote operation",
);
pub const RM_PASSWORD: Topic = (
    "Remote password",
    "The password set on the radio computer under 'Let other computers operate this one'. It's never sent: the radio computer sends a challenge and this computer proves it knows the password.",
    "Remote operation",
);
pub const RM_CONNECT: Topic = (
    "Connect to the radio computer",
    "Operate the radio computer from here. Everything (search, CQ, chat, files, the Depot, Radio Controls, Settings) acts on it; Listen plays its radio's audio here; files you receive are copied here.",
    "Remote operation",
);
pub const RM_DISCONNECT: Topic =
    ("Disconnect", "Stop operating the radio computer (it carries on as it was) and go back to this computer's own station.", "Remote operation");
pub const RM_SERVE: Topic = (
    "Share this station",
    "Let another Whistlestop on your home network operate this one. Only private (LAN) addresses can connect, and only with the password. Off by default.",
    "Remote operation",
);
pub const RM_SERVE_PW: Topic = (
    "Sharing password",
    "At least 4 characters. Anyone on your LAN who knows it can operate your station (and transmit): pick one others won't guess.",
    "Remote operation",
);
