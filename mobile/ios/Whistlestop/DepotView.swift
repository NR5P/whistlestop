import SwiftUI

/// The Depot: mail for stations you'll link with later, and mail for you.
struct DepotView: View {
    @EnvironmentObject var core: Core
    @State private var box = Box.arrivals
    @State private var writing = false
    @State private var replyTo = ""

    enum Box: String, CaseIterable { case arrivals = "Arrivals", departures = "Departures", transit = "In Transit", delivered = "Delivered" }

    var body: some View {
        let depot = core.state.depot ?? Depot()
        let mail: [Mail] = switch box {
        case .arrivals: depot.arrivals
        case .departures: depot.departures
        case .transit: depot.transit
        case .delivered: depot.delivered
        }
        NavigationStack {
            VStack(spacing: 0) {
                HStack {
                    Picker("Box", selection: $box) {
                        ForEach(Box.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    HelpButton(topic: .depotLists)
                }
                .padding(.horizontal, 16).padding(.vertical, 8)
                if mail.isEmpty {
                    ContentUnavailableView("No mail", systemImage: "tray",
                                           description: Text(emptyText))
                        .frame(maxHeight: .infinity)
                } else {
                    List(mail.sorted { $0.created > $1.created }) { m in
                        NavigationLink {
                            MailDetail(mail: m, box: box) { from in
                                replyTo = from
                                writing = true
                            }
                        } label: {
                            MailRow(mail: m, box: box)
                        }
                        .listRowBackground(Theme.card)
                    }
                    .scrollContentBackground(.hidden)
                }
            }
            .background(Theme.bg)
            .navigationTitle("Depot")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) { HelpButton(topic: .depot) }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { replyTo = ""; writing = true } label: { Image(systemName: "square.and.pencil") }
                        .accessibilityLabel("Write mail")
                }
            }
            .sheet(isPresented: $writing) { WriteMail(to: replyTo) }
        }
    }

    private var emptyText: String {
        switch box {
        case .arrivals: "Mail for you lands here."
        case .departures: "Mail you write waits here for its station."
        case .transit: "Mail you carry for others waits here."
        case .delivered: "Confirmed mail is kept here."
        }
    }
}

struct MailRow: View {
    var mail: Mail
    var box: DepotView.Box
    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(box == .arrivals ? mail.from : "To \(mail.to)").font(.body.weight(.semibold))
                if let via = mail.via, !via.isEmpty { Text("via \(via)").font(.caption).foregroundStyle(Theme.dim) }
                Spacer()
                if box == .arrivals && mail.read != true {
                    Text("NEW").font(.caption2.weight(.heavy)).foregroundStyle(Theme.ink)
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(Theme.brass, in: Capsule())
                }
                Text(Fmt.utc(mail.created)).font(.caption.monospacedDigit()).foregroundStyle(Theme.dim)
            }
            Text(mail.text).font(.callout).foregroundStyle(Theme.dim).lineLimit(2)
        }
    }
}

struct MailDetail: View {
    @EnvironmentObject var core: Core
    @Environment(\.dismiss) private var dismiss
    var mail: Mail
    var box: DepotView.Box
    var reply: (String) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("From \(mail.from) to \(mail.to)").font(.headline)
                if let via = mail.via, !via.isEmpty { Text("Via \(via)").foregroundStyle(Theme.dim) }
                Text(Fmt.utc(mail.created)).font(.caption).foregroundStyle(Theme.dim)
                Text(mail.text).textSelection(.enabled)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.card, in: RoundedRectangle(cornerRadius: 10))
                HStack(spacing: 8) {
                    if box == .arrivals {
                        Button("Reply") { dismiss(); reply(mail.from) }.buttonStyle(BrassButtonStyle())
                    }
                    if box == .departures || box == .transit {
                        Button("Connect now") { core.depotConnect(mail.via ?? mail.to) }
                            .buttonStyle(BrassButtonStyle())
                    }
                    Button("Delete", role: .destructive) { core.depotDelete(mail.id); dismiss() }
                        .buttonStyle(QuietButtonStyle())
                }
            }
            .padding(16)
        }
        .background(Theme.bg)
        .onAppear { if box == .arrivals && mail.read != true { core.depotRead(mail.id) } }
    }
}

struct WriteMail: View {
    @EnvironmentObject var core: Core
    @Environment(\.dismiss) private var dismiss
    @State var to: String
    @State private var via = ""
    @State private var text = ""

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 10) {
                HStack { Text("Write mail").font(.headline); Spacer(); HelpButton(topic: .depotPost) }
                Field(placeholder: "To (callsign)", text: $to)
                Field(placeholder: "Via (optional relay)", text: $via)
                TextEditor(text: $text)
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .frame(minHeight: 160)
                    .background(Theme.panel, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.border, lineWidth: 1))
                Text("\(text.utf8.count) bytes").font(.caption).foregroundStyle(Theme.dim)
                Spacer()
            }
            .padding(16)
            .background(Theme.bg)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Post") {
                        core.depotPost(to: to, via: via.isEmpty ? nil : via, text: text)
                        dismiss()
                    }
                    .disabled(to.trimmingCharacters(in: .whitespaces).isEmpty || text.isEmpty)
                }
            }
        }
    }
}
