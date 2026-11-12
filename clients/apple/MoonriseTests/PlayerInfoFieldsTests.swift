// The Apple TV player's Info tab: Moonrise/Player/PlayerInfoFields.swift, turned into
// AVKit metadata by TVNativePlayer.
import Foundation
import Testing
@testable import Moonrise

@Suite struct PlayerInfoFieldsTests {
    private func year(_ d: Date?) -> Int? {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        return d.map { utc.component(.year, from: $0) }
    }

    /// A film's heading is its title: its subtitle (only the year) is left out, and the year,
    /// genres and age rating are in the details.
    @Test func aFilmsHeadingIsItsTitle() {
        var film = Fixture.movie("m1")
        film.overview = "What happens."
        film.genres = ["Drama", "Mystery", "Thriller"]
        film.officialRating = "PG-13"
        film.productionYear = 2010
        let f = PlayerInfoFields.make(item: film, title: "Film m1", subtitle: "2010", details: film, nearEnd: false)
        #expect(f.title == "Film m1")
        #expect(f.subtitle == nil)
        #expect(f.overview == "What happens.")
        #expect(f.genres == "Drama, Mystery", "at most two")
        #expect(f.rating == "PG-13")
        #expect(year(f.releaseDate) == 2010)
    }

    /// An episode keeps its subtitle (the heading); rating and genres come from its show; the
    /// year is when it aired.
    @Test func anEpisodeHasItsShowsDetails() {
        var ep = Fixture.episode("e1", series: "s1", season: 2, number: 3)
        ep.premiereDate = "2015-01-01T00:00:00Z"
        ep.productionYear = 2014
        var show = Fixture.series("s1")
        show.genres = ["Comedy"]
        show.officialRating = "TV-14"
        let f = PlayerInfoFields.make(item: ep, title: "Show s1", subtitle: "S2, E3 · Episode 3", details: show, nearEnd: false)
        #expect(f.subtitle == "S2, E3 · Episode 3")
        #expect(f.genres == "Comedy" && f.rating == "TV-14")
        #expect(year(f.releaseDate) == 2015, "the air date, the same year in every time zone")
        let noShowYet = PlayerInfoFields.make(item: ep, title: "Show s1", subtitle: "S2, E3", details: nil, nearEnd: false)
        #expect(noShowYet.genres == nil && noShowYet.rating == nil, "until the show has been fetched")
        #expect(PlayerInfoFields.make(item: ep, title: "Show s1", subtitle: "", details: show, nearEnd: false).subtitle == nil)
    }

    /// Near the end: title and subtitle only (no Info tab, so Next Episode comes first).
    @Test func nearTheEndOnlyTheHeading() {
        var ep = Fixture.episode("e1", series: "s1")
        ep.overview = "What happens."
        ep.productionYear = 2014
        var show = Fixture.series("s1")
        show.genres = ["Comedy"]
        let f = PlayerInfoFields.make(item: ep, title: "Show s1", subtitle: "S1, E1", details: show, nearEnd: true)
        #expect(f == PlayerInfoFields(title: "Show s1", subtitle: "S1, E1"))
    }

    @Test func releaseDate() {
        var item = Fixture.movie("m2")
        #expect(PlayerInfoFields.releaseDate(item) == nil)
        item.productionYear = 1999
        #expect(year(PlayerInfoFields.releaseDate(item)) == 1999)
        item.premiereDate = "2000-12-31"
        #expect(year(PlayerInfoFields.releaseDate(item)) == 2000)
        item.premiereDate = "bad"
        #expect(year(PlayerInfoFields.releaseDate(item)) == 1999, "an unreadable date: the production year")
        #expect(PlayerInfoFields.releaseDate(nil) == nil)
    }
}
