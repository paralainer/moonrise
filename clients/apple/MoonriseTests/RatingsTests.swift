// Ratings beside TMDB's on title pages: Moonrise/Server/ExternalRatings.swift.
import Foundation
import Testing
@testable import Moonrise

@Suite struct RatingsTests {
    /// An episode keeps its own IMDb rating; the critics' scores come from its show.
    @Test func anEpisodeTakesItsShowsCriticsScores() {
        let ep = ExternalRatings(imdb: 9.1, rottenTomatoes: nil, metacritic: nil, imdbVotes: 1200)
        let show = ExternalRatings(imdb: 8.5, rottenTomatoes: 96, metacritic: 81)
        let r = ExternalRatings.page(own: ep, show: show)
        #expect(r == ExternalRatings(imdb: 9.1, rottenTomatoes: 96, metacritic: 81, imdbVotes: 1200))
        // No IMDb rating of its own: none (not the show's).
        #expect(ExternalRatings.page(own: nil, show: show).imdb == nil)
    }

    /// The device keys older versions kept (an OMDb key, an OpenSubtitles account) are removed:
    /// the server does those lookups.
    @MainActor @Test func olderDeviceKeysAreForgotten() throws {
        let d = try #require(UserDefaults(suiteName: "moonrise.tests.keys"))
        d.set("k", forKey: "omdbKey"); d.set("k", forKey: "osApiKey"); d.set("u", forKey: "osUser"); d.set(true, forKey: "onlineSegments")
        AppSettings.forgetDeviceKeys(d)
        #expect(d.object(forKey: "omdbKey") == nil && d.object(forKey: "osApiKey") == nil && d.object(forKey: "osUser") == nil)
        #expect(d.bool(forKey: "onlineSegments"), "other settings stay")
        d.removePersistentDomain(forName: "moonrise.tests.keys")
    }

    /// Apple TV's Rating line: TMDB's star first, then each score there is; nothing: no line.
    @Test func theAppleTVLine() {
        let all = ExternalRatings(imdb: 8.1, rottenTomatoes: 94, metacritic: 78)
        #expect(ExternalRatings.line(tmdb: 7.9, all) == "★ 7.9   IMDb 8.1   RT 94%   MC 78")
        #expect(ExternalRatings.line(tmdb: 7.9, ExternalRatings()) == "★ 7.9", "as before")
        #expect(ExternalRatings.line(tmdb: nil, ExternalRatings(imdb: 6.0, rottenTomatoes: nil, metacritic: 55)) == "IMDb 6.0   MC 55")
        #expect(ExternalRatings.line(tmdb: nil, ExternalRatings()) == nil)
    }

    /// The item answer's `ratings` field becomes the item's (and an older server's answer, without
    /// it, decodes as before).
    @Test func decodesTheServersRatings() throws {
        let json = #"{"id": "m1", "kind": "movie", "title": "Film", "community_rating": 7.9,"#
            + #""ratings": {"imdb": {"value": 8.1, "votes": 123456}, "rotten_tomatoes": {"value": 94}, "metacritic": {"value": 78}}}"#
        let item = try MoonriseClient.decodeItem(Data(json.utf8))
        #expect(item.ratings == ExternalRatings(imdb: 8.1, rottenTomatoes: 94, metacritic: 78, imdbVotes: 123456))
        #expect(item.communityRating == 7.9)
        let old = try MoonriseClient.decodeItem(Data(#"{"id": "m1", "kind": "movie", "title": "Film"}"#.utf8))
        #expect(old.ratings == nil)
    }
}
