import SwiftUI

/// Find stations and be found: the waterfall, search, whistle, heard list,
/// CQ and a direct connect.
struct OperateView: View {
    @EnvironmentObject var core: Core
    @State private var khz = ""
    @State private var call = ""
    @State private var band = "20m"

    private let bands = ["160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "10m"]

    var body: some View {
        let s = core.state
        ScrollView {
            VStack(spacing: 12) {
                Card(title: "Waterfall", help: .waterfall) {
                    Waterfall(rows: core.rows).frame(height: 150)
                }

                if let link = s.link {
                    LinkCard(link: link)
                }

                Card(title: "Dial", help: .dial) {
                    HStack(spacing: 8) {
                        Field(placeholder: "kHz, e.g. 14103", text: $khz, caps: false)
                            .keyboardType(.decimalPad)
                        Button("Tune") {
                            if let k = Double(khz.replacingOccurrences(of: ",", with: ".")) {
                                core.setFreq(khz: k)
                                khz = ""
                            }
                        }
                        .buttonStyle(QuietButtonStyle())
                        .frame(width: 96)
                        .disabled(Double(khz) == nil)
                    }
                }

                Card(title: "Search", help: .search) {
                    if let search = s.search, search.active {
                        VStack(alignment: .leading, spacing: 6) {
                            ProgressView(value: Double(search.index), total: Double(max(search.total, 1)))
                                .tint(Theme.teal)
                            Text(search.note.isEmpty
                                 ? "Channel \(search.index + 1) of \(search.total) · \(Fmt.dial(search.channelHz)) kHz"
                                 : search.note)
                                .font(.callout).foregroundStyle(Theme.text)
                                .lineLimit(2, reservesSpace: true)
                            Button("Stop search") { core.stopSearch() }
                                .buttonStyle(QuietButtonStyle())
                        }
                    } else {
                        VStack(spacing: 8) {
                            Button("Search current band") { core.startSearch(.currentBand) }
                                .buttonStyle(BrassButtonStyle())
                            HStack(spacing: 8) {
                                Picker("Band", selection: $band) {
                                    ForEach(bands, id: \.self) { Text($0) }
                                }
                                .pickerStyle(.menu)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 4)
                                .background(Theme.cardHi, in: RoundedRectangle(cornerRadius: 10))
                                Button("Search \(band)") { core.startSearch(.band(band)) }
                                    .buttonStyle(QuietButtonStyle())
                            }
                            Button("Search all bands") { core.startSearch(.allBands) }
                                .buttonStyle(QuietButtonStyle())
                        }
                    }
                    HStack {
                        Toggle("Answer automatically", isOn: Binding(
                            get: { core.bool("auto_answer") },
                            set: { core.config["auto_answer"] = $0; core.saveConfig() }))
                            .font(.callout)
                        HelpButton(topic: .autoAnswer)
                    }
                }

                Card(title: "Whistle", help: .whistle) {
                    if let w = s.whistle {
                        HStack {
                            Image(systemName: "waveform.path").foregroundStyle(Theme.brass)
                            VStack(alignment: .leading) {
                                Text("\(Fmt.dial(w.dialHz + w.offsetHz)) kHz")
                                    .font(.body.monospacedDigit().weight(.semibold))
                                Text("score \(String(format: "%.1f", w.score)) · \(Fmt.ago(w.at)) ago")
                                    .font(.caption).foregroundStyle(Theme.dim)
                            }
                        }
                    } else {
                        Text("None heard yet.").font(.callout).foregroundStyle(Theme.dim)
                    }
                }

                Card(title: "Heard", help: .heard) {
                    let heard = s.heard ?? []
                    if heard.isEmpty {
                        Text("No CQs decoded yet.").font(.callout).foregroundStyle(Theme.dim)
                    } else {
                        ForEach(heard.reversed().prefix(8)) { h in
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(h.call).font(.body.weight(.semibold))
                                    Text("\(Fmt.dial(h.dialHz)) kHz · \(h.what) · \(Fmt.utc(h.at))")
                                        .font(.caption).foregroundStyle(Theme.dim).lineLimit(1)
                                }
                                Spacer()
                                Button("Answer") {
                                    core.setFreq(khz: h.dialHz / 1000)
                                    core.connect(h.call)
                                }
                                .buttonStyle(BrassButtonStyle())
                                .frame(width: 110)
                                .disabled(s.link != nil)
                            }
                        }
                    }
                }

                Card(title: "Call CQ", help: .cq) {
                    if let cq = s.cq, cq.active {
                        Text("Cycle \(cq.cycle) of \(cq.cycles) · \(cq.phase)")
                            .font(.callout).foregroundStyle(Theme.text)
                            .lineLimit(1, reservesSpace: true)
                        Button("Stop CQ") { core.stopCq() }
                            .buttonStyle(BrassButtonStyle(color: Theme.red))
                    } else {
                        Button("Whistle and call CQ") { core.callCq() }
                            .buttonStyle(BrassButtonStyle())
                            .disabled(s.link != nil || s.mercuryOk != true)
                    }
                }

                Card(title: "Connect", help: .connect) {
                    HStack(spacing: 8) {
                        Field(placeholder: "Callsign", text: $call)
                        Button("Connect") {
                            core.connect(call)
                            call = ""
                        }
                        .buttonStyle(QuietButtonStyle())
                        .frame(width: 110)
                        .disabled(call.trimmingCharacters(in: .whitespaces).isEmpty || s.link != nil)
                    }
                }
            }
            .padding(16)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Theme.bg)
    }
}

/// Who you're linked with, and a way to hang up or move the link.
struct LinkCard: View {
    @EnvironmentObject var core: Core
    var link: Link
    @State private var qsyKhz = ""

    var body: some View {
        Card(title: "Linked", help: .link) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(link.peer).font(.title3.weight(.bold)).foregroundStyle(Theme.teal)
                    Text([link.peerName, link.peerGrid].compactMap { $0?.isEmpty == false ? $0 : nil }.joined(separator: " · "))
                        .font(.caption).foregroundStyle(Theme.dim)
                    Text("\(link.mode ?? "") · \(Int(link.bps ?? 0)) bps · \(Fmt.ago(link.since ?? 0))")
                        .font(.caption.monospacedDigit()).foregroundStyle(Theme.dim)
                }
                Spacer()
                Button("Disconnect") { core.disconnect() }
                    .buttonStyle(QuietButtonStyle())
                    .frame(width: 130)
            }
            HStack(spacing: 8) {
                Field(placeholder: "Move to kHz", text: $qsyKhz, caps: false)
                    .keyboardType(.decimalPad)
                Button("Ask") {
                    if let k = Double(qsyKhz) { core.qsy(khz: k); qsyKhz = "" }
                }
                .buttonStyle(QuietButtonStyle())
                .frame(width: 80)
                .disabled(Double(qsyKhz) == nil)
                HelpButton(topic: .qsy)
            }
            if let note = core.state.qsyNote, !note.isEmpty {
                Text(note).font(.caption).foregroundStyle(Theme.brass)
            }
        }
    }
}
