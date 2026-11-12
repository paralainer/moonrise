import Foundation
import Observation

/// Home's content and loading, kept outside the view so a load isn't cancelled when the view
/// re-renders or the tab changes, and so a failed refresh keeps the last good shelves. The last
/// good result is also saved to disk and shown immediately on launch.
@MainActor
@Observable
final class HomeStore {
    static let shared = HomeStore()

    private(set) var sections: [HomeSection] = []
    private(set) var hero: [MediaItem] = []
    private(set) var loading = false
    /// The last load failed: a message for an error the server answered with, "" when it
    /// couldn't be reached. Nil when it loaded.
    private(set) var failure: String?
    /// The server has answered since sign-in, and what it said is what Home shows.
    private(set) var answered = false

    /// While Home is covered (a title page is pushed, another tab is open) results are held here
    /// and applied when Home is back, so the card a title page zooms back into is still in place.
    @ObservationIgnored private var pending: (sections: [HomeSection], hero: [MediaItem])?
    @ObservationIgnored var isVisible = true {
        didSet { if !isVisible { applyTask?.cancel() } }
    }
    @ObservationIgnored private var applyTask: Task<Void, Never>?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var again = false
    /// When the last load against a server started, for `refreshIfStale`.
    @ObservationIgnored private var lastLoad = Date.distantPast

    /// How often coming back to the app (or to Home) loads Home again.
    nonisolated static let refreshInterval: TimeInterval = 30

    private init() { restore() }

    /// Starts a load, or queues one more after the current one. Never cancels a load in flight.
    func reload() {
        if task != nil { again = true; return }
        task = Task {
            repeat {
                again = false
                await load()
            } while again
            task = nil
        }
    }

    /// The app came back to the front, or Home is on screen again: load again so what was played
    /// on another device meanwhile shows up (Continue Watching, the Top Shelf). Not while a load is
    /// running, and at most every `refreshInterval` seconds unless the last load failed.
    func refreshIfStale() {
        guard Self.isStale(loading: task != nil, lastLoad: lastLoad, failed: failure != nil, now: .now) else { return }
        reload()
    }

    /// Whether `refreshIfStale` loads again: not while a load runs, nor before the first
    /// one (Home's own first load is on its way), and at most every `refreshInterval` seconds
    /// unless the last load failed.
    nonisolated static func isStale(loading: Bool, lastLoad: Date, failed: Bool, now: Date) -> Bool {
        guard !loading, lastLoad != .distantPast || failed else { return false }
        if !failed, now.timeIntervalSince(lastLoad) < refreshInterval { return false }
        return true
    }

    /// Whether Home says it has nothing to show: the server has answered with no rows. Not
    /// before the first answer, during a load (the spinner) or after a failure (its own message).
    nonisolated static func isEmpty(answered: Bool, loading: Bool, failed: Bool, rows: Int) -> Bool {
        answered && !loading && !failed && rows == 0
    }

    func reloadAndWait() async {
        reload()
        await task?.value
    }

    private func load() async {
        loading = true
        defer { loading = false }
        await ConnectionRouter.shared.ready()
        guard let server = SourceStore.shared.server, let client = MediaServers.client(for: server) else { return }
        lastLoad = .now
        var loaded: ([HomeSection], [MediaItem])?
        do {
            loaded = try await Self.fetch(source: server, client: client)
        } catch where error.isCancellation {
            return
        } catch {
            // Maybe the route went stale (left home Wi-Fi): re-check once and retry before reporting.
            if error.isConnectivity {
                await ConnectionRouter.shared.refresh()
                if ConnectionRouter.shared.route != .unreachable {
                    loaded = try? await Self.fetch(source: server, client: client)
                }
            }
            if loaded == nil { failure = error.isConnectivity ? "" : error.friendlyMessage }
        }
        // Signed out (or into another server) meanwhile: these results are stale.
        guard SourceStore.shared.server?.id == server.id else { return }
        // A failed load keeps the previous shelves.
        guard let (newSections, allHero) = loaded else { return }
        failure = nil
        let newHero = Array(allHero.prefix(6))
        if isVisible {
            // Unchanged (a refresh on coming back to the app): leave the views, focus and scroll alone.
            if sections != newSections { sections = newSections }
            if hero != newHero { hero = newHero }
        } else if pending != nil || sections != newSections || hero != newHero {
            pending = (newSections, newHero)
        }
        if pending == nil { answered = true }
        persist(server.id, newSections, newHero)
        prefetchHeaders(client, newSections, newHero)
        // The Apple TV's Top Shelf shows its Continue Watching.
        TopShelfStore.update(sections: newSections, source: server)
    }

    private static func fetch(source: Source, client: any MediaServerClient) async throws -> ([HomeSection], [MediaItem]) {
        var out: [HomeSection] = []
        var heroOut: [MediaItem] = []
        let recentIds = RecentsStore.entries.filter { $0.sourceId == source.id }.map(\.itemId)
        async let played = client.items(ids: recentIds)
        let home = try await client.home()
        // One row, the server's (the same on every device), with this device's newest plays laid over it.
        let cards = home.continueWatching ?? RecentsStore.legacyCards(resume: home.resume, nextUp: home.nextUp)
        let row = RecentsStore.merge(source: source.id, fetched: (try? await played) ?? [], server: cards)
        if !row.isEmpty { out.append(HomeSection(id: HomeSection.continueID, title: "Continue Watching", style: .landscape, items: row)) }

        for row in home.latest {
            let view = row.library, latest = row.items
            guard !latest.isEmpty else { continue }
            // By type across libraries ("Recently Added Movies", "Recently Added Shows").
            out.append(HomeSection(id: "latest-\(view.id)", title: "Recently Added \(view.displayTitle)", style: .poster, items: latest))
            if heroOut.count < 6 {
                heroOut += latest.filter { !$0.isEpisode && !$0.isSeason && $0.images.backdrop != nil }.prefix(3)
            }
        }
        return (out, heroOut)
    }

    /// Home is on screen again: apply held results once the zoom back into its card has finished.
    func homeAppeared(apply: @escaping @MainActor ([HomeSection], [MediaItem]) -> Void) {
        isVisible = true
        applyTask?.cancel()
        applyTask = Task {
            try? await Task.sleep(for: .milliseconds(700))
            guard !Task.isCancelled else { return }
            if let p = pending {
                pending = nil
                apply(p.sections, p.hero)
            }
            // Back from a title page: catch up with other devices too (throttled).
            refreshIfStale()
        }
    }

    func setContent(_ sections: [HomeSection], _ hero: [MediaItem]) {
        self.sections = sections
        self.hero = hero
        answered = true
    }

    /// Signed out: nothing of that server's Home stays (on screen or on disk).
    func clear() {
        pending = nil
        lastLoad = .distantPast
        answered = false
        sections = []
        hero = []
        failure = nil
        try? FileManager.default.removeItem(at: cacheURL)
        TopShelfStore.clear()
    }

    /// Warm the image cache with the page headers of the titles you're most likely to open
    /// (Continue Watching, banner), so the zoom into a title page lands on its artwork.
    private func prefetchHeaders(_ client: any MediaServerClient, _ sections: [HomeSection], _ hero: [MediaItem]) {
        var urls: [URL] = []
        for section in sections where section.style == .landscape {
            for item in section.items.prefix(10) {
                let page = item.isEpisode || item.isSeason ? ItemDetailView.showStub(for: item) : item
                if let u = client.backdropURL(page) { urls.append(u) }
                if let u = client.logoURL(page) { urls.append(u) }
            }
        }
        for item in hero {
            if let u = client.logoURL(item) { urls.append(u) }
            #if os(tvOS)
            // The TV showcase slides between these backdrops; on disk they're there for the first frame.
            if let u = client.backdropURL(item, maxWidth: 1920) { urls.append(u) }
            #endif
        }
        Task.detached(priority: .utility) {
            for u in urls { await ImageCache.shared.prefetch(u) }
        }
    }

    // MARK: Disk cache

    /// The saved items' model: bump when `MediaItem` changes incompatibly. (Older versions:
    /// `home.json` and `home-v2.json`, deleted on launch.)
    private static let cacheVersion = 3

    private struct Saved: Codable {
        struct Section: Codable { let id, title: String; let landscape: Bool; let items: [MediaItem] }
        let version: Int
        /// The server it came from.
        let sourceId: UUID
        let sections: [Section]
        let hero: [MediaItem]
    }

    private static var cachesDirectory: URL { FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0] }

    private var cacheURL: URL { Self.cachesDirectory.appendingPathComponent("home-v\(Self.cacheVersion).json") }

    private func persist(_ sourceId: UUID, _ sections: [HomeSection], _ hero: [MediaItem]) {
        let saved = Saved(
            version: Self.cacheVersion, sourceId: sourceId,
            sections: sections.map { .init(id: $0.id, title: $0.title, landscape: $0.style == .landscape, items: $0.items) },
            hero: hero)
        try? JSONEncoder().encode(saved).write(to: cacheURL, options: .atomic)
    }

    private func restore() {
        // Caches of older models.
        for old in ["home.json", "home-v2.json"] {
            let url = Self.cachesDirectory.appendingPathComponent(old)
            if FileManager.default.fileExists(atPath: url.path) { try? FileManager.default.removeItem(at: url) }
        }
        guard let data = try? Data(contentsOf: cacheURL),
              let saved = try? JSONDecoder().decode(Saved.self, from: data), saved.version == Self.cacheVersion,
              saved.sourceId == SourceStore.shared.server?.id else { return }
        sections = saved.sections.compactMap { s in
            // Saved by a version with Recents and Next Up: Recents stands in for Continue Watching
            // until the first load.
            if s.id == "next" { return nil }
            let (id, title) = s.id == "recents" ? (HomeSection.continueID, "Continue Watching") : (s.id, s.title)
            return HomeSection(id: id, title: title, style: s.landscape ? .landscape : .poster, items: s.items)
        }
        hero = saved.hero
    }
}
