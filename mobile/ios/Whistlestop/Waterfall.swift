import SwiftUI

/// The radio's audio passband (0–3 kHz) as a waterfall, newest at the top.
/// Same colours and noise tracking as the desktop's: the floor follows the
/// 30th percentile of each row, and 45 dB above it is white.
struct Waterfall: View {
    var rows: [[Float]]
    /// Where the whistle band (1000–2000 Hz) is, marked faintly.
    var markWhistleBand = true

    private static let lut: [(UInt8, UInt8, UInt8)] = {
        let stops: [(Float, [Float])] = [
            (0.00, [6, 10, 22]), (0.25, [22, 38, 92]), (0.50, [110, 44, 140]),
            (0.72, [226, 96, 60]), (0.88, [246, 186, 70]), (1.00, [255, 250, 225]),
        ]
        return (0..<256).map { i in
            let t = Float(i) / 255
            var k = stops.lastIndex { $0.0 <= t } ?? 0
            k = min(k, stops.count - 2)
            let (a, b) = (stops[k], stops[k + 1])
            let u = (t - a.0) / (b.0 - a.0)
            func c(_ j: Int) -> UInt8 { UInt8(max(0, min(255, a.1[j] + (b.1[j] - a.1[j]) * u))) }
            return (c(0), c(1), c(2))
        }
    }()

    static let cols = 256
    static let rangeDb: Float = 45

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: 8).fill(Color(red: 6 / 255, green: 10 / 255, blue: 22 / 255))
                if let img = image() {
                    Image(decorative: img, scale: 1)
                        .resizable()
                        .interpolation(.medium)
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                }
                if markWhistleBand {
                    let w = geo.size.width
                    Rectangle()
                        .fill(Theme.brass.opacity(0.08))
                        .frame(width: w / 3, height: geo.size.height)
                        .offset(x: w / 3)
                        .allowsHitTesting(false)
                }
                HStack {
                    ForEach([0, 1000, 2000, 3000], id: \.self) { hz in
                        Text(hz == 0 ? "0" : "\(hz / 1000)k")
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.dim)
                        if hz != 3000 { Spacer() }
                    }
                }
                .padding(.horizontal, 4)
                .frame(maxHeight: .infinity, alignment: .bottom)
                .padding(.bottom, 2)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Waterfall")
    }

    /// Rows → an RGBA image, newest row on top. Each row is 256 bins over
    /// 0–4 kHz; the picture shows 0–3 kHz.
    private func image() -> CGImage? {
        guard !rows.isEmpty else { return nil }
        let w = Self.cols, h = rows.count
        var px = [UInt8](repeating: 0, count: w * h * 4)
        var noise: Float = -100
        var line = [Float](repeating: 0, count: w)
        for (n, row) in rows.enumerated() {
            guard row.count > 2 else { continue }
            // Resample 0–3 kHz first, and track the noise on what's shown
            // (the radio's filter edge above it would drag the floor down).
            for c in 0..<w {
                let f = (Float(c) + 0.5) / Float(w) * 3000
                let bin = f / 4000 * Float(row.count)
                let i = min(Int(bin), row.count - 2)
                let u = bin - Float(i)
                line[c] = row[i] * (1 - u) + row[i + 1] * u
            }
            let finite = line.filter { $0.isFinite }.sorted()
            guard !finite.isEmpty else { continue }
            let p = finite[finite.count * 3 / 10]
            noise = n == 0 ? p : 0.93 * noise + 0.07 * p
            let floor = noise - 4
            let y = h - 1 - n  // newest on top
            for c in 0..<w {
                let db = line[c]
                let t = max(0, min(1, (db - floor) / Self.rangeDb))
                let rgb = Self.lut[Int(t * 255)]
                let o = (y * w + c) * 4
                px[o] = rgb.0; px[o + 1] = rgb.1; px[o + 2] = rgb.2; px[o + 3] = 255
            }
        }
        let data = Data(px) as CFData
        guard let provider = CGDataProvider(data: data) else { return nil }
        return CGImage(
            width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }
}
