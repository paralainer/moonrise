// The `/playback` query's Dolby Vision flag: `dv=1` only for a display that shows Dolby
// Vision and HDR. MoonriseServer.dolbyVisionQuery in Moonrise/Server/MoonriseServer.swift.
import Foundation
import Testing
@testable import Moonrise

@Suite struct DolbyVisionQueryTests {
    @Test func onlyForADolbyVisionDisplay() {
        #expect(MoonriseServer.dolbyVisionQuery(hdr: true, dolbyVision: true) == URLQueryItem(name: "dv", value: "1"))
        // Not Dolby Vision: the request stays as before (the base layer plays as HDR10).
        #expect(MoonriseServer.dolbyVisionQuery(hdr: true, dolbyVision: false) == nil)
        // HDR off (a display set to SDR): nothing, the server tone-maps.
        #expect(MoonriseServer.dolbyVisionQuery(hdr: false, dolbyVision: true) == nil)
    }
}
