import SwiftUI

/// The player of a title page (iPhone, iPad, Mac; the Apple TV keeps its full-screen player):
/// Play turns the page's header into the video. One PlayerModel — one AVPlayer and layer — is
/// drawn in the header or full screen, so switching between them doesn't interrupt playback.
/// Leaving the page stops it (reporting the position), unless Picture in Picture shows it.
@MainActor
@Observable
final class InlinePlayback {
    private(set) var model: PlayerModel?
    /// The title or episode playing (known before the model has loaded it).
    private(set) var item: MediaItem?
    /// iPad/Mac: full screen asked for with the button (the iPhone is full screen in landscape).
    var fullScreen = false
    /// A sheet of the player is up (subtitle search, episodes): the page is covered, not left.
    @ObservationIgnored var modalOpen = false
    @ObservationIgnored private var visible = true
    @ObservationIgnored private weak var coordinator: PlaybackCoordinator?

    func play(_ request: PlaybackRequest, coordinator: PlaybackCoordinator) {
        var request = request
        // The same title again (Stream Instead): go on from where it is.
        if request.startAt == nil, let m = model, case let .server(_, it) = request.target, it.id == item?.id {
            request.startAt = m.currentTime
        }
        self.coordinator = coordinator
        coordinator.stopPictureInPicture() // a title left playing in PiP
        model?.close()
        let m = PlayerModel(request: request)
        // Up Next and the full-screen episode picker play the episode here, on the page.
        m.onPlayEpisode = { [weak self] ep in
            guard let self, let next = request.episode(ep) else { return }
            // After the callback (it may come from the ending engine's notification).
            Task { @MainActor in self.play(next, coordinator: coordinator) }
        }
        if case let .server(_, it) = request.target { item = it } else { item = nil }
        model = m
        Task { await m.start() }
    }

    /// The header's close button: stop, the artwork comes back.
    func stop() {
        model?.close()
        model = nil
        item = nil
        setFullScreen(false)
    }

    func pageAppeared() { visible = true }

    /// Navigating back or to another tab. Checked a moment later: a covering sheet or a view
    /// update can make the page disappear and appear again.
    func pageDisappeared() {
        visible = false
        Task { [self] in
            try? await Task.sleep(for: .milliseconds(400))
            guard !visible, !modalOpen, let m = model else { return }
            if m.isInPictureInPicture, let coordinator {
                // Plays on in PiP until it ends.
                model = nil
                item = nil
                setFullScreen(false)
                        coordinator.keepInPictureInPicture(m)
            } else {
                stop()
            }
        }
    }

    func setFullScreen(_ on: Bool) {
        guard fullScreen != on else { return }
        fullScreen = on
        #if os(macOS)
        // The window goes full screen too (and back, unless it already was). Leaving the window's
        // full screen another way (green button, ⌃⌘F, the View menu) ends the player's too.
        MacPlayerHub.shared.windowLeftFullScreen = on ? { [weak self] in
            guard let self, self.fullScreen else { return }
            self.window = nil
            self.fullScreen = false
            MacPlayerHub.shared.playerFullScreen = false
        } : nil
        MacPlayerHub.shared.playerFullScreen = on
        if on, let w = MacWindow.shared.window ?? NSApp.keyWindow, !w.styleMask.contains(.fullScreen) {
            window = w
            w.toggleFullScreen(nil)
        } else if !on, let w = window {
            window = nil
            if w.styleMask.contains(.fullScreen) { w.toggleFullScreen(nil) }
        }
        #endif
    }
    #if os(macOS)
    @ObservationIgnored private weak var window: NSWindow?
    #endif
}
