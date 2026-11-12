import SwiftUI

/// Help mode: a phone has no hover, so with Help on every card and control
/// group shows a ? that explains it (the desktop's hover help, same words).
enum HelpTopic: String, CaseIterable, Identifiable {
    case helpMode, status, dial, waterfall, search, whistle, heard, cq, connect,
         link, chat, qsy, away, depot, depotPost, depotLists, log,
         station, modem, radio, demo, autoAnswer,
         encryption, linkE2e, cqKind, roundtable, tableJoin, tableJoinWait, tableMembers, tableSay

    var id: String { rawValue }

    var title: String { text.0 }
    var body: String { text.1 }

    private var text: (String, String) {
        switch self {
        case .helpMode:
            ("Help mode", "When on, a ? appears on each part of the screen; tap it for an explanation like this one. Turn it off once you know your way around.")
        case .status:
            ("Status", "Modem: Mercury is running and answering. Radio: Whistlestop can read and set the radio (CAT or the demo radio). Audio: the radio's sound card is open. TX lights while transmitting.")
        case .dial:
            ("Dial frequency", "The radio's dial frequency (USB) and mode, read from the radio about once a second, including changes you make on the radio itself. Type a frequency in kHz and tap Tune to move it.")
        case .waterfall:
            ("Waterfall", "The radio's audio passband, newest at the top. A whistle shows as a zig-zag between 1000 and 2000 Hz; Mercury's data as a solid block.")
        case .search:
            ("Search", "Steps the radio through the digital segment(s): half a second per channel (up to 2 s where something looks whistle-like), 2 kHz apart. It stops on a whistle, retunes onto it to within a few Hz and waits for the caller's CQ. Current band searches the band you're on; All bands goes band after band.")
        case .whistle:
            ("Whistle", "The last whistle heard: where, and how strong (the detector's score). Whistles are 1000→2000→1000 Hz chirps that a scanning station can find in half a second.")
        case .heard:
            ("Heard", "Stations whose CQ (or beacon) Mercury decoded. Tap Answer to connect.")
        case .cq:
            ("Call CQ", "Starts the cycle: key up and whistle so searching stations find you, then send Mercury's CQ with your callsign, then listen for answers. Repeats until someone answers or you tap Stop.")
        case .connect:
            ("Connect", "Call a station directly by callsign on the current frequency, without a CQ.")
        case .link:
            ("Link", "Who you're linked with, for how long, and the speed Mercury is managing on this path right now.")
        case .chat:
            ("Chat", "Keyboard-to-keyboard over the link. A tick means the other station's Mercury confirmed it.")
        case .qsy:
            ("Move the link (QSY)", "Ask the linked station to move to another frequency with you, to clear the calling frequency. Both radios move, and both go back home when the link ends. They can refuse by their own settings (band, distance from home, SWR).")
        case .away:
            ("Away", "You're away from the radio: stations that connect get your away message, what they type is saved to your Arrivals, and quiet links hang up so others can get through.")
        case .depot:
            ("Depot", "Whistlestop's mailbox: Arrivals (mail for you), Departures (mail you wrote, waiting for its station), In Transit (mail you're carrying for others) and Delivered.")
        case .depotPost:
            ("Write mail", "To: the station it's for; it goes the next time you're linked with them. Via: optionally hand it to a relay station that carries it the rest of the way. Short is kind on HF: every byte is airtime.")
        case .depotLists:
            ("Mail lists", "Arrivals: mail for you, NEW until opened. Departures: waiting for its station. In Transit: store and forward for others. Delivered: confirmed by the next station.")
        case .log:
            ("Log", "What the engine did and heard, newest first, in UTC.")
        case .station:
            ("Station", "Your callsign (sent in every CQ and link), name and grid square (sent when you link).")
        case .modem:
            ("Modem", "Where Mercury's TNC listens. The app runs Mercury itself; change these only to use a Mercury elsewhere on your network.")
        case .radio:
            ("Radio", "How Whistlestop reaches the radio's CAT. On an iPhone that's a serial-to-network adapter at host:port that speaks the radio's own CAT (iPhones can't drive USB serial chips).")
        case .demo:
            ("Demo station", "Try Whistlestop without a radio: a simulated band with a demo station (K7BOT) that whistles, calls CQ and chats back.")
        case .encryption:
            ("Encryption", "Off (the default), Prefer (encrypt when the other station also wants to) or Require (encrypt or hang up, never plain text). Protects 1:1 chat on the link, with fresh keys every link that are never saved. Roundtables are never encrypted. NOT legal on amateur bands in the US (47 CFR 97.113(a)(4)) and most other countries.")
        case .linkE2e:
            ("Encryption on this link", "Encrypted: chat on this link is sealed with keys made for this link only. Read the 6-digit code to the other operator by voice; if their screen shows the same code, no one is in the middle. If it differs, disconnect. Setting up: keys are being exchanged, and chat waits. Not encrypted: you prefer encryption, but this link is plain text.")
        case .cqKind:
            ("What the CQ is for", "Chat (1:1): someone answers and you have a private link for chat. Roundtable: a group conversation on your frequency; give it a name, and others join.")
        case .roundtable:
            ("Roundtable", "A group conversation on one frequency, wherever its host called it. The host sets the speed: it ramps up to the fastest mode every member can copy, and drops back if someone fades. To host one, use Call CQ → Roundtable on Operate. Roundtables are never encrypted.")
        case .tableJoin:
            ("Join a Roundtable", "Join the named Roundtable on the frequency you're on (the host's frequency); Join under Heard does this for you. Off during a 1:1 link: disconnect first. Stations at a table with a different name ignore each other.")
        case .tableJoinWait:
            ("Waiting to join", "You asked to join while a link was still closing. Mercury can't switch to Roundtable listening until its disconnect is done, so the join is retried every 2 s for up to 60 s. Cancel to stop waiting.")
        case .tableMembers:
            ("At the table", "Who's at the table. Tap a member to pass them the floor (the talking stick): a courtesy so people take turns, not a lock. The number is how well they hear the host. Leave says goodbye and stops listening.")
        case .tableSay:
            ("Roundtable message", "Goes to everyone at the table as broadcast frames. They aren't acknowledged, so a station missing part of a message asks for just that part again.")
        case .autoAnswer:
            ("Answer automatically", "When a search locks onto a whistle and the caller's CQ is decoded, connect at once instead of waiting for you to tap Answer.")
        }
    }
}

struct HelpButton: View {
    var topic: HelpTopic
    @AppStorage("helpMode") private var helpMode = false
    @State private var showing = false

    var body: some View {
        if helpMode {
            Button { showing = true } label: {
                Image(systemName: "questionmark.circle.fill")
                    .foregroundStyle(Theme.brass)
                    .font(.body)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Help: \(topic.title)")
            .popover(isPresented: $showing) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(topic.title).font(.headline)
                    Text(topic.body).font(.callout).fixedSize(horizontal: false, vertical: true)
                }
                .padding()
                .frame(idealWidth: 320)
                .presentationCompactAdaptation(.popover)
            }
        }
    }
}
