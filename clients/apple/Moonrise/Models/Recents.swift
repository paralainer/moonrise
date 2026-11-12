import Foundation

/// Titles played in this app, most recent first, one entry per movie/show. Continue Watching
/// comes from the server (the same on every device); these only lay over it what this device
/// played that the server hasn't heard about yet (offline, or before its reports arrive), and
/// titles removed here before the server has confirmed it.
enum RecentsStore {
    struct Entry: Codable, Hashable {
        var sourceId: UUID
        /// Series id for episodes, item id otherwise. Nil for downloads that predate series tracking;
        /// those are folded in after the item is fetched.
        var titleId: String?
        var itemId: String
        var date: Date
    }

    private static let key = "recents.v1"
    private static let dismissedKey = "recents.dismissed.v1"
    private static let limit = 40
    private static let d = UserDefaults.standard

    static var entries: [Entry] {
        guard let data = d.data(forKey: key) else { return [] }
        return (try? JSONDecoder().decode([Entry].self, from: data)) ?? []
    }

    static func record(source: UUID, item: MediaItem) {
        record(source: source, itemId: item.id, titleId: item.isEpisode ? item.seriesId : item.id)
    }

    static func record(source: UUID, itemId: String, titleId: String?) {
        var list = entries.filter { e in
            !(e.sourceId == source && (e.itemId == itemId || (titleId != nil && e.titleId == titleId)))
        }
        list.insert(Entry(sourceId: source, titleId: titleId, itemId: itemId, date: .now), at: 0)
        save(Array(list.prefix(limit)))
        var dismissed = self.dismissed
        if dismissed.removeValue(forKey: titleId ?? itemId) != nil { d.set(dismissed, forKey: dismissedKey) }
    }

    /// Forgets what was played from these servers (dropped ones).
    static func forget(sources ids: Set<UUID>) {
        let list = entries
        let kept = list.filter { !ids.contains($0.sourceId) }
        if kept.count != list.count { save(kept) }
    }

    /// Hides a title from Continue Watching here, at once and until it's played again (the server
    /// is told too, for the other devices).
    static func remove(source: UUID, titleId: String) {
        save(entries.filter { !($0.sourceId == source && ($0.titleId == titleId || $0.itemId == titleId)) })
        var dismissed = self.dismissed
        dismissed[titleId] = Date.now.timeIntervalSince1970
        d.set(dismissed, forKey: dismissedKey)
    }

    /// titleId -> time it was removed.
    static var dismissed: [String: Double] {
        d.dictionary(forKey: dismissedKey) as? [String: Double] ?? [:]
    }

    /// Continue Watching: the server's cards, one per title, latest first, with this device's
    /// plays laid over them where they're newer than what the server knows of that title (its
    /// `lastPlayed`, fetched in `fetched`), minus titles removed here meanwhile.
    static func merge(source: UUID, fetched: [MediaItem], server: [HomeContent.ContinueCard]) -> [MediaItem] {
        let byId = Dictionary(fetched.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let dismissed = self.dismissed
        var order: [String] = []
        var picked: [String: (item: MediaItem, date: Date)] = [:]
        for card in server {
            let title = card.item.titleId
            if let hidden = dismissed[title], card.at.timeIntervalSince1970 <= hidden { continue }
            guard picked[title] == nil else { continue }
            picked[title] = (card.item, card.at)
            order.append(title)
        }
        var seen = Set<String>()
        for e in entries where e.sourceId == source {
            guard let item = byId[e.itemId], seen.insert(item.titleId).inserted else { continue }
            // Finished: nothing to resume, and what comes next is the server's to say.
            if item.user?.played == true, item.resumeSeconds == nil { continue }
            // The server already heard about this play (or a later one): its card stands.
            if let known = item.lastPlayed, known >= e.date.addingTimeInterval(-1) { continue }
            if let hidden = dismissed[item.titleId], e.date.timeIntervalSince1970 <= hidden { continue }
            if picked[item.titleId] == nil { order.append(item.titleId) }
            picked[item.titleId] = (item, e.date)
        }
        return order.compactMap { picked[$0] }
            .enumerated()
            .sorted { $0.element.date != $1.element.date ? $0.element.date > $1.element.date : $0.offset < $1.offset }
            .map(\.element.item)
    }

    /// A server before 0.24.0: its resume points (by when they were played) and then its next
    /// episodes, as cards.
    static func legacyCards(resume: [MediaItem], nextUp: [MediaItem]) -> [HomeContent.ContinueCard] {
        let inProgress = Set(resume.filter { $0.isEpisode && $0.user?.played != true }.compactMap(\.seriesId))
        let resumed = resume.map { HomeContent.ContinueCard(item: $0, reason: .resume, at: $0.lastPlayed ?? .distantPast) }
        let next = nextUp.filter { !inProgress.contains($0.seriesId ?? "") }
            .map { HomeContent.ContinueCard(item: $0, reason: .nextEpisode, at: .distantPast) }
        return resumed + next
    }

    private static func save(_ list: [Entry]) {
        d.set(try? JSONEncoder().encode(list), forKey: key)
    }
}
