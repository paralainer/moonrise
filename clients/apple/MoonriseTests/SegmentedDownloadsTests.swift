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
}
