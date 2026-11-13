import CoreGraphics
import Foundation

/// Mac: what the window shows around the player on screen, and whether the Playback menu (and the
/// plain keys) drive it. Kept by `MacPlayerHub`; a value type so MoonriseTests can check it on any
/// platform.
///
/// The player's full screen and its keys are separate things: a sheet over the player (Episodes,
/// Find Subtitles) takes the keys, but the player still fills the screen under it, so the window's
/// toolbar (tabs, search, back) stays hidden. Only the player leaving full screen, or going away,
/// brings it back. The app coming to the front again changes nothing here.
struct PlayerChrome: Equatable {
    /// The player registered last (a title page's, or the Downloads tab's).
    private(set) var owner: UUID?
    /// That player fills the window (and the screen).
    var playerFullScreen = false
    /// One of its sheets is over it: the keys are the sheet's.
    private(set) var sheetOpen = false

    /// The player on screen tells where it is; called again whenever that changes.
    mutating func register(_ id: UUID, fullScreen: Bool, sheetOpen: Bool) {
        owner = id
        playerFullScreen = fullScreen
        self.sheetOpen = sheetOpen
    }

    /// The player went away. One that was replaced already (its `onDisappear` coming after the
    /// next one's `onAppear`) changes nothing. True when it was the registered one.
    @discardableResult
    mutating func unregister(_ id: UUID) -> Bool {
        guard owner == id else { return false }
        owner = nil
        playerFullScreen = false
        sheetOpen = false
        return true
    }

    /// The Playback menu and the plain keys (Space, arrows, M, F) act on the player.
    var drivesPlayer: Bool { owner != nil && !sheetOpen }

    /// The window's toolbar shows unless the player fills the screen.
    var toolbarVisible: Bool { !playerFullScreen }
}

/// The player's episode list on the Mac: a sheet over the window. A List in a NavigationStack has
/// no size of its own in a macOS sheet (it came up as a sliver), so it's given one that fits the
/// smallest window.
enum EpisodeSheetSize {
    static let min = CGSize(width: 520, height: 420)
    static let ideal = CGSize(width: 640, height: 720)
}

/// The Mac window's smallest content size.
enum MacWindowSize {
    static let min = CGSize(width: 800, height: 540)
}
