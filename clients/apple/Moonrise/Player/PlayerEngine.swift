import CoreGraphics
import Foundation

enum EngineStatus: Equatable {
    case loading
    case ready
    case ended
    case failed(String)
}

struct EngineTrack: Identifiable, Hashable {
    let id: Int
    let title: String
    let language: String?
}

/// Common surface over AVPlayer and VLC so the controls/model don't care which one runs.
@MainActor
protocol PlayerEngine: AnyObject {
    var onTime: ((Double) -> Void)? { get set }
    var onStatus: ((EngineStatus) -> Void)? { get set }
    var onPlayingChanged: ((Bool) -> Void)? { get set }
    var onBuffering: ((Bool) -> Void)? { get set }
    var onTracksChanged: (() -> Void)? { get set }

    var view: PlatformView { get }
    var currentTime: Double { get }
    var duration: Double { get }
    var isPlaying: Bool { get }
    var isNative: Bool { get }

    func play()
    func pause()
    func seek(to seconds: Double)
    func setRate(_ rate: Float)
    func setFill(_ fill: Bool)
    /// App-level volume 0...1 (fallback when the system volume can't be driven).
    func setVolume(_ volume: Float)
    func stop()

    var audioTracks: [EngineTrack] { get }
    var currentAudioTrack: Int? { get }
    func selectAudioTrack(_ id: Int)

    /// Subtitle tracks the engine renders itself (embedded tracks in offline files).
    var subtitleTracks: [EngineTrack] { get }
    func selectSubtitleTrack(_ id: Int?)

    var supportsPictureInPicture: Bool { get }
    func togglePictureInPicture()

    /// The video's picture size as it shows (its display aspect); zero until known.
    var presentationSize: CGSize { get }
}

extension PlayerEngine {
    var presentationSize: CGSize { .zero }
}
