import Foundation
import MediaPlayer

/// Lock screen / Control Center / Dynamic Island "Now Playing" integration.
@MainActor
final class NowPlayingController {
    private weak var model: PlayerModel?
    private var targets: [(MPRemoteCommand, Any)] = []

    init(model: PlayerModel) {
        self.model = model
        let c = MPRemoteCommandCenter.shared()
        let s = AppSettings.shared
        c.skipForwardCommand.preferredIntervals = [NSNumber(value: s.skipForward)]
        c.skipBackwardCommand.preferredIntervals = [NSNumber(value: s.skipBackward)]

        add(c.playCommand) { m, _ in if !m.isPlaying { m.togglePlay() } }
        add(c.pauseCommand) { m, _ in if m.isPlaying { m.togglePlay() } }
        add(c.togglePlayPauseCommand) { m, _ in m.togglePlay() }
        add(c.skipForwardCommand) { m, _ in m.skip(s.skipForward) }
        add(c.skipBackwardCommand) { m, _ in m.skip(-s.skipBackward) }
        add(c.changePlaybackPositionCommand) { m, e in
            if let e = e as? MPChangePlaybackPositionCommandEvent { m.seek(to: e.positionTime) }
        }
        update()
    }

    private func add(_ cmd: MPRemoteCommand, _ action: @escaping @MainActor (PlayerModel, MPRemoteCommandEvent) -> Void) {
        cmd.isEnabled = true
        let t = cmd.addTarget { [weak self] event in
            MainActor.assumeIsolated {
                guard let m = self?.model else { return }
                action(m, event)
            }
            return .success
        }
        targets.append((cmd, t))
    }

    func update() {
        guard let m = model else { return }
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: m.title,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: m.currentTime,
            MPMediaItemPropertyPlaybackDuration: m.duration,
            MPNowPlayingInfoPropertyPlaybackRate: m.isPlaying ? Double(m.rate) : 0,
            MPNowPlayingInfoPropertyMediaType: MPNowPlayingInfoMediaType.video.rawValue,
        ]
        if let s = m.subtitle { info[MPMediaItemPropertyArtist] = s }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
        #if os(macOS)
        MPNowPlayingInfoCenter.default().playbackState = m.isPlaying ? .playing : .paused
        #endif
    }

    func teardown() {
        for (cmd, t) in targets { cmd.removeTarget(t) }
        targets.removeAll()
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }
}
