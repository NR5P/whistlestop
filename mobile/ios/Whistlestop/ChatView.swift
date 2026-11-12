import SwiftUI

/// The Chat tab: the 1:1 link's chat, or the Roundtable.
struct ChatView: View {
    @EnvironmentObject var core: Core

    var body: some View {
        VStack(spacing: 0) {
            Picker("Conversation", selection: $core.chatPane) {
                Text("1:1 chat").tag(ChatPane.link)
                Text(core.state.table != nil ? "Roundtable ●" : "Roundtable").tag(ChatPane.table)
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16).padding(.vertical, 8)
            .background(Theme.panel)
            switch core.chatPane {
            case .link: LinkChatView()
            case .table: TableView()
            }
        }
        .background(Theme.bg)
    }
}

/// Keyboard-to-keyboard over the link.
struct LinkChatView: View {
    @EnvironmentObject var core: Core
    @State private var text = ""
    @FocusState private var typing: Bool

    var body: some View {
        let s = core.state
        let lines = s.chat ?? []
        VStack(spacing: 0) {
            HStack {
                if let link = s.link {
                    Circle().fill(Theme.teal).frame(width: 8, height: 8)
                    Text("Linked with \(link.peer)").font(.subheadline.weight(.semibold))
                } else {
                    Circle().fill(Theme.dim).frame(width: 8, height: 8)
                    Text("Not linked").font(.subheadline).foregroundStyle(Theme.dim)
                }
                Spacer()
                HelpButton(topic: .chat)
                if s.link != nil {
                    Button("Disconnect") { core.disconnect() }.font(.subheadline)
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
            .background(Theme.panel)

            if let e2e = s.link?.e2e, e2e != .off {
                E2eBadge(e2e: e2e)
                    .padding(.horizontal, 16).padding(.vertical, 6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.panel)
            }

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 8) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { i, line in
                            Bubble(line: line).id(i)
                        }
                    }
                    .padding(16)
                }
                .onChange(of: lines.count) { _, n in
                    withAnimation { proxy.scrollTo(n - 1, anchor: .bottom) }
                }
                .onAppear { proxy.scrollTo(lines.count - 1, anchor: .bottom) }
            }

            HStack(spacing: 8) {
                TextField(s.link == nil ? "Link with a station to chat" : "Type a message", text: $text, axis: .vertical)
                    .lineLimit(1...4)
                    .focused($typing)
                    .padding(10)
                    .background(Theme.panel, in: RoundedRectangle(cornerRadius: 18))
                    .overlay(RoundedRectangle(cornerRadius: 18).stroke(Theme.border, lineWidth: 1))
                    .disabled(s.link == nil)
                    .onSubmit(send)
                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                }
                .disabled(s.link == nil || text.trimmingCharacters(in: .whitespaces).isEmpty)
                .accessibilityLabel("Send")
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(Theme.panel)
        }
        .background(Theme.bg)
    }

    private func send() {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        core.chat(t)
        text = ""
    }
}

/// Encryption on the link: setting up, the check code, or plain text.
struct E2eBadge: View {
    var e2e: E2e
    var body: some View {
        HStack(spacing: 8) {
            switch e2e {
            case .off:
                EmptyView()
            case .negotiating:
                Image(systemName: "hourglass").foregroundStyle(Theme.brass)
                Text("Setting up encryption…").font(.callout).foregroundStyle(Theme.brass)
            case .encrypted(let sas):
                Image(systemName: "lock.fill").foregroundStyle(Theme.teal)
                Text("Encrypted").font(.callout.weight(.semibold)).foregroundStyle(Theme.teal)
                Text("code \(sas)")
                    .font(.system(.body, design: .monospaced).weight(.bold))
                    .foregroundStyle(Theme.text)
                    .accessibilityLabel("check code \(sas.map { String($0) }.joined(separator: " "))")
            case .plain(let why):
                Image(systemName: "lock.open").foregroundStyle(Theme.dim)
                Text("Not encrypted: \(why)").font(.caption).foregroundStyle(Theme.dim).lineLimit(2)
            }
            Spacer(minLength: 0)
            HelpButton(topic: .linkE2e)
        }
    }
}

/// A Roundtable: a group conversation on one frequency.
struct TableView: View {
    @EnvironmentObject var core: Core
    @State private var name = ""
    @State private var text = ""

    var body: some View {
        let s = core.state
        if let t = s.table {
            atTable(t, s)
        } else {
            ScrollView {
                VStack(spacing: 12) {
                    Card(title: "Roundtable", help: .roundtable) {
                        Text("A group conversation on one frequency. No fixed channel: it's wherever its host calls it.")
                            .font(.callout).foregroundStyle(Theme.text)
                        Text("The host sets the speed: it ramps up to the fastest mode every member can copy, and drops back if someone fades.")
                            .font(.caption).foregroundStyle(Theme.dim)
                    }
                    Card(title: "Join", help: s.tablePending != nil ? .tableJoinWait : .tableJoin) {
                        if let pending = s.tablePending {
                            Text("Joining “\(pending)” as soon as Mercury is free (the last link is still closing)…")
                                .font(.callout).foregroundStyle(Theme.brass)
                                .lineLimit(3, reservesSpace: true)
                            Button("Cancel") { core.leaveTable() }
                                .buttonStyle(QuietButtonStyle())
                        } else {
                            Field(placeholder: "Roundtable name", text: $name, caps: false)
                            Button("Join here") { core.joinTable(name); name = "" }
                                .buttonStyle(BrassButtonStyle())
                                .disabled(s.link != nil || name.trimmingCharacters(in: .whitespaces).isEmpty)
                            Text(s.link != nil
                                 ? "Disconnect the 1:1 link first."
                                 : "Tune to the host's frequency first, or tap Join under Heard on Operate.")
                                .font(.caption).foregroundStyle(Theme.dim)
                        }
                    }
                    Text("To host one, use Call CQ → Roundtable on the Operate tab.")
                        .font(.caption).foregroundStyle(Theme.dim)
                }
                .padding(16)
            }
            .scrollDismissesKeyboard(.interactively)
        }
    }

    private func atTable(_ t: TableState, _ s: EngineState) -> some View {
        let me = s.mycall ?? ""
        return VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) {
                    Text(t.name).font(.title3.weight(.bold)).foregroundStyle(Theme.brass).lineLimit(1)
                    Text(t.host ? "you host" : "member").font(.caption).foregroundStyle(Theme.dim)
                    Spacer()
                    HelpButton(topic: .tableMembers)
                    Button("Leave") { core.leaveTable() }.font(.subheadline)
                }
                HStack(spacing: 8) {
                    Text(t.mode).font(.caption.monospacedDigit()).foregroundStyle(Theme.teal)
                    if let f = t.floor, !f.isEmpty {
                        Text("floor: \(f)").font(.caption).foregroundStyle(Theme.brass)
                    }
                }
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        Text("\(me) (you)").font(.caption.weight(.semibold))
                            .padding(.horizontal, 10).padding(.vertical, 5)
                            .background(Theme.cardHi, in: Capsule())
                        ForEach(t.members.keys.sorted(), id: \.self) { call in
                            let snr = t.members[call]?.snrOfHost.map { String(format: " %+d dB", $0) } ?? ""
                            Button("\(call)\(snr)") { core.passFloor(call) }
                                .font(.caption.monospacedDigit())
                                .padding(.horizontal, 10).padding(.vertical, 5)
                                .background(t.floor == call ? Theme.brass.opacity(0.3) : Theme.cardHi, in: Capsule())
                                .foregroundStyle(Theme.text)
                                .accessibilityHint("Pass the floor")
                        }
                    }
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
            .background(Theme.panel)

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(Array(t.lines.enumerated()), id: \.offset) { i, l in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text(Fmt.utc(l.at)).font(.caption2.monospacedDigit()).foregroundStyle(Theme.dim)
                                if l.from.isEmpty {
                                    Text(l.text).font(.callout.italic()).foregroundStyle(Theme.dim)
                                } else {
                                    (Text(l.from).bold().foregroundColor(l.from == me ? Theme.brass : Theme.teal)
                                     + Text("  ") + Text(l.text).foregroundColor(Theme.text))
                                        .font(.callout)
                                }
                            }
                            .id(i)
                        }
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .onChange(of: t.lines.count) { _, n in
                    withAnimation { proxy.scrollTo(n - 1, anchor: .bottom) }
                }
                .onAppear { proxy.scrollTo(t.lines.count - 1, anchor: .bottom) }
            }

            VStack(spacing: 4) {
                HStack(spacing: 8) {
                    TextField("Say something to the table", text: $text, axis: .vertical)
                        .lineLimit(1...4)
                        .padding(10)
                        .background(Theme.panel, in: RoundedRectangle(cornerRadius: 18))
                        .overlay(RoundedRectangle(cornerRadius: 18).stroke(Theme.border, lineWidth: 1))
                        .onSubmit(say)
                    Button(action: say) {
                        Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                    }
                    .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
                    .accessibilityLabel("Send to the table")
                    HelpButton(topic: .tableSay)
                }
                Text(t.sendingEtaS > 0 ? "On air: \(Fmt.ago(t.sendingEtaS)) left" : "Broadcast to everyone at the table")
                    .font(.caption2.monospacedDigit()).foregroundStyle(Theme.dim)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(Theme.panel)
        }
        .background(Theme.bg)
    }

    private func say() {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        core.tableSay(t)
        text = ""
    }
}

struct Bubble: View {
    var line: ChatLine
    var body: some View {
        if line.system {
            Text(line.text)
                .font(.caption).foregroundStyle(Theme.dim)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 2)
        } else {
            HStack {
                if line.mine { Spacer(minLength: 48) }
                VStack(alignment: line.mine ? .trailing : .leading, spacing: 3) {
                    Text(line.text)
                        .foregroundStyle(line.mine ? Theme.ink : Theme.text)
                        .padding(.horizontal, 12).padding(.vertical, 8)
                        .background(line.mine ? Theme.brass : Theme.cardHi, in: RoundedRectangle(cornerRadius: 14))
                    HStack(spacing: 4) {
                        if !line.mine, let from = line.from, !from.isEmpty { Text(from) }
                        Text(Fmt.utc(line.at))
                        if line.mine { Image(systemName: line.delivered == true ? "checkmark" : "clock") }
                    }
                    .font(.caption2).foregroundStyle(Theme.dim)
                }
                if !line.mine { Spacer(minLength: 48) }
            }
        }
    }
}

/// What the engine did and heard, newest first.
struct LogView: View {
    @EnvironmentObject var core: Core
    var body: some View {
        let log = (core.state.log ?? []).reversed()
        List {
            Section {
                ForEach(Array(log.enumerated()), id: \.offset) { _, l in
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(Fmt.utc(l.at)).font(.caption.monospacedDigit()).foregroundStyle(Theme.dim)
                        Text(l.text).font(.callout)
                    }
                    .listRowBackground(Theme.card)
                }
            } header: {
                HStack { Text("Log"); Spacer(); HelpButton(topic: .log) }
            }
        }
        .scrollContentBackground(.hidden)
        .background(Theme.bg)
    }
}
