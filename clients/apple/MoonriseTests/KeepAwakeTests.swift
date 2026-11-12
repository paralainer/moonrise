// When the display is kept awake during playback (Moonrise/Player/KeepAwake.swift):
// only while the video really plays. Here the rule and, on iOS, the idle timer; the
// Mac's real power assertion is in scripts/tests/keep-awake, run on the build Mac by
// scripts/test-apple.sh.
import Foundation
import Testing
#if os(iOS) || os(tvOS)
import UIKit
#endif
@testable import Moonrise

@Suite(.serialized) struct KeepAwakeTests {
    private func hold(playing: Bool = true, rate: Float = 1, finished: Bool = false, error: Bool = false, closed: Bool = false) -> Bool {
        KeepAwake.shouldHold(isPlaying: playing, rate: rate, isFinished: finished, hasError: error, closed: closed)
    }

    @Test func onlyWhileTheVideoPlays() {
        #expect(hold(), "playing at normal speed")
        #expect(hold(rate: 2), "playing faster")
        #expect(hold(rate: 0.5), "playing slower")
        #expect(!hold(playing: false), "paused")
        #expect(!hold(rate: 0), "rate 0")
        #expect(!hold(finished: true), "ended")
        #expect(!hold(error: true), "failed")
        #expect(!hold(closed: true), "player closed")
        #expect(!hold(playing: false, closed: true), "paused and closed")
    }

    /// A paused video lets the screen sleep, and playing again takes the hold back.
    /// Buffering or seeking while playing keeps `isPlaying` true (PlayerModel+Engine), so it holds.
    @MainActor @Test func pauseReleasesAndResumeHoldsAgain() {
        let a = KeepAwakeAssertion()
        var held: [Bool] = []
        for playing in [true, false, true, false] {
            a.set(hold(playing: playing), reason: "Moonrise playing test")
            held.append(a.isHeld)
        }
        #expect(held == [true, false, true, false], "play, pause, resume, pause")
        a.set(hold(), reason: "")
        a.set(hold(finished: true), reason: "")
        #expect(!a.isHeld, "ended releases")
        a.set(hold(), reason: "")
        a.set(hold(error: true), reason: "")
        #expect(!a.isHeld, "an error releases")
    }

    @Test func reasonNamesTheTitle() {
        #expect(KeepAwake.reason(title: "Film A") == "Moonrise playing Film A")
        #expect(KeepAwake.reason(title: "  ") == "Moonrise playing video", "no title yet")
    }

    /// The assertion is held on every platform: the Mac's power assertion, or the idle timer.
    @MainActor @Test func assertionFollowsSet() {
        let a = KeepAwakeAssertion()
        a.set(true, reason: "Moonrise playing test")
        #expect(a.isHeld)
        a.set(false, reason: "Moonrise playing test")
        #expect(!a.isHeld)
    }

    #if os(iOS) || os(tvOS)
    /// The idle timer is off only while a player plays; paused lets the screen sleep.
    @MainActor @Test func idleTimerFollowsPlayback() {
        let app = UIApplication.shared
        let a = KeepAwakeAssertion()
        a.set(true, reason: "")
        #expect(app.isIdleTimerDisabled, "playing")
        a.set(true, reason: "")
        #expect(KeepAwakeAssertion.holders == 1, "set twice holds once")
        a.set(false, reason: "")
        #expect(!app.isIdleTimerDisabled, "paused")
        a.set(true, reason: "")
        #expect(app.isIdleTimerDisabled, "resumed")
        a.set(false, reason: "")
        #expect(!app.isIdleTimerDisabled, "ended")
    }

    /// Two players (one in Picture in Picture, one on a page): the timer stays off while either
    /// plays, and a player that goes away while playing gives its hold back.
    @MainActor @Test func idleTimerIsSharedBetweenPlayers() {
        let app = UIApplication.shared
        let a = KeepAwakeAssertion()
        do {
            let b = KeepAwakeAssertion()
            a.set(true, reason: "")
            b.set(true, reason: "")
            a.set(false, reason: "")
            #expect(app.isIdleTimerDisabled, "the other still plays")
        }
        #expect(!app.isIdleTimerDisabled, "released when the playing player goes away")
        #expect(KeepAwakeAssertion.holders == 0)
    }
    #endif
}
