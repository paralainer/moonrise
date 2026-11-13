// The API contract's example answers (api/examples/*.json at the repository's root, each
// `{summary, value}`; copied into the test bundle as the folder `examples`, project.yml) decode
// with the app's own models and decoders: a field renamed or retyped in the spec, or in a model,
// fails here. Every example is either decoded or listed in `notUsed` with the reason, so a new
// example fails until it is one or the other.
import Foundation
import Testing
@testable import Moonrise

private final class BundleToken {}

@MainActor
struct APIExamplesTests {
    typealias C = MoonriseClient
    typealias S = MoonriseServer

    /// The app's decoders: the client's and the playback layer's (snake_case keys), and the plain
    /// ones some answers are read with (their keys spelled out).
    private static let client = MoonriseClient.decoder
    private static let snake = MoonriseServer.snake
    private static let plain = JSONDecoder()

    private static func decodes<T: Decodable>(_ type: T.Type, _ decoder: JSONDecoder = client,
                                              check: @escaping (T) throws -> Void = { _ in }) -> (Data) throws -> Void {
        { data in try check(try decoder.decode(T.self, from: data)) }
    }

    /// Example file → how the app decodes that answer.
    private static let decoded: [String: (Data) throws -> Void] = [
        // Server and sign-in
        "info.json": decodes(C.Info.self) { #expect($0.unclaimed) },
        "sign-in.json": decodes(C.AuthResult.self),
        "me.json": decodes(C.Me.self),
        "device-code.json": decodes(C.DeviceCode.self),
        "device-poll-approved.json": decodes(C.DevicePollAnswer.self) { #expect($0.status == "approved" && $0.token != nil) },
        "setup-claim.json": decodes(C.AuthResult.self) { #expect($0.user.isAdmin == true) },
        // Browsing
        "libraries.json": decodes(RList<RLibrary>.self) { #expect(!$0.items.map(\.mediaItem).isEmpty) },
        "latest.json": decodes(RList<RItem>.self) { #expect(!$0.items.map(\.mediaItem).isEmpty) },
        "items.json": decodes(C.ItemsAnswer.self) { #expect(!$0.items.map(\.mediaItem).isEmpty) },
        "facets.json": decodes(C.FacetsAnswer.self),
        "item-movie.json": { _ = try MoonriseClient.decodeItem($0) },
        "item-episode.json": { _ = try MoonriseClient.decodeItem($0) },
        "seasons.json": decodes(RList<RItem>.self) { #expect(!$0.items.map(\.mediaItem).isEmpty) },
        "episodes.json": decodes(RList<RItem>.self) { #expect(!$0.items.map(\.mediaItem).isEmpty) },
        "search.json": decodes(C.TitleSearchAnswer.self) { _ = ($0.titles + $0.episodes).map(\.mediaItem) },
        "search-start.json": decodes(C.SearchStartAnswer.self) { #expect(!$0.recent.map(\.mediaItem).isEmpty && !$0.suggestions.map(\.mediaItem).isEmpty) },
        "home.json": decodes(C.HomeAnswer.self) { _ = $0.resume.map(\.mediaItem) },
        "show-continue.json": decodes(C.ContinueAnswer.self) { #expect($0.item != nil) },
        // Playback
        "playback.json": decodes(S.Playback.self, plain) {
            #expect($0.supported && $0.master != nil)
            #expect($0.tracks?.sameFile == true && $0.tracks?.audio?.index == 2 && $0.tracks?.subtitle?.language == "fr")
        },
        "segments.json": decodes(C.SegmentsAnswer.self),
        "segments-merged.json": decodes(C.SegmentsAnswer.self),
        "trickplay.json": decodes(ScrubThumbnails.Answer.self, plain),
        // Subtitles
        "subtitles.json": decodes(S.SubtitleList.self, snake) { #expect(!$0.subtitles.isEmpty) },
        "subtitle-cues.json": decodes(S.SubtitleWindow.self, snake),
        "subtitle-picture-cues.json": decodes(S.SubtitleWindow.self, snake),
        "subtitle-timing.json": decodes(S.SubtitleTiming.self, snake),
        "subtitle-search.json": decodes(S.SearchAnswer.self, snake),
        "subtitle-track.json": decodes(S.SubtitleTrack.self, snake),
        // Downloads
        "download-job.json": decodes(S.DownloadJob.self, snake),
        "offline.json": decodes(SegmentedDownloads.Set_.self, client),
        // Manage Server and the setup
        "admin-libraries.json": { data in
            let l = try client.decode(C.LibraryList.self, from: data)
            #expect(l.items.contains { lib in lib.roots.contains { root in root.source == "smb" && !root.online } })
            _ = try client.decode(C.AdminLibraryList.self, from: data)
        },
        "admin-library.json": decodes(C.AdminLibrary.self),
        "admin-fs-browse.json": decodes(C.FolderListing.self) { #expect(!$0.dirs.isEmpty) },
        "admin-smb-discover.json": decodes(C.FileServerList.self) { #expect(!$0.servers.isEmpty) },
        "admin-smb-shares.json": decodes(C.ShareList.self) { #expect(!$0.shares.isEmpty) },
        "admin-smb-browse.json": decodes(C.ShareListing.self),
        "admin-smb-test.json": decodes(C.ShareTest.self) { #expect($0.ok) },
        "admin-smb-failure.json": { data in
            // Read from the error's body, as the app does.
            let e = MoonriseClientError.http(502, String(decoding: data, as: UTF8.self))
            #expect(e.shareProblem?.problem == "unreachable")
        },
        "admin-scan-progress.json": decodes(C.ScanProgress.self) { #expect(!$0.libraries.isEmpty) },
        "admin-review.json": decodes(C.ReviewList.self) { #expect($0.items.contains { $0.isEpisode }) },
        "admin-match.json": decodes(C.MatchInfo.self) { #expect($0.fileList?.isEmpty == false) },
        "admin-match-search.json": decodes(C.MatchCandidateList.self) { #expect($0.items.first?.inLibrary != nil) },
        "admin-match-search-server-posters.json": decodes(C.MatchCandidateList.self) { #expect($0.items.first?.posterPath == "/example-poster.jpg") },
        "admin-match-episodes.json": decodes(C.TMDBSeason.self),
        "admin-identify.json": decodes(C.IdentifyAnswer.self) { #expect($0.item != nil) },
        "admin-users.json": decodes(C.AccountList.self) { #expect($0.items.contains { $0.me }) },
        "admin-devices.json": decodes(C.DeviceList.self),
        "admin-invite.json": decodes(C.Invite.self) { #expect($0.code != nil) },
        "admin-invites.json": decodes(C.InviteList.self),
        "admin-system.json": decodes(C.SystemInfo.self) { #expect($0.transcode != nil) },
        "admin-activity.json": decodes(C.Activity.self) { #expect(!$0.sessions.isEmpty) },
        "admin-logs.json": decodes(C.LogLines.self),
        "admin-services.json": decodes(C.ServiceList.self) { #expect(!$0.services.isEmpty) },
        "admin-settings.json": { data in
            // Read as JSON, as Manage Server does (`settingsDocument`).
            let d = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
            let features = (d["settings"] as? [String: Any])?["features"] as? [String: Any]
            #expect(features?["trickplay"] is Bool)
            let fields = try #require(d["fields"] as? [String: Any])
            #expect((fields["features.trickplay"] as? [String: Any])?["source"] is String)
        },
        "admin-llm.json": decodes(C.LLMOverview.self) { #expect(!$0.presets.isEmpty) },
        "admin-llm-models.json": decodes(C.LLMModelList.self),
        "admin-llm-test.json": decodes(C.LLMTestResult.self) { #expect(!$0.ok && $0.problem != nil) },
        "admin-chatgpt-start.json": decodes(C.ChatGPTStartAnswer.self) { #expect(URL(string: $0.url) != nil) },
        "admin-chatgpt-finish.json": decodes(C.ChatGPTSignedIn.self),
        "admin-chatgpt-sign-out.json": decodes(C.ChatGPTSignedOut.self) { #expect($0.revoked == true) },
    ]

    /// Examples of answers the app doesn't read, and why.
    private static let notUsed: [String: String] = [
        "report.json": "playback reports: the answer isn't read",
        "local-reports.json": "reports of downloads played: the answer isn't read",
        "continue-hidden.json": "Remove from Continue Watching: the answer isn't read",
        "played.json": "played marks: the answer isn't read",
        "search-recent-recorded.json": "a title opened from Search: the answer isn't read",
        "search-recent-cleared.json": "clearing Search's Recent: the answer isn't read",
        "version.json": "choosing a version: the answer isn't read",
        "download-jobs.json": "the list of prepared downloads isn't asked for (each job is)",
        "admin-user.json": "adding an account: the answer isn't read (the list is asked for again)",
        "admin-smb-credentials.json": "saved share logins aren't listed in the app",
        "admin-opensubtitles-check.json": "checking an account: only the status is read",
    ]

    private static func examples() throws -> URL {
        try #require(Bundle(for: BundleToken.self).url(forResource: "examples", withExtension: nil),
                     "api/examples isn't in the test bundle (project.yml)")
    }

    @Test func everyExampleDecodesOrIsListed() throws {
        let dir = try Self.examples()
        let files = try FileManager.default.contentsOfDirectory(atPath: dir.path).filter { $0.hasSuffix(".json") }.sorted()
        #expect(files.count > 60)
        for name in files {
            if Self.notUsed[name] != nil { continue }
            guard let decode = Self.decoded[name] else {
                Issue.record("\(name): not decoded and not listed as unused (APIExamplesTests)")
                continue
            }
            let file = try Data(contentsOf: dir.appendingPathComponent(name))
            let example = try #require(try JSONSerialization.jsonObject(with: file) as? [String: Any], "\(name)")
            let value = try #require(example["value"], "\(name): no value")
            let data = try JSONSerialization.data(withJSONObject: value)
            do {
                try decode(data)
            } catch {
                Issue.record("\(name): \(error)")
            }
        }
        for name in Set(Self.decoded.keys).union(Self.notUsed.keys) where !files.contains(name) {
            Issue.record("\(name): listed, but there is no such example")
        }
    }

    @Test func aRenamedFieldFails() {
        // The check itself: an answer whose field was renamed (`name` → `title`) doesn't decode.
        let broken = Data(#"{"server_id": "x", "title": "n", "version": "1", "capabilities": []}"#.utf8)
        #expect(throws: (any Error).self) { try Self.decodes(C.Info.self)(broken) }
    }
}
