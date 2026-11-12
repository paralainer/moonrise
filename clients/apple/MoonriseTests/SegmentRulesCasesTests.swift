// The segment rules' shared cases (api/segment-rules.json at the repository's root, copied into
// the test bundle by project.yml): the server applies the same rules for `segments?merged=1`
// (server/src/media/segment_rules.rs) and the Android app runs the same cases
// (clients/android/core/src/test/.../SegmentRulesCasesTest.kt).
import Foundation
import Testing
@testable import Moonrise

private final class CasesBundleToken {}

@Suite struct SegmentRulesCasesTests {
    struct File: Decodable {
        struct Seg: Decodable { let kind: String; let start: Double; let end: Double; let source: String? }
        struct Case: Decodable {
            let name: String
            let runtime: Double
            let movie: Bool?
            let pending: Bool?
            let community_pending: Bool?
            let server: [Seg]
            let online: [[Seg]]
            let expected: [Seg]
        }
        let cases: [Case]
    }

    /// As the client decodes the server's answer: only `detected` is kept as a source.
    private static func segments(_ list: [File.Seg]) -> [MediaSegment] {
        list.compactMap { s in
            MediaSegment.Kind(rawValue: s.kind).map { MediaSegment(kind: $0, start: s.start, end: s.end, source: s.source == "detected" ? "detected" : nil) }
        }
    }

    @Test func theSharedCases() throws {
        let url = try #require(Bundle(for: CasesBundleToken.self).url(forResource: "segment-rules", withExtension: "json"),
                               "api/segment-rules.json isn't in the test bundle (project.yml)")
        let file = try JSONDecoder().decode(File.self, from: Data(contentsOf: url))
        #expect(file.cases.count >= 10)
        for c in file.cases {
            let got = SegmentRules.merge(server: Self.segments(c.server), online: c.online.map(Self.segments), runtime: c.runtime,
                                         pending: c.pending ?? false, movie: c.movie ?? false,
                                         communityPending: c.community_pending ?? false)
            let want = Self.segments(c.expected)
            #expect(got.map { "\($0.kind.rawValue) \($0.start) \($0.end)" } == want.map { "\($0.kind.rawValue) \($0.start) \($0.end)" },
                    Comment(rawValue: c.name))
        }
    }
}

/// What the app does with the server's segments answer: a merged answer (servers 0.28+)
/// is offered as it is; an older server's is the file's own, put through the same rules without
/// any community database (the app asks no one else).
@Suite struct ServerSegmentsTests {
    private func answer(_ json: String) throws -> ServerSegments {
        MoonriseClient.serverSegments(try MoonriseClient.decoder.decode(MoonriseClient.SegmentsAnswer.self, from: Data(json.utf8)))
    }

    @Test func aMergedAnswerIsOfferedAsItIs() throws {
        let s = try answer(#"{"items": [{"kind": "intro", "start": 0, "end": 52, "source": "chapters"}, {"kind": "credits", "start": 6852, "end": 7210.828, "source": "theintrodb"}], "total": 2, "pending": false, "merged": true, "community_pending": true}"#)
        #expect(s.merged && s.communityPending && !s.pending)
        #expect(s.items.map(\.source) == [nil, nil])
        let offered = SegmentService.offered(s, runtime: 7268.64, provisional: false, movie: true)
        #expect(offered.map { "\($0.kind.rawValue) \($0.start) \($0.end)" } == ["intro 0.0 52.0", "credits 6852.0 7210.828"])
    }

    @Test func anOlderServersAnswerGetsTheRulesWithoutDatabases() throws {
        // A film's credits chapter running to the end isn't offered; the episode's found credits are.
        let film = try answer(#"{"items": [{"kind": "credits", "start": 8251.285, "end": 8978.528, "source": "chapters"}], "total": 1}"#)
        #expect(!film.merged && !film.communityPending)
        #expect(SegmentService.offered(film, runtime: 8978.528, provisional: false, movie: true).isEmpty)
        let episode = try answer(#"{"items": [{"kind": "credits", "start": 3180.1, "end": 3324.672, "source": "detected"}], "total": 1, "pending": false}"#)
        #expect(SegmentService.offered(episode, runtime: 3324.672, provisional: false, movie: false).map(\.start) == [3180.1])
        // Still looking in the file: nothing yet, unless provisional (then still nothing: no databases).
        let pending = try answer(#"{"items": [], "total": 0, "pending": true}"#)
        #expect(SegmentService.offered(pending, runtime: 3324.672, provisional: true, movie: false).isEmpty)
    }

    /// An answer that isn't final is asked for again on entering the last 10 minutes and
    /// 2 minutes before the end, once each; jumping straight to the last 2 minutes asks once.
    @Test func askedAgainNearTheEnd() {
        let d = 7268.64
        #expect(SegmentRules.nearEndAsksDue(duration: d, time: 0) == 0)
        #expect(SegmentRules.nearEndAsksDue(duration: d, time: d - 601) == 0)
        #expect(SegmentRules.nearEndAsksDue(duration: d, time: d - 600) == 1)
        #expect(SegmentRules.nearEndAsksDue(duration: d, time: d - 121) == 1)
        #expect(SegmentRules.nearEndAsksDue(duration: d, time: d - 100) == 2)
        #expect(SegmentRules.nearEndAsksDue(duration: 0, time: 100) == 0)
    }
}
