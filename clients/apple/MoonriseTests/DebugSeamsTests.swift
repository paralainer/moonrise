// How the launch-environment test seams (Moonrise/Support/DebugSeams.swift) read their values: the
// scripts (sim.sh, tvsim.sh) rely on these exact names and on unset, empty and "0" meaning off.
import Testing
@testable import Moonrise

@Suite struct DebugSeamsTests {
    private func seams(_ env: [String: String]) -> DebugSeams { DebugSeams(env: env) }

    @Test func everyKeyIsAMoonriseVariable() {
        let names = DebugSeams.Key.allCases.map(\.rawValue)
        #expect(names.allSatisfy { $0.hasPrefix("MOONRISE_") })
        #expect(Set(names).count == names.count)
        #expect(names.count == 42)
    }

    @Test func flagsAreOnOnlyAsOne() {
        #expect(seams(["MOONRISE_NO_REPORT": "1"]).noReport)
        #expect(!seams(["MOONRISE_NO_REPORT": "0"]).noReport)  // sim.sh's REPORT=1
        #expect(!seams(["MOONRISE_NO_REPORT": ""]).noReport)
        #expect(!seams([:]).noReport)
        #expect(!seams(["MOONRISE_DEMO": "yes"]).demo)
    }

    @Test func textIsNilWhenEmpty() {
        #expect(seams(["MOONRISE_AUTOPLAY": "abc"]).autoplay == "abc")
        #expect(seams(["MOONRISE_AUTOPLAY": ""]).autoplay == nil)  // the scripts export every seam, most empty
        #expect(seams(["MOONRISE_MANAGE": "fix:42"]).manage == "fix:42")
    }

    @Test func numbers() {
        #expect(seams([:]).autoplayAt == 0)
        #expect(seams(["MOONRISE_AUTOPLAY_AT": ""]).autoplayAt == 0)
        #expect(seams(["MOONRISE_AUTOPLAY_AT": "90.5"]).autoplayAt == 90.5)
        #expect(seams(["MOONRISE_AUTOPLAY_AUDIO": "2"]).autoplayAudio == 2)
        #expect(seams(["MOONRISE_AUTOPLAY_AUDIO": "x"]).autoplayAudio == nil)
        #expect(seams(["MOONRISE_OPEN_SUBSEARCH": "0"]).openSubSearch == 0)
        // These count only above 0.
        #expect(seams(["MOONRISE_DISCOVER": "0"]).discover == nil)
        #expect(seams(["MOONRISE_SLOW_ANIM": "-1"]).slowAnim == nil)
        #expect(seams(["MOONRISE_RECOVERY_GIVEUP": "5"]).recoveryGiveUp == 5)
    }

    @Test func landscapeAndSignIn() {
        #expect(seams(["MOONRISE_LANDSCAPE": "1"]).landscape == true)
        #expect(seams(["MOONRISE_LANDSCAPE": "0"]).landscape == false)
        #expect(seams(["MOONRISE_LANDSCAPE": ""]).landscape == nil)
        #expect(seams(["MOONRISE_URL": "http://192.168.1.10:8097", "MOONRISE_USER": "u"]).signIn == nil)
        let s = seams(["MOONRISE_URL": "http://192.168.1.10:8097", "MOONRISE_USER": "u", "MOONRISE_PASS": "p"]).signIn
        #expect(s?.address == "http://192.168.1.10:8097" && s?.user == "u" && s?.pass == "p")
    }
}
