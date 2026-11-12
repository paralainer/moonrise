import SwiftUI

/// Cached remote image that takes exactly the size it's offered and crops to it. The image is drawn
/// in an overlay so its natural (often wider-than-screen) size never leaks into layout. Images
/// already in memory render on the first frame, so revisiting a screen never flashes placeholders.
struct RemoteImage: View {
    let url: URL?
    var contentMode: ContentMode = .fill
    var alignment: Alignment = .center
    /// Read the disk cache synchronously on first render (page headers only — costs a decode).
    var immediate = false
    @State private var loaded: (url: URL, image: PlatformImage)?

    var body: some View {
        // Memory hit: draw synchronously, no fade.
        let image = loaded.flatMap { $0.url == url ? $0.image : nil }
            ?? url.flatMap { immediate ? ImageCache.shared.cachedIncludingDisk($0) : ImageCache.shared.cached($0) }
        Color.clear
            .overlay(alignment: alignment) {
                if let image {
                    Image(platformImage: image).resizable().aspectRatio(contentMode: contentMode)
                        .transition(.opacity)
                } else {
                    placeholder
                }
            }
            .clipped()
            .task(id: url) {
                guard let url, image == nil else { return }
                // A few retries: a load can fail while the route switches (home Wi-Fi <-> Tailscale).
                for attempt in 0..<3 {
                    if let img = await ImageCache.shared.image(for: url) {
                        withAnimation(.easeOut(duration: 0.2)) { loaded = (url, img) }
                        return
                    }
                    try? await Task.sleep(for: .seconds(1 + Double(attempt) * 2))
                    if Task.isCancelled { return }
                }
            }
    }

    @ViewBuilder private var placeholder: some View {
        if contentMode == .fill { Rectangle().fill(Theme.surface) } else { Color.clear }
    }
}

struct ProgressBar: View {
    let value: Double
    var body: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.black.opacity(0.5))
                Capsule().fill(Theme.accent).frame(width: g.size.width * min(1, max(0, value)))
            }
        }
        .frame(height: 4)
    }
}

/// 2:3 poster with title + year below.
struct PosterCard: View {
    let item: MediaItem
    let client: (any MediaServerClient)?
    /// nil: the window's size (`Metrics`).
    var width: CGFloat? = nil
    @Environment(\.metrics) private var metrics
    private var w: CGFloat { width ?? metrics.posterWidth }
    /// Zoom-transition source id when this card opens a page.
    var zoomID: String? = nil
    @Environment(\.zoomNamespace) private var zoom

    var body: some View {
        let item = WatchState.shared.current(self.item) // marked from a long-press menu
        #if os(tvOS)
        // A system lockup (borderless button label): the artwork lifts on focus and the title
        // moves down out of its way. In a VStack the grown artwork would cover the title.
        artwork(item).lockupArtwork()
        labels(item).frame(width: w, alignment: .leading)
        #else
        VStack(alignment: .leading, spacing: 6) {
            artwork(item)
                .shadow(color: .black.opacity(0.4), radius: 6, y: 3)
            labels(item)
        }
        .frame(width: w)
        #endif
    }

    private func artwork(_ item: MediaItem) -> some View {
        ZStack(alignment: .bottom) {
            RemoteImage(url: client?.posterURL(item))
                .frame(width: w, height: w * 1.5)
                .overlay {
                    if client?.posterURL(item) == nil {
                        // Some library items have no artwork at all.
                        ZStack {
                            LinearGradient(colors: [Color.white.opacity(0.14), Color.white.opacity(0.04)],
                                           startPoint: .topLeading, endPoint: .bottomTrailing)
                            VStack(spacing: 8) {
                                Image(systemName: item.isSeries ? "tv" : "film")
                                    .font(.system(size: w * 0.22, weight: .light))
                                Text(item.displayTitle)
                                    .font(.caption.weight(.semibold))
                                    .multilineTextAlignment(.center)
                                    .lineLimit(3)
                            }
                            .foregroundStyle(.white.opacity(0.75))
                            .padding(8)
                        }
                    }
                }
            if let p = item.progress { ProgressBar(value: p).padding(6) }
        }
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .hoverHighlight(cornerRadius: 12)
        .zoomSource(zoomID, in: zoom, cornerRadius: 12)
        .overlay(alignment: .topTrailing) { WatchedMark(item: item) }
        .overlay(alignment: .topLeading) { DownloadMark(item: item) }
    }

    /// Seasons/episodes (e.g. in Recently Added) carry the show's poster, so lead with the show's name.
    private func labels(_ item: MediaItem) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text((item.isSeason || item.isEpisode) ? (item.seriesName ?? item.displayTitle) : item.displayTitle)
                .font(.caption.weight(.medium)).lineLimit(1).foregroundStyle(.white)
            Text(item.isSeason ? item.displayTitle
                 : item.isEpisode ? ([item.episodeLabel, item.name].compactMap { $0 }.joined(separator: " · "))
                 : (item.productionYear.map(String.init) ?? " "))
                .font(.caption2).lineLimit(1).foregroundStyle(Theme.secondaryText)
        }
    }
}

/// 16:9 card for Continue Watching / episodes.
struct LandscapeCard: View {
    let item: MediaItem
    let client: (any MediaServerClient)?
    /// nil: the window's size (`Metrics`).
    var width: CGFloat? = nil
    @Environment(\.metrics) private var metrics
    private var w: CGFloat { width ?? metrics.landscapeWidth }
    var zoomID: String? = nil
    @Environment(\.zoomNamespace) private var zoom

    var body: some View {
        let item = WatchState.shared.current(self.item) // marked from a long-press menu
        VStack(alignment: .leading, spacing: 6) {
            ZStack(alignment: .bottom) {
                RemoteImage(url: client?.thumbURL(item))
                    .frame(width: w, height: w * 9 / 16)
                    .overlay {
                        if client?.thumbURL(item) == nil {
                            // No still or backdrop: the title on a soft gradient, as on posters.
                            ZStack {
                                LinearGradient(colors: [Color.white.opacity(0.14), Color.white.opacity(0.04)],
                                               startPoint: .topLeading, endPoint: .bottomTrailing)
                                VStack(spacing: 6) {
                                    Image(systemName: item.isEpisode ? "tv" : "film")
                                        .font(.system(size: w * 0.12, weight: .light))
                                    Text(item.isEpisode ? (item.name ?? item.displayTitle) : item.displayTitle)
                                        .font(.caption.weight(.semibold))
                                        .multilineTextAlignment(.center)
                                        .lineLimit(2)
                                }
                                .foregroundStyle(.white.opacity(0.75))
                                .padding(10)
                            }
                        }
                    }
                LinearGradient(colors: [.clear, .black.opacity(0.6)], startPoint: .center, endPoint: .bottom)
                if let p = item.progress { ProgressBar(value: p).padding(8) }
            }
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
            .hoverHighlight(cornerRadius: 16)
            .overlay(alignment: .topLeading) { DownloadMark(item: item) }
            .zoomSource(zoomID, in: zoom)
            #if os(tvOS)
            .lockupArtwork()
            #else
            .shadow(color: .black.opacity(0.4), radius: 6, y: 3)
            #endif

            Text(item.isEpisode ? (item.seriesName ?? item.displayTitle) : item.displayTitle)
                .font(.subheadline.weight(.medium)).lineLimit(1).foregroundStyle(.white)
            Text(item.isEpisode ? [item.episodeLabel, item.name].compactMap { $0 }.joined(separator: " · ") : (item.productionYear.map(String.init) ?? " "))
                .font(.caption).lineLimit(1).foregroundStyle(Theme.secondaryText)
        }
        .frame(width: w)
    }
}

struct WatchedMark: View {
    let item: MediaItem
    var body: some View {
        if item.user?.played == true {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: Self.checkSize))
                .symbolRenderingMode(.palette)
                .foregroundStyle(.black, Theme.accent)
                .padding(Self.inset)
        } else if let n = item.user?.unplayedItemCount, n > 0, item.isSeries {
            Text("\(n)")
                .font(.caption2.weight(.bold))
                .padding(.horizontal, Self.inset > 5 ? 10 : 6).padding(.vertical, 2)
                .background(Theme.accent, in: Capsule())
                .foregroundStyle(.black)
                .padding(Self.inset)
        }
    }

    // Apple TV: big enough to read from the couch, clear of the artwork's rounded corner.
    #if os(tvOS)
    private static let checkSize: CGFloat = 32
    private static let inset: CGFloat = 10
    #else
    private static let checkSize: CGFloat = 18
    private static let inset: CGFloat = 5
    #endif
}

struct Shelf<Content: View>: View {
    let title: String
    /// Item id at the row's leading edge. Bound by callers that need the row to stay on the same
    /// item when its contents change (rather than on the same offset).
    var position: Binding<String?>? = nil
    /// A small action at the header's trailing edge (Clear).
    var action: (title: String, run: () -> Void)? = nil
    @ViewBuilder let content: () -> Content
    @Environment(\.metrics) private var metrics

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionHeader(title: title, action: action)
            #if os(macOS)
            MacShelfRow(position: position, spacing: spacing, content: content)
            #else
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(alignment: .top, spacing: spacing) { content() }
                    .scrollTargetLayout()
                    #if os(tvOS)
                    .padding(.vertical, 30)
                    #endif
            }
            // Margins (not padding) so a scroll-to-item lands inside them, not flush to the edge.
            .contentMargins(.horizontal, metrics.hPadding, for: .scrollContent)
            .scrollPosition(id: position ?? .constant(nil), anchor: .leading)
            #if os(tvOS)
            .scrollClipDisabled()
            #else
            // iPad: a flick settles on whole cards at the margin.
            .scrollTargetBehavior(ShelfSnap(on: metrics.regular))
            #endif
            #endif
        }
    }

    private var spacing: CGFloat {
        #if os(tvOS)
        return 40
        #else
        return 14
        #endif
    }
}

#if os(macOS)
/// The Mac's row: scrolls with a trackpad swipe or Shift-wheel, and, for a mouse, shows a
/// chevron at each end that has more while the pointer is over the row (the TV app on the Mac);
/// a click pages by the row's width.
private struct MacShelfRow<Content: View>: View {
    var position: Binding<String?>?
    let spacing: CGFloat
    @ViewBuilder let content: () -> Content
    @Environment(\.metrics) private var metrics
    @State private var scroll = ScrollPosition(idType: String.self)
    @State private var offset: CGFloat = 0
    @State private var maxOffset: CGFloat = 0
    @State private var visibleWidth: CGFloat = 0
    @State private var hovering = false

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            LazyHStack(alignment: .top, spacing: spacing) { content() }
                .scrollTargetLayout()
        }
        .contentMargins(.horizontal, metrics.hPadding, for: .scrollContent)
        .scrollPosition($scroll, anchor: .leading)
        .scrollTargetBehavior(.viewAligned)
        .onScrollGeometryChange(for: [CGFloat].self) { g in
            [g.contentOffset.x + g.contentInsets.leading, max(0, g.contentSize.width + g.contentInsets.leading + g.contentInsets.trailing - g.containerSize.width), g.containerSize.width]
        } action: { _, v in
            offset = v[0]; maxOffset = v[1]; visibleWidth = v[2]
        }
        // The row stays on the same item when its contents change (Home's rows).
        .onAppear { if let id = position?.wrappedValue { scroll.scrollTo(id: id, anchor: .leading) } }
        .onChange(of: scroll.viewID(type: String.self)) { _, id in
            if let id, let position, position.wrappedValue != id { position.wrappedValue = id }
        }
        .overlay(alignment: .leading) { arrow(forward: false).opacity(hovering && offset > 2 ? 1 : 0) }
        .overlay(alignment: .trailing) { arrow(forward: true).opacity(hovering && offset < maxOffset - 2 ? 1 : 0) }
        .animation(.easeOut(duration: 0.15), value: hovering)
        .onHover { hovering = $0 }
    }

    private func arrow(forward: Bool) -> some View {
        Button {
            let page = max(100, visibleWidth - metrics.hPadding * 2)
            let x = min(max(0, offset + (forward ? page : -page)), maxOffset)
            withAnimation(.smooth(duration: 0.35)) { scroll.scrollTo(x: x) }
        } label: {
            Image(systemName: forward ? "chevron.right" : "chevron.left")
                .font(.title3.weight(.semibold))
                .foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .circle)
        .help(forward ? "Next" : "Previous")
        .accessibilityLabel(forward ? "Next" : "Previous")
        // In the margin, over the cards' edge.
        .padding(.horizontal, max(4, metrics.hPadding - 20))
        // Centred on the artwork, not on the artwork and its two lines of text.
        .padding(.bottom, 34)
    }
}
#endif

/// View-aligned scrolling, or the free scrolling of before (iPhone).
struct ShelfSnap: ScrollTargetBehavior {
    let on: Bool
    func updateTarget(_ target: inout ScrollTarget, context: TargetContext) {
        if on { ViewAlignedScrollTargetBehavior().updateTarget(&target, context: context) }
    }
}

struct EmptyStateView: View {
    let icon: String
    let title: String
    let message: String
    /// Optional MOSS-generated artwork from the asset catalog, shown above the text.
    var artwork: String? = nil
    #if os(iOS)
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    #endif

    /// iPhone landscape: a smaller picture, so the text shows without scrolling.
    private var short: Bool {
        #if os(iOS)
        return verticalSizeClass == .compact
        #else
        return false
        #endif
    }

    var body: some View {
        VStack(spacing: 12) {
            if let artwork {
                Color.clear
                    .frame(maxWidth: 520)
                    .frame(height: short ? 110 : 300)
                    .overlay { Image(artwork).resizable().aspectRatio(contentMode: .fill) }
                    .clipped()
                    .mask(RadialGradient(colors: [.black, .black, .clear], center: .center, startRadius: 40, endRadius: 260))
            } else {
                Image(systemName: icon).font(.system(size: 48)).foregroundStyle(Theme.accent)
            }
            Text(title).font(.title3.weight(.semibold))
            Text(message).font(.subheadline).foregroundStyle(Theme.secondaryText).multilineTextAlignment(.center)
        }
        .padding(short ? 16 : 40)
        .frame(maxWidth: .infinity)
    }
}

/// Round Download button (title pages) with an inline quality menu; shows progress once queued.
/// Fills a `size` circle (the page puts it in glass), its icons as big and bold as Play's.
/// A title page's player on the iPhone: the title to download from its controls.
struct InlineDownload {
    let item: MediaItem
    let source: Source
}

struct DownloadButton: View {
    let item: MediaItem
    let source: Source
    var size: CGFloat = 52
    /// The icons' font (the player's buttons: their own size and weight).
    var glyph: Font = DownloadButton.iconFont
    /// The progress ring's inset, as a share of `size` (the player's buttons: a smaller ring,
    /// as big as their symbols).
    var ringInset: CGFloat = 0.12
    /// Its menu opened (true) or closed: the player keeps its controls up meanwhile.
    var onMenu: ((Bool) -> Void)? = nil
    private var manager: DownloadManager { DownloadManager.shared }
    /// The file's details when `item` came from a list without them (episode rows).
    @State private var loadedVersion: MediaVersion?
    /// The title with its versions, when it has several (episode rows come without them).
    @State private var full: MediaItem?
    private var version: MediaVersion? { item.versions?.first ?? loadedVersion }

    var body: some View {
        if !DownloadManager.isSupported {
            EmptyView()
        } else if let rec = manager.record(forItem: item.id) {
            Menu {
                Section(rec.state == .completed ? "Downloaded · \(rec.downloadLabel)" : "Download · \(rec.downloadLabel)") {
                    menuTracker
                    switch rec.state {
                    case .downloading, .queued:
                        Button { manager.pause(rec.id) } label: { Label("Pause Download", systemImage: "pause") }
                    case .paused, .failed:
                        Button { manager.resume(rec.id) } label: { Label("Resume Download", systemImage: "arrow.clockwise") }
                    case .completed:
                        EmptyView()
                    }
                    Button(role: .destructive) { manager.delete(rec.id) } label: { Label("Delete Download", systemImage: "trash") }
                }
            } label: {
                stateIcon(rec)
            }
            .drawnMenu()
            .accessibilityLabel("Download options")
        } else {
            qualityMenu
        }
    }

    /// Menu contents exist only while the menu is shown: their appear/disappear marks it opening
    /// and closing (SwiftUI's Menu has no presentation callback).
    @ViewBuilder private var menuTracker: some View {
        if let onMenu {
            Color.clear.frame(width: 0, height: 0)
                .onAppear { onMenu(true) }
                .onDisappear { onMenu(false) }
        }
    }

    /// Play's icon size (headline, 17 pt); bold, as heavy as its filled triangle looks.
    static let iconFont = Font.headline.weight(.bold)

    @ViewBuilder private func stateIcon(_ rec: DownloadRecord) -> some View {
        ZStack {
            switch rec.state {
            case .completed:
                Image(systemName: "checkmark").font(glyph)
            case .failed:
                Image(systemName: "exclamationmark").font(glyph).foregroundStyle(.red)
            case .paused, .downloading, .queued:
                // A ring just inside the circle, the state in its middle.
                let ring = (size * ringInset).rounded()
                ZStack {
                    Circle().stroke(Color.white.opacity(0.22), lineWidth: 3)
                    Circle().trim(from: 0, to: max(rec.progress, 0.001))
                        .stroke(Theme.accent.opacity(rec.state == .paused ? 0.6 : 1), style: StrokeStyle(lineWidth: 3, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                }
                .padding(ring)
                Image(systemName: rec.state == .paused ? "pause.fill" : "arrow.down")
                    .font(.footnote.weight(.bold))
            }
        }
        .frame(width: size, height: size)
        .contentShape(Circle())
    }

    /// The qualities to download a version in (its size in each).
    @ViewBuilder private func qualities(_ v: MediaVersion?) -> some View {
        ForEach(QualityOption.options(for: v)) { q in
            Button {
                manager.download(item: full ?? item, from: source, quality: q, version: v)
            } label: {
                Text(q == .original ? "Original" : q.label)
                if let size = q.sizeLabel(runtime: v?.runtime ?? item.runtime, sourceSize: v?.size) {
                    Text(size)
                }
            }
        }
    }

    @ViewBuilder private var qualityMenu: some View {
        let label = Image(systemName: "arrow.down")
            .font(glyph)
            .frame(width: size, height: size)
            .contentShape(Circle())
        if let versions = (full ?? item).versions, versions.count > 1 {
            // Several versions: which one first (a tap opens the menu), then the quality.
            Menu {
                Section("Download") {
                    menuTracker
                    ForEach(versions) { v in
                        Menu {
                            qualities(v)
                        } label: {
                            Text(PlayerModel.versionTitle(v))
                            if let d = PlayerModel.versionDetails(v) { Text(d) }
                        }
                        .disabled(v.isBroken)
                    }
                }
            } label: { label }
            .drawnMenu()
            .accessibilityLabel("Download")
        } else {
            Menu {
                Section("Download") {
                    menuTracker
                    qualities(version)
                }
            } label: { label } primaryAction: {
                manager.download(item: item, from: source, quality: QualityOption.find(AppSettings.shared.downloadQuality).capped(to: version))
            }
            .drawnMenu()
            .accessibilityLabel("Download")
            .task(id: item.id) {
                guard item.versions?.first == nil, loadedVersion == nil,
                      let f = try? await MediaServers.client(for: source)?.item(item.id) else { return }
                loadedVersion = f.versions?.first
                if f.hasVersions { full = f }
            }
        }
    }
}
