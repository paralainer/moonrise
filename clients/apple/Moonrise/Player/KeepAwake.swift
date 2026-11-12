import Foundation
#if os(macOS)
import IOKit.pwr_mgt
#elseif os(iOS) || os(tvOS)
import UIKit
#endif

/// Keeping the display awake during playback: only while the video plays. The
/// player model holds a `KeepAwakeAssertion` that follows `shouldHold`: on the Mac a power
/// assertion, on iPhone, iPad and Apple TV the app's idle timer.
enum KeepAwake {
    /// Whether the display (and with it the system) should stay awake: only while the video is
    /// really moving, or buffering on its way to play (`isPlaying` stays true then). Paused,
    /// ended, failed or closed lets the device sleep or start its screensaver as usual.
    static func shouldHold(isPlaying: Bool, rate: Float, isFinished: Bool, hasError: Bool, closed: Bool) -> Bool {
        isPlaying && rate > 0 && !isFinished && !hasError && !closed
    }

    /// The assertion's name, as `pmset -g assertions` lists it.
    static func reason(title: String) -> String {
        let t = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? "Moonrise playing video" : "Moonrise playing \(t)"
    }
}

/// Holds the display awake while `set(true, …)`; released by `set(false, …)` or when this goes
/// away. On the Mac one "prevent idle display sleep" assertion (which also prevents idle system
/// sleep), dropped by the system if the app quits or crashes. On iPhone, iPad and Apple TV the
/// idle timer, which is one switch for the whole app: it stays off while any player holds it
/// (a title in Picture in Picture and one on a page).
final class KeepAwakeAssertion {
    #if os(macOS)
    private var id: IOPMAssertionID = IOPMAssertionID(kIOPMNullAssertionID)
    #endif
    private(set) var isHeld = false

    #if os(iOS) || os(tvOS)
    /// How many assertions hold the idle timer off. Main thread only.
    private(set) static var holders = 0

    private static func change(by delta: Int) {
        holders = max(0, holders + delta)
        UIApplication.shared.isIdleTimerDisabled = holders > 0
    }
    #endif

    func set(_ hold: Bool, reason: @autoclosure () -> String) {
        guard hold != isHeld else { return }
        #if os(macOS)
        if hold {
            var newId = IOPMAssertionID(kIOPMNullAssertionID)
            let r = IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleDisplaySleep as CFString,
                                                IOPMAssertionLevel(kIOPMAssertionLevelOn), reason() as CFString, &newId)
            guard r == kIOReturnSuccess else { return }
            id = newId
        } else {
            IOPMAssertionRelease(id)
            id = IOPMAssertionID(kIOPMNullAssertionID)
        }
        #elseif os(iOS) || os(tvOS)
        Self.change(by: hold ? 1 : -1)
        #endif
        isHeld = hold
        #if DEBUG
        fputs("MoonriseKeepAwake: \(hold ? "held" : "released")\n", stderr)
        #endif
    }

    deinit {
        guard isHeld else { return }
        #if os(macOS)
        IOPMAssertionRelease(id)
        #elseif os(iOS) || os(tvOS)
        if Thread.isMainThread { Self.change(by: -1) } else { DispatchQueue.main.async { Self.change(by: -1) } }
        #endif
    }
}
