// The Mac's real keep-awake power assertion (Moonrise/Player/KeepAwake.swift): held once
// while playing, gone after pause and when released. macOS only, so not in MoonriseTests (iOS),
// which checks when it should be held (KeepAwakeTests). scripts/test-apple.sh runs this on the
// build Mac; by hand, on a Mac:
//   swiftc -o /tmp/keep-awake Moonrise/Player/KeepAwake.swift scripts/tests/keep-awake/main.swift && /tmp/keep-awake
import Foundation
import IOKit.pwr_mgt

var failures = 0
func check(_ ok: Bool, _ what: String, line: Int = #line) {
    if !ok { failures += 1; print("FAIL line \(line): \(what)") }
}

// MARK: - The real assertion

/// This process's assertion names of the display-sleep type.
func held() -> [String] {
    var dict: Unmanaged<CFDictionary>?
    guard IOPMCopyAssertionsByProcess(&dict) == kIOReturnSuccess,
          let all = dict?.takeRetainedValue() as? [NSNumber: [[String: Any]]] else { return [] }
    let mine = all[NSNumber(value: getpid())] ?? []
    return mine.filter { ($0[kIOPMAssertionTypeKey] as? String) == kIOPMAssertionTypePreventUserIdleDisplaySleep }
        .compactMap { $0[kIOPMAssertionNameKey] as? String }
}

let name = "Moonrise playing keep-awake test"
let a = KeepAwakeAssertion()
check(held().isEmpty, "nothing held at first")
a.set(true, reason: name)
check(a.isHeld && held() == [name], "held while playing: \(held())")
a.set(true, reason: name)
check(held() == [name], "set twice holds one assertion: \(held())")
a.set(false, reason: name)
check(!a.isHeld && held().isEmpty, "released on pause: \(held())")
do {
    let b = KeepAwakeAssertion()
    b.set(true, reason: name)
    check(held() == [name], "held again")
}
check(held().isEmpty, "released when the player goes away: \(held())")

print(failures == 0 ? "keep-awake: all passed" : "keep-awake: \(failures) failed")
exit(failures == 0 ? 0 : 1)
