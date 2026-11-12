import Foundation
import SwiftUI
import WhistlestopCore

/// The Rust engine, as SwiftUI sees it. It polls the engine's change
/// counter ten times a second and decodes the state only when it moved.
@MainActor
final class Core: ObservableObject {
    @Published private(set) var state = EngineState()
    @Published private(set) var started = false
    @Published private(set) var startError: String?
    /// Settings as the engine keeps them (every key, so saving never drops
    /// one this app doesn't show).
    @Published var config: [String: Any] = [:]
    /// Which half of the Chat tab shows: the 1:1 link or the Roundtable.
    @Published var chatPane: ChatPane = .link
    /// Waterfall rows, newest last.
    @Published private(set) var rows: [[Float]] = []

    private var seen: UInt64 = .max
    private var timer: Timer?
    private let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    static let maxRows = 160

    func start() {
        guard !started else { return }
        var dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        // UI tests: a fresh demo station in a folder of its own.
        if ProcessInfo.processInfo.environment["WS_UITEST"] != nil {
            dir = FileManager.default.temporaryDirectory.appendingPathComponent("uitest")
            try? FileManager.default.removeItem(at: dir)
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let toml = "callsign = \"N0PHN\"\ngrid = \"DM79\"\ndemo = true\ndemo_real_mercury = false\n"
            try? toml.write(to: dir.appendingPathComponent("whistlestop.toml"), atomically: true, encoding: .utf8)
        }
        if ws_start(dir.path) != 0 {
            startError = "The engine couldn't start."
            return
        }
        started = true
        loadConfig()
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
    }

    private func take(_ p: UnsafeMutablePointer<CChar>?) -> String {
        guard let p else { return "" }
        defer { ws_free(p) }
        return String(cString: p)
    }

    private func poll() {
        let n = ws_changes()
        let newRows = take(ws_rows())
        if let data = newRows.data(using: .utf8),
           let r = try? JSONDecoder().decode([[Float]].self, from: data), !r.isEmpty {
            rows.append(contentsOf: r)
            if rows.count > Self.maxRows { rows.removeFirst(rows.count - Self.maxRows) }
        }
        guard n != seen else { return }
        seen = n
        let json = take(ws_state())
        if let data = json.data(using: .utf8) {
            do {
                state = try decoder.decode(EngineState.self, from: data)
            } catch {
                print("whistlestop: state didn't decode: \(error)")
            }
        }
    }

    // MARK: commands

    /// One engine command: a bare name (`"Stop"`) or `{name: value}`.
    func send(_ name: String, _ value: Any? = nil) {
        let json: String
        if let value {
            guard let data = try? JSONSerialization.data(
                withJSONObject: [name: value], options: [.fragmentsAllowed]),
                  let s = String(data: data, encoding: .utf8) else {
                print("whistlestop: \(name) couldn't be encoded")
                return
            }
            json = s
        } else {
            json = "\"\(name)\""
        }
        if ws_send(json) != 0 { print("whistlestop: the engine refused \(json)") }
        seen = .max  // look again next tick
    }

    func setFreq(khz: Double) { send("SetFreq", khz * 1000) }
    func connect(_ call: String) { send("Connect", call.uppercased()) }
    func disconnect() { send("Disconnect") }
    func chat(_ text: String) { send("Chat", text) }
    func startSearch(_ scope: SearchChoice) {
        switch scope {
        case .currentBand: send("StartSearch", "CurrentBand")
        case .allBands: send("StartSearch", "AllBands")
        case .band(let b): send("StartSearch", ["Band": b])
        }
    }
    func stopSearch() { send("StopSearch") }
    /// Call CQ for a 1:1 chat (`table` nil) or to host a named Roundtable.
    func callCq(table: String? = nil) {
        let t = table?.trimmingCharacters(in: .whitespaces) ?? ""
        send("CallCq", ["table": t.isEmpty ? NSNull() : t as Any])
    }
    func joinTable(_ name: String) { send("JoinTable", name.trimmingCharacters(in: .whitespaces)) }
    func leaveTable() { send("LeaveTable") }
    func tableSay(_ text: String) { send("TableSay", text) }
    func passFloor(_ call: String) { send("PassFloor", call) }
    func stopCq() { send("StopCq") }
    func setAway(_ on: Bool) { send("SetAway", on) }
    func depotPost(to: String, via: String?, text: String) {
        let relay: Any = via.map { $0.uppercased() } ?? NSNull()
        send("DepotPost", ["to": to.uppercased(), "via": relay, "text": text])
    }
    func depotRead(_ id: UInt64) { send("DepotRead", NSNumber(value: id)) }
    func depotDelete(_ id: UInt64) { send("DepotDelete", NSNumber(value: id)) }
    func depotConnect(_ call: String) { send("DepotConnect", call.uppercased()) }
    func qsy(khz: Double) { send("Qsy", khz * 1000) }
    func beaconNow() { send("BeaconNow") }
    func nudge(_ hz: Double) { send("Nudge", hz) }

    // MARK: settings

    func loadConfig() {
        let s = take(ws_config())
        if let data = s.data(using: .utf8),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            config = obj
        }
    }

    /// Save and apply. Returns false if the engine refused the settings.
    @discardableResult
    func saveConfig() -> Bool {
        guard let data = try? JSONSerialization.data(withJSONObject: config),
              let s = String(data: data, encoding: .utf8) else { return false }
        let ok = ws_set_config(s) == 0
        loadConfig()
        return ok
    }

    func string(_ key: String) -> String { config[key] as? String ?? "" }
    func bool(_ key: String) -> Bool { config[key] as? Bool ?? false }
    func number(_ key: String) -> Double { (config[key] as? NSNumber)?.doubleValue ?? 0 }

    /// A binding into the settings, for forms.
    func binding(_ key: String) -> Binding<String> {
        Binding(get: { self.string(key) }, set: { self.config[key] = $0 })
    }
    func boolBinding(_ key: String) -> Binding<Bool> {
        Binding(get: { self.bool(key) }, set: { self.config[key] = $0 })
    }
    func intBinding(_ key: String) -> Binding<Int> {
        Binding(get: { Int(self.number(key)) }, set: { self.config[key] = $0 })
    }
}

enum ChatPane: Hashable { case link, table }

enum SearchChoice: Hashable {
    case currentBand, allBands
    case band(String)
}
