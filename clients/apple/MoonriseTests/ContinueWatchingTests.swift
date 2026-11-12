// Continue Watching: the row's threads, the device's plays laid over the server's cards,
// Home's reload when the app comes back, and the Apple TV Top Shelf's cards and deep links.
import Foundation
import Testing
@testable import Moonrise

@MainActor
@Suite(.serialized) struct ContinueWatchingTests {
    /// A card keeps its thread when another device moves it on to the next episode or
    /// film, so focus can follow it.
    @Test func threads() {
        #expect(Fixture.episode("e1", series: "s1").continueThread == "series:s1")
        #expect(Fixture.episode("e2", series: "s1", number: 2).continueThread == "series:s1")
        #expect(Fixture.series("s1").continueThread == "series:s1")
        #expect(Fixture.movie("m1", collection: 7).continueThread == "collection:7")
        #expect(Fixture.movie("m2", collection: 7).continueThread == "collection:7")
        #expect(Fixture.movie("m3").continueThread == "m3")
    }

    /// Home loads again when the app comes back, at most every 30 s, never while a load
    /// runs or before the first; at once after a failed load.
    @Test func homeReloadsWhenStale() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let interval = HomeStore.refreshInterval
        #expect(interval == 30)
        #expect(HomeStore.isStale(loading: false, lastLoad: now.addingTimeInterval(-interval - 1), failed: false, now: now))
        #expect(!HomeStore.isStale(loading: false, lastLoad: now.addingTimeInterval(-5), failed: false, now: now))
        #expect(HomeStore.isStale(loading: false, lastLoad: now.addingTimeInterval(-5), failed: true, now: now))
        #expect(!HomeStore.isStale(loading: true, lastLoad: now.addingTimeInterval(-600), failed: true, now: now))
        #expect(!HomeStore.isStale(loading: false, lastLoad: .distantPast, failed: false, now: now), "Home's first load is on its way")
        #expect(HomeStore.isStale(loading: false, lastLoad: .distantPast, failed: true, now: now))
    }

    /// Home stayed blank when the server answered with no rows.
    @Test func homeSaysSoOnlyWhenTheServerAnsweredWithNothing() {
        #expect(HomeStore.isEmpty(answered: true, loading: false, failed: false, rows: 0))
        #expect(!HomeStore.isEmpty(answered: false, loading: false, failed: false, rows: 0), "before the first answer")
        #expect(!HomeStore.isEmpty(answered: true, loading: true, failed: false, rows: 0), "while loading")
        #expect(!HomeStore.isEmpty(answered: true, loading: false, failed: true, rows: 0), "after a failure")
        #expect(!HomeStore.isEmpty(answered: true, loading: false, failed: false, rows: 1), "with rows")
    }

    /// The server's cards, one per title, latest first; this device's newer play of a title the
    /// server hasn't heard about yet is laid over it.
    @Test func localPlaysLayOverTheServersCards() {
        let source = UUID()
        defer { RecentsStore.forget(sources: [source]) }
        let old = Date(timeIntervalSinceNow: -3600)
        let s1e3 = Fixture.episode("e3", series: "s1", number: 3)
        let film = Fixture.movie("m1")
        let server: [HomeContent.ContinueCard] = [
            .init(item: s1e3, reason: .nextEpisode, at: old),
            .init(item: film, reason: .resume, at: old.addingTimeInterval(-60)),
            // A second card for the same show: only the first counts.
            .init(item: Fixture.episode("e9", series: "s1", number: 9), reason: .resume, at: old.addingTimeInterval(-120)),
        ]
        #expect(RecentsStore.merge(source: source, fetched: [], server: server).map(\.id) == ["e3", "m1"])

        // Played here just now (the server's lastPlayed is older): the film moves first, as played here.
        RecentsStore.record(source: source, itemId: "m1", titleId: "m1")
        var played = film
        played.user = UserState(position: 600, lastPlayed: old)
        #expect(RecentsStore.merge(source: source, fetched: [played], server: server).map(\.id) == ["m1", "e3"])

        // The server already knows this play: its card stands, in its place.
        played.user = UserState(position: 600, lastPlayed: .now.addingTimeInterval(5))
        #expect(RecentsStore.merge(source: source, fetched: [played], server: server).map(\.id) == ["e3", "m1"])

        // Finished here: what comes next is the server's to say.
        played.user = UserState(played: true, lastPlayed: old)
        #expect(RecentsStore.merge(source: source, fetched: [played], server: server).map(\.id) == ["e3", "m1"])
    }

    /// A server before 0.24.0: resume points, then next episodes of shows not already in progress.
    @Test func legacyCards() {
        let resume = [Fixture.episode("e5", series: "s1", number: 5, user: UserState(position: 100))]
        let nextUp = [Fixture.episode("e6", series: "s1", number: 6), Fixture.episode("x1", series: "s2")]
        #expect(RecentsStore.legacyCards(resume: resume, nextUp: nextUp).map(\.item.id) == ["e5", "x1"])
    }

    /// The Top Shelf: Continue Watching in order, playable items once each,
    /// progress only for a title part-way through, at most `limit` cards.
    @Test func topShelfCards() {
        let partWay = Fixture.episode("e1", series: "s1", runtime: 2000, user: UserState(position: 500))
        let next = Fixture.episode("e2", series: "s2")
        let rewatch = Fixture.movie("m1", runtime: 6000, user: UserState(position: 1500, played: true))
        let sections = [
            HomeSection(id: "latest", title: "Latest", style: .landscape, items: [Fixture.movie("other")]),
            HomeSection(id: HomeSection.continueID, title: "Continue Watching", style: .landscape,
                        items: [partWay, next, Fixture.series("s3"), partWay, rewatch]),
        ]
        let cards = TopShelfStore.cards(from: sections)
        #expect(cards.map(\.item.id) == ["e1", "e2", "m1"])
        #expect(cards[0].progress == 0.25)
        #expect(cards[1].progress == nil)
        #expect(cards[2].progress == 0.25, "a rewatch part-way through shows its progress")

        let many = (0..<30).map { Fixture.movie("m\($0)") }
        #expect(TopShelfStore.cards(from: [HomeSection(id: HomeSection.continueID, title: "", style: .landscape, items: many)]).count == TopShelfStore.limit)
        #expect(TopShelfStore.title(Fixture.episode("e1", series: "s1", season: 2, number: 3)) == "Show s1 · S2 · E3")
        #expect(TopShelfStore.title(Fixture.movie("m1")) == "Film m1")
    }

    /// The Top Shelf's deep links: open/play and back, any id, nothing else accepted.
    @Test func topShelfLinks() {
        let scheme = TopShelfShared.scheme
        for id in ["01abc", "a b/c?d#e", "ünï"] {
            #expect(TopShelfShared.Link(TopShelfShared.Link.open(id).url) == .open(id))
            #expect(TopShelfShared.Link(TopShelfShared.Link.play(id).url) == .play(id))
        }
        #expect(TopShelfShared.Link(URL(string: "\(scheme)://play/")!) == nil)
        #expect(TopShelfShared.Link(URL(string: "\(scheme)://delete/x")!) == nil)
        #expect(TopShelfShared.Link(URL(string: "other-\(scheme)://open/x")!) == nil)
        #expect(TopShelfShared.Link(URL(string: "https://example.com/open/x")!) == nil)
    }

    /// Down from the banner goes to the first card of the row below, if any.
    @Test func downFromTheBannerIsTheFirstCard() {
        let row = HomeSection(id: "latest", title: "Latest", style: .poster, items: [Fixture.movie("m1"), Fixture.movie("m2")])
        #expect(HomeFocus.firstLockup([row]) == "latest/m1")
        #expect(HomeFocus.firstLockup([]) == nil)
    }

    /// Removing a Continue Watching card from its menu on Apple TV moves focus to the card
    /// now in its place (the new last one at the end of the row), even when focus went with the
    /// removed card's view; nothing to follow without a focused or removed card.
    @Test func focusAfterRemovingAContinueCard() {
        let a = Fixture.movie("m1"), b = Fixture.episode("e1", series: "s1"), c = Fixture.movie("m3")
        let latest = HomeSection(id: "latest", title: "Latest", style: .poster, items: [Fixture.movie("m9")])
        func home(_ items: [MediaItem]) -> [HomeSection] {
            [HomeSection(id: HomeSection.continueID, title: "Continue Watching", style: .landscape, items: items), latest]
        }
        let before = home([a, b, c])
        #expect(HomeFocus.lockups(before[0]).map(\.id) == ["continue/m1", "continue/series:s1", "continue/m3"])
        // The middle card goes: its neighbour slides into its place.
        #expect(HomeFocus.target(focused: nil, removed: "continue/series:s1", old: before, new: home([a, c])) == "continue/m3")
        #expect(HomeFocus.target(focused: "continue/series:s1", removed: "continue/series:s1", old: before, new: home([a, c])) == "continue/m3")
        // The last card goes: the new last one.
        #expect(HomeFocus.target(focused: nil, removed: "continue/m3", old: before, new: home([a, b])) == "continue/series:s1")
        // Not gone yet (the local reload before the server's): it stays focused.
        #expect(HomeFocus.target(focused: nil, removed: "continue/m3", old: before, new: before) == "continue/m3")
        // The row's only card goes: the first card left on Home.
        #expect(HomeFocus.target(focused: nil, removed: "continue/m1", old: home([a]), new: [latest]) == "latest/m9")
        // Focus elsewhere wins over the removed card.
        #expect(HomeFocus.target(focused: "latest/m9", removed: "continue/m3", old: before, new: home([a, b])) == "latest/m9")
        #expect(HomeFocus.target(focused: nil, removed: nil, old: before, new: home([a])) == nil)
    }
}
