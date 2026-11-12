import Foundation

/// A title page made from what downloads saved: a download opens the normal title page,
/// and with no server that page is this: the title (a show: as its episodes' downloads described
/// it), seasons made from the downloaded episodes, and each season's downloaded episodes.
struct OfflineTitlePage: Equatable {
    var item: MediaItem
    var seasons: [MediaItem]
    /// The downloaded episodes that play from the device, by season id, in airing order.
    var episodesBySeason: [String: [MediaItem]]

    func episodes(season: MediaItem?) -> [MediaItem] {
        season.flatMap { episodesBySeason[$0.id] } ?? []
    }

    /// The season an episode is in on this page.
    func season(of episode: MediaItem?) -> MediaItem? {
        guard let episode else { return nil }
        return seasons.first { Self.seasonKey(episode) == $0.id }
    }

    /// The page of `itemId` (a show's, or a film's or video's own) from `records`; nil when none of
    /// its downloads plays from the device.
    static func build(itemId: String, records: [DownloadRecord]) -> OfflineTitlePage? {
        let playable = records.filter(\.isPlayable)
        let episodes = playable.filter { $0.seriesId == itemId }.compactMap(\.item)
            .sorted { MediaItem.order($0).lexicographicallyPrecedes(MediaItem.order($1)) }
        if let first = episodes.first {
            let show = playable.lazy.compactMap { $0.seriesId == itemId ? $0.series : nil }.first
                ?? MediaItem(id: itemId, kind: .series, name: first.seriesName, images: first.seriesImages ?? ImageSet())
            var seasons: [MediaItem] = []
            var bySeason: [String: [MediaItem]] = [:]
            for ep in episodes {
                let key = seasonKey(ep)
                if bySeason[key] == nil {
                    seasons.append(MediaItem(id: key, kind: .season, name: seasonName(ep), seriesId: itemId,
                                             seriesName: ep.seriesName, seasonNumber: ep.seasonNumber))
                }
                bySeason[key, default: []].append(ep)
            }
            return OfflineTitlePage(item: show, seasons: seasons, episodesBySeason: bySeason)
        }
        guard let rec = playable.first(where: { $0.itemId == itemId }), let item = rec.pageItem else { return nil }
        return OfflineTitlePage(item: item, seasons: [], episodesBySeason: [:])
    }

    /// The server's season id, else one made from the number.
    static func seasonKey(_ ep: MediaItem) -> String {
        ep.seasonId ?? "season:\(ep.seasonNumber ?? 0)"
    }

    private static func seasonName(_ ep: MediaItem) -> String {
        if let n = ep.seasonName, !n.isEmpty { return n }
        guard let s = ep.seasonNumber else { return "Episodes" }
        return s == 0 ? "Specials" : "Season \(s)"
    }
}

extension DownloadRecord {
    /// The title page that plays this download: the normal page, which starts playing it
    /// (from the device) as it opens. `startAt` nil: resume.
    func pageRoute(startAt: Double? = nil) -> Route? {
        pageItem.map { .item($0, autoplay: Autoplay(startAt: startAt)) }
    }

    /// The title as the download saved it, else as much as the record knows of it.
    var pageItem: MediaItem? {
        if let item { return item }
        guard let itemId else { return nil }
        return MediaItem(id: itemId, kind: .movie, name: title, productionYear: year)
    }

    /// Wide artwork saved with it (an episode's still, a film's backdrop), else its poster.
    var pageArtwork: URL? { backdropURL ?? posterURL }
}
