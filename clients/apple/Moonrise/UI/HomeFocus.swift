import Foundation

/// Apple TV Home's lockups and where focus goes when a refresh takes the focused one away,
/// or it was removed from Continue Watching with its menu. Outside the tvOS-only
/// views so the unit tests can run it.
enum HomeFocus {
    /// A lockup on Home, with the key that identifies its view and its focus.
    struct Lockup: Identifiable {
        let id: String
        let item: MediaItem
    }

    /// A row's lockups. Continue Watching keys its cards by thread (show, collection, lone film),
    /// so when a refresh moves a card on to the next episode or film, it stays the same view and
    /// keeps focus. Other rows key by title.
    static func lockups(_ section: HomeSection) -> [Lockup] {
        var seen = Set<String>()
        return section.items.map { item in
            let thread = section.id == HomeSection.continueID ? item.continueThread : item.id
            // Never two views with one key (a local overlay card can share a thread).
            let key = seen.insert(thread).inserted ? thread : item.id
            return Lockup(id: "\(section.id)/\(key)", item: item)
        }
    }

    static func lockups(_ sections: [HomeSection]) -> [Lockup] { sections.flatMap { lockups($0) } }

    /// The card Down from the banner lands on: the first of the row below it.
    static func firstLockup(_ sections: [HomeSection]) -> String? { lockups(sections).first?.id }

    /// Where focus goes when the focused lockup `key` is gone after a refresh: the card now in its
    /// place in the same row (the last one if the row got shorter), else the first card on Home.
    static func replacement(for key: String, old: [HomeSection], new: [HomeSection]) -> String? {
        if let row = old.first(where: { lockups($0).contains { $0.id == key } }),
           let index = lockups(row).firstIndex(where: { $0.id == key }),
           let now = new.first(where: { $0.id == row.id }).map({ lockups($0) }), !now.isEmpty {
            return now[min(index, now.count - 1)].id
        }
        return lockups(new).first?.id
    }

    /// The lockup to focus after a refresh: the focused one (or, when focus went with the view of
    /// a card just removed from its menu, that card) if it's still there, else the one now in its
    /// place. Nil when there is nothing to follow.
    static func target(focused: String?, removed: String?, old: [HomeSection], new: [HomeSection]) -> String? {
        guard let key = focused ?? removed else { return nil }
        return lockups(new).contains { $0.id == key } ? key : replacement(for: key, old: old, new: new)
    }
}
