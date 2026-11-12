import Foundation
import Testing
@testable import Moonrise

/// Apps talk only to the server, so a candidate's poster is never loaded from anywhere else.
struct FixMatchPosterTests {
    private let client = MoonriseClient(sourceId: nil, baseURL: URL(string: "http://192.168.1.10:8097")!, token: "fake-token")

    @Test func aPosterIsOnTheServer() throws {
        let u = try #require(client.matchPosterURL("/abc.jpg"))
        let c = try #require(URLComponents(url: u, resolvingAgainstBaseURL: false))
        #expect(c.host == "192.168.1.10")
        #expect(c.port == 8097)
        #expect(c.path == "/v1/admin/match/poster")
        #expect(c.queryItems?.first { $0.name == "path" }?.value == "/abc.jpg")
        #expect(c.queryItems?.first { $0.name == "size" }?.value == "w185")
    }

    @Test(arguments: [nil, "", "abc.jpg", "/abc", "/abc.svg", "/a/b.jpg", "/a.b.jpg", "/abc.jpg?x=1", "//evil.example/x.jpg",
                      "https://evil.example/x.jpg", "https://image.tmdb.org/t/p/w342/abc.jpg", "/../x.jpg", "/abc.jpg\n"] as [String?])
    func anythingElseIsNoPoster(_ path: String?) {
        #expect(client.matchPosterURL(path) == nil)
    }

    /// An older server's answer has only TMDB's address: the row has no poster, and nothing is loaded from TMDB.
    @Test func aCandidateWithOnlyATmdbAddressHasNoPoster() throws {
        let json = #"{"tmdb_id":1,"title":"A","poster":"https://image.tmdb.org/t/p/w342/abc.jpg"}"#
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        let c = try d.decode(MoonriseClient.MatchCandidate.self, from: Data(json.utf8))
        #expect(c.posterPath == nil)
        #expect(client.matchPosterURL(c.posterPath) == nil)
    }
}
