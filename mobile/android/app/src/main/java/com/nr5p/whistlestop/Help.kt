package com.nr5p.whistlestop

/** Help-mode explanations: the same words as the iPhone app (mobile/ios/Whistlestop/Help.swift). */
enum class HelpTopic(val title: String, val body: String) {
    helpMode(
        "Help mode",
        "When on, a ? appears on each part of the screen; tap it for an explanation like this one. Turn it off once you know your way around.",
    ),
    status(
        "Status",
        "Modem: Mercury is running and answering. Radio: Whistlestop can read and set the radio (CAT or the demo radio). Audio: the radio's sound card is open. TX lights while transmitting.",
    ),
    dial(
        "Dial frequency",
        "The radio's dial frequency (USB) and mode, read from the radio about once a second, including changes you make on the radio itself. Type a frequency in kHz and tap Tune to move it.",
    ),
    waterfall(
        "Waterfall",
        "The radio's audio passband, newest at the top. A whistle shows as a zig-zag between 1000 and 2000 Hz; Mercury's data as a solid block.",
    ),
    search(
        "Search",
        "Steps the radio through the digital segment(s): half a second per channel (up to 2 s where something looks whistle-like), 2 kHz apart. It stops on a whistle, retunes onto it to within a few Hz and waits for the caller's CQ. Current band searches the band you're on; All bands goes band after band.",
    ),
    whistle(
        "Whistle",
        "The last whistle heard: where, and how strong (the detector's score). Whistles are 1000→2000→1000 Hz chirps that a scanning station can find in half a second.",
    ),
    heard(
        "Heard",
        "Stations whose CQ (or beacon) Mercury decoded. Tap Answer to connect.",
    ),
    cq(
        "Call CQ",
        "Starts the cycle: key up and whistle so searching stations find you, then send Mercury's CQ with your callsign, then listen for answers. Repeats until someone answers or you tap Stop.",
    ),
    connect(
        "Connect",
        "Call a station directly by callsign on the current frequency, without a CQ.",
    ),
    link(
        "Link",
        "Who you're linked with, for how long, and the speed Mercury is managing on this path right now.",
    ),
    chat(
        "Chat",
        "Keyboard-to-keyboard over the link. A tick means the other station's Mercury confirmed it.",
    ),
    qsy(
        "Move the link (QSY)",
        "Ask the linked station to move to another frequency with you, to clear the calling frequency. Both radios move, and both go back home when the link ends. They can refuse by their own settings (band, distance from home, SWR).",
    ),
    away(
        "Away",
        "You're away from the radio: stations that connect get your away message, what they type is saved to your Arrivals, and quiet links hang up so others can get through.",
    ),
    depot(
        "Depot",
        "Whistlestop's mailbox: Arrivals (mail for you), Departures (mail you wrote, waiting for its station), In Transit (mail you're carrying for others) and Delivered.",
    ),
    depotPost(
        "Write mail",
        "To: the station it's for; it goes the next time you're linked with them. Via: optionally hand it to a relay station that carries it the rest of the way. Short is kind on HF: every byte is airtime.",
    ),
    depotLists(
        "Mail lists",
        "Arrivals: mail for you, NEW until opened. Departures: waiting for its station. In Transit: store and forward for others. Delivered: confirmed by the next station.",
    ),
    log(
        "Log",
        "What the engine did and heard, newest first, in UTC.",
    ),
    station(
        "Station",
        "Your callsign (sent in every CQ and link), name and grid square (sent when you link).",
    ),
    modem(
        "Modem",
        "Where Mercury's TNC listens. The app runs Mercury itself; change these only to use a Mercury elsewhere on your network.",
    ),
    radio(
        "Radio",
        "How Whistlestop reaches the radio's CAT. On a phone that's a TCP bridge (host:port), e.g. a rigctld or a serial-to-network adapter on your network.",
    ),
    demo(
        "Demo station",
        "Try Whistlestop without a radio: a simulated band with a demo station (K7BOT) that whistles, calls CQ and chats back.",
    ),
    autoAnswer(
        "Answer automatically",
        "When a search locks onto a whistle and the caller's CQ is decoded, connect at once instead of waiting for you to tap Answer.",
    ),
}
