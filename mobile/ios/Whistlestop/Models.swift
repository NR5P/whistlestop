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
    /// A call going out (Connect, or an automatic one), until it links.
    var calling: String? = nil
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
    var mailAlerts: [MailAlert]? = nil
    var table: TableState? = nil
    /// A Roundtable we'll join as soon as Mercury is free.
    var tablePending: String? = nil

    var freqHz: Double { rig?.freqHz ?? 0 }
}

/// Depot mail news (Rust `engine::MailAlert`).
struct MailAlert: Decodable {
    var call: String
    /// We hold mail for them (true), or they hold mail for us (false).
    var ours: Bool
    var count: Int
    /// Seconds ago.
    @Num var at: Double
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
    var e2e: E2e? = nil
}

/// End-to-end encryption on the link (Rust `engine::E2eView`): `"Off"`,
/// `"Negotiating"`, `{"Encrypted":{"sas":…}}` or `{"Plain":{"why":…}}`.
enum E2e: Decodable, Equatable {
    case off, negotiating
    case encrypted(sas: String)
    case plain(why: String)

    private struct Key: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }
    private struct Sas: Decodable { var sas: String }
    private struct Why: Decodable { var why: String }

    init(from decoder: Decoder) throws {
        if let s = try? decoder.singleValueContainer().decode(String.self) {
            self = s == "Negotiating" ? .negotiating : .off
            return
        }
        let c = try decoder.container(keyedBy: Key.self)
        if let v = try? c.decode(Sas.self, forKey: Key(stringValue: "Encrypted")) {
            self = .encrypted(sas: v.sas)
        } else if let v = try? c.decode(Why.self, forKey: Key(stringValue: "Plain")) {
            self = .plain(why: v.why)
        } else {
            self = .off
        }
    }
}

/// The Roundtable we're at (Rust `engine::TableState`).
struct TableState: Decodable {
    var name: String
    var id: Int
    var host: Bool
    /// Mercury's broadcast mode, e.g. "DATAC3".
    var mode: String
    /// Callsign → how well they hear the host.
    var members: [String: Member] = [:]
    /// Who holds the floor (the talking stick).
    var floor: String? = nil
    var lines: [TableLine] = []
    /// Air time still to go for frames being sent.
    @Num var sendingEtaS: Double = 0
}

struct Member: Decodable {
    var snrOfHost: Int? = nil
}

/// `[unix seconds, from, text]`; `from` is empty for the table's own notices.
struct TableLine: Decodable {
    var at: UInt64
    var from: String
    var text: String
    init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        at = try c.decode(UInt64.self)
        from = try c.decode(String.self)
        text = try c.decode(String.self)
    }
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
    /// The link ended before the other station confirmed it.
    var unconfirmed: Bool? = nil
    var mine: Bool { who == "Me" }
    var system: Bool { who == "System" }
}

struct Heard: Decodable, Identifiable {
    var at: UInt64
    var call: String
    @Num var dialHz: Double
    var what: String
    var table: String? = nil
    var id: String { "\(call)-\(at)" }
}

struct Whistle: Decodable {
    /// Seconds ago.
    @Num var at: Double
    @Num var dialHz: Double
    @Num var offsetHz: Double
    @Num var score: Double
}

struct Search: Decodable {
    var active: Bool
    var scope: String
    var index: Int
    var total: Int
    @Num var channelHz: Double
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
    /// A kHz number as typed: "14103.5" or, with a comma decimal, "14103,5".
    static func kHz(_ s: String) -> Double? {
        Double(s.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: ",", with: "."))
    }
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

/// A number from the engine that may arrive as `null`: serde writes a NaN or
/// an infinity that way, and one such value in a plain `Double` field would
/// fail the whole state, freezing the app on the last good one. Read as 0.
@propertyWrapper
struct Num: Decodable {
    var wrappedValue: Double
    init(wrappedValue: Double) { self.wrappedValue = wrappedValue }
    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        wrappedValue = c.decodeNil() ? 0 : ((try? c.decode(Double.self)) ?? 0)
    }
}

extension KeyedDecodingContainer {
    /// A `@Num` field that's missing altogether is 0 too.
    func decode(_ type: Num.Type, forKey key: Key) throws -> Num {
        try decodeIfPresent(type, forKey: key) ?? Num(wrappedValue: 0)
    }
}
