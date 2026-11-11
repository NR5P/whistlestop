import Foundation

/// The engine's state (Rust `engine::State`), decoded leniently: every field
/// is optional, and fields this app doesn't show are simply not listed.
struct EngineState: Decodable {
    var running: Bool? = nil
    var demo: Bool? = nil
    var mycall: String? = nil
    var rigOk: Bool? = nil
    var mercuryOk: Bool? = nil
    var audioOk: Bool? = nil
    var rig: Rig? = nil
    var tx: Bool? = nil
    var snrDb: Double? = nil
    var bandwidth: String? = nil
    var link: Link? = nil
    var incomingCall: Bool? = nil
    var chat: [ChatLine]? = nil
    var heard: [Heard]? = nil
    var whistle: Whistle? = nil
    var search: Search? = nil
    var cq: Cq? = nil
    var log: [LogLine]? = nil
    var estBps: Double? = nil
    var modemVersion: String? = nil
    var rigDesc: String? = nil
    var depot: Depot? = nil
    var away: Bool? = nil
    var qsyNote: String? = nil
    var mailAlerts: [String]? = nil

    var freqHz: Double { rig?.freqHz ?? 0 }
}

struct Rig: Decodable {
    var freqHz: Double? = nil
    var mode: String? = nil
    var ptt: Bool? = nil
    var strengthDb: Double? = nil
}

struct Link: Decodable {
    var peer: String
    var bw: Int? = nil
    /// Seconds since the link came up.
    var since: Double? = nil
    var peerName: String? = nil
    var peerGrid: String? = nil
    var peerApp: String? = nil
    var mode: String? = nil
    var bps: Double? = nil
}

/// Listed with `ForEach(Array(chat.enumerated()), id: \.offset)`: lines are
/// only ever appended, so the position is a stable identity.
struct ChatLine: Decodable {
    var at: UInt64
    var who: String
    var from: String? = nil
    var text: String
    var id: UInt64? = nil
    var delivered: Bool? = nil
    var mine: Bool { who == "Me" }
    var system: Bool { who == "System" }
}

struct Heard: Decodable, Identifiable {
    var at: UInt64
    var call: String
    var dialHz: Double
    var what: String
    var table: String? = nil
    var id: String { "\(call)-\(at)" }
}

struct Whistle: Decodable {
    /// Seconds ago.
    var at: Double
    var dialHz: Double
    var offsetHz: Double
    var score: Double
}

struct Search: Decodable {
    var active: Bool
    var scope: String
    var index: Int
    var total: Int
    var channelHz: Double
    var note: String
}

struct Cq: Decodable {
    var active: Bool
    var cycle: Int
    var cycles: Int
    var phase: String
    var table: String? = nil
}

/// `[unix seconds, text]`.
struct LogLine: Decodable, Identifiable {
    var at: UInt64
    var text: String
    var id: String { "\(at)-\(text)" }
    init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        at = try c.decode(UInt64.self)
        text = try c.decode(String.self)
    }
}

struct Depot: Decodable {
    var arrivals: [Mail] = []
    var departures: [Mail] = []
    var transit: [Mail] = []
    var delivered: [Mail] = []
}

struct Mail: Decodable, Identifiable {
    var id: UInt64
    var from: String
    var to: String
    var created: UInt64
    var text: String
    var read: Bool? = nil
    var deliveredAt: UInt64? = nil
    var via: String? = nil
}

// MARK: formatting

enum Fmt {
    /// 14.103 150 MHz → "14.103.15"
    static func dial(_ hz: Double) -> String {
        guard hz > 0 else { return "—" }
        let khz = hz / 1000
        return String(format: "%.2f", khz)
    }
    static func mhz(_ hz: Double) -> String {
        guard hz > 0 else { return "—" }
        return String(format: "%.3f", hz / 1_000_000)
    }
    static func utc(_ t: UInt64) -> String {
        let s = t % 86_400
        return String(format: "%02d:%02dZ", s / 3600, (s % 3600) / 60)
    }
    static func ago(_ secs: Double) -> String {
        let s = Int(secs)
        if s < 60 { return "\(s) s" }
        if s < 3600 { return "\(s / 60) min" }
        return "\(s / 3600) h \(s % 3600 / 60) min"
    }
}
