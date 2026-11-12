// Manage Server → Playback & Hardware → Scrubbing Thumbnails: the server's progress
// (`/v1/admin/activity` → `trickplay`) as the app words it, and the space the thumbnails use.
import Foundation
import Testing
@testable import Moonrise

struct ManageThumbnailsTests {
    private func trickplay(_ json: String) throws -> MoonriseClient.Activity.Trickplay {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return try d.decode(MoonriseClient.Activity.Trickplay.self, from: Data(json.utf8))
    }

    @Test func progressIsWordedForEachState() throws {
        let running = try trickplay(#"{"state":"running","files":281,"done":120,"failed":0,"skipped":0,"bytes":0,"max_bytes":0}"#)
        #expect(PlaybackAdminView.thumbnailsText(running) == "120 of 281")
        let paused = try trickplay(#"{"state":"paused","files":281,"done":120,"failed":1}"#)
        #expect(PlaybackAdminView.thumbnailsText(paused) == "Paused while playing · 120 of 281")
        let full = try trickplay(#"{"state":"full","files":281,"done":120}"#)
        #expect(PlaybackAdminView.thumbnailsText(full) == "Space full · 120 of 281")
        let idle = try trickplay(#"{"state":"idle","files":0,"done":0,"failed":2,"skipped":3}"#)
        #expect(PlaybackAdminView.thumbnailsText(idle) == "All titles ready · 2 failed · 3 skipped")
        #expect(PlaybackAdminView.thumbnailsText(try trickplay(#"{"state":"off"}"#)) == "Off")
        #expect(PlaybackAdminView.thumbnailsText(nil) == "…")
    }

    @Test func spaceUsedIsShownOnceTheServerSaysIt() throws {
        let t = try trickplay(#"{"state":"idle","bytes":1073741824,"max_bytes":10737418240}"#)
        #expect(PlaybackAdminView.thumbnailSpaceText(t) == "\(AdminFmt.bytes(1 << 30)) of \(AdminFmt.bytes(10 << 30))")
        // A 0.27.0 server sends both; an answer without them shows no row.
        #expect(PlaybackAdminView.thumbnailSpaceText(try trickplay(#"{"state":"idle"}"#)) == nil)
        #expect(PlaybackAdminView.thumbnailSpaceText(nil) == nil)
    }
}
