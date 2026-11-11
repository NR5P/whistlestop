import SwiftUI

/// The desktop app's palette: night-blue panels, brass for the whistle and
/// calls to action, teal for "on" and linked.
enum Theme {
    static let bg = Color(red: 13 / 255, green: 17 / 255, blue: 23 / 255)
    static let panel = Color(red: 19 / 255, green: 25 / 255, blue: 33 / 255)
    static let card = Color(red: 26 / 255, green: 33 / 255, blue: 43 / 255)
    static let cardHi = Color(red: 34 / 255, green: 43 / 255, blue: 56 / 255)
    static let border = Color(red: 44 / 255, green: 55 / 255, blue: 70 / 255)
    static let text = Color(red: 222 / 255, green: 228 / 255, blue: 236 / 255)
    static let dim = Color(red: 134 / 255, green: 148 / 255, blue: 165 / 255)
    static let brass = Color(red: 242 / 255, green: 169 / 255, blue: 59 / 255)
    static let teal = Color(red: 60 / 255, green: 200 / 255, blue: 180 / 255)
    static let red = Color(red: 232 / 255, green: 92 / 255, blue: 84 / 255)
    static let ink = Color(red: 20 / 255, green: 16 / 255, blue: 10 / 255)
}

/// A rounded card with a title row.
struct Card<Content: View>: View {
    var title: String
    var help: HelpTopic? = nil
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(title.uppercased())
                    .font(.caption.weight(.semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.dim)
                Spacer()
                if let help { HelpButton(topic: help) }
            }
            content
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.border, lineWidth: 1))
    }
}

/// A status pill: a dot and a word.
struct Pill: View {
    var label: String
    var on: Bool
    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(on ? Theme.teal : Theme.red).frame(width: 7, height: 7)
            Text(label).font(.caption2.weight(.medium)).foregroundStyle(Theme.text)
        }
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(Theme.cardHi, in: Capsule())
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label) \(on ? "ready" : "not ready")")
    }
}

struct BrassButtonStyle: ButtonStyle {
    var color: Color = Theme.brass
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundStyle(Theme.ink)
            .padding(.vertical, 11).padding(.horizontal, 16)
            .frame(maxWidth: .infinity)
            .background(color.opacity(configuration.isPressed ? 0.75 : 1), in: RoundedRectangle(cornerRadius: 10))
    }
}

struct QuietButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.medium))
            .foregroundStyle(Theme.text)
            .padding(.vertical, 11).padding(.horizontal, 16)
            .frame(maxWidth: .infinity)
            .background(Theme.cardHi.opacity(configuration.isPressed ? 0.6 : 1), in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border, lineWidth: 1))
    }
}

/// A text field styled like the desktop's.
struct Field: View {
    var placeholder: String
    @Binding var text: String
    var caps = true
    var body: some View {
        TextField(placeholder, text: $text)
            .textInputAutocapitalization(caps ? .characters : .sentences)
            .autocorrectionDisabled()
            .padding(10)
            .background(Theme.panel, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.border, lineWidth: 1))
            .foregroundStyle(Theme.text)
    }
}
