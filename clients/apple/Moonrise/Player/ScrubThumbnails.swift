import SwiftUI
import CoreGraphics
import ImageIO

/// The scrubbing thumbnails of the title playing, from the Moonrise server (capability
/// `trickplay`, `GET /v1/items/{id}/trickplay`): one picture every `interval` seconds, on JPEG
/// sheets of `columns` × `rows`. Sheets are fetched when first needed and kept while the player
/// is open. The Apple TV doesn't use this: its system player shows the server's I-frame stream.
@MainActor @Observable
final class ScrubThumbnails {
    struct Answer: Decodable {
        var state: String
        var interval: Double?
        var width: Int?
        var height: Int?
        var columns: Int?
        var rows: Int?
        var count: Int?
        var sheets: [String]?
    }

    /// Where picture `index` is: its sheet, and its rectangle on that sheet.
    struct Tile: Equatable {
        let sheet: Int
        let rect: CGRect
    }

    let interval: Double
    let width: Int
    let height: Int
    let columns: Int
    let rows: Int
    let count: Int
    let sheets: [URL]
    @ObservationIgnored private let headers: [String: String]
    @ObservationIgnored private var images: [Int: CGImage] = [:]
    @ObservationIgnored private var loading: Set<Int> = []
    /// Changes when a sheet arrives, so the preview asks again.
    private(set) var arrived = 0

    init?(_ a: Answer, base: URL, headers: [String: String]) {
        guard a.state == "ready", let interval = a.interval, interval > 0, let width = a.width, let height = a.height,
              let columns = a.columns, let rows = a.rows, let count = a.count, count > 0, columns > 0, rows > 0,
              let sheets = a.sheets, !sheets.isEmpty else { return nil }
        self.interval = interval
        self.width = width
        self.height = height
        self.columns = columns
        self.rows = rows
        self.count = count
        self.sheets = sheets.compactMap { URL(string: $0, relativeTo: base)?.absoluteURL }
        self.headers = headers
        guard self.sheets.count == sheets.count else { return nil }
    }

    /// The width / height of a picture.
    var aspect: CGFloat { CGFloat(width) / CGFloat(max(1, height)) }

    /// Where the picture for `time` is (nil: before the start or no pictures).
    nonisolated static func tile(at time: Double, interval: Double, count: Int, columns: Int, rows: Int, width: Int, height: Int) -> Tile? {
        guard count > 0, interval > 0, columns > 0, rows > 0, time.isFinite else { return nil }
        let i = min(count - 1, max(0, Int(time / interval)))
        let perSheet = columns * rows
        let k = i % perSheet
        return Tile(sheet: i / perSheet,
                    rect: CGRect(x: (k % columns) * width, y: (k / columns) * height, width: width, height: height))
    }

    /// The picture for `time`, or nil while its sheet is on its way (it is asked for then).
    func image(at time: Double) -> CGImage? {
        _ = arrived
        guard let t = Self.tile(at: time, interval: interval, count: count, columns: columns, rows: rows, width: width, height: height),
              t.sheet < sheets.count else { return nil }
        guard let sheet = images[t.sheet] else {
            load(t.sheet)
            return nil
        }
        return sheet.cropping(to: t.rect)
    }

    private func load(_ n: Int) {
        guard !loading.contains(n) else { return }
        loading.insert(n)
        let req = HTTP.request(sheets[n], headers: headers, timeout: 20)
        Task { [weak self] in
            let image: CGImage? = await Task.detached {
                guard let (data, status) = try? await HTTP.data(for: req), status == 200,
                      let src = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
                return CGImageSourceCreateImageAtIndex(src, 0, nil)
            }.value
            guard let self else { return }
            self.loading.remove(n)
            if let image {
                self.images[n] = image
                self.arrived += 1
            }
        }
    }

    /// The thumbnails of what a session plays (nil: the server has none, or not yet).
    static func fetch(base: URL, itemId: String, sessionId: String?, headers: [String: String]) async -> ScrubThumbnails? {
        guard await MoonriseServer.serves("trickplay", base: base) == true else { return nil }
        var c = URLComponents(url: base.appendingPathComponent("v1/items/\(itemId)/trickplay"), resolvingAgainstBaseURL: false)
        if let sessionId { c?.queryItems = [URLQueryItem(name: "sid", value: sessionId)] }
        guard let url = c?.url else { return nil }
        let req = HTTP.request(url, headers: headers, timeout: 10, cachePolicy: .reloadIgnoringLocalCacheData)
        guard let (data, status) = try? await HTTP.data(for: req), status == 200,
              let a = try? JSONDecoder().decode(Answer.self, from: data) else { return nil }
        return ScrubThumbnails(a, base: base, headers: headers)
    }
}

/// The picture above the scrubber's knob while dragging, with its time.
struct ScrubPreview: View {
    let image: CGImage?
    let aspect: CGFloat
    let time: Double
    static var width: CGFloat {
        #if os(macOS)
        220
        #else
        UIDevice.current.userInterfaceIdiom == .pad ? 220 : 168
        #endif
    }

    var body: some View {
        VStack(spacing: 4) {
            ZStack {
                Rectangle().fill(Color.black.opacity(0.6))
                if let image {
                    Image(decorative: image, scale: 1).resizable().interpolation(.medium)
                }
            }
            .frame(width: Self.width, height: Self.width / max(0.5, aspect))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Color.white.opacity(0.85), lineWidth: 1.5))
            Text(Fmt.duration(time))
                .font(.footnote.monospacedDigit().weight(.semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, 8).padding(.vertical, 2)
                .background(.black.opacity(0.5), in: Capsule())
        }
        .shadow(color: .black.opacity(0.45), radius: 8)
        .allowsHitTesting(false)
    }
}
