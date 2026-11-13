import AVFoundation
import AVKit

#if os(macOS)
import AppKit

final class PlayerLayerView: NSView {
    let playerLayer = AVPlayerLayer()
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer = CALayer()
        layer?.backgroundColor = NSColor.black.cgColor
        layer?.addSublayer(playerLayer)
    }
    required init?(coder: NSCoder) { fatalError() }
    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        playerLayer.frame = bounds
        CATransaction.commit()
    }
}
#else
import UIKit

final class PlayerLayerView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }
    var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }
    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
    }
    required init?(coder: NSCoder) { fatalError() }
}
#endif

/// Native engine: AVPlayer + AVPlayerLayer, with Picture in Picture on iOS/macOS.
@MainActor
final class AVPlayerEngine: NSObject, PlayerEngine {
    var onTime: ((Double) -> Void)?
    var onStatus: ((EngineStatus) -> Void)?
    var onPlayingChanged: ((Bool) -> Void)?
    var onBuffering: ((Bool) -> Void)?
    var onTracksChanged: (() -> Void)?
    /// The selected audio rendition changed (its index), by anyone.
    var onAudioSelectionChanged: ((Int) -> Void)?

    let player = AVPlayer()
    private let layerView = PlayerLayerView()
    private var timeObserver: Any?
    private var observations: [NSKeyValueObservation] = []
    /// The current item's: status, end, error log, stalls (replaced with the item).
    private var itemObservations: [NSKeyValueObservation] = []
    private var itemObservers: [NSObjectProtocol] = []
    private var audioGroup: AVMediaSelectionGroup?
    fileprivate var legibleGroup: AVMediaSelectionGroup?
    private var pendingStart: Double?
    private var desiredRate: Float = 1
    /// Keeps a resource loader alive for the asset's lifetime.
    private var retained: AnyObject?
    /// AirPlay (external playback) started or ended.
    var onExternalPlayback: ((Bool) -> Void)?
    /// An entry in the item's error log: HTTP status (or a negative NSURLError code), the URI.
    /// AVPlayer logs every failed request, including ones it retries by itself.
    var onErrorLogEntry: ((_ status: Int, _ uri: String?) -> Void)?
    /// The item ran out of data while playing (it waits for more).
    var onStalled: (() -> Void)?
    /// Picture in Picture started (true) or ended (false).
    var onPictureInPicture: ((Bool) -> Void)?
    /// Which native subtitle rendition mirrors the overlay's track. It is shown where the overlay
    /// can't draw: in Picture in Picture and on an AirPlay receiver.
    var nativeSubtitle: (name: String?, language: String?)? {
        didSet { applyNativeSubtitles() }
    }
    /// AirPlay is on (the player model decides: it keeps the state across engine swaps).
    var externalActive = false {
        didSet { if externalActive != oldValue { applyNativeSubtitles() } }
    }
    private var pipActive = false
    /// The rendition selected for PiP/AirPlay (its name), nil when none is.
    private(set) var nativeSelection: String?
    /// The stream's subtitle renditions (names), nil until they are loaded.
    var nativeSubtitleNames: [String]? { groupsLoaded ? (legibleGroup?.options.flatMap(names) ?? []) : nil }
    private var groupsLoaded = false
    /// A subtitle track of the file itself (offline files) chosen in the app, shown natively.
    private var inlineSubtitle: Int?

    #if !os(tvOS)
    private var pip: AVPictureInPictureController?
    private var foregroundObserver: NSObjectProtocol?
    #endif

    var view: PlatformView { layerView }
    var isNative: Bool { true }

    init(asset: AVURLAsset, start: Double?, retaining: AnyObject? = nil) {
        super.init()
        retained = retaining
        pendingStart = start
        layerView.playerLayer.player = player
        layerView.playerLayer.videoGravity = .resizeAspect
        player.allowsExternalPlayback = true
        // We choose subtitle renditions ourselves (overlay in-app, native only in PiP).
        player.appliesMediaSelectionCriteriaAutomatically = false
        #if os(iOS)
        player.audiovisualBackgroundPlaybackPolicy = .continuesIfPossible
        #endif

        observePlayer()
        load(asset, start: start)

        #if !os(tvOS)
        if AVPictureInPictureController.isPictureInPictureSupported() {
            pip = AVPictureInPictureController(playerLayer: layerView.playerLayer)
            #if os(iOS)
            pip?.canStartPictureInPictureAutomaticallyFromInline = true
            #endif
            pip?.delegate = self
        }
        // Coming back to the app ends PiP and puts the video back in the full-screen player.
        #if os(iOS)
        let activeNote = UIApplication.didBecomeActiveNotification
        #else
        let activeNote = NSApplication.didBecomeActiveNotification
        #endif
        foregroundObserver = NotificationCenter.default.addObserver(forName: activeNote, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                if let pip = self?.pip, pip.isPictureInPictureActive { pip.stopPictureInPicture() }
            }
        }
        #endif
    }

    /// A new item for the asset (a new stream session): the player, its layer, Picture in
    /// Picture and AirPlay stay as they are.
    private func load(_ asset: AVURLAsset, start: Double?) {
        pendingStart = start
        groupsLoaded = false
        audioGroup = nil
        legibleGroup = nil
        optionTitles = [:]
        nativeSelection = nil
        let item = AVPlayerItem(asset: asset)
        item.preferredForwardBufferDuration = 30
        observe(item)
        player.replaceCurrentItem(with: item)
        // Resume: seek right away so nothing plays from 0:00 before the position is reached.
        if let s = start, s > 1 {
            player.seek(to: CMTime(seconds: s, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero)
        }
        loadSelectionGroups(asset)
    }

    /// Plays another stream of the same title in this player (a new session after the old one
    /// was lost). PiP and AirPlay carry on; the subtitle rendition for them is chosen again
    /// once the new stream's renditions are known.
    func replace(asset: AVURLAsset, start: Double?) {
        player.pause()
        load(asset, start: start)
    }

    /// The current item failed (it plays nothing more).
    var hasFailed: Bool { player.currentItem?.status == .failed }
    var presentationSize: CGSize { player.currentItem?.presentationSize ?? .zero }

    /// The item has nothing buffered at the playhead (AVPlayer stops for that, not the viewer).
    var isBufferEmpty: Bool { player.currentItem?.isPlaybackBufferEmpty ?? false }

    /// After a stall: play again, waiting until enough is buffered. (`play()` starts with
    /// `playImmediately`, and a player started that way pauses when it runs dry, for good.)
    func resumeWhenBuffered() {
        player.automaticallyWaitsToMinimizeStalling = true
        player.rate = desiredRate
    }

    private func observe(_ item: AVPlayerItem) {
        itemObservations = [
            item.observe(\.status, options: [.new]) { [weak self] item, _ in
                Task { @MainActor in
                    // A late change of an item replaced since.
                    guard let self, self.player.currentItem === item else { return }
                    self.itemStatusChanged(item)
                }
            },
        ]
        itemObservers.forEach(NotificationCenter.default.removeObserver)
        itemObservers = [
            NotificationCenter.default.addObserver(forName: AVPlayerItem.didPlayToEndTimeNotification, object: item, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.onStatus?(.ended) }
            },
            NotificationCenter.default.addObserver(forName: AVPlayerItem.newErrorLogEntryNotification, object: item, queue: .main) { [weak self, weak item] _ in
                MainActor.assumeIsolated {
                    guard let self, let e = item?.errorLog()?.events.last else { return }
                    #if DEBUG
                    fputs("MoonriseErrorLog: \(e.errorStatusCode) \(e.errorDomain) \(e.errorComment ?? "") \(e.uri.map(Self.redacted) ?? "")\n", stderr)
                    #endif
                    self.onErrorLogEntry?(e.errorStatusCode, e.uri)
                }
            },
            NotificationCenter.default.addObserver(forName: AVPlayerItem.playbackStalledNotification, object: item, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.onStalled?() }
            },
            // The system player's own Audio menu (Apple TV's transport bar) selects a rendition
            // directly: report what is selected now (a late note about our own pick is no change).
            NotificationCenter.default.addObserver(forName: AVPlayerItem.mediaSelectionDidChangeNotification, object: item, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self, self.groupsLoaded, let i = self.currentAudioTrack else { return }
                    self.onAudioSelectionChanged?(i)
                }
            },
        ]
    }

    #if DEBUG
    /// A stream URL for the log: the path only (session ids are random, but keep logs short).
    static func redacted(_ uri: String) -> String {
        guard let u = URL(string: uri) else { return uri }
        return u.path
    }
    #endif

    private func observePlayer() {
        observations = [
            player.observe(\.isExternalPlaybackActive, options: [.new]) { [weak self] player, _ in
                Task { @MainActor in self?.externalPlaybackChanged(player.isExternalPlaybackActive) }
            },
            player.observe(\.timeControlStatus, options: [.new]) { [weak self] player, _ in
                Task { @MainActor in
                    guard let self else { return }
                    self.onPlayingChanged?(player.timeControlStatus != .paused)
                    self.onBuffering?(player.timeControlStatus == .waitingToPlayAtSpecifiedRate)
                }
            },
        ]
        timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 4), queue: .main) { [weak self] t in
            MainActor.assumeIsolated { self?.onTime?(t.seconds) }
        }
    }

    private func itemStatusChanged(_ item: AVPlayerItem) {
        switch item.status {
        case .readyToPlay:
            if let s = pendingStart, s > 1 {
                pendingStart = nil
                // Already there when the early seek took (it may not have, before loading).
                guard abs(player.currentTime().seconds - s) > 1 else { onStatus?(.ready); return }
                player.seek(to: CMTime(seconds: s, preferredTimescale: 600), toleranceBefore: .zero, toleranceAfter: .zero)
            }
            onStatus?(.ready)
        case .failed:
            #if DEBUG
            fputs("MoonrisePlayerError: \(item.error.map { "\($0)" } ?? "nil")\n", stderr)
            #endif
            onStatus?(.failed(item.error?.localizedDescription ?? "Playback failed"))
        default: break
        }
    }

    private func loadSelectionGroups(_ asset: AVURLAsset) {
        Task { [weak self] in
            let audio = try? await asset.loadMediaSelectionGroup(for: .audible)
            let legible = try? await asset.loadMediaSelectionGroup(for: .legible)
            var titles: [ObjectIdentifier: [String]] = [:]
            for o in legible?.options ?? [] { titles[ObjectIdentifier(o)] = await Self.titles(o) }
            // A replaced item's groups: the new item loads its own.
            guard let self, (self.player.currentItem?.asset as? AVURLAsset) === asset else { return }
            self.audioGroup = audio
            self.legibleGroup = legible
            self.optionTitles = titles
            self.groupsLoaded = true
            // We render text subtitles ourselves; embedded ones stay off unless PiP/AirPlay needs them.
            self.applyNativeSubtitles()
            #if DEBUG
            // Test seams: MOONRISE_NATIVE_SUBS=1 shows the PiP subtitle rendition inline (PiP is unavailable
            // in the simulator); with it or MOONRISE_EXTPLAY the native cues are logged ("MoonriseCue").
            let seams = DebugSeams.launch
            if seams.nativeSubs || seams.extPlay != nil {
                let out = AVPlayerItemLegibleOutput()
                out.setDelegate(self.cueLog, queue: .main)
                self.player.currentItem?.add(out)
            }
            #endif
            self.onTracksChanged?()
        }
    }
    #if DEBUG
    private let cueLog = CueLog()
    #endif

    var currentTime: Double {
        let t = player.currentTime().seconds
        return t.isFinite ? t : 0
    }

    var duration: Double {
        let d = player.currentItem?.duration.seconds ?? 0
        return d.isFinite ? d : 0
    }

    var isPlaying: Bool { player.timeControlStatus != .paused }

    func play() { player.playImmediately(atRate: desiredRate) }
    func pause() { player.pause() }

    func seek(to seconds: Double) {
        let tol = CMTime(seconds: 0.5, preferredTimescale: 600)
        player.seek(to: CMTime(seconds: max(0, seconds), preferredTimescale: 600), toleranceBefore: tol, toleranceAfter: tol)
    }

    func setRate(_ rate: Float) {
        desiredRate = rate
        if isPlaying { player.rate = rate }
    }

    func setVolume(_ volume: Float) { player.volume = volume }

    func setFill(_ fill: Bool) {
        layerView.playerLayer.videoGravity = fill ? .resizeAspectFill : .resizeAspect
    }

    func stop() {
        if let timeObserver { player.removeTimeObserver(timeObserver) }
        timeObserver = nil
        itemObservers.forEach(NotificationCenter.default.removeObserver)
        itemObservers = []
        itemObservations.removeAll()
        #if !os(tvOS)
        if let foregroundObserver { NotificationCenter.default.removeObserver(foregroundObserver) }
        if pip?.isPictureInPictureActive == true { pip?.stopPictureInPicture() }
        #endif
        observations.removeAll()
        player.pause()
        player.replaceCurrentItem(with: nil)
    }

    // MARK: Tracks

    private func tracks(_ group: AVMediaSelectionGroup?) -> [EngineTrack] {
        guard let group else { return [] }
        return group.options.enumerated().map { i, o in
            EngineTrack(id: i, title: o.displayName, language: o.extendedLanguageTag ?? o.locale?.identifier)
        }
    }

    var audioTracks: [EngineTrack] { tracks(audioGroup) }

    var currentAudioTrack: Int? {
        guard let g = audioGroup, let sel = player.currentItem?.currentMediaSelection.selectedMediaOption(in: g) else { return nil }
        return g.options.firstIndex(of: sel)
    }

    func selectAudioTrack(_ id: Int) {
        guard let g = audioGroup, g.options.indices.contains(id) else { return }
        player.currentItem?.select(g.options[id], in: g)
    }

    var subtitleTracks: [EngineTrack] { tracks(legibleGroup) }

    func selectSubtitleTrack(_ id: Int?) {
        inlineSubtitle = id
        applyNativeSubtitles()
    }

    // MARK: Native subtitles (PiP, AirPlay)

    /// AirPlay started or ended (KVO, or the EXTPLAY test seam).
    private func externalPlaybackChanged(_ on: Bool) {
        onExternalPlayback?(on)
    }

    /// AirPlay is on for this player (the EXTPLAY test seam counts).
    var isExternalPlaybackActive: Bool {
        #if DEBUG
        if let simulated = Self.debugExternal { return simulated }
        #endif
        return player.isExternalPlaybackActive
    }

    #if DEBUG
    /// Test seam (MOONRISE_EXTPLAY): the simulator can't AirPlay; this reports what AVPlayer would.
    static var debugExternal: Bool?
    func debugSetExternal(_ on: Bool) {
        Self.debugExternal = on
        externalPlaybackChanged(on)
    }
    #endif

    /// Selects the rendition matching the overlay's track while in PiP or on AirPlay; otherwise
    /// only a file's own track chosen in the app (or none: the overlay draws subtitles).
    private func applyNativeSubtitles() {
        #if DEBUG && !os(tvOS)
        let forced = DebugSeams.launch.nativeSubs
        #else
        let forced = false
        #endif
        setNativeSubtitles(on: pipActive || externalActive || forced)
    }

    /// A rendition's names: `displayName` is its playlist NAME only when the device's language is
    /// the rendition's (else it is the language's name, "Russian"); the NAME is also its title.
    private func names(_ o: AVMediaSelectionOption) -> [String] {
        [o.displayName] + (optionTitles[ObjectIdentifier(o)] ?? [])
    }
    private var optionTitles: [ObjectIdentifier: [String]] = [:]

    private static func titles(_ o: AVMediaSelectionOption) async -> [String] {
        var out: [String] = []
        for m in o.commonMetadata where m.commonKey == .commonKeyTitle {
            if let t = try? await m.load(.stringValue), !out.contains(t) { out.append(t) }
        }
        return out
    }

    /// Match the overlay's track against the stream's subtitle renditions (by name, then language).
    private func setNativeSubtitles(on: Bool) {
        nativeSelection = nil
        guard let group = legibleGroup, let item = player.currentItem else { return }
        var option: AVMediaSelectionOption?
        if on, let want = nativeSubtitle {
            let options = group.options.filter { !$0.hasMediaCharacteristic(.containsOnlyForcedSubtitles) }
            let byName = want.name.flatMap { n in options.first { names($0).contains { $0.caseInsensitiveCompare(n) == .orderedSame } } }
            let byLang = want.language.flatMap { l in options.first { $0.extendedLanguageTag?.hasPrefix(l.prefix(2).lowercased()) == true || $0.locale?.language.languageCode?.identifier(.alpha3) == l } }
            option = byName ?? byLang
            nativeSelection = byName != nil ? want.name : option?.displayName
        } else if let id = inlineSubtitle, group.options.indices.contains(id) {
            option = group.options[id]
        }
        if item.currentMediaSelection.selectedMediaOption(in: group) != option { item.select(option, in: group) }
        #if DEBUG
        if on || option != nil {
            fputs("MoonriseNativeSubs: \(pipActive ? "pip " : "")\(externalActive ? "airplay " : "")want \(nativeSubtitle.map { "\($0.name ?? "-")/\($0.language ?? "-")" } ?? "none") -> \(option?.displayName ?? "none") [\(nativeSelection ?? "-")] of \(group.options.map(names))\n", stderr)
        }
        #endif
    }

    // MARK: PiP

    var supportsPictureInPicture: Bool {
        #if os(tvOS)
        return false
        #else
        return pip != nil
        #endif
    }

    var isPictureInPictureActive: Bool {
        #if os(tvOS)
        return false
        #else
        return pip?.isPictureInPictureActive == true
        #endif
    }

    func togglePictureInPicture() {
        #if !os(tvOS)
        guard let pip else { return }
        if pip.isPictureInPictureActive { pip.stopPictureInPicture() } else { pip.startPictureInPicture() }
        #endif
    }
}

#if DEBUG
/// Test seam: logs when native subtitle cues show, to check PiP timing against the SRT.
final class CueLog: NSObject, AVPlayerItemLegibleOutputPushDelegate {
    func legibleOutput(_ output: AVPlayerItemLegibleOutput, didOutputAttributedStrings strings: [NSAttributedString],
                       nativeSampleBuffers: [Any], forItemTime itemTime: CMTime) {
        let text = strings.map(\.string).joined(separator: " / ").replacingOccurrences(of: "\n", with: " ")
        fputs(String(format: "MoonriseCue %.3f %@\n", itemTime.seconds, text), stderr)
    }
}
#endif

#if !os(tvOS)
extension AVPlayerEngine: AVPictureInPictureControllerDelegate {
    nonisolated func pictureInPictureControllerWillStartPictureInPicture(_ controller: AVPictureInPictureController) {
        MainActor.assumeIsolated { pipActive = true; applyNativeSubtitles(); onPictureInPicture?(true) }
    }

    nonisolated func pictureInPictureControllerDidStopPictureInPicture(_ controller: AVPictureInPictureController) {
        MainActor.assumeIsolated { pipActive = false; applyNativeSubtitles(); onPictureInPicture?(false) }
    }

    /// The PiP window's "return to app" button: the player (full-screen or on the title page) is
    /// still there, so just restore; a page left during PiP closes its player when PiP stops.
    nonisolated func pictureInPictureController(_ controller: AVPictureInPictureController,
                                                restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler: @escaping (Bool) -> Void) {
        completionHandler(true)
    }
}
#endif
