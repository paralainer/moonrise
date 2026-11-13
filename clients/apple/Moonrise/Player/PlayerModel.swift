import AVFoundation
import Foundation
import MediaPlayer
import Observation

struct PlaybackRequest: Identifiable {
    enum Target {
        /// A title of the Moonrise server. Downloaded, its copy on the device plays (offline too;
        /// unless `preferStream`).
        case server(Source, MediaItem)
    }

    let id = UUID()
    let target: Target
    /// nil = resume from the saved position.
    var startAt: Double?
    /// Stream from the server even when the title is downloaded ("Stream Instead").
    var preferStream = false
    /// The version (file) to play; nil: the one chosen for the title, else the server's.
    var versionId: String? = nil

    /// Another episode of the show this plays (Up Next, the episode picker): its download when it
    /// has one, else the server's stream.
    @MainActor func episode(_ ep: MediaItem) -> PlaybackRequest? {
        switch target {
        case let .server(source, _):
            return PlaybackRequest(target: .server(source, ep))
        }
    }
}

struct AudioOption: Identifiable, Hashable {
    let id: Int
    let title: String
    /// Secondary line in the menu: "Commentary · 5.1 · Dolby Digital".
    var detail: String? = nil
}

@MainActor
@Observable
final class PlayerModel {
    let request: PlaybackRequest
    var engine: PlayerEngine?
    /// Bumped when the engine is swapped so the video surface re-hosts the new view.
    var engineGeneration = 0
    /// The player's controls are up. Kept here, not in the view, because a title page draws this
    /// model in two views (the page's header, full screen) and rotating switches between them:
    /// the controls stay as they were.
    var controlsVisible = true

    var title = ""
    var subtitle: String?
    var currentTime: Double = 0
    var duration: Double = 0
    var isPlaying = false { didSet { updateKeepAwake() } }
    var isBuffering = true
    /// What stopped playback, in a line (with `errorTitle` and `errorDetail` when it is a
    /// `PlaybackProblem`: its title, and the server's own text behind Details).
    var errorMessage: String? {
        didSet {
            if errorMessage == nil { errorTitle = nil; errorDetail = nil }
            updateKeepAwake()
        }
    }
    var errorTitle: String?
    var errorDetail: String?
    var isFinished = false { didSet { updateKeepAwake() } }

    var quality: QualityOption
    var qualityAvailable = false
    var playMethodLabel = ""

    var audioOptions: [AudioOption] = []
    var selectedAudio: Int?

    var subtitleOptions: [SubtitleOption] = []
    var selectedSubtitleId: String?
    var subtitleText: String?
    var subtitleDelay: Double = 0
    /// Transient, non-blocking message (shown as a toast).
    var notice: String? {
        didSet {
            guard notice != nil else { return }
            noticeTask?.cancel()
            noticeTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(4))
                if !Task.isCancelled { self?.notice = nil }
            }
        }
    }
    @ObservationIgnored private var noticeTask: Task<Void, Never>?
    var subtitleDoc: SubtitleDocument?
    /// The selected picture track's cues known so far (`PictureCue`), and those showing now.
    var pictureDoc: PictureDocument?
    var pictureCues: [PictureCue] = []
    /// The video's picture size as it shows (for placing picture cues; the Mac's controls sit on
    /// the picture).
    var videoSize: CGSize = .zero
    /// The selected server track fetched in windows around the playhead (it isn't extracted yet).
    @ObservationIgnored var windowLoader: SubtitleWindowLoader?
    /// Bumped when a track's timing changes (the menus read it).
    var subtitleTimingRevision = 0
    @ObservationIgnored var nudgeTask: Task<Void, Never>?
    @ObservationIgnored var timingWatch: Task<Void, Never>?

    var rate: Float = 1 { didSet { engine?.setRate(rate); updateKeepAwake() } }
    /// macOS: keeps the display and the Mac awake while the video plays.
    @ObservationIgnored let keepAwake = KeepAwakeAssertion()
    var fill = false { didSet { engine?.setFill(fill) } }
    /// Player-level volume, used only when the system volume isn't scriptable.
    var playerVolume: Float = 1 { didSet { engine?.setVolume(playerVolume) } }

    // Skippable segments (intro, recap, credits…)
    var segments: [MediaSegment] = []
    /// Scrubbing thumbnails of what plays (Moonrise server, `trickplay`; not on the Apple TV,
    /// whose system player shows the server's I-frame stream).
    var thumbnails: ScrubThumbnails?
    /// The segment playback is inside right now, if it can be skipped.
    var activeSegment: MediaSegment?
    /// 0…1 while an auto-skip countdown runs (in playback time, so pausing pauses it); nil otherwise.
    var autoSkipProgress: Double?
    /// The skip button shows for a few seconds on entering a segment, then only with the controls.
    var segmentPromptVisible = false
    @ObservationIgnored var handledSegments = Set<String>()
    @ObservationIgnored var autoSkipFrom: Double = 0
    @ObservationIgnored var promptTask: Task<Void, Never>?

    // Episodes of the show being played (for the episode picker and Up Next)
    var seriesEpisodes: [MediaItem] = []
    /// Seconds left before the next episode starts; nil when no countdown is running.
    /// Set by the player screen: switch playback to another episode.
    @ObservationIgnored var onPlayEpisode: ((MediaItem) -> Void)?
    /// The video is in Picture in Picture.
    var isInPictureInPicture = false
    /// Picture in Picture ended (set for a title page's player that was left while in PiP).
    @ObservationIgnored var onPictureInPictureEnded: (() -> Void)?
    /// The video plays on an AirPlay receiver.
    var isExternalPlayback = false
    /// The receiver shows the selected subtitle itself (the stream's rendition), so the overlay
    /// draws nothing.
    var subtitlesOnReceiver = false {
        didSet {
            #if DEBUG
            if subtitlesOnReceiver != oldValue { mark("overlay \(subtitlesOnReceiver ? "hidden: subtitles on the receiver" : "shown")") }
            #endif
        }
    }
    /// The download that played before AirPlay started (the server's stream plays meanwhile).
    @ObservationIgnored var airPlayDownload: DownloadRecord?
    /// Subtitles the stream was restarted for (or sent to the server) during this AirPlay session.
    @ObservationIgnored var airPlayFallbacks = Set<String>()
    @ObservationIgnored var externalCheck: Task<Void, Never>?

    // Media server state
    var client: (any MediaServerClient)?
    /// The server item playing, if any.
    var item: MediaItem?
    /// The version (file) playing, as the server's playback answer describes it.
    var mediaSource: MediaVersion?
    /// The title's versions (several: the Version menu), and the one asked for.
    var versions: [MediaVersion] = []
    @ObservationIgnored var wantedVersion: String?
    var playSessionId: String?
    var playMethod = PlayMethod.directPlay
    /// The server's audio index of each of its stream's audio renditions, in order.
    @ObservationIgnored var moonriseAudio: [Int] = []
    /// Those of `moonriseAudio` the server converts (DTS, TrueHD, MP3… to AAC/E-AC-3).
    @ObservationIgnored var moonriseConverted: Set<Int> = []
    /// The Moonrise server session playing (its master playlist), closed when playback ends or
    /// restarts so the server stops its jobs, and the video variant it serves.
    @ObservationIgnored var moonriseMaster: URL?
    @ObservationIgnored var moonriseVariant: MoonriseServer.Playback.Variant?
    /// The title's subtitle tracks as the Moonrise server lists them (it extracts the text ones and
    /// keeps tracks fetched from OpenSubtitles), or nil when it didn't list them.
    @ObservationIgnored var moonriseSubtitles: [MoonriseServer.SubtitleTrack]?
    #if DEBUG
    @ObservationIgnored var debugAudioApplied = false
    @ObservationIgnored var debugSeekTarget: Double?
    /// `MOONRISE_AUTOPLAY_PAUSE` has paused playback.
    @ObservationIgnored var debugPaused = false
    #endif
    // MARK: Stream recovery state (see "Stream recovery" below)

    /// Recovery of a lost server stream: none, under way (a spinner shows), or given up (a
    /// message with Retry shows).
    enum Recovery: Equatable {
        case none
        case recovering
        case failed(StreamFailure)
    }
    struct StreamFailure: Equatable {
        let title: String
        let detail: String
        /// The server's own text, behind Details.
        var more: String? = nil

        init(title: String, detail: String, more: String? = nil) {
            self.title = title
            self.detail = detail
            self.more = more
        }

        init(_ p: PlaybackProblem) {
            self.init(title: p.title, detail: p.message, more: p.detail)
        }
    }
    var recovery: Recovery = .none
    @ObservationIgnored var recoveryTask: Task<Void, Never>?
    @ObservationIgnored var watchdog: Task<Void, Never>?
    /// Where playback resumes (a seek during recovery moves it), and whether it plays then.
    @ObservationIgnored var recoveryPosition: Double = 0
    @ObservationIgnored var recoveryOpenedAt: Double = 0
    @ObservationIgnored var recoverPlaying = true
    /// A new session's item was put in the player; recovery ends when it is ready.
    @ObservationIgnored var awaitingNewItem = false
    /// Recoveries since playback last ran for a while (a stream that keeps failing gives up).
    @ObservationIgnored var recoveryAttempts = 0
    /// The route the current server stream was opened on.
    @ObservationIgnored var streamRoute: ConnectionRouter.Route?
    /// How many times the stream has moved to another route.
    @ObservationIgnored var routeMoves = 0
    /// When the stream was last moved to another route (`StreamRouteRules`).
    @ObservationIgnored var routeMovedAt = Date.distantPast
    /// When the stream was first lost, in a run of tries (the viewer's wait, for giving up).
    @ObservationIgnored var streamLostAt: Date?
    /// Seconds without a working stream before recovery gives up (a test sets it).
    @ObservationIgnored var recoveryGiveUp = PlayerModel.giveUpAfter
    /// When a stall last made the router look for a route again.
    @ObservationIgnored var routeAskedAt = Date.distantPast
    @ObservationIgnored var playedSinceRecovery: Double = 0
    /// Play (true) or pause as the viewer last had it; AVPlayer pausing a failed item doesn't count.
    @ObservationIgnored var playIntent = true
    @ObservationIgnored var intentChangedAt = Date.distantPast
    /// When the item last ran out of data (AVPlayer pauses then: that isn't the viewer's pause).
    @ObservationIgnored var stalledAt = Date.distantPast
    /// The current item has been ready to play (a first start may take longer than a stall).
    @ObservationIgnored var itemReady = false
    @ObservationIgnored var everReady = false
    /// The last position the engine reported (a failing item may report 0:00).
    @ObservationIgnored var lastPosition: Double = 0
    @ObservationIgnored var stuckSince: Date?
    @ObservationIgnored var lastTick: Double = -1
    @ObservationIgnored var probing = false
    @ObservationIgnored var lastProbe = Date.distantPast
    /// The last stream request the server refused (status, address): asked again for its answer's
    /// text when recovery gives up, to say why.
    @ObservationIgnored var lastStreamError: (status: Int, uri: String)?
    var lastReport = Date.distantPast

    var progressKey: String?
    var nowPlaying: NowPlayingController?

    /// The download playing (from the device), if any: its reports are queued (`LocalReports`).
    var local: DownloadRecord?
    /// This playback, for local reports (a viewing counts as played once).
    @ObservationIgnored let viewing = UUID().uuidString
    @ObservationIgnored var localRetried = false
    @ObservationIgnored var localGrowing = false
    #if DEBUG
    @ObservationIgnored var lastLocalLog = Date.distantPast
    #endif

    @ObservationIgnored var stallTask: Task<Void, Never>?

    /// A download playing from the device that stalls (a segment it waited for came late, or
    /// the server was away): AVPlayer can stay stuck after its requests time out, so nudge it
    /// (seek to where it is), and after a few tries start the playback again there.
    @ObservationIgnored var stallTries = 0
    @ObservationIgnored var stallAt: Double = -1

    /// Where the last seek is going. Until playback gets there the engine keeps reporting the old
    /// position (a remux/transcode may need seconds to restart), so hold the target instead of
    /// letting the scrubber snap back.
    @ObservationIgnored var seekTarget: Double?
    @ObservationIgnored var seekStarted = Date.distantPast
    /// How far before seekTarget playback may land and still count as there.
    @ObservationIgnored var seekSlack = 1.5

    @ObservationIgnored var subtitleEdgeTask: Task<Void, Never>?

    @ObservationIgnored var readyAt: Date?

    /// The tracks the user picked last in this title (the server's playback answer, or a
    /// download's own), applied when it starts (`TrackRules.rememberedAudio`, `rememberedSubtitle`).
    @ObservationIgnored var trackMemory: TrackRules.Memory?
    /// The tracks a title starts with have been picked: reports may say which play.
    @ObservationIgnored var tracksSettled = false

    @ObservationIgnored var localAudioApplied = false
    @ObservationIgnored var localKeepAudio: Int?
    @ObservationIgnored var localSubtitlePending = false

    /// The server was still looking for the end credits in the file when the segments were
    /// fetched: asked again from 5 minutes before the end, then each minute while it still is,
    /// until the last minute.
    @ObservationIgnored var segmentsPending = false
    @ObservationIgnored var segmentsAskedAt: Double = -.infinity
    /// Times the segments were asked for again while the server looked the title up in the
    /// community databases (`SegmentService.awaitingCommunity`).
    @ObservationIgnored var segmentsLookupRetries = 0
    /// The near-end asks made for a title whose answer wasn't final (`SegmentRules.nearEndAsksDue`).
    @ObservationIgnored var segmentsNearEndAsks = 0

    #if DEBUG
    @ObservationIgnored var loggedNextOffer = false
    #endif

    /// A run of skips (buttons, keys, double-taps): each adds to one target, and a single seek
    /// goes there once the presses pause, instead of a seek (and a stream restart on a transcode)
    /// per press. The controls show the target time and the run's total meanwhile.
    var skipTarget: Double?
    /// Where the run started.
    var skipOrigin: Double?
    @ObservationIgnored var skipTask: Task<Void, Never>?

    /// Server subtitle files by URL, fetched in the background when playback starts.
    @ObservationIgnored var subtitlePrefetch: [URL: Task<Data, Error>] = [:]

    @ObservationIgnored var closed = false

    /// The server searches subtitles (`subtitle-search`: it has an OpenSubtitles key). Checked
    /// when playback starts; true until then.
    var serverSubtitleSearch = true

    init(request: PlaybackRequest) {
        self.request = request
        wantedVersion = request.versionId
        quality = QualityOption.find(AppSettings.shared.streamingQuality)
    }

    // MARK: - Start

    func start() async {
        #if DEBUG
        startClock = Date()
        mark("start")
        #endif
        configureAudioSession()
        switch request.target {
        case let .server(source, item):
            title = item.displayTitle
            subtitle = item.isEpisode ? [item.seriesName, item.episodeLabel].compactMap { $0 }.joined(separator: " · ") : item.productionYear.map(String.init)
            guard let client = MediaServers.client(for: source) else { errorMessage = "Server not configured"; return }
            self.client = client
            self.item = item
            RecentsStore.record(source: source.id, item: item)
            progressKey = ProgressStore.key(itemId: item.id)
            // Downloaded: the copy on the device plays (unless asked to stream).
            if !request.preferStream, let rec = DownloadManager.shared.playable(forItem: item.id) {
                await startLocal(rec, source: source)
                return
            }
            qualityAvailable = !(client is DemoClient)  // the demo streams one file
            if let base = client.moonriseBaseURL {
                Task { if let ok = await MoonriseServer.serves("subtitle-search", base: base) { serverSubtitleSearch = ok } }
            }
            // Several versions known: the one chosen for the title (else the best for this device).
            if wantedVersion == nil, item.hasVersions { wantedVersion = VersionChoices.shared.version(for: item)?.id }
            // A version's own resume point (another cut keeps its own), else the title's.
            let versionStart = wantedVersion.flatMap { v in item.versions?.first { $0.id == v } }.map { $0.position ?? 0 }
            let start = request.startAt ?? versionStart.flatMap { $0 > 0 ? $0 : nil } ?? (versionStart == nil ? item.resumeSeconds : nil) ?? 0
            versions = item.versions ?? []
            segmentsAskedAt = -.infinity
            segmentsLookupRetries = 0
            segmentsNearEndAsks = 0
            Task { await loadSegments(item, client: client) }
            if item.isEpisode, let sid = item.seriesId {
                Task { seriesEpisodes = (try? await client.episodes(seriesId: sid, seasonId: nil)) ?? [] }
            }
            await loadServer(at: start)
        }
    }

    /// The server the download came from can be reached on some route now.
    func serverReachable(_ sourceId: UUID) -> Bool {
        SourceStore.shared.source(sourceId) != nil && ConnectionRouter.shared.isReachable
    }

    #if DEBUG
    /// Startup timing in the console (tap to playback: server request, ready).
    private var startClock = Date()
    func mark(_ what: String) {
        fputs(String(format: "MoonriseTiming %.3f [%@] %@\n", Date().timeIntervalSince(startClock), ISO8601DateFormatter.string(from: Date(), timeZone: .gmt, formatOptions: [.withTime, .withFractionalSeconds]), what), stderr)
    }
    #endif

    private func configureAudioSession() {
        #if os(iOS) || os(tvOS)
        try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
        try? AVAudioSession.sharedInstance().setActive(true)
        #endif
    }

    /// Idempotent: called from the close button and again when the player view goes away
    /// (including when it's replaced by another episode).
    func close() {
        guard !closed else { return }
        closed = true
        updateKeepAwake()
        skipTask?.cancel()
        windowLoader?.stop()
        #if DEBUG
        mark(String(format: "close at %.1f", currentTime))
        #endif
        saveProgress(final: false)
        if local != nil {
            localReport("stop")
        } else if let client, item != nil, reportingEnabled {
            let r = report(paused: true)
            Task { await client.report(.stopped, r) }
        }
        if let client, local == nil, let id = playSessionId { Task { await client.stop(sessionId: id) } }
        if let master = moonriseMaster {
            moonriseMaster = nil
            Task { await MoonriseServer.close(master: master) }
        }
        stallTask?.cancel()
        externalCheck?.cancel()
        recoveryTask?.cancel()
        watchdog?.cancel()
        engine?.stop()
        engine = nil
        nowPlaying?.teardown()
        nowPlaying = nil
        NotificationCenter.default.post(name: .playbackDidEnd, object: nil)
    }

}

struct SubtitleSearchContext {
    var title: String
    var server: (Source, MediaItem)?
    /// A download's server and title (`<item>~<version>`): searched for through that server.
    var download: (Source, String)?
    var imdbId: String?
    var tmdbId: String?
    var season: Int?
    var episode: Int?
    static let needsServer = "Finding subtitles goes through your Moonrise server: connect to it and try again."
}

extension Notification.Name {
    static let playbackDidEnd = Notification.Name("moonrise.playbackDidEnd")
}
