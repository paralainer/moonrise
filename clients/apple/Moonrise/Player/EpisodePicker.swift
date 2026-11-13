import SwiftUI

/// Episodes of the playing show, by season, for switching from the full-screen player.
struct EpisodePickerView: View {
    let model: PlayerModel
    @Environment(\.dismiss) private var dismiss

    private var seasons: [(String, [MediaItem])] {
        var order: [String] = []
        var groups: [String: [MediaItem]] = [:]
        for ep in model.seriesEpisodes {
            let key = ep.seasonName ?? ep.seasonNumber.map { "Season \($0)" } ?? "Episodes"
            if groups[key] == nil { order.append(key) }
            groups[key, default: []].append(ep)
        }
        return order.map { ($0, groups[$0]!) }
    }

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                List {
                    ForEach(seasons, id: \.0) { season, episodes in
                        Section(season) {
                            ForEach(episodes) { ep in row(ep).id(ep.id) }
                        }
                    }
                }
                // Opens at the playing episode: once the list is laid out (scrolling on appear
                // came too early, it stayed at Season 1), and again if the episodes arrive later.
                .task(id: model.seriesEpisodes.count) {
                    try? await Task.sleep(for: .milliseconds(120))
                    if let id = model.item?.id { proxy.scrollTo(id, anchor: .center) }
                }
            }
            .navigationTitle(model.item?.seriesName ?? "Episodes")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } } }
        }
        #if os(macOS)
        // A sheet over the window: without a size it's a sliver (the list has none of its own).
        .frame(minWidth: EpisodeSheetSize.min.width, idealWidth: EpisodeSheetSize.ideal.width,
               minHeight: EpisodeSheetSize.min.height, idealHeight: EpisodeSheetSize.ideal.height)
        #endif
        .preferredColorScheme(.dark)
    }

    /// Picture, title, runtime and a short synopsis (tap: play it; long press: Mark as Watched /
    /// Unwatched), and Download in a column of its own at the trailing edge (it never plays).
    private func row(_ episode: MediaItem) -> some View {
        let ep = WatchState.shared.current(episode) // marked from its long-press menu
        let current = ep.id == model.item?.id
        return HStack(alignment: .center, spacing: 14) {
            Button { model.playEpisode(ep); dismiss() } label: {
                HStack(alignment: .top, spacing: 14) {
                    ZStack(alignment: .bottom) {
                        RemoteImage(url: model.client?.thumbURL(ep, maxWidth: 400))
                        if let p = ep.progress, !current { ProgressBar(value: p).padding(5) }
                    }
                    .frame(width: 128, height: 72)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay {
                        if current { RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(.white, lineWidth: 2) }
                    }
                    VStack(alignment: .leading, spacing: 3) {
                        Text(ep.episodeListTitle)
                            .font(.subheadline.weight(current ? .bold : .regular))
                            .foregroundStyle(.white)
                            .lineLimit(2)
                        HStack(spacing: 6) {
                            if current { Text("Now Playing") }
                            else if let r = Fmt.runtime(seconds: ep.runtime) { Text(r) }
                            if ep.user?.played == true { Image(systemName: "checkmark.circle.fill") }
                        }
                        .font(.caption)
                        .foregroundStyle(Theme.secondaryText)
                        if let text = ep.overview, !text.isEmpty {
                            Text(text)
                                .font(.caption)
                                .foregroundStyle(Theme.secondaryText)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                                .padding(.top, 1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .contextMenu { WatchedMenuItems(item: episode, client: model.client) }
            EpisodeDownloadControl(episode: ep, source: model.source)
        }
    }
}

/// Download in the player's episode list (the sheet): the title page's
/// round glass Download (its menu: quality; then pause / resume / delete), in a trailing column.
/// It never starts playback.
struct EpisodeDownloadControl: View {
    let episode: MediaItem
    let source: Source?
    /// The title page's Download beside a status line: a touch target.
    static let size: CGFloat = 44

    var body: some View {
        if DownloadManager.isSupported, let source, episode.isPlayable {
            DownloadButton(item: episode, source: source, size: Self.size)
                .frame(width: Self.size, height: Self.size)
                .glassEffect(.regular.interactive(), in: .circle)
                .foregroundStyle(.white)
                .buttonStyle(.plain)
        }
    }
}
