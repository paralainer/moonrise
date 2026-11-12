// Where the scrubber finds a picture on the server's thumbnail sheets (Moonrise/Player/ScrubThumbnails.swift):
// picture i stands for i × interval up to the next one, sheets of columns × rows, row by row.
import CoreGraphics
import Foundation
import Testing
@testable import Moonrise

struct ScrubThumbnailsTests {
    private func tile(_ t: Double, count: Int = 250) -> ScrubThumbnails.Tile? {
        ScrubThumbnails.tile(at: t, interval: 10, count: count, columns: 10, rows: 10, width: 320, height: 180)
    }

    @Test func picturesAreFoundOnTheirSheets() {
        #expect(tile(0) == .init(sheet: 0, rect: CGRect(x: 0, y: 0, width: 320, height: 180)))
        #expect(tile(9.9) == .init(sheet: 0, rect: CGRect(x: 0, y: 0, width: 320, height: 180)))
        #expect(tile(10) == .init(sheet: 0, rect: CGRect(x: 320, y: 0, width: 320, height: 180)))
        // Picture 23: row 2, column 3; picture 123: the same place on the second sheet.
        #expect(tile(235) == .init(sheet: 0, rect: CGRect(x: 960, y: 360, width: 320, height: 180)))
        #expect(tile(1234) == .init(sheet: 1, rect: CGRect(x: 960, y: 360, width: 320, height: 180)))
        // Past the end: the last picture (249); before the start: the first.
        #expect(tile(99_999) == .init(sheet: 2, rect: CGRect(x: 2880, y: 720, width: 320, height: 180)))
        #expect(tile(-3) == .init(sheet: 0, rect: CGRect(x: 0, y: 0, width: 320, height: 180)))
        #expect(tile(.nan) == nil)
        #expect(tile(5, count: 0) == nil)
    }

    @Test @MainActor func onlyReadyAnswersMakeThumbnails() {
        let base = URL(string: "http://server:8097")!
        var a = ScrubThumbnails.Answer(state: "ready", interval: 10, width: 320, height: 134, columns: 10, rows: 10, count: 3,
                                       sheets: ["/v1/trickplay/0123456789abcdef/0.jpg"])
        let t = ScrubThumbnails(a, base: base, headers: [:])
        #expect(t?.sheets == [URL(string: "http://server:8097/v1/trickplay/0123456789abcdef/0.jpg")!])
        #expect(abs((t?.aspect ?? 0) - 320.0 / 134.0) < 0.001)
        a.state = "pending"
        #expect(ScrubThumbnails(a, base: base, headers: [:]) == nil)
        #expect(ScrubThumbnails(.init(state: "ready"), base: base, headers: [:]) == nil)
    }
}
