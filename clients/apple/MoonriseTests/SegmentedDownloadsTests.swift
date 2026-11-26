// Playing a segmented download while it still downloads (Moonrise/Downloads/SegmentedDownloads.swift):
// which segment the download fetches next, from what the player asks the local server for.
import Foundation
import Testing
@testable import Moonrise

@MainActor
struct SegmentedDownloadsTests {
    typealias Position = SegmentedDownloads.PlayerPosition

    /// The segment the download fetches next for a player that asked for `paths` (in this order).
    private func next(after paths: [String], have: IndexSet, count: Int = 120) -> Int? {
        var p = Position()
        for path in paths { p.asked(path) }
        return SegmentedDownloads.nextSegment(have: have, count: count, from: p.from(have: have) ?? 0)
    }

    @Test func aSeekPastTheDownloadedPartFetchesTheVideoSegmentThePlayerWaitsForFirst() {
        // Downloaded up to segment 66; the viewer seeks to 10:00. AVPlayer gets video 100 and
        // audio 100-102, waits for video 101 and asks for audio 103 meanwhile: video 101 comes
        // first, not 103 on (the newest request), which left 101 for the very end.
        let have = IndexSet(0..<67).union(IndexSet(integer: 100))
        let asked = ["video/100.m4s", "audio/1/100.m4s", "audio/1/101.m4s", "audio/1/102.m4s", "video/101.m4s", "audio/1/103.m4s"]
        #expect(next(after: asked, have: have) == 101)
    }

    @Test func afterACutTheSegmentTheVideoStoppedAtComesFirst() {
        // The link went during video 70; the audio had asked for 71 and 72 already. Once it is
        // back, the download starts at 70, not after the audio's 72 (which put 70 at the very end).
        let have = IndexSet(0..<70)
        let asked = ["video/69.m4s", "audio/1/70.m4s", "audio/1/71.m4s", "video/70.m4s", "audio/1/72.m4s"]
        #expect(next(after: asked, have: have) == 70)
    }

    @Test func aSegmentThePlayerGaveUpOnComesBeforeTheOnesItAskedForAfter() {
        // During a cut AVPlayer gave up on video 27 and went on to ask for 28-30; it stalls at 27.
        let have = IndexSet(0...26).union(IndexSet(integer: 29))
        let asked = ["video/26.m4s", "audio/1/27.m4s", "video/27.m4s", "audio/1/28.m4s", "video/28.m4s", "audio/1/29.m4s",
                     "video/29.m4s", "audio/1/30.m4s", "video/30.m4s"]
        #expect(next(after: asked, have: have) == 27)
    }

    @Test func anOldRequestDoesNotPullTheDownloadBack() {
        // Asked for 5 long ago (the viewer was there, then seeked ahead): it doesn't come first.
        var p = Position()
        let t0 = Date()
        p.asked("video/5.m4s", at: t0)
        p.asked("video/80.m4s", at: t0.addingTimeInterval(200))
        let have = IndexSet(0..<4)
        #expect(p.from(have: have, at: t0.addingTimeInterval(201)) == 80)
        #expect(p.from(have: IndexSet(0..<80), at: t0.addingTimeInterval(201)) == 80)
        #expect(p.from(have: IndexSet(0...80), at: t0.addingTimeInterval(201)) == 80)
        // A seek ahead a moment after the player gave up on 11: the download goes to 100.
        var q = Position()
        q.asked("video/11.m4s", at: t0)
        q.asked("video/100.m4s", at: t0.addingTimeInterval(3))
        q.asked("audio/1/100.m4s", at: t0.addingTimeInterval(3))
        #expect(q.from(have: IndexSet(0..<11), at: t0.addingTimeInterval(4)) == 100)
    }

    @Test func segmentsTheDeviceHasAreSkipped() {
        #expect(next(after: ["video/10.m4s", "audio/1/12.m4s"], have: IndexSet(0..<15)) == 15)
    }

    @Test func pastTheEndItGoesOnFromTheStart() {
        #expect(SegmentedDownloads.nextSegment(have: IndexSet(5..<120), count: 120, from: 30) == 0)
        #expect(SegmentedDownloads.nextSegment(have: IndexSet(0..<120), count: 120, from: 30) == nil)
        #expect(SegmentedDownloads.nextSegment(have: IndexSet(), count: 120, from: 500) == 0)
    }

    @Test func nothingAskedStartsFromTheBeginning() {
        #expect(Position().segment() == nil)
        #expect(next(after: [], have: IndexSet(0..<3)) == 3)
        // Playlists and init sections say nothing about where the player is.
        #expect(next(after: ["video/init.mp4", "video/index.m3u8"], have: IndexSet()) == 0)
    }

    @Test func aTrackNoLongerPlayedDoesNotHoldTheDownloadBack() {
        // Audio track 1 played up to 20, then another track was picked; the viewer is at 80 now.
        var p = Position()
        let t0 = Date()
        p.asked("audio/1/20.m4s", at: t0)
        p.asked("video/80.m4s", at: t0.addingTimeInterval(60))
        p.asked("audio/2/81.m4s", at: t0.addingTimeInterval(60))
        #expect(p.segment() == 80)
    }

    @Test func aPlayerWaitingForASegmentCountsAsWatchingHoweverLong() {
        // While it waits, AVPlayer asks for nothing else: the download must keep to one fetch at a
        // time so the waiting request has the link, not fan out because no request came lately.
        var p = Position()
        let t0 = Date()
        p.asked("video/70.m4s", at: t0)
        #expect(p.active(at: t0.addingTimeInterval(5)))
        #expect(!p.active(at: t0.addingTimeInterval(60)))
        p.waits(true)
        #expect(p.active(at: t0.addingTimeInterval(60)))
        p.waits(false)
        #expect(!p.active(at: t0.addingTimeInterval(60)))
        // Unbalanced ends never go below nothing waiting.
        p.waits(false)
        p.waits(true)
        #expect(p.active(at: t0.addingTimeInterval(60)))
    }

    /// A media playlist of `n` segments of 6 s (the last one 4 s).
    private func playlist(_ n: Int) -> String {
        var lines = ["#EXTM3U", "#EXT-X-TARGETDURATION:6", "#EXT-X-PLAYLIST-TYPE:VOD", "#EXT-X-MAP:URI=\"init.mp4\""]
        for i in 0..<n { lines += ["#EXTINF:\(i == n - 1 ? "4.000" : "6.000"),", "\(i).m4s"] }
        return (lines + ["#EXT-X-ENDLIST"]).joined(separator: "\n")
    }

    @Test func aPlayerStalledAtTheEndOfASegmentNeedsTheNextOne() {
        // Stalled at 191.9 (segment 31 ends at 192): it waits for 32. When that is on the device,
        // playback starts again at once instead of after several nudges AVPlayer ignores.
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(120), at: 191.9) == [31, 32])
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(120), at: 189) == [31])
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(120), at: 0) == [0])
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(120), at: 192) == [32])
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(3), at: 11.5) == [1, 2])
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(3), at: 15.5) == [2])
        #expect(SegmentedDownloads.segmentsNeeded(playlist: playlist(3), at: 16) == [])
    }

    // MARK: - A bad link (`Retry`)

    typealias Retry = SegmentedDownloads.Retry
    typealias Failure = SegmentedDownloads.Failure

    @Test func aDroppedConnectionIsTriedAgainAtOnce() async throws {
        // The link drops the connection of one file twice: the third try gets it, after two short
        // pauses, and the run goes on without waiting.
        var tries = 0
        var pauses: [Double] = []
        let got = try await Retry.file(pause: { pauses.append($0) }) {
            tries += 1
            if tries < 3 { throw Failure.transient("The network connection was lost.") }
            return "file"
        }
        #expect(got == "file")
        #expect(tries == 3)
        #expect(pauses == [0.5, 1.5])
    }

    @Test func aFileThatKeepsFailingGivesUpAfterAFewTries() async {
        var tries = 0
        await #expect(throws: Failure.self) {
            try await Retry.file(pause: { _ in }) { () async throws -> Int in
                tries += 1
                throw Failure.transient("unreachable")
            }
        }
        #expect(tries == Retry.fileTries)
        // A permanent failure (the sign-in expired, the title is gone) isn't tried again.
        tries = 0
        await #expect(throws: Failure.self) {
            try await Retry.file(pause: { _ in }) { () async throws -> Int in
                tries += 1
                throw Failure.permanent("gone")
            }
        }
        #expect(tries == 1)
    }

    @Test func theWaitGrowsOnlyWhileNothingGetsThrough() {
        var r = Retry()
        // The server can't be reached: 2, 4, 8, 15, 15 s; "Waiting for the server…" from the second.
        let down = (0..<5).map { _ in r.failed(progressed: false) }
        #expect(down.map(\.seconds) == [2, 4, 8, 15, 15])
        #expect(down.map(\.waiting) == [false, true, true, true, true])
        // Segments arrived again, then a connection dropped: the shortest wait, and no
        // "Waiting for the server…" (it used to be 15 s for every drop from then on).
        let blip = r.failed(progressed: true)
        #expect(blip.seconds == 2)
        #expect(!blip.waiting)
        #expect(r.failed(progressed: true).seconds == 2)
        // Nothing since: it grows again.
        let again = r.failed(progressed: false)
        #expect(again.seconds == 4)
        #expect(again.waiting)
    }

    @Test func aDownloadDeletedWhileAFileIsInFlightLeavesNoFolder() async throws {
        // A segment is on its way when the viewer deletes the download. It must not make the
        // download's folder again when it arrives (the space would never come back).
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [HeldResponse.self]
        let downloads = SegmentedDownloads(root: root, session: URLSession(configuration: config))
        var rec = DownloadRecord(kind: .segments, title: "Test", sourceId: UUID(), qualityLabel: "480p", fileExtension: "mp4")
        rec.setBase = "/v1/items/abc/offline/h264-854x480-1500k"
        let client = MoonriseClient(sourceId: nil, baseURL: URL(string: "http://192.168.1.10:8097")!, token: "fake-token")
        let folder = root.appendingPathComponent(rec.id.uuidString)
        try FileManager.default.createDirectory(at: downloads.hlsDirectory(rec.id), withIntermediateDirectories: true)
        HeldResponse.reset()

        let fetch = Task { try await downloads.fetch(rec, "video/7.m4s", client: client, base: client.baseURL) }
        try await HeldResponse.waitForRequest()
        // Delete, as DownloadManager.delete does it.
        downloads.forget(rec.id)
        try FileManager.default.removeItem(at: folder)
        // The request is cancelled; were it not, its answer arrives now.
        let cancelled = await HeldResponse.waitForCancel()
        HeldResponse.release()
        _ = try? await fetch.value

        #expect(cancelled)
        #expect(!FileManager.default.fileExists(atPath: folder.path))
    }

    @Test func onMobileDataWithTheSettingOffADownloadWaitsForWiFi() {
        let may = SegmentedDownloads.mayFetch
        #expect(may(false, false))  // Wi-Fi
        #expect(may(false, true))
        #expect(!may(true, false))  // mobile data, "Download over Cellular" off
        #expect(may(true, true))    // mobile data, on
    }

    @Test func aDownloadThroughTheTailscaleForwarderOnMobileDataFetchesNothingUntilAllowed() async throws {
        // Away from home the server is reached through the Tailscale forwarder on 127.0.0.1,
        // which the system never counts as cellular: the download itself must hold back. And
        // turning the setting on applies at once (the session used to be built once, with the
        // setting as it was at launch).
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [CountedResponse.self]
        let downloads = SegmentedDownloads(root: root, session: URLSession(configuration: config))
        var allowed = false
        downloads.onCellular = { true }
        downloads.allowCellular = { allowed }
        var rec = DownloadRecord(kind: .segments, title: "Test", sourceId: UUID(), qualityLabel: "480p", fileExtension: "mp4")
        rec.setBase = "/v1/items/abc/offline/h264-854x480-1500k"
        let client = MoonriseClient(sourceId: nil, baseURL: URL(string: "http://127.0.0.1:41641")!, token: "fake-token")
        let path = "video/\(Int.random(in: 1000..<100_000)).m4s"
        try FileManager.default.createDirectory(at: downloads.hlsDirectory(rec.id), withIntermediateDirectories: true)

        do {
            try await downloads.fetch(rec, path, client: client, base: client.baseURL)
            Issue.record("fetched over mobile data with Download over Cellular off")
        } catch SegmentedDownloads.Failure.waitingForWiFi {
        } catch {
            Issue.record("unexpected \(error)")
        }
        #expect(CountedResponse.requests(path) == 0)
        #expect(!downloads.exists(rec.id, path))

        allowed = true
        #expect(try await downloads.fetch(rec, path, client: client, base: client.baseURL))
        #expect(CountedResponse.requests(path) == 1)
        #expect(downloads.exists(rec.id, path))
    }

    @Test func foldersNoDownloadNamesAreFoundAtLaunch() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let kept = UUID(), left = UUID()
        let fm = FileManager.default
        for d in [kept, left] { try fm.createDirectory(at: root.appendingPathComponent("\(d.uuidString)/hls"), withIntermediateDirectories: true) }
        try fm.createDirectory(at: root.appendingPathComponent("other"), withIntermediateDirectories: true)
        try Data("[]".utf8).write(to: root.appendingPathComponent("index.json"))
        try Data().write(to: root.appendingPathComponent(UUID().uuidString))

        let orphans = DownloadManager.orphanFolders(in: root, keeping: [kept])
        #expect(orphans.map(\.lastPathComponent) == [left.uuidString])
    }
}

/// A server that holds its answer until the test lets it go (`release`).
private final class HeldResponse: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var current: HeldResponse?
    nonisolated(unsafe) private static var stopped = false

    static func reset() { lock.withLock { current = nil; stopped = false } }
    static func waitForCancel() async -> Bool {
        for _ in 0..<100 {
            if lock.withLock({ stopped }) { return true }
            try? await Task.sleep(for: .milliseconds(20))
        }
        return false
    }

    static func waitForRequest() async throws {
        for _ in 0..<200 {
            if lock.withLock({ current != nil }) { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        throw URLError(.timedOut)
    }

    /// The answer arrives (unless the request was cancelled meanwhile).
    static func release() {
        guard let p = lock.withLock({ stopped ? nil : current }) else { return }
        let resp = HTTPURLResponse(url: p.request.url!, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: nil)!
        p.client?.urlProtocol(p, didReceive: resp, cacheStoragePolicy: .notAllowed)
        p.client?.urlProtocol(p, didLoad: Data(repeating: 1, count: 1024))
        p.client?.urlProtocolDidFinishLoading(p)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { Self.lock.withLock { Self.current = self } }
    override func stopLoading() { Self.lock.withLock { Self.stopped = true } }
}

/// A server that answers every request at once, and counts them by path.
private final class CountedResponse: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var seen: [String: Int] = [:]

    /// Requests whose URL path ends with `suffix`.
    static func requests(_ suffix: String) -> Int {
        lock.withLock { seen.filter { $0.key.hasSuffix(suffix) }.values.reduce(0, +) }
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.lock.withLock { Self.seen[request.url!.path, default: 0] += 1 }
        let resp = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: resp, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(repeating: 1, count: 1024))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
