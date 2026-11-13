import AVFoundation
import Foundation

/// What the UI and the player need from the media server: `MoonriseClient`, the Moonrise server's API
/// (`/v1/…`). Answers are the neutral models (`MediaModels.swift`).
///
/// The Moonrise server's playback layer (`MoonriseServer`: its HLS, audio conversion, transcodes,
/// subtitle files, downloads, OpenSubtitles): `moonriseBaseURL` says where it answers on the
/// current route, and `headers` authenticate requests to it.
protocol MediaServerClient: AnyObject, Sendable {
    /// Authentication headers for API requests and streams.
    var headers: [String: String] { get }
    /// The Moonrise server's playback layer on the current route, nil when it isn't reachable.
    var moonriseBaseURL: URL? { get }

    // MARK: Browsing

    /// Movie and show libraries.
    func libraries() async throws -> [MediaItem]
    /// A page of a library (or a folder's children).
    func items(_ query: ItemQuery) async throws -> ItemPage
    /// Several items in one request (order not guaranteed).
    func items(ids: [String]) async throws -> [MediaItem]
    /// The full item: people, versions with tracks, chapters, user state.
    func item(_ id: String) async throws -> MediaItem
    func seasons(seriesId: String) async throws -> [MediaItem]
    /// A season's episodes, or (`seasonId` nil) every episode of the show in airing order,
    /// specials last.
    func episodes(seriesId: String, seasonId: String?) async throws -> [MediaItem]
    /// Titles (movies and shows) first, then episodes.
    func search(_ term: String) async throws -> [MediaItem]
    /// What Search offers before typing; nil: this server doesn't (capability `search-start`).
    func searchStart() async throws -> SearchStart?
    func recordSearchOpened(_ id: String) async throws
    func clearSearchRecent() async throws
    /// What a library's filters can pick under `filter` (`only`: within these titles), with counts.
    func facets(libraryId: String, filter: LibraryFilter, only: [String]?) async throws -> LibraryFacets
    /// Recently added titles of a library, new episodes grouped into their show.
    func latest(libraryId: String, limit: Int) async throws -> [MediaItem]
    /// Home's server rows: Continue Watching (resume, next up), and recently added per library.
    func home() async throws -> HomeContent
    /// The episode to continue a show with: the one in progress, else the next one.
    func continueEpisode(seriesId: String) async throws -> MediaItem?
    func setPlayed(_ id: String, _ played: Bool) async throws
    /// Remove from Continue Watching, on every device: the title's show (or film) stays out of
    /// the row until it's played again.
    func removeFromContinue(_ id: String) async throws

    // MARK: Images

    func imageURL(_ ref: ImageRef, width: Int) -> URL

    // MARK: Playback

    /// How to play a title: which stream, its tracks, and the session for reports.
    func playback(_ options: PlaybackOptions) async throws -> PlaybackPlan
    /// Playback reports (they make the server's watch state).
    func report(_ event: PlaybackEvent, _ report: PlaybackReport) async
    /// Stop the server's jobs for a playback session (a restart or the end of playback).
    func stop(sessionId: String) async
    /// A text subtitle track as a file (SRT), when the server serves it that way.
    func subtitleURL(itemId: String, versionId: String, index: Int) -> URL?
    /// The skippable segments to offer (intro, credits…), of a version when one is given: merged
    /// with the community databases' by the server (`merged=1`, servers 0.28+; `provisional`: their
    /// credits while the file is still looked at; `community`: false leaves them out). An older
    /// server answers with the file's own (`merged` false).
    func segments(itemId: String, version: String?, provisional: Bool, community: Bool) async -> ServerSegments

    // MARK: Subtitle search

    /// Subtitles on OpenSubtitles (or the server's own provider) for a title.
    func searchSubtitles(itemId: String, language: String, query: String?) async throws -> SubtitleSearch
    /// Fetch a result: it becomes a track of the title.
    func fetchSubtitle(itemId: String, result: RemoteSubtitle) async throws -> SubtitleFetch

    // MARK: Downloads

    /// The original file, as it is.
    func downloadURL(itemId: String) -> URL?

    // MARK: Requests

    /// An authenticated GET (images, subtitle files).
    func fetch(_ url: URL, timeout: TimeInterval) async throws -> Data
}

extension MediaServerClient {
    func fetch(_ url: URL) async throws -> Data { try await fetch(url, timeout: 180) }
    /// Nothing is kept on the server (the demo).
    func removeFromContinue(_ id: String) async throws {}
    func searchStart() async throws -> SearchStart? { nil }
    func recordSearchOpened(_ id: String) async throws {}
    func clearSearchRecent() async throws {}

    // Artwork by role, as the cards and pages ask for it.
    func posterURL(_ item: MediaItem, maxWidth: Int = 400) -> URL? { item.images.poster.map { imageURL($0, width: maxWidth) } }
    func backdropURL(_ item: MediaItem, maxWidth: Int = 1600) -> URL? { item.images.backdrop.map { imageURL($0, width: maxWidth) } }
    /// Landscape artwork for "Continue Watching" style cards.
    func thumbURL(_ item: MediaItem, maxWidth: Int = 700) -> URL? { item.images.thumb.map { imageURL($0, width: maxWidth) } }
    func logoURL(_ item: MediaItem) -> URL? { item.images.logo.map { imageURL($0, width: 800) } }

    func streamAsset(_ url: URL) -> AVURLAsset { AVURLAsset(url: url, options: MediaStream.assetOptions(headers)) }
}

enum MediaStream {
    static let userAgent = "Moonrise/1 AppleCoreMedia"

    static func assetOptions(_ headers: [String: String]) -> [String: Any] {
        ["AVURLAssetHTTPHeaderFieldsKey": headers.filter { $0.key != "User-Agent" },
         AVURLAssetHTTPUserAgentKey: userAgent]
    }
}

/// The client for the server (nil for a bad address).
enum MediaServers {
    static func client(for source: Source) -> (any MediaServerClient)? {
        switch source.kind {
        case .moonrise: MoonriseClient.for(source)
        case .demo: DemoClient.shared
        default: nil
        }
    }
}

// MARK: - Browsing

struct ItemQuery {
    /// The library or folder.
    var parent: MediaItem
    var filter: LibraryFilter = LibraryFilter()
    /// Within these titles only (the downloads, for the Downloaded filter).
    var only: [String]?
    var start: Int
    var limit: Int
}

struct ItemPage {
    /// Where a letter starts in a title-sorted list (offsets into the whole list).
    struct Letter: Hashable {
        var letter: String
        var offset: Int
    }
    var items: [MediaItem]
    var total: Int?
    var index: [Letter] = []
}

struct HomeContent {
    struct Latest {
        var library: MediaItem
        var items: [MediaItem]
    }
    /// A card of Continue Watching as the server decided it (the same on every device).
    struct ContinueCard {
        enum Reason: String { case resume, nextEpisode = "next_episode", collectionNext = "collection_next" }
        var item: MediaItem
        var reason: Reason
        /// When the activity that put it there happened (for a next episode: the previous one finished).
        var at: Date
    }
    var resume: [MediaItem]
    var nextUp: [MediaItem]
    /// Per library, in library order.
    var latest: [Latest]
    /// Continue Watching, latest activity first. Nil from servers before 0.24.0 (the app builds
    /// it from `resume` and `nextUp`).
    var continueWatching: [ContinueCard]? = nil
}

// MARK: - Playback

struct PlaybackOptions {
    var itemId: String
    /// The version played so far (a restart keeps it).
    var versionId: String?
    /// Seconds.
    var start: Double
    /// Bits/s; nil: the original.
    var maxBitrate: Int?
    /// The audio track wanted (nil: the version's default).
    var audioIndex: Int?
    /// A new session after the previous stream failed (the one retry).
    var isRetry = false
}

struct PlaybackPlan {
    /// The Moonrise server's HLS master playlist: it starts exactly at the position asked for and
    /// carries every audio track it serves as a rendition.
    var master: URL
    var offer: MoonriseServer.Playback
    /// The version played: its tracks, default audio, bitrate and size.
    var version: MediaVersion
    /// The playback session, for reports and `stop(sessionId:)`.
    var sessionId: String?
    /// All of the title's versions (the player's Version menu).
    var versions: [MediaVersion] = []
}

enum PlaybackEvent { case start, progress, stopped }

enum PlayMethod: String { case directPlay = "DirectPlay", directStream = "DirectStream", transcode = "Transcode" }

struct PlaybackReport {
    var itemId: String
    var versionId: String?
    var sessionId: String?
    /// Seconds.
    var position: Double
    var isPaused: Bool
    var playMethod: PlayMethod
    var audioIndex: Int?
    /// The subtitle track showing (the server's index).
    var subtitleIndex: Int?
    /// Where the end credits start (`SegmentRules.creditsStart`).
    var creditsStart: Double? = nil
    /// The audio and subtitle tracks playing (nil: not known yet), kept by the server for the file.
    var tracks: TrackRules.Choice? = nil
}

// MARK: - Subtitle search

/// A subtitle found online for a title.
struct RemoteSubtitle: Identifiable, Hashable {
    enum Origin: Hashable {
        /// The Moonrise server's OpenSubtitles account: a fetched result is kept on the server.
        case moonriseServer
    }
    let id: String
    var origin: Origin
    var name: String
    var language: String?
    var provider: String?
    var format: String?
    var downloads: Int?
    var hashMatch: Bool
    var hearingImpaired: Bool
    var machineTranslated: Bool
    /// Already fetched for this title.
    var fetched: Bool
    /// Moonrise server results, as it answered (fetching sends them back).
    var moonriseResult: MoonriseServer.SearchResult?
}

struct SubtitleSearch {
    /// Who searched (and fetches a result).
    var origin: RemoteSubtitle.Origin
    var results: [RemoteSubtitle]
    /// Downloads left today on the account that fetches.
    var remainingDownloads: Int?
    /// The server's quota (tier, left, reset), from servers 0.12.0 on.
    var quota: MoonriseServer.SubtitleQuota?
}

enum SubtitleFetch {
    /// A new track of the title, listed by the Moonrise server.
    case moonriseTrack(MoonriseServer.SubtitleTrack)
}

/// What the server knows of a title's skippable segments.
struct ServerSegments {
    var items: [MediaSegment] = []
    /// The server is still looking for the end credits in the file: ask again later.
    var pending = false
    /// The server answered (false: it couldn't be asked; ask again later).
    var reached = true
    /// The server merged them with the community databases' (the segments to offer, as they are).
    var merged = false
    /// The server is still looking the title up in the community databases: ask again shortly.
    var communityPending = false
}
