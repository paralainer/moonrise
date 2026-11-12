import Testing
@testable import Moonrise

struct SearchStartTests {
    private func ids(_ items: [MediaItem]) -> [String] { items.map(\.id) }

    @Test func openedTitleGoesFirstOnceAndLeavesSuggestions() {
        let s = SearchStart(recent: [Fixture.movie("a"), Fixture.movie("b")], suggestions: [Fixture.movie("c"), Fixture.movie("d")])
        let fromSuggestions = s.opened(Fixture.movie("c"))
        #expect(ids(fromSuggestions.recent) == ["c", "a", "b"])
        #expect(ids(fromSuggestions.suggestions) == ["d"])
        let again = s.opened(Fixture.movie("b"))
        #expect(ids(again.recent) == ["b", "a"])
        #expect(ids(again.suggestions) == ["c", "d"])
    }

    @Test func recentKeepsTheLatest() {
        var s = SearchStart()
        for n in 0..<(SearchStart.recentKept + 5) { s = s.opened(Fixture.movie("m\(n)")) }
        #expect(s.recent.count == SearchStart.recentKept)
        #expect(s.recent.first?.id == "m\(SearchStart.recentKept + 4)")
    }

    @Test func clearingRecentKeepsSuggestions() {
        let s = SearchStart(recent: [Fixture.movie("a")], suggestions: [Fixture.movie("b")]).recentCleared()
        #expect(s.recent.isEmpty)
        #expect(ids(s.suggestions) == ["b"])
        #expect(!s.isEmpty)
        #expect(SearchStart().isEmpty)
    }

    @Test func rowsOnlyWhileNothingIsTyped() {
        #expect(SearchStart.shows(query: ""))
        #expect(SearchStart.shows(query: "   "))
        #expect(!SearchStart.shows(query: "a"))
    }
}
