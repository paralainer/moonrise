// A stream that stalls while its session answers and data arrives slower than the stream needs:
// recovery goes down a quality (or says the connection is too slow) instead of opening the same
// stream again, which throws away what had arrived and can never get ahead.
import Foundation
import Testing
@testable import Moonrise

struct SlowLinkRulesTests {
    private let p720 = QualityOption.find("720p · 4 Mbps")
    private let p480 = QualityOption.find("480p · 1.5 Mbps")
    /// A 1080p film at 8 Mbit/s: Original, 720p, 480p.
    private var options: [QualityOption] { QualityOption.options(sourceBitrate: 8_000_000, sourceHeight: 1080) }

    private func action(answered: Bool = true, measured: Double?, needed: Double? = 8_000_000, restarts: Int = 0,
                        current: QualityOption = .original) -> SlowLinkRules.Action {
        SlowLinkRules.action(sessionAnswered: answered, measured: measured, needed: needed, answeredRestarts: restarts,
                             current: current, options: options, sourceBitrate: 8_000_000)
    }

    @Test func originalOverALinkSlowerThanTheFileGoesDownAQuality() {
        // 600 kB/s under an 8 Mbit/s copy, the server answering every check: not the same stream again.
        #expect(action(measured: 4_800_000) == .lower(p480))
        // A link that carries 720p with room to spare.
        #expect(action(measured: 7_000_000) == .lower(p720))
    }

    @Test func aDeadConnectionOrAGoneServerOpensTheSameStreamAgain() {
        // Nothing arrived: a connection the player sits on; a new stream fixes it.
        #expect(action(measured: 0) == .restart)
        // The server didn't answer: ordinary recovery (it waits for the server).
        #expect(action(answered: false, measured: 4_800_000) == .restart)
        // Data arrives as fast as the stream needs: not the link.
        #expect(action(measured: 9_000_000) == .restart)
    }

    @Test func withoutAMeasureTheSecondAnsweredStallGoesDown() {
        #expect(action(measured: nil, restarts: 0) == .restart)
        #expect(action(measured: nil, restarts: 1) == .lower(p720))
        #expect(action(measured: nil, needed: nil, restarts: 1) == .lower(p720))
    }

    @Test func nothingLowerSaysTheConnectionIsTooSlow() {
        #expect(action(measured: 600_000, needed: 1_500_000, current: p480) == .tooSlow)
    }

    @Test func aLinkTooSlowForEveryQualityTakesTheLowest() {
        #expect(action(measured: 1_000_000) == .lower(p480))
        #expect(action(measured: 2_000_000, needed: 4_000_000, current: p720) == .lower(p480))
    }
}
