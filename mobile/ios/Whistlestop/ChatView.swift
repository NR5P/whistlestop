import SwiftUI

/// Keyboard-to-keyboard over the link.
struct ChatView: View {
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
