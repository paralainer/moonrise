import SwiftUI

extension Notification.Name {
    /// A title was marked watched or unwatched (object: its id).
    static let watchStateDidChange = Notification.Name("MoonriseWatchStateDidChange")
}

/// Mark Watched / Unwatched from a card's long-press menu. Cards draw `current(item)`, so the
/// change shows at once on every card of that title (Home, Library, Search) without each screen
/// reloading: a mark applies to item values with the watch state it was made on, and stops
/// applying once a fresh one (with the server's new state) replaces them.
@MainActor @Observable
final class WatchState {
    static let shared = WatchState()

    private struct Mark { let before: UserState?; let played: Bool }
    private var marks: [String: Mark] = [:]

    /// `item` with any mark made on it applied: watched or not, no resume position.
    func current(_ item: MediaItem) -> MediaItem {
        guard let m = marks[item.id], m.before == item.user else { return item }
        var out = item
        var u = item.user ?? UserState()
        u.played = m.played
        u.position = 0
        u.playedPercentage = nil
        if item.isSeries || item.isSeason { u.unplayedItemCount = m.played ? 0 : nil }
        out.user = u
        return out
    }

    func isPlayed(_ item: MediaItem) -> Bool { current(item).user?.played == true }

    /// Marks `item` (a show or season: all its episodes) on the server. `item` as the card has
    /// it (not `current(item)`). False when the server refused: the mark is undone.
    @discardableResult
    func set(_ item: MediaItem, played: Bool, client: (any MediaServerClient)?) async -> Bool {
        guard let client else { return false }
        marks[item.id] = Mark(before: item.user, played: played)
        do {
            try await client.setPlayed(item.id, played)
            #if DEBUG
            fputs("MoonriseWatch: \(item.id) marked \(played ? "watched" : "unwatched")\n", stderr)
            #endif
            NotificationCenter.default.post(name: .watchStateDidChange, object: item.id)
            return true
        } catch {
            marks[item.id] = nil
            #if DEBUG
            fputs("MoonriseWatch: \(item.id) mark failed: \(error)\n", stderr)
            #endif
            return false
        }
    }
}

/// One Mark as Watched / Mark as Unwatched action.
struct WatchAction: Identifiable {
    let played: Bool
    var id: Bool { played }
    var title: String { played ? "Mark as Watched" : "Mark as Unwatched" }
    var icon: String { played ? "eye" : "eye.slash" }

    /// What a title's long-press menu offers: watched → Unwatched; part-way through (a resume
    /// position, or a show/season with some episodes watched) → both (Unwatched also clears the
    /// resume position); not started → Watched.
    @MainActor static func options(for item: MediaItem) -> [WatchAction] {
        let state = WatchState.shared
        let current = state.current(item)
        if current.user?.played == true { return [WatchAction(played: false)] }
        let started = (item.isSeries || item.isSeason)
            ? current.user?.lastPlayed != nil
            : (current.resumeSeconds != nil || current.progress != nil)
        return started ? [WatchAction(played: true), WatchAction(played: false)] : [WatchAction(played: true)]
    }
}

/// The Mark as Watched / Mark as Unwatched items of a title's long-press menu (every list of
/// titles and episodes uses these, the Apple TV's too).
struct WatchedMenuItems: View {
    let item: MediaItem
    let client: (any MediaServerClient)?
    /// After the server took it (e.g. the title page reloads its episodes).
    var done: (() async -> Void)? = nil

    var body: some View {
        ForEach(WatchAction.options(for: item)) { action in
            Button {
                Task {
                    await WatchState.shared.set(item, played: action.played, client: client)
                    await done?()
                }
            } label: { Label(action.title, systemImage: action.icon) }
        }
        #if !os(tvOS)
        FixMatchMenuItem(item: item)
        #endif
    }
}

extension View {
    /// Long press (Mac: right click): Mark as Watched / Unwatched.
    func watchedMenu(_ item: MediaItem, client: (any MediaServerClient)?) -> some View {
        contextMenu { WatchedMenuItems(item: item, client: client) }
    }
}
