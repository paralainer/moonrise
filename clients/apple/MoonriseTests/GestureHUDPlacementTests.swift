// Where the swipe gestures' brightness/volume level and the skip count sit on the player
// (GestureHUD.Placement in Moonrise/Player/PlayerView.swift).
import SwiftUI
import Testing
@testable import Moonrise

@Suite struct GestureHUDPlacementTests {
    /// iPhone/iPad full screen: brightness and volume go to the top row's right end, with the
    /// controls' side and top insets, off the middle of the picture.
    @Test func levelsTopRightOnThePhoneAndPad() {
        let row = PlayerControls.rowInsets(compact: false)
        for hud in [GestureHUD.brightness(0.3), .volume(0.8), .volume(0)] {
            let place = GestureHUD.Placement(hud, compact: false, mac: false)
            #expect(place.alignment == .topTrailing)
            #expect(place.insets.top == row.height)
            #expect(place.insets.trailing == row.width)
            #expect(place.insets.bottom == 0)
        }
    }

    /// The pill is as tall as the top row's buttons, so it lines up with them.
    @Test func pillMatchesTheTopRow() {
        #expect(PlayerControls.barButtonSize == 44)
    }

    /// A double-tap's skip count stays in the middle, where the tap was aimed.
    @Test func skipStaysCentred() {
        for mac in [false, true] {
            let place = GestureHUD.Placement(.skip(10), compact: false, mac: mac)
            #expect(place.alignment == .center)
            #expect(place.insets == EdgeInsets())
        }
    }

    /// The Mac's keys' volume keeps its place under the top bar (its right end holds buttons).
    @Test func macUnderTheTopBar() {
        #expect(GestureHUD.Placement(.volume(0.5), compact: false, mac: true)
            == GestureHUD.Placement(alignment: .top, insets: EdgeInsets(top: 76, leading: 0, bottom: 0, trailing: 0)))
        #expect(GestureHUD.Placement(.volume(0.5), compact: true, mac: true).insets.top == 64)
    }

    /// The app's own platform picks the phone/pad placement in the iOS test host.
    @Test func defaultIsThisPlatform() {
        #expect(GestureHUD.Placement(.brightness(1), compact: false)
            == GestureHUD.Placement(.brightness(1), compact: false, mac: false))
    }
}
