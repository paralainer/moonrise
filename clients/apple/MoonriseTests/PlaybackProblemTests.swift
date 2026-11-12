// How a playback failure is named: the shared cases (api/playback-problem-rules.json at
// the repository's root, copied into the test bundle by project.yml) that the Android apps
// (PlaybackProblemCasesTest.kt) and the Windows app (SharedCasesTests.cs) run too, and the Apple
// apps' own uses of the rule.
import Foundation
import Testing
@testable import Moonrise

private final class ProblemCasesBundleToken {}

@Suite struct PlaybackProblemTests {
    struct File: Decodable {
        struct Case: Decodable { let name: String; let status: Int?; let text: String; let pending: Bool?; let expected: String }
        let cases: [Case]
    }

    @Test func theSharedCases() throws {
        let url = try #require(Bundle(for: ProblemCasesBundleToken.self).url(forResource: "playback-problem-rules", withExtension: "json"),
                               "api/playback-problem-rules.json isn't in the test bundle (project.yml)")
        let file = try JSONDecoder().decode(File.self, from: Data(contentsOf: url))
        #expect(file.cases.count >= 15)
        for c in file.cases {
            let p = PlaybackProblem.classify(status: c.status, text: c.text, pending: c.pending ?? false)
            #expect(p.kind.rawValue == c.expected, Comment(rawValue: c.name))
        }
        // Every kind is in the cases.
        #expect(Set(file.cases.map(\.expected)) == Set(PlaybackProblem.Kind.allCases.map(\.rawValue)))
    }

    @Test func shortWordsWithTheServersTextAsDetails() {
        let raw = "probe: ffprobe: [matroska,webm @ 0x61e4] EBML header parsing failed\n/media/movies/Film.mkv: Invalid data found when processing input"
        let p = PlaybackProblem.classify(status: 422, text: "  \(raw)\n")
        #expect(p.title == "Damaged file")
        #expect(!p.message.contains("ffprobe") && !p.message.contains("/media/"), "the line is plain")
        #expect(p.detail == raw, "the server's words, trimmed, behind Details")
        #expect(PlaybackProblem.classify(status: 502, text: " ").detail == nil, "nothing to show: no Details")
        for kind in PlaybackProblem.Kind.allCases {
            let k = PlaybackProblem(kind: kind, detail: nil)
            #expect(k.title.count <= 24, "\(kind) title is short")
            #expect(k.message.count <= 110, "\(kind) line fits a small player")
        }
    }

    @Test func errorsFromAskingForAStream() {
        typealias E = MoonriseServer.OfferError
        #expect(PlaybackProblem(E.unreachable("The request timed out.")).kind == .unreachable)
        #expect(PlaybackProblem(E.notFound("no such item")).kind == .missing)
        #expect(PlaybackProblem(E.refused(422, "probe: ffprobe: moov atom not found")).kind == .damaged)
        #expect(PlaybackProblem(E.refused(500, "Error opening input: No such file or directory")).kind == .missing)
        #expect(PlaybackProblem(E.unsupported(["no keyframe index: no Cues found"], pending: false)).kind == .unindexed)
        #expect(PlaybackProblem(E.unsupported(["keyframe index pending (scanning the file)"], pending: true)).kind == .preparing)
        #expect(PlaybackProblem(E.notAsAsked).kind == .unsupported)
        #expect(PlaybackProblem(MoonriseServer.ServerError.http(503, "")).kind == .server)
        #expect(PlaybackProblem(URLError(.cannotConnectToHost)).kind == .unreachable)
    }

    @Test func aTitleWhoseFilesAreAllDamaged() {
        var bad = Fixture.version("a", broken: true)
        bad.brokenReason = "damaged or incomplete: truncated"
        let unreadable = { () -> MediaVersion in
            var v = Fixture.version("b", broken: true)
            v.brokenReason = "ffprobe: [matroska,webm @ 0x6] EBML header parsing failed"
            return v
        }()
        let healthy = Fixture.version("c")

        let one = Fixture.movie("m1", versions: [bad])
        #expect(one.allVersionsUnplayable)
        #expect(one.unplayableProblem?.kind == .damaged)
        #expect(one.unplayableProblem?.detail == "damaged or incomplete: truncated")

        #expect(Fixture.movie("m2", versions: [bad, unreadable]).unplayableProblem?.kind == .damaged)
        #expect(Fixture.movie("m3", versions: [bad, healthy]).unplayableProblem == nil, "a healthy version plays")
        #expect(Fixture.movie("m4", versions: [healthy]).unplayableProblem == nil)
        #expect(Fixture.movie("m5", versions: nil).unplayableProblem == nil, "a list item without its files says nothing")

        // No file found on disk at the last scan (the server lists them as missing).
        var missing = Fixture.movie("m6", versions: nil)
        missing.missingFiles = ["Film (2017).mkv"]
        #expect(missing.unplayableProblem?.kind == .missing)
        #expect(missing.unplayableProblem?.detail?.contains("Film (2017).mkv") == true)
        var stillOne = Fixture.movie("m7", versions: [healthy])
        stillOne.missingFiles = ["Film (2017).mkv"]
        #expect(stillOne.unplayableProblem == nil, "a file still there plays")

        // Broken with no reason, or a reason the rule doesn't know: still damaged.
        #expect(PlaybackProblem.broken(reason: nil).kind == .damaged)
        #expect(PlaybackProblem.broken(reason: "something odd").kind == .damaged)
        #expect(PlaybackProblem.broken(reason: "no keyframe index: no Cues found").kind == .unindexed)
    }
}
