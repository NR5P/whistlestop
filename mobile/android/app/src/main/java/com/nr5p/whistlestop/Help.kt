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
        "Steps the radio through the digital segment(s): half a second per channel (up to 2 s where something looks whistle-like), 2 kHz apart. It stops on a whistle, retunes onto it to within a few Hz and waits for the caller's CQ. Current band searches the band you're on; All bands goes band after band. Off while you're at a Roundtable: leave it first.",
    ),
    whistle(
        "Whistle",
        "The last whistle heard: where, and how strong (the detector's score). Whistles are 1000→2000→1000 Hz chirps that a scanning station can find in half a second.",
    ),
    heard(
        "Heard",
        "Stations whose CQ (or beacon) Mercury decoded. Tap Answer to connect (not while you're at a Roundtable: leave it first).",
    ),
    cq(
        "Call CQ",
        "Starts the cycle: key up and whistle so searching stations find you, then send Mercury's CQ with your callsign, then listen for answers. Repeats until someone answers or you tap Stop. While you're at a Roundtable, only a CQ for your own table is allowed.",
    ),
    connect(
        "Connect",
        "Call a station directly by callsign on the current frequency, without a CQ. Off while you're at a Roundtable: leave it first.",
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
        "How Whistlestop reaches the radio's CAT. USB cable: plug the radio into the phone; Whistlestop finds it (Silicon Labs, FTDI, CH340, Prolific or USB CDC, as in Icom, Yaesu, Kenwood, Elecraft, Xiegu and QRP Labs radios), works out the protocol and speed, and Mercury keys the radio through it. Android asks once to allow the USB connection. Serial bridge: a serial-to-network adapter speaking the radio's own CAT at host:port. rigctld: a Hamlib rigctld on your network. None: no CAT; key with VOX. Protocol: leave on Automatic unless the radio isn't found. Plug the radio (and its USB sound card) in before opening the app; the radio's USB audio is used for the modem.",
    ),
    demo(
        "Demo station",
        "Try Whistlestop without a radio: a simulated band with a demo station (K7BOT) that whistles, calls CQ and chats back.",
    ),
    autoAnswer(
        "Answer automatically",
        "When a search locks onto a whistle and the caller's CQ is decoded, connect at once instead of waiting for you to tap Answer. A CQ for a Roundtable is answered by joining the table.",
    ),
    encryption(
        "Encryption",
        "Off (the default), Prefer (encrypt when the other station also wants to) or Require (encrypt or hang up, never plain text). Protects 1:1 chat on the link, with fresh keys every link that are never saved. Roundtables are never encrypted. NOT legal on amateur bands in the US (47 CFR 97.113(a)(4)) and most other countries.",
    ),
    linkE2e(
        "Encryption on this link",
        "Encrypted: chat on this link is sealed with keys made for this link only. Read the 6-digit code to the other operator by voice; if their screen shows the same code, no one is in the middle. If it differs, disconnect. Setting up: keys are being exchanged, and chat waits. Not encrypted: you prefer encryption, but this link is plain text.",
    ),
    cqKind(
        "What the CQ is for",
        "Chat (1:1): someone answers and you have a private link for chat. Roundtable: a group conversation on your frequency; give it a name, and others join.",
    ),
    roundtable(
        "Roundtable",
        "A group conversation on one frequency, wherever its host called it. The host sets the speed: it ramps up to the fastest mode every member can copy, and drops back if someone fades. To host one, use Call CQ → Roundtable on Operate. Roundtables are never encrypted.",
    ),
    tableJoin(
        "Join a Roundtable",
        "Join the named Roundtable on the frequency you're on (the host's frequency); Join under Heard does this for you. Off during a 1:1 link: disconnect first. Stations at a table with a different name ignore each other.",
    ),
    tableJoinWait(
        "Waiting to join",
        "You asked to join while a link was still closing. Mercury can't switch to Roundtable listening until its disconnect is done, so the join is retried every 2 s for up to 60 s. Cancel to stop waiting.",
    ),
    tableMembers(
        "At the table",
        "Who's at the table. Tap a member to pass them the floor (the talking stick): a courtesy so people take turns, not a lock. The number is how well they hear the host. Leave says goodbye and stops listening.",
    ),
    tableSay(
        "Roundtable message",
        "Goes to everyone at the table as broadcast frames. They aren't acknowledged, so a station missing part of a message asks for just that part again.",
    ),
}
