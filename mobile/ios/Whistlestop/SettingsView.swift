import SwiftUI

/// The settings a phone needs; the rest keep their saved values (the whole
/// settings file goes back to the engine, so nothing is lost).
struct SettingsView: View {
    @EnvironmentObject var core: Core
    @AppStorage("helpMode") private var helpMode = false
    @State private var saved = false

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
                        TextField("8300", value: core.intBinding("mercury_port"), format: .number.grouping(.never))
                            .keyboardType(.numberPad)
                            .multilineTextAlignment(.trailing)
                    }
                } header: { header("Modem (Mercury)", .modem) }

                Section {
                    LabeledContent("Modem", value: core.state.modemVersion ?? "—")
                    LabeledContent("App", value: Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "")
                } header: { Text("About") } footer: {
                    Text("Whistlestop is free software (GPL-3.0). It includes the Mercury modem by Rhizomatica.")
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(saved ? "Saved" : "Save") {
                        // A demo change needs a restart of the engine's
                        // station, which Start does.
                        let demoChanged = core.bool("demo") != (core.state.demo ?? false)
                        if core.saveConfig() {
                            if demoChanged { restart() }
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

    /// The CAT port inside the `cat` table.
    private var catPort: Binding<String> {
        Binding(
            get: { (core.config["cat"] as? [String: Any])?["port"] as? String ?? "" },
            set: {
                var cat = core.config["cat"] as? [String: Any] ?? [:]
                cat["port"] = $0
                core.config["cat"] = cat
            })
    }

    private func restart() {
        core.send("Stop")
        if let data = try? JSONSerialization.data(withJSONObject: core.config),
           let obj = try? JSONSerialization.jsonObject(with: data) {
            core.send("Start", obj)
        }
    }
}
