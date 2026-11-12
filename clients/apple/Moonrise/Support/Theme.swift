import SwiftUI

enum Theme {
    /// Neutral, Apple TV–style accent: content artwork carries the colour, chrome stays white/glass.
    static let accent = Color.white
    static let background = Color(red: 0.05, green: 0.06, blue: 0.08)
    static let surface = Color.white.opacity(0.07)
    static let surfaceStrong = Color.white.opacity(0.12)
    static let secondaryText = Color.white.opacity(0.62)

    static var backgroundGradient: LinearGradient {
        LinearGradient(
            colors: [Color(red: 0.10, green: 0.11, blue: 0.15), background],
            startPoint: .top, endPoint: .bottom)
    }

    #if os(tvOS)
    static let posterWidth: CGFloat = 220
    static let landscapeWidth: CGFloat = 400
    /// Seven posters across the 1760-pt content width, like Home's poster rows.
    static let gridMin: CGFloat = 200
    /// The system's safe area already insets content 80 pt from the screen's sides (Apple's
    /// tvOS margins): no extra padding, so every screen shares Home's leading edge.
    static let hPadding: CGFloat = 0
    #elseif os(macOS)
    static let posterWidth: CGFloat = 150
    static let landscapeWidth: CGFloat = 280
    static let gridMin: CGFloat = 140
    static let hPadding: CGFloat = 28
    #else
    static let posterWidth: CGFloat = 118
    static let landscapeWidth: CGFloat = 250
    static let gridMin: CGFloat = 108
    static let hPadding: CGFloat = 16
    #endif
}

/// Layout sizes for the window the views are in. iPhone, Mac, Apple TV and iPad windows of compact
/// width get exactly `Theme`'s constants; an iPad window of regular width (full screen, wide
/// windows) gets sizes from its measured width: iPad margins, and rows that show about as many
/// posters as the width holds (with the next one peeking in) instead of the phone's small cards.
struct Metrics: Equatable {
    /// iPad, regular width.
    var regular = false
    /// The window's size (zero until measured; only read when `regular`).
    var width: CGFloat = 0
    var height: CGFloat = 0
    var hPadding: CGFloat = Theme.hPadding
    var posterWidth: CGFloat = Theme.posterWidth
    var landscapeWidth: CGFloat = Theme.landscapeWidth
    /// Library grid: the smallest poster.
    var gridMin: CGFloat = Theme.gridMin
    /// The Library's sidebar layout: iPad of regular width, and any window wider than tall and at
    /// least 700 pt wide (the iPhone in landscape).
    var sidebarLibrary = false

    /// Wider than tall: title pages and Home's banner put the title and Play over the picture.
    var wide: Bool { regular && width > height }

    /// Room for a second column on title pages (details beside the overview).
    var twoColumns: Bool { regular && width >= 1000 }

    /// The iPad page margin: the system's own (where navigation titles and the back button sit).
    static let padMargin: CGFloat = 20
    static let rowSpacing: CGFloat = 14

    static func pad(width: CGFloat, height: CGFloat) -> Metrics {
        var m = Metrics()
        m.regular = true
        m.width = width
        m.height = height
        m.hPadding = padMargin
        // Items visible across a row (the fraction peeks in at the trailing edge).
        let posters: CGFloat = width < 900 ? 5.3 : width < 1250 ? 6.3 : 7.3
        let cards: CGFloat = width < 900 ? 2.6 : width < 1250 ? 3.3 : 3.5
        func size(_ count: CGFloat) -> CGFloat {
            ((width - padMargin - rowSpacing * count.rounded(.down)) / count).rounded(.down)
        }
        m.posterWidth = max(Theme.posterWidth, size(posters))
        m.landscapeWidth = max(Theme.landscapeWidth, size(cards))
        m.gridMin = 150
        return m
    }
}

extension EnvironmentValues {
    @Entry var metrics = Metrics()
}

#if os(iOS)
/// Measures the window and puts its `Metrics` in the environment (iPad of regular width only).
struct WindowMetrics: ViewModifier {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var size: CGSize = .zero

    func body(content: Content) -> some View {
        let regular = UIDevice.current.userInterfaceIdiom == .pad && sizeClass == .regular && size.width > 0
        var m = regular ? Metrics.pad(width: size.width, height: size.height) : Metrics()
        m.sidebarLibrary = regular || (size.width > size.height && size.width >= 700)
        if !regular, m.sidebarLibrary { m.width = size.width; m.height = size.height }
        return content
            .environment(\.metrics, m)
            .onGeometryChange(for: CGSize.self) { proxy in
                let s = proxy.size, i = proxy.safeAreaInsets
                return CGSize(width: s.width + i.leading + i.trailing, height: s.height + i.top + i.bottom)
            } action: { size = $0 }
    }
}
#elseif os(macOS)
/// The Mac lays out as the iPad does at regular width (top tabs, Library sidebar, title pages at
/// the leading margin), from the window's live size: rows hold more posters as the window widens.
struct WindowMetrics: ViewModifier {
    @State private var size: CGSize = .zero

    func body(content: Content) -> some View {
        var m = size.width > 0 ? Metrics.pad(width: size.width, height: size.height) : Metrics()
        m.sidebarLibrary = size.width > 0
        return content
            .environment(\.metrics, m)
            .onGeometryChange(for: CGSize.self) { proxy in
                let s = proxy.size, i = proxy.safeAreaInsets
                return CGSize(width: s.width + i.leading + i.trailing, height: s.height + i.top + i.bottom)
            } action: { size = $0 }
    }
}
#endif

struct SectionHeader: View {
    let title: String
    /// A small action at the row's trailing edge (Clear).
    var action: (title: String, run: () -> Void)? = nil
    @Environment(\.metrics) private var metrics
    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title)
                .font(.title3.weight(.semibold))
                .foregroundStyle(.white)
            if let action {
                Spacer(minLength: 16)
                Button(action.title, action: action.run)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, metrics.hPadding)
    }
}

/// Small rounded technical badge ("4K", "HDR", "5.1").
struct Badge: View {
    let text: String
    var body: some View {
        Text(text)
            .font(.caption2.weight(.bold))
            #if os(macOS)
            .fixedSize() // macOS clipped "Dolby Vision" to "Dolby Vis…" with room to spare
            #endif
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.white.opacity(0.5), lineWidth: 1))
            .foregroundStyle(.white.opacity(0.85))
    }
}
