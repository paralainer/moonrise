// The `/playback` query's lossless flag: `lossless=1` only for an audio output with more
// than 2 channels, never on the Mac. MoonriseServer.losslessQuery in Moonrise/Server/MoonriseServer.swift.
import Foundation
import Testing
@testable import Moonrise

@Suite struct PlaybackQueryTests {
    /// A multichannel output (HDMI to a TV or receiver) asks for lossless multichannel audio.
    @Test func multichannelOutputAsksForLossless() {
        #expect(MoonriseServer.losslessQuery(multichannel: true) == URLQueryItem(name: "lossless", value: "1"))
    }

    /// A stereo output (TV speakers, headphones, the phone's speaker) keeps today's request.
    @Test func stereoOutputAsksAsBefore() {
        #expect(MoonriseServer.losslessQuery(multichannel: false) == nil)
    }

    /// The Mac app asks for neither lossless audio nor Dolby Vision, whatever its outputs.
    @Test func macAsksForNeither() {
        #expect(MoonriseServer.losslessQuery(multichannel: true, mac: true) == nil)
        #expect(MoonriseServer.dolbyVisionQuery(hdr: true, dolbyVision: true, mac: true) == nil)
    }
}
