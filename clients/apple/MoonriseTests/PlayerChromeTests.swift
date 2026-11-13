// The Mac player's full screen: the window's toolbar (tabs, search, back) stays hidden while the
// player fills the screen, also under its Episodes and Find Subtitles sheets, and the episode
// sheet has a size of its own (PlayerChrome and EpisodeSheetSize in
// Moonrise/Player/PlayerChrome.swift).
import CoreGraphics
import Foundation
import Testing
@testable import Moonrise

@Suite struct PlayerChromeTests {
    private let player = UUID()

    /// Opening the episode list over the full-screen player: the sheet has the keys, but the
    /// player still fills the screen, so no toolbar. (A sheet used to unregister the player,
    /// which dropped its full screen and brought the toolbar back.)
    @Test func aSheetOverTheFullScreenPlayerKeepsTheToolbarHidden() {
        var c = PlayerChrome()
        c.register(player, fullScreen: true, sheetOpen: false)
        #expect(!c.toolbarVisible && c.drivesPlayer)
        c.register(player, fullScreen: true, sheetOpen: true)
        #expect(!c.toolbarVisible)
        #expect(!c.drivesPlayer)
        c.register(player, fullScreen: true, sheetOpen: false)
        #expect(!c.toolbarVisible && c.drivesPlayer)
    }

    /// Back from another app: the player registers again as it is; the toolbar stays hidden.
    @Test func registeringAgainKeepsTheFullScreen() {
        var c = PlayerChrome()
        c.register(player, fullScreen: true, sheetOpen: false)
        let before = c
        c.register(player, fullScreen: true, sheetOpen: false)
        #expect(c == before && !c.toolbarVisible)
    }

    /// Leaving full screen (the player's button, Escape, the green button), or the player going
    /// away, brings the toolbar back.
    @Test func leavingFullScreenOrThePlayerShowsTheToolbar() {
        var c = PlayerChrome()
        c.register(player, fullScreen: true, sheetOpen: false)
        c.playerFullScreen = false
        #expect(c.toolbarVisible && c.drivesPlayer)
        c.register(player, fullScreen: true, sheetOpen: true)
        let wasRegistered = c.unregister(player)
        #expect(wasRegistered)
        #expect(c.toolbarVisible && !c.drivesPlayer)
    }

    /// A replaced player's late `onDisappear` doesn't take the new one's full screen away.
    @Test func anOldPlayersLeavingChangesNothing() {
        var c = PlayerChrome()
        let old = UUID()
        c.register(old, fullScreen: true, sheetOpen: false)
        c.register(player, fullScreen: true, sheetOpen: false)
        let oldWasRegistered = c.unregister(old)
        #expect(!oldWasRegistered)
        #expect(!c.toolbarVisible && c.drivesPlayer)
    }

    /// The episode list's sheet on the Mac has a real size (it was a sliver) that fits the
    /// smallest window.
    @Test func episodeSheetHasASizeThatFitsTheSmallestWindow() {
        let min = EpisodeSheetSize.min, ideal = EpisodeSheetSize.ideal
        #expect(min.width >= 400 && min.height >= 400)
        #expect(ideal.width >= min.width && ideal.height >= min.height)
        #expect(min.width <= MacWindowSize.min.width && min.height <= MacWindowSize.min.height)
    }
}
