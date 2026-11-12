import Foundation
import Observation

/// What Search offers before anything is typed (`GET /v1/search/start`, capability `search-start`):
/// Recent, the titles opened from Search (on every device), and Suggestions from the library.
/// The same rules as Android's `SearchStart`.
struct SearchStart {
    static let recentKept = 20
    var recent: [MediaItem] = []
    var suggestions: [MediaItem] = []

    var isEmpty: Bool { recent.isEmpty && suggestions.isEmpty }

    /// A title was opened from Search: first in Recent at once, and no longer suggested.
    func opened(_ item: MediaItem) -> SearchStart {
        var s = self
        s.recent = Array(([item] + recent.filter { $0.id != item.id }).prefix(Self.recentKept))
        s.suggestions.removeAll { $0.id == item.id }
        return s
    }

    func recentCleared() -> SearchStart {
        var s = self
        s.recent = []
        return s
    }

    /// The rows are shown only while nothing is typed.
    static func shows(query: String) -> Bool {
        query.trimmingCharacters(in: .whitespaces).isEmpty
    }
}

/// Search's Recent and Suggestions, kept while a title page is on top (Back shows them at once).
@MainActor
@Observable
final class SearchStartStore {
    static let shared = SearchStartStore()

    /// nil: an older server (or not known yet).
    private(set) var start: SearchStart?
    /// Asked the server at least once.
    private(set) var loaded = false

    func load(_ client: (any MediaServerClient)?) async {
        guard let client else { start = nil; loaded = true; return }
        if let s = try? await client.searchStart() { start = s } else if !loaded { start = nil }
        loaded = true
    }

    /// A title was opened from Search (its page has appeared): first in Recent, and on the server
    /// for every device. Coming back to the page from the player doesn't count again.
    func opened(_ item: MediaItem, client: (any MediaServerClient)?) {
        guard let start, start.recent.first?.id != item.id else { return }
        self.start = start.opened(item)
        Task { try? await client?.recordSearchOpened(item.id) }
    }

    func clearRecent(client: (any MediaServerClient)?) {
        start = start?.recentCleared()
        Task { try? await client?.clearSearchRecent() }
    }
}
