// Which version of a title plays (Moonrise/Models/VersionChoices.swift).
import Foundation
import Testing
@testable import Moonrise

@MainActor
@Suite struct VersionChoicesTests {
    /// A version the server found damaged is never the one while the title has another,
    /// even when it is the server's first (its default) or the one the user chose.
    @Test func aBrokenVersionIsPassedOver() {
        let broken = Fixture.version("a", broken: true)
        let healthy = Fixture.version("b")
        #expect(VersionChoices.playable([broken, healthy]) == [healthy])
        #expect(VersionChoices.best([broken, healthy], hdrDisplay: true, language: .some(nil))?.id == "b")
        #expect(VersionChoices.best([broken, healthy], hdrDisplay: false, language: .some(nil))?.id == "b")

        let choices = VersionChoices()
        var item = Fixture.movie("m1", versions: [broken, healthy])
        item.chosenVersionId = "a"
        #expect(choices.version(for: item)?.id == "b", "a chosen version that is broken gives way")
        choices.set("a", for: "m1")
        #expect(choices.version(for: item)?.id == "b", "a pick made here that is broken gives way")
    }

    /// When every version is damaged they all stay (the app still tries one, and says why).
    @Test func allBrokenKeepsThemAll() {
        let vs = [Fixture.version("a", broken: true), Fixture.version("b", broken: true)]
        #expect(VersionChoices.playable(vs) == vs)
        #expect(VersionChoices.best(vs, hdrDisplay: true, language: .some(nil))?.id == "a")
    }

    /// On an SDR screen: without an HDR display the SDR version as sharp plays, but
    /// never a broken one.
    @Test func sdrOnlyFromTheHealthyOnes() {
        let hdr = Fixture.version("hdr", height: 2160, hdr: "HDR10")
        let sdrBroken = Fixture.version("sdr", height: 2160, broken: true)
        #expect(VersionChoices.best([hdr, sdrBroken], hdrDisplay: false, language: .some(nil))?.id == "hdr")
        let sdr = Fixture.version("sdr2", height: 2160)
        #expect(VersionChoices.best([hdr, sdrBroken, sdr], hdrDisplay: false, language: .some(nil))?.id == "sdr2")
        #expect(VersionChoices.best([hdr, sdr], hdrDisplay: true, language: .some(nil))?.id == "hdr")
        // A sharper HDR version isn't traded for a softer SDR one.
        let soft = Fixture.version("soft", height: 1080)
        #expect(VersionChoices.best([hdr, soft], hdrDisplay: false, language: .some(nil))?.id == "hdr")
    }

    @Test func preferredLanguageWhereTheVersionsDiffer() {
        let en = Fixture.version("en", audio: ["eng"])
        let fr = Fixture.version("fr", audio: ["fre", "eng"])
        #expect(VersionChoices.best([en, fr], hdrDisplay: true, language: "fr")?.id == "fr")
        #expect(VersionChoices.best([en, fr], hdrDisplay: true, language: "en")?.id == "en")
        // A broken version in that language doesn't win.
        let frBroken = Fixture.version("fr", audio: ["fre"], broken: true)
        #expect(VersionChoices.best([en, frBroken], hdrDisplay: true, language: "fr")?.id == "en")
    }
}
