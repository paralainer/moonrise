import Foundation
#if os(tvOS)
import TVServices
#endif

/// Keeps the Apple TV's Top Shelf (`MoonriseTopShelf`) in step with Home: Continue Watching is
/// saved where the extension reads them (`TopShelfSnapshot`), then the system is asked
/// to reload the shelf. Home refreshes on launch, when playback stops and when a title is marked
/// watched or unwatched, and each refresh updates the shelf. Signed out, or in the demo, the
/// snapshot is deleted so the shelf shows the static image. Other platforms: nothing.
@MainActor
enum TopShelfStore {
    /// At most this many cards.
    nonisolated static let limit = 20
    /// Card artwork width: the system's wide cards on a 4K screen.
    nonisolated static let imageWidth = 1100

    /// One card: the item and its progress (nil for a title not started).
    struct Card: Equatable {
        let item: MediaItem
        let progress: Double?
    }

    /// The shelf's cards: Home's Continue Watching, in its order. A title part-way through has
    /// its progress; a next episode or film none (watched before or not: a rewatch carries on).
    /// Anything that can't play is left out.
    nonisolated static func cards(from sections: [HomeSection]) -> [Card] {
        var seen = Set<String>()
        var out: [Card] = []
        for item in sections.first(where: { $0.id == HomeSection.continueID })?.items ?? [] {
            let resume = item.resumeSeconds != nil
            guard item.isPlayable, seen.insert(item.id).inserted else { continue }
            out.append(Card(item: item, progress: resume ? progress(item) : nil))
        }
        return Array(out.prefix(limit))
    }

    /// The card's title: the film, or "Show · S1 · E3" for an episode.
    nonisolated static func title(_ item: MediaItem) -> String {
        guard item.isEpisode else { return item.displayTitle }
        return [item.seriesName ?? item.displayTitle, item.episodeLabel].compactMap { $0 }.filter { !$0.isEmpty }
            .joined(separator: " · ")
    }

    nonisolated private static func progress(_ item: MediaItem) -> Double? {
        if let p = item.progress { return min(max(p, 0), 1) }
        if let t = item.resumeSeconds, let d = item.runtime, d > 0 { return min(max(t / d, 0), 1) }
        return nil
    }

    #if os(tvOS)
    /// Home loaded for `source`: save its Continue Watching for the shelf, and ask the
    /// system to reload the shelf when they changed.
    static func update(sections: [HomeSection], source: Source) {
        guard source.kind == .moonrise else { return }
        // Artwork by the server's LAN address (the Top Shelf loads it with no token, and the app's
        // current route may be a Tailscale forwarder that only lives while the app runs).
        let images = MoonriseClient.normalize(source.address).map { MoonriseClient(sourceId: nil, baseURL: $0, token: nil) }
        let list = cards(from: sections)
        let snapshot = TopShelfSnapshot(sections: list.isEmpty ? [] : [
            TopShelfSnapshot.Section(title: "Continue Watching", items: list.map { card in
                TopShelfSnapshot.Item(id: card.item.id, title: title(card.item), progress: card.progress,
                                      image: card.item.images.thumb.flatMap { ref in images?.imageURL(ref, width: imageWidth) })
            }),
        ])
        guard !snapshot.sections.isEmpty else { return clear() }
        guard TopShelfShared.load() != snapshot else { return }
        if TopShelfShared.save(snapshot) {
            TVTopShelfContentProvider.topShelfContentDidChange()
        }
    }

    /// No shelf content: signed out, another server, or the demo.
    static func clear() {
        guard TopShelfShared.load() != nil else { return }
        TopShelfShared.save(nil)
        TVTopShelfContentProvider.topShelfContentDidChange()
    }
    #else
    static func update(sections: [HomeSection], source: Source) {}
    static func clear() {}
    #endif
}
