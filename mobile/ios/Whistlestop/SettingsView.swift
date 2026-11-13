import SwiftUI

/// The settings a phone needs; the rest keep their saved values (the whole
/// settings file goes back to the engine, so nothing is lost).
struct SettingsView: View {
    @EnvironmentObject var core: Core
    @AppStorage("helpMode") private var helpMode = false
    @State private var saved = false
    /// Encryption asked for, waiting for the legal warning to be confirmed.
    @State private var e2eAsk: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Toggle("Help mode", isOn: $helpMode)
                } footer: {
                    Text(HelpTopic.helpMode.body)
                }

                Section {
                    LabeledContent("Callsign") {
                        TextField("e.g. N0CALL", text: core.binding("callsign"))
                            .textInputAutocapitalization(.characters).autocorrectionDisabled()
                            .multilineTextAlignment(.trailing)
                    }
                    LabeledContent("Name") {
                        TextField("First name", text: core.binding("name"))
                            .multilineTextAlignment(.trailing)
                    }
                    LabeledContent("Grid") {
                        TextField("e.g. DM79", text: core.binding("grid"))
                            .textInputAutocapitalization(.characters).autocorrectionDisabled()
                            .multilineTextAlignment(.trailing)
                    }
                } header: { header("Station", .station) }

                Section {
                    Toggle("Demo station (no radio)", isOn: core.boolBinding("demo"))
                } header: { header("Demo", .demo) } footer: {
                    Text("Takes effect when you tap Save.")
                }

                Section {
                    LabeledContent("CAT bridge") {
                        TextField("host:port", text: catPort)
                            .keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                            .multilineTextAlignment(.trailing)
                    }
                } header: { header("Radio", .radio) }

                Section {
                    LabeledContent("Host") {
                        TextField("127.0.0.1", text: core.binding("mercury_host"))
                            .keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                            .multilineTextAlignment(.trailing)
                    }
                    LabeledContent("Port") {
                        NumberField(key: "mercury_port", placeholder: "8300", range: 1...65534, decimal: false)
                    }
                } header: { header("Modem (Mercury)", .modem) }

                Section {
                    Toggle("Accept files by myself", isOn: core.boolBinding("auto_accept_away"))
                    LabeledContent("Each up to (MB)") {
                        NumberField(key: "auto_accept_max_mb", placeholder: "5", range: 0.1...200, decimal: true)
                            .frame(width: 80)
                    }
                    .disabled(!core.bool("auto_accept_away"))
                } header: { header("While away", .autoAccept) } footer: {
                    Text("Off by default. A bigger file waits for you. Takes effect when you tap Save.")
                }

                Section {
                    Picker("Encryption", selection: e2e) {
                        Text("Off").tag("off")
                        Text("Prefer").tag("prefer")
                        Text("Require").tag("require")
                    }
                    .pickerStyle(.segmented)
                } header: { header("Encryption", .encryption) } footer: {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(e2eNote)
                        Text("⚠ " + Self.legalWarning).foregroundStyle(Theme.red)
                        Text("Roundtables are never encrypted. Keys are made per link and never saved. Takes effect when you tap Save, from the next link.")
                    }
                }

                Section {
                    LabeledContent("Modem", value: core.state.modemVersion ?? "—")
                    LabeledContent("App", value: Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "")
                } header: { Text("About") } footer: {
                    Text("Whistlestop is free software (GPL-3.0). It includes the Mercury modem by Rhizomatica.")
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .alert("Turn on encryption?", isPresented: Binding(get: { e2eAsk != nil }, set: { if !$0 { e2eAsk = nil } })) {
                Button("Turn on", role: .destructive) {
                    if let want = e2eAsk { core.config["e2e"] = want }
                    e2eAsk = nil
                }
                Button("Cancel", role: .cancel) { e2eAsk = nil }
            } message: {
                Text(Self.legalWarning)
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(saved ? "Saved" : "Save") {
                        // The engine restarts the station itself when a
                        // connection setting (demo, modem, radio, callsign)
                        // changed.
                        if core.saveConfig() {
                            saved = true
                            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { saved = false }
                        }
                    }
                    .frame(minWidth: 56)
                }
            }
        }
    }

    private func header(_ title: String, _ topic: HelpTopic) -> some View {
        HStack { Text(title); Spacer(); HelpButton(topic: topic) }
    }

    /// The engine's `e2e::LEGAL_WARNING`, word for word.
    static let legalWarning = "Encryption is NOT legal on amateur radio bands in the US (47 CFR 97.113(a)(4)) and in most other countries: amateur transmissions may not be encoded to obscure their meaning. Use it only where it is lawful (for example on non-amateur frequencies you are licensed for, or over a wired or test link)."

    /// `e2e`: off, prefer or require. Leaving Off asks first.
    private var e2e: Binding<String> {
        Binding(
            get: { core.string("e2e").isEmpty ? "off" : core.string("e2e") },
            set: { want in
                let now = core.string("e2e").isEmpty ? "off" : core.string("e2e")
                if now == "off" && want != "off" { e2eAsk = want } else { core.config["e2e"] = want }
            })
    }

    private var e2eNote: String {
        switch core.string("e2e") {
        case "prefer": "Encrypt when the other station does too; otherwise plain text."
        case "require": "Encrypt or hang up: never plain text."
        default: "Links are plain text (the default)."
        }
    }

    /// The CAT port inside the `cat` table.
    private var catPort: Binding<String> {
        Binding(
            get: { (core.config["cat"] as? [String: Any])?["port"] as? String ?? "" },
            set: {
                var cat = core.config["cat"] as? [String: Any] ?? [:]
                cat["port"] = $0
                core.config["cat"] = cat
                // A bridge speaks the radio's own CAT: Whistlestop drives it
                // directly (and shares it with Mercury).
                core.config["rig_mode"] = $0.trimmingCharacters(in: .whitespaces).isEmpty ? "External" : "Direct"
            })
    }
}

/// A number setting typed as text. Every keystroke that makes a number in
/// `range` goes into the settings at once (so Save never misses the last
/// edit), and what you type stays as typed ("2." while on the way to "2.5").
/// Out of range or not a number: shown in red and not taken.
struct NumberField: View {
    @EnvironmentObject var core: Core
    let key: String
    let placeholder: String
    let range: ClosedRange<Double>
    let decimal: Bool
    @State private var text = ""

    private func show(_ v: Double) -> String {
        decimal && v != v.rounded() ? String(v) : String(Int(v))
    }
    private var valid: Bool { Fmt.kHz(text).map { range.contains($0) } ?? false }

    var body: some View {
        TextField(placeholder, text: $text)
            .keyboardType(decimal ? .decimalPad : .numberPad)
            .multilineTextAlignment(.trailing)
            .monospacedDigit()
            .foregroundStyle(valid || text.isEmpty ? Theme.text : Theme.red)
            .onAppear { text = show(core.number(key)) }
            .onChange(of: text) { _, t in
                if let v = Fmt.kHz(t), range.contains(v) {
                    core.config[key] = decimal ? v : Double(Int(v))
                }
            }
            // Reloaded (after Save, or a revert): show the saved value,
            // unless it's what's typed already.
            .onChange(of: core.number(key)) { _, v in
                if Fmt.kHz(text) != v { text = show(v) }
            }
    }
}
