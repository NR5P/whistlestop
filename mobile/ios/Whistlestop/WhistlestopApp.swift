import SwiftUI

@main
struct WhistlestopApp: App {
    @StateObject private var core = Core()

    init() {
        // Like the desktop: Help mode is off every time Whistlestop starts.
        UserDefaults.standard.set(false, forKey: "helpMode")
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(core)
                .preferredColorScheme(.dark)
                .tint(Theme.brass)
                .onAppear { core.start() }
        }
    }
}

struct RootView: View {
    @EnvironmentObject var core: Core
    @State private var tab = Tab.operate

    enum Tab: Hashable { case operate, chat, depot, log, settings }

    var body: some View {
        VStack(spacing: 0) {
            StationHeader()
            TabView(selection: $tab) {
                OperateView()
                    .tabItem { Label("Operate", systemImage: "dot.radiowaves.left.and.right") }
                    .tag(Tab.operate)
                ChatView()
                    .tabItem { Label("Chat", systemImage: "bubble.left.and.bubble.right") }
                    .tag(Tab.chat)
                DepotView()
                    .tabItem { Label("Depot", systemImage: "tray.full") }
                    .tag(Tab.depot)
                    .badge(core.state.depot?.arrivals.filter { $0.read != true }.count ?? 0)
                LogView()
                    .tabItem { Label("Log", systemImage: "list.bullet.rectangle") }
                    .tag(Tab.log)
                SettingsView()
                    .tabItem { Label("Settings", systemImage: "gearshape") }
                    .tag(Tab.settings)
            }
        }
        .background(Theme.bg.ignoresSafeArea())
        .onChange(of: core.state.link?.peer) { _, peer in
            if peer != nil { tab = .chat; core.chatPane = .link }
        }
        .onChange(of: core.state.table?.name ?? core.state.tablePending) { _, name in
            if name != nil { tab = .chat; core.chatPane = .table }
        }
    }
}

/// Callsign, dial, mode and the status pills, always on top.
struct StationHeader: View {
    @EnvironmentObject var core: Core
    @AppStorage("helpMode") private var helpMode = false

    var body: some View {
        let s = core.state
        VStack(spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(s.mycall?.isEmpty == false ? s.mycall! : "NOCALL")
                        .font(.headline.weight(.bold))
                        .foregroundStyle(Theme.brass)
                    Text(s.demo == true ? "Demo station" : (s.rigDesc ?? ""))
                        .font(.caption2).foregroundStyle(Theme.dim).lineLimit(1)
                }
                Spacer()
                VStack(alignment: .trailing, spacing: 2) {
                    Text(Fmt.mhz(s.freqHz))
                        .font(.system(size: 28, weight: .semibold, design: .monospaced))
                        .foregroundStyle(s.tx == true ? Theme.red : Theme.text)
                        .monospacedDigit()
                        .accessibilityLabel("Dial \(Fmt.dial(s.freqHz)) kilohertz")
                    Text("MHz · \(s.rig?.mode ?? "—")")
                        .font(.caption2).foregroundStyle(Theme.dim)
                }
            }
            HStack(spacing: 6) {
                Pill(label: "Modem", on: s.mercuryOk == true)
                Pill(label: "Radio", on: s.rigOk == true)
                Pill(label: "Audio", on: s.audioOk == true || s.demo == true)
                if s.tx == true {
                    Text("TX").font(.caption2.weight(.heavy)).foregroundStyle(.white)
                        .padding(.horizontal, 8).padding(.vertical, 4)
                        .background(Theme.red, in: Capsule())
                }
                Spacer()
                HelpButton(topic: .status)
                HelpButton(topic: .away)
                Button {
                    core.setAway(!(s.away ?? false))
                } label: {
                    Text(s.away == true ? "Away" : "Here")
                        .font(.caption.weight(.semibold))
                        .frame(width: 44)
                        .padding(.vertical, 4)
                        .background(s.away == true ? Theme.brass : Theme.cardHi, in: Capsule())
                        .foregroundStyle(s.away == true ? Theme.ink : Theme.text)
                }
                .accessibilityLabel(s.away == true ? "Away. Tap when you're back" : "Here. Tap when you go away")
                Button { helpMode.toggle() } label: {
                    Image(systemName: helpMode ? "questionmark.circle.fill" : "questionmark.circle")
                        .font(.title3)
                        .foregroundStyle(helpMode ? Theme.brass : Theme.dim)
                }
                .accessibilityLabel("Help mode")
                .accessibilityValue(helpMode ? "on" : "off")
            }
        }
        .padding(.horizontal, 16).padding(.top, 8).padding(.bottom, 10)
        .background(Theme.panel)
        .overlay(alignment: .bottom) { Rectangle().fill(Theme.border).frame(height: 1) }
    }
}
