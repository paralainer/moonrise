import AVFoundation
import Foundation

enum MoonriseClientError: LocalizedError {
    case badURL
    case notSignedIn
    case http(Int, String)
    case audioUnavailable

    var errorDescription: String? {
        switch self {
        case .badURL: "Invalid server address."
        case .notSignedIn: "Not signed in to this Moonrise server."
        case let .http(401, _): "The Moonrise server doesn't accept this sign-in any more. Sign out in Settings and sign in again."
        case let .http(code, body): body.isEmpty ? "The Moonrise server returned \(code)." : "The Moonrise server returned \(code). \(body.prefix(200))"
        case .audioUnavailable: "The Moonrise server can't play this audio track."
        }
    }
}

/// A source that is a Moonrise server (`server/`): its own library, accounts, watch state and
/// streams (`/v1/…`, Bearer tokens).
/// Its playback layer is the one `MoonriseServer` speaks to for every client, and here it is the
/// source itself (`moonriseBaseURL` is the current route).
final class MoonriseClient: @unchecked Sendable {
    let sourceId: UUID?
    /// Current route to the server: the LAN address, or a Tailscale loopback forwarder when away.
    var baseURL: URL
    /// The configured address, kept so the router can switch back when home.
    let lanBaseURL: URL
    var token: String?

    /// Answers aren't cached (watch state changes under the same URLs; images have their own cache).
    private var session: URLSession { HTTP.api }

    // MARK: - The server's client

    private static let lock = NSLock()
    nonisolated(unsafe) private static var shared: MoonriseClient?

    /// The client for the signed-in server, kept so its route (`baseURL`) is shared.
    static func `for`(_ source: Source) -> MoonriseClient? {
        guard source.kind == .moonrise else { return nil }
        lock.lock(); defer { lock.unlock() }
        if let c = shared, c.sourceId == source.id { return c }
        guard let url = normalize(source.address) else { return nil }
        let c = MoonriseClient(sourceId: source.id, baseURL: url, token: source.secret)
        shared = c
        return c
    }

    static func forget(_ id: UUID) {
        lock.lock(); defer { lock.unlock() }
        if shared?.sourceId == id { shared = nil }
    }

    /// `host`, `host:port` or a URL; a bare host gets the Moonrise server's port (8097).
    static func normalize(_ address: String) -> URL? {
        var s = address.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !s.isEmpty else { return nil }
        if !s.contains("://") { s = "http://" + s }
        while s.hasSuffix("/") { s.removeLast() }
        guard var c = URLComponents(string: s), c.host?.isEmpty == false else { return nil }
        if c.port == nil, c.scheme == "http" { c.port = MoonriseServer.port }
        c.path = ""
        return c.url
    }

    init(sourceId: UUID?, baseURL: URL, token: String?) {
        self.sourceId = sourceId
        self.baseURL = baseURL
        self.lanBaseURL = baseURL
        self.token = token
    }

    var headers: [String: String] { token.map { ["Authorization": "Bearer \($0)"] } ?? [:] }

    // MARK: - Requests

    static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    func url(_ path: String, _ query: [String: String?] = [:]) -> URL {
        var comps = URLComponents(url: baseURL.appendingPathComponent(path), resolvingAgainstBaseURL: false)!
        let items = query.compactMap { k, v in v.map { URLQueryItem(name: k, value: $0) } }
        if !items.isEmpty {
            comps.queryItems = items.sorted { $0.name < $1.name }
            // URLComponents leaves "+" as it is, and the server's form decoding reads it as a space:
            // "MA 15+", "R 18+", "Disney+" and "Paramount+" matched nothing.
            comps.percentEncodedQuery = comps.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
        }
        return comps.url!
    }

    func send(_ method: String, _ url: URL, body: [String: Any]? = nil, timeout: TimeInterval? = nil,
                      authorized: Bool = true) async throws -> Data {
        // Watch state changes under the same URL: never answer from the cache.
        var req = HTTP.request(url, method: method, headers: authorized ? headers : [:], timeout: timeout,
                               cachePolicy: .reloadIgnoringLocalCacheData)
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body {
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, code) = try await HTTP.data(for: req, on: session)
        guard (200..<300).contains(code) else {
            throw MoonriseClientError.http(code, String(decoding: data.prefix(300), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return data
    }

    func get<T: Decodable>(_ path: String, _ query: [String: String?] = [:], timeout: TimeInterval? = nil) async throws -> T {
        try Self.decoder.decode(T.self, from: try await send("GET", url(path, query), timeout: timeout))
    }

    func post<T: Decodable>(_ path: String, _ body: [String: Any], authorized: Bool = true) async throws -> T {
        try Self.decoder.decode(T.self, from: try await send("POST", url(path), body: body, authorized: authorized))
    }

    // MARK: - Server and sign-in

    struct Info: Decodable, Hashable {
        let serverId: String
        let name: String
        let version: String
        let capabilities: [String]
        /// "unclaimed": a new server, set up with its setup code (`SetupViews`); "claimed"; nil: older servers.
        var setup: String? = nil

        var unclaimed: Bool { setup == "unclaimed" }
    }

    struct User: Decodable { let id: String; let name: String; let isAdmin: Bool? }
    struct AuthResult: Decodable { let token: String; let user: User }

    /// `GET /v1/info` (no sign-in needed): the server's id, name, version and what it can do.
    func info(timeout: TimeInterval = 8) async throws -> Info {
        try Self.decoder.decode(Info.self, from: try await send("GET", url("v1/info"), timeout: timeout, authorized: false))
    }

    var device: [String: Any] { ["device_id": AppInfo.deviceId, "device_name": AppInfo.deviceName] }

    func login(username: String, password: String) async throws -> AuthResult {
        let r: AuthResult = try await post("v1/auth/login", ["username": username, "password": password].merging(device) { a, _ in a },
                                           authorized: false)
        token = r.token
        return r
    }

    /// Revoke this device's token on the server (removing the source).
    func logout() async {
        _ = try? await send("POST", url("v1/auth/logout"), timeout: 5)
    }

    /// A device code: shown on this device, approved on another one (or at `verifyURL`).
    struct DeviceCode: Decodable {
        let deviceCode: String
        let userCode: String
        let verifyUrl: String
        let verifyUrlComplete: String
        let interval: Int
        let expiresIn: Int
    }

    enum DevicePoll { case pending, slowDown, expired, approved(AuthResult) }

    func startDeviceCode() async throws -> DeviceCode {
        try await post("v1/auth/device", device, authorized: false)
    }

    /// `POST /v1/auth/device/poll`'s answer.
    struct DevicePollAnswer: Decodable { let status: String; let token: String?; let user: User? }

    func pollDeviceCode(_ code: String) async throws -> DevicePoll {
        let a: DevicePollAnswer = try await post("v1/auth/device/poll", ["device_code": code], authorized: false)
        switch a.status {
        case "approved":
            guard let t = a.token, let u = a.user else { return .expired }
            token = t
            return .approved(AuthResult(token: t, user: u))
        case "slow_down": return .slowDown
        case "expired": return .expired
        default: return .pending
        }
    }

    /// Approve another device's code with this device's sign-in. Answers that device's name.
    func approveDevice(userCode: String) async throws -> String? {
        struct Answer: Decodable { let approved: Bool?; let deviceName: String? }
        let a: Answer = try await post("v1/auth/device/approve", ["user_code": userCode])
        return a.deviceName
    }

    // MARK: - Browsing

    func libraries() async throws -> [MediaItem] {
        let r: RList<RLibrary> = try await get("v1/libraries")
        return r.items.map(\.mediaItem)
    }

    /// `GET /v1/items`'s answer: a page, and where each letter starts (a title sort).
    struct ItemsAnswer: Decodable {
        struct Letter: Decodable { let letter: String; let offset: Int }
        let items: [RItem]
        let total: Int?
        let index: [Letter]?
    }

    func items(_ q: ItemQuery) async throws -> ItemPage {
        var query: [String: String?] = [
            "sort": Self.sort(q.filter.sort), "order": q.filter.descending ? "desc" : "asc",
            "offset": "\(q.start)", "limit": "\(q.limit)",
        ]
        if q.parent.kind == .library {
            // The Library's views (`@movie`, `@series`): a type across libraries; else one library.
            if q.parent.id.hasPrefix("@") { query["type"] = String(q.parent.id.dropFirst()) } else { query["library"] = q.parent.id }
            query.merge(q.filter.serverQuery) { _, b in b }
            if let only = q.only { query["only"] = only.joined(separator: ",") }
        } else {
            query["parent"] = q.parent.id
        }
        let r: ItemsAnswer = try await get("v1/items", query)
        return ItemPage(items: r.items.map(\.mediaItem), total: r.total,
                        index: (r.index ?? []).map { ItemPage.Letter(letter: $0.letter, offset: $0.offset) })
    }

    /// The facets' answer (`GET /v1/facets`, `GET /v1/libraries/{id}/facets`).
    struct FacetsAnswer: Decodable {
        struct Named: Decodable { let name: String; let count: Int; let backdrop: String? }
        struct Decade: Decodable { let decade: Int; let count: Int; let backdrop: String? }
        struct Coll: Decodable { let id: Int; let name: String; let count: Int; let poster: String?; let backdrop: String? }
        struct Watched: Decodable { let unplayed: Int; let inProgress: Int; let played: Int }
        struct Recent: Decodable { let count: Int }
        struct Lib: Decodable { let id: String; let name: String; let count: Int }
        let total: Int
        let genres: [Named]
        let studios: [Named]
        let decades: [Decade]
        let ratings: [Named]
        let collections: [Coll]
        let resolutions: [Named]
        let hdr: Int
        let watched: Watched?
        let recentlyAdded: Recent?
        let libraries: [Lib]?
    }

    func facets(libraryId: String, filter: LibraryFilter, only: [String]?) async throws -> LibraryFacets {
        var query = filter.serverQuery
        if let only { query["only"] = only.joined(separator: ",") }
        query["recent_days"] = "\(LibraryFilter.recentDays)"
        let a: FacetsAnswer
        if libraryId.hasPrefix("@") {
            query["type"] = String(libraryId.dropFirst())
            a = try await get("v1/facets", query)
        } else {
            a = try await get("v1/libraries/\(libraryId)/facets", query)
        }
        let value = { (n: FacetsAnswer.Named) in LibraryFacets.Value(name: n.name, count: n.count, backdrop: n.backdrop.map(MoonriseImages.ref)) }
        return LibraryFacets(
            total: a.total, genres: a.genres.map(value), studios: a.studios.map(value),
            decades: a.decades.map { LibraryFacets.Decade(decade: $0.decade, count: $0.count, backdrop: $0.backdrop.map(MoonriseImages.ref)) },
            ratings: a.ratings.map(value),
            collections: a.collections.map { LibraryFacets.Collection(id: $0.id, name: $0.name, count: $0.count, poster: $0.poster.map(MoonriseImages.ref),
                                                                       backdrop: $0.backdrop.map(MoonriseImages.ref)) },
            resolutions: a.resolutions.map(value), hdr: a.hdr, recentlyAdded: a.recentlyAdded?.count ?? 0,
            watched: a.watched.map { [.unwatched: $0.unplayed, .inProgress: $0.inProgress, .watched: $0.played] },
            libraries: (a.libraries ?? []).map { LibraryFacets.Library(id: $0.id, name: $0.name, count: $0.count) })
    }

    private static func sort(_ sort: LibrarySort) -> String {
        switch sort {
        case .title: "title"
        case .added: "added"
        case .year: "year"
        case .rating: "rating"
        case .played: "played"
        case .runtime: "runtime"
        }
    }

    func items(ids: [String]) async throws -> [MediaItem] {
        guard !ids.isEmpty else { return [] }
        let r: RList<RItem> = try await get("v1/items", ["ids": ids.joined(separator: ",")])
        return r.items.map(\.mediaItem)
    }

    func item(_ id: String) async throws -> MediaItem {
        let r: RItem = try await get("v1/items/\(id)")
        return r.mediaItem
    }

    /// One item as `GET /v1/items/{id}` answers it (tests).
    static func decodeItem(_ data: Data) throws -> MediaItem {
        try decoder.decode(RItem.self, from: data).mediaItem
    }

    func seasons(seriesId: String) async throws -> [MediaItem] {
        let r: RList<RItem> = try await get("v1/shows/\(seriesId)/seasons")
        return r.items.map(\.mediaItem)
    }

    func episodes(seriesId: String, seasonId: String?) async throws -> [MediaItem] {
        let r: RList<RItem> = try await get("v1/shows/\(seriesId)/episodes", ["season": seasonId])
        return r.items.map(\.mediaItem)
    }

    /// `GET /v1/search`'s answer.
    struct TitleSearchAnswer: Decodable { let titles: [RItem]; let episodes: [RItem] }

    func search(_ term: String) async throws -> [MediaItem] {
        let a: TitleSearchAnswer = try await get("v1/search", ["q": term])
        return (a.titles + a.episodes).map(\.mediaItem)
    }

    /// `GET /v1/search/start`'s answer.
    struct SearchStartAnswer: Decodable { let recent: [RItem]; let suggestions: [RItem] }

    func searchStart() async throws -> SearchStart? {
        guard await MoonriseServer.serves("search-start", base: baseURL) == true else { return nil }
        let a: SearchStartAnswer = try await get("v1/search/start")
        return SearchStart(recent: a.recent.map(\.mediaItem), suggestions: a.suggestions.map(\.mediaItem))
    }

    func recordSearchOpened(_ id: String) async throws {
        _ = try await send("PUT", url("v1/search/recent/\(id)"))
    }

    func clearSearchRecent() async throws {
        _ = try await send("DELETE", url("v1/search/recent"))
    }

    func latest(libraryId: String, limit: Int) async throws -> [MediaItem] {
        let r: RList<RItem> = try await get("v1/libraries/\(libraryId)/latest", ["limit": "\(limit)"])
        return r.items.map(\.mediaItem)
    }

    /// `GET /v1/home`'s answer.
    struct HomeAnswer: Decodable {
        struct Row: Decodable { let type: String; let title: String; let items: [RItem] }
        let resume: [RItem]; let nextUp: [RItem]; let latestTypes: [Row]
        /// Continue Watching (0.24.0 and later).
        let `continue`: [RItem]?
    }

    func home() async throws -> HomeContent {
        let a: HomeAnswer = try await get("v1/home")
        // Recently added by type across libraries (libraries have no type).
        return HomeContent(
            resume: a.resume.map(\.mediaItem), nextUp: a.nextUp.map(\.mediaItem),
            latest: a.latestTypes.map { row in
                // As the Library's views (`LibraryList.views`).
                let view = MediaItem(id: "@" + row.type, kind: .library, collectionType: row.type == "series" ? "tvshows" : "movies",
                                     isFolder: true, name: row.title)
                return .init(library: view, items: row.items.map(\.mediaItem))
            },
            continueWatching: a.continue?.map { (r: RItem) -> HomeContent.ContinueCard in
                let item = r.mediaItem
                let reason = r.continue.flatMap { HomeContent.ContinueCard.Reason(rawValue: $0.reason) } ?? .resume
                return HomeContent.ContinueCard(item: item, reason: reason,
                                                at: r.continue?.at.flatMap(UserState.parseDate) ?? item.lastPlayed ?? .distantPast)
            })
    }

    func removeFromContinue(_ id: String) async throws {
        _ = try await send("DELETE", url("v1/continue/\(id)"))
    }

    /// `GET /v1/shows/{id}/continue`'s answer.
    struct ContinueAnswer: Decodable { let item: RItem? }

    func continueEpisode(seriesId: String) async throws -> MediaItem? {
        let a: ContinueAnswer = try await get("v1/shows/\(seriesId)/continue")
        return a.item?.mediaItem
    }

    /// Remembers the version (file) this user plays of a title, on every device.
    func chooseVersion(itemId: String, fileId: String) async {
        _ = try? await send("PUT", url("v1/items/\(itemId)/version"), body: ["file_id": fileId], timeout: 10)
    }

    func setPlayed(_ id: String, _ played: Bool) async throws {
        _ = try await send(played ? "PUT" : "DELETE", url("v1/items/\(id)/played"))
    }

    // MARK: - Images

    /// Image ids are hashes of the artwork (immutable): the id is the cache tag.
    func imageURL(_ ref: ImageRef, width: Int) -> URL {
        url("v1/images/\(ref.id)", ["w": "\(width)"])
    }

    // MARK: - Playback

    /// The Moonrise server's stream. A retry (its stream failed once) asks for a new session; the
    /// player shows the error when that fails too.
    func playback(_ o: PlaybackOptions) async throws -> PlaybackPlan {
        let base = baseURL, headers = self.headers
        #if DEBUG
        if o.isRetry { fputs("MoonriseClient: the stream failed; asking for a new session\n", stderr) }
        #endif
        async let detail = item(o.itemId)
        let (master, offer) = try await MoonriseServer.offer(base: base, itemId: o.itemId, start: o.start, quality: o.maxBitrate,
                                                         version: o.versionId, headers: headers, timeout: 15)
        let item = try await detail
        if let a = o.audioIndex, !offer.audio.contains(where: { $0.index == a }) {
            if let s = offer.session { await stop(sessionId: s) }
            throw MoonriseClientError.audioUnavailable
        }
        // The version played: the one asked for, else the one the server picked.
        let version = item.versions?.first(where: { $0.id == offer.mediaSourceId }) ?? item.versions?.first
            ?? MediaVersion(id: offer.mediaSourceId ?? o.itemId, runtime: offer.duration)
        return PlaybackPlan(master: master, offer: offer, version: version, sessionId: offer.session, versions: item.versions ?? [])
    }

    func report(_ event: PlaybackEvent, _ r: PlaybackReport) async {
        guard let sid = r.sessionId else { return }
        let path = switch event {
        case .start: "start"
        case .progress: "progress"
        case .stopped: "stop"
        }
        var body: [String: Any] = ["position": r.position, "paused": r.isPaused]
        if let a = r.audioIndex { body["audio"] = a }
        if let s = r.subtitleIndex { body["subtitle"] = s }
        if let c = r.creditsStart { body["credits"] = c }
        if let t = r.tracks { body["tracks"] = t.json }
        do {
            _ = try await send("POST", url("v1/sessions/\(sid)/\(path)"), body: body, timeout: 10)
        } catch {
            #if DEBUG
            fputs("MoonriseReport: \(path) failed: \(error.localizedDescription)\n", stderr)
            #endif
        }
    }

    /// Playback of downloads on the device (`LocalReports`), possibly made offline long ago.
    func reportLocal(_ reports: [[String: Any]]) async throws {
        _ = try await send("POST", url("v1/sessions/local"), body: ["reports": reports], timeout: 15)
    }

    /// The session's transcode and conversion runs stop (the same as `MoonriseServer.close`).
    func stop(sessionId: String) async {
        _ = try? await send("DELETE", url("v1/s/\(sessionId)"), timeout: 5)
    }

    func subtitleURL(itemId: String, versionId: String, index: Int) -> URL? {
        url("v1/items/\(itemId)/subtitles/\(index).srt")
    }

    /// `GET /v1/items/{id}/segments`'s answer.
    struct SegmentsAnswer: Decodable {
        struct Segment: Decodable { let kind: String; let start: Double; let end: Double; let source: String? }
        let items: [Segment]
        let pending: Bool?
        /// Servers 0.28+, asked with `merged=1`.
        let merged: Bool?
        let communityPending: Bool?
    }

    func segments(itemId: String, version: String?, provisional: Bool, community: Bool) async -> ServerSegments {
        let query: [String: String?] = ["version": version, "merged": "1", "provisional": provisional ? "1" : nil, "community": community ? nil : "0"]
        guard let r: SegmentsAnswer = try? await get("v1/items/\(itemId)/segments", query, timeout: 10) else { return ServerSegments(reached: false) }
        return Self.serverSegments(r)
    }

    /// The answer as the app uses it: only `detected` is kept as a source (the rules tell it apart).
    static func serverSegments(_ r: SegmentsAnswer) -> ServerSegments {
        let items = r.items.compactMap { s in
            MediaSegment.Kind(rawValue: s.kind).map { MediaSegment(kind: $0, start: s.start, end: s.end, source: s.source == "detected" ? "detected" : nil) }
        }
        return ServerSegments(items: items, pending: r.pending ?? false, merged: r.merged ?? false, communityPending: r.communityPending ?? false)
    }

    // MARK: - Subtitle search

    func searchSubtitles(itemId: String, language: String, query: String?) async throws -> SubtitleSearch {
        let a = try await MoonriseServer.searchSubtitles(base: baseURL, itemId: itemId, language: language, query: query, headers: headers)
        return SubtitleSearch(origin: .moonriseServer, results: a.results.map(RemoteSubtitle.init(moonrise:)), remainingDownloads: a.remainingDownloads,
                              quota: a.quota)
    }

    func fetchSubtitle(itemId: String, result: RemoteSubtitle) async throws -> SubtitleFetch {
        guard let r = result.moonriseResult else { throw MoonriseClientError.http(400, "Not a Moonrise server result.") }
        return .moonriseTrack(try await MoonriseServer.fetchSubtitle(base: baseURL, itemId: itemId, result: r, headers: headers))
    }

    // MARK: - Downloads

    func downloadURL(itemId: String) -> URL? { MoonriseServer.originalDownloadURL(base: baseURL, itemId: itemId) }

    func fetch(_ url: URL, timeout: TimeInterval) async throws -> Data {
        try await send("GET", url, timeout: timeout)
    }
}

extension MoonriseClient: MediaServerClient {
    var moonriseBaseURL: URL? { baseURL }
}

// MARK: - The server's JSON (snake_case; times in seconds; dates RFC 3339)
// Internal, not private: the tests decode the API contract's examples with them (APIExamplesTests).

struct RList<T: Decodable>: Decodable {
    let items: [T]
    let total: Int?
}

struct RLibrary: Decodable {
    let id: String
    let name: String
    let collectionType: String?
    let backdrop: String?

    var mediaItem: MediaItem {
        let art = backdrop.map(MoonriseImages.ref)
        return MediaItem(id: id, kind: .library, collectionType: collectionType, isFolder: true, name: name,
                         images: ImageSet(poster: nil, backdrop: art, thumb: art, logo: nil, primary: nil))
    }
}

struct RImages: Decodable {
    let poster: String?
    let backdrop: [String]?
    let logo: String?
    let thumb: String?
    let seasonPoster: String?
}

struct RUser: Decodable {
    let position: Double?
    let version: String?
    let played: Bool?
    let playCount: Int?
    let lastPlayedAt: String?
    let playedPercentage: Double?
    let unplayedItemCount: Int?
}

struct RTrack: Decodable {
    let index: Int
    let kind: String
    let codec: String?
    let language: String?
    let title: String?
    let displayTitle: String?
    let isDefault: Bool?
    let isForced: Bool?
    let isExternal: Bool?
    let isText: Bool?
    let width: Int?
    let height: Int?
    let channels: Int?
    let profile: String?
    let bitrate: Int?
    let videoRangeType: String?

    var trackInfo: TrackInfo {
        TrackInfo(kind: MoonriseImages.trackKind(kind), index: index, codec: codec, language: language, title: title,
                  displayTitle: displayTitle, isDefault: isDefault, isForced: isForced, isExternal: isExternal,
                  isText: isText, width: width, height: height, channels: channels, bitRate: bitrate,
                  profile: profile, videoRange: videoRangeType)
    }
}

struct RFile: Decodable {
    let id: String
    let name: String?
    let container: String?
    let size: Int64?
    let duration: Double?
    let bitrate: Int?
    let versionLabel: String?
    let missing: Bool?
    let tracks: [RTrack]?
    let version: RVersion?
    let position: Double?
    let manual: Bool?
    let broken: Bool?
    let brokenReason: String?

    struct RVersion: Decodable {
        let label: String?
        let details: String?
        let edition: String?
        let hdr: String?
        let audioLanguages: [String]?
    }

    /// The file's extension ("mkv", "mp4", "avi"): what the app calls its container.
    var ext: String? {
        guard let n = name, let dot = n.lastIndex(of: "."), dot < n.endIndex else { return nil }
        let e = n[n.index(after: dot)...].lowercased()
        return e.isEmpty || e.count > 5 ? nil : e
    }

    /// The audio track to start with: one in the device's language, else the file's default track, else the first.
    /// ISO 639-2/B codes files carry where they differ from 639-2/T.
    private static let bibliographic = ["fra": "fre", "deu": "ger", "zho": "chi", "ces": "cze", "nld": "dut", "ell": "gre",
                                        "fas": "per", "hye": "arm", "kat": "geo", "mkd": "mac", "msa": "may", "mya": "bur",
                                        "ron": "rum", "slk": "slo", "sqi": "alb", "eus": "baq", "isl": "ice", "cym": "wel", "bod": "tib"]

    static func defaultAudio(_ audio: [TrackInfo]) -> TrackInfo? {
        let preferred = Locale.preferredLanguages.compactMap { Locale(identifier: $0).language.languageCode }
        for code in preferred {
            let t = code.identifier(.alpha3)
            let ids = Set([code.identifier, t, t.flatMap { bibliographic[$0] }].compactMap { $0 })
            let matches = audio.filter { $0.language.map { ids.contains($0.lowercased()) } ?? false }
            if let t = matches.first(where: { $0.isDefault == true }) ?? matches.first { return t }
        }
        return audio.first { $0.isDefault == true } ?? audio.first
    }

    var mediaVersion: MediaVersion {
        let t = (tracks ?? []).map(\.trackInfo)
        let audio = t.filter { $0.kind == .audio }
        let subs = t.filter { $0.kind == .subtitle }
        return MediaVersion(id: id, name: versionLabel ?? name, path: nil, container: ext, size: size, bitrate: bitrate,
                            runtime: duration, defaultAudioIndex: Self.defaultAudio(audio)?.index,
                            defaultSubtitleIndex: subs.first { $0.isDefault == true }?.index, tracks: t,
                            label: version?.label, details: version?.details, edition: version?.edition, hdr: version?.hdr,
                            audioLanguages: version?.audioLanguages, position: position, manual: manual,
                            broken: broken, brokenReason: brokenReason)
    }
}

/// The track summary list items carry (tech badges), without stream indices.
struct RMedia: Decodable {
    struct Video: Decodable { let codec: String?; let width: Int?; let height: Int?; let range: String?; let title: String? }
    struct Audio: Decodable { let codec: String?; let channels: Int?; let profile: String?; let language: String?; let title: String?; let `default`: Bool? }
    struct Subtitle: Decodable { let codec: String?; let language: String?; let text: Bool?; let forced: Bool? }
    let video: Video?
    let audio: [Audio]?
    let subtitles: [Subtitle]?

    var tracks: [TrackInfo] {
        var out: [TrackInfo] = []
        if let v = video {
            out.append(TrackInfo(kind: .video, index: out.count, codec: v.codec, displayTitle: v.title, width: v.width, height: v.height, videoRange: v.range))
        }
        for a in audio ?? [] {
            out.append(TrackInfo(kind: .audio, index: out.count, codec: a.codec, language: a.language, displayTitle: a.title,
                                 isDefault: a.default, channels: a.channels, profile: a.profile))
        }
        for s in subtitles ?? [] {
            out.append(TrackInfo(kind: .subtitle, index: out.count, codec: s.codec, language: s.language, isForced: s.forced, isText: s.text))
        }
        return out
    }
}

struct RPerson: Decodable {
    let id: String
    let name: String?
    let kind: String?
    let role: String?
    let image: String?
}

struct RChapter: Decodable {
    let start: Double?
    let name: String?
}

struct RContinue: Decodable {
    let reason: String
    let at: String?
}

/// `ratings`: `{imdb: {value, votes}, rotten_tomatoes: {value}, metacritic: {value}}` (0.26).
struct RRatings: Decodable {
    struct Score: Decodable {
        let value: Double?
        let votes: Int?
    }
    let imdb: Score?
    let rottenTomatoes: Score?
    let metacritic: Score?

    var ratings: ExternalRatings? {
        let r = ExternalRatings(imdb: imdb?.value, rottenTomatoes: rottenTomatoes?.value.map { Int($0.rounded()) },
                                metacritic: metacritic?.value.map { Int($0.rounded()) }, imdbVotes: imdb?.votes)
        return r.isEmpty ? nil : r
    }
}

struct RItem: Decodable {
    let id: String
    let kind: String
    let title: String?
    let year: Int?
    let premiereDate: String?
    let overview: String?
    let officialRating: String?
    let communityRating: Double?
    let criticRating: Double?
    let ratings: RRatings?
    let genres: [String]?
    let studios: [String]?
    let collection: MediaCollectionRef?
    let addedAt: String?
    let providerIds: [String: String]?
    let runtime: Double?
    let images: RImages?
    let parentImages: RImages?
    let seriesId: String?
    let seriesTitle: String?
    let seasonId: String?
    let seasonTitle: String?
    let seasonNo: Int?
    let episodeNo: Int?
    let episodeEnd: Int?
    let part: Int?
    let parts: Int?
    let libraryId: String?
    let user: RUser?
    let media: RMedia?
    let files: [RFile]?
    /// Lists: how many files (versions) the title has.
    let versions: Int?
    let people: [RPerson]?
    let chapters: [RChapter]?
    /// In `/v1/home`'s Continue Watching: why it's there and since when.
    let `continue`: RContinue?

    var mediaItem: MediaItem {
        let k: MediaItem.Kind = switch kind {
        case "movie": .movie
        case "series": .series
        case "season": .season
        case "episode": .episode
        default: MediaItem.Kind(rawValue: "moonrise:" + kind)
        }
        let files = (self.files ?? []).filter { $0.missing != true }
        let versions = files.isEmpty ? nil : files.map(\.mediaVersion)
        var item = MediaItem(
            id: id, kind: k, collectionType: nil, isFolder: k == .series || k == .season,
            name: title, overview: overview, genres: genres, officialRating: officialRating,
            communityRating: communityRating, criticRating: criticRating, productionYear: year,
            premiereDate: premiereDate, runtime: runtime, container: files.first?.ext,
            studios: studios, collection: collection, addedAt: addedAt.flatMap(UserState.parseDate),
            seriesId: seriesId, seriesName: seriesTitle, seasonId: seasonId, seasonName: seasonTitle,
            seasonNumber: seasonNo, episodeNumber: k == .season ? nil : episodeNo, libraryId: libraryId,
            images: imageSet(k), seriesImages: seriesImageSet(k),
            people: people?.map { p in
                MediaPerson(id: p.id, name: p.name, role: p.role, kind: p.kind.map(MoonriseImages.personKind), image: p.image.map(MoonriseImages.ref))
            },
            providerIds: providerIds,
            user: user.map { u in
                UserState(position: u.position, playedPercentage: u.playedPercentage, played: u.played, isFavorite: nil,
                          unplayedItemCount: u.unplayedItemCount, lastPlayed: u.lastPlayedAt.flatMap(UserState.parseDate))
            },
            versions: versions,
            tracks: versions?.first?.tracks ?? media?.tracks,
            chapters: chapters?.map { Chapter(start: $0.start, name: $0.name) })
        item.versionCount = self.versions.map { $0 > 0 ? $0 : 1 } ?? item.versions?.count
        item.chosenVersionId = user?.version
        if versions == nil, let all = self.files, !all.isEmpty { item.missingFiles = all.map { $0.name ?? "" } }
        item.ratings = ratings?.ratings
        if k == .episode {
            item.episodeEnd = episodeEnd
            item.part = part
            item.parts = parts
        }
        return item
    }

    /// Artwork by role: an episode's poster is its
    /// show's, its card art its own still; a card falls back from thumb to the show's, the
    /// backdrop and the poster.
    private func imageSet(_ k: MediaItem.Kind) -> ImageSet {
        let own = images, parent = parentImages
        let poster = (own?.poster ?? parent?.poster).map(MoonriseImages.ref)
        let backdrop = (own?.backdrop?.first ?? parent?.backdrop?.first).map(MoonriseImages.ref)
        let still = k == .episode ? own?.thumb.map(MoonriseImages.ref) : nil
        let thumb = still ?? (own?.thumb ?? parent?.thumb).map(MoonriseImages.ref) ?? backdrop ?? poster
        let primary = k == .episode ? still : own?.poster.map(MoonriseImages.ref)
        return ImageSet(poster: poster, backdrop: backdrop, thumb: thumb, logo: own?.logo.map(MoonriseImages.ref), primary: primary)
    }

    /// The show's backdrop and logo as an episode or season carries them.
    private func seriesImageSet(_ k: MediaItem.Kind) -> ImageSet? {
        guard k == .episode || k == .season, let p = parentImages else { return nil }
        let backdrop = p.backdrop?.first.map(MoonriseImages.ref)
        return ImageSet(poster: p.poster.map(MoonriseImages.ref), backdrop: backdrop, thumb: backdrop, logo: p.logo.map(MoonriseImages.ref), primary: nil)
    }
}

private enum MoonriseImages {
    static func ref(_ id: String) -> ImageRef { ImageRef(id: id, type: nil, index: nil, tag: id) }

    static func trackKind(_ k: String) -> TrackInfo.Kind {
        switch k.lowercased() {
        case "video": .video
        case "audio": .audio
        case "subtitle": .subtitle
        default: .other
        }
    }

    /// "Actor", "Director", "Writer"… as the pages filter on them.
    static func personKind(_ k: String) -> String {
        switch k.lowercased() {
        case "actor", "cast", "guest_star", "gueststar": "Actor"
        case "director": "Director"
        case "writer", "screenplay": "Writer"
        default: k.prefix(1).uppercased() + k.dropFirst()
        }
    }
}

extension RemoteSubtitle {
    init(moonrise r: MoonriseServer.SearchResult) {
        self.init(id: r.id, origin: .moonriseServer, name: r.name, language: r.language, provider: "OpenSubtitles",
                  format: nil, downloads: r.downloads, hashMatch: r.hashMatch, hearingImpaired: r.hearingImpaired,
                  machineTranslated: r.machineTranslated == true || r.aiTranslated == true, fetched: r.fetched == true,
                  moonriseResult: r)
    }
}

extension UserState {
    /// RFC 3339, with or without fractional seconds (any number of digits).
    static func parseDate(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = f.date(from: s) { return date }
        if let dot = s.firstIndex(of: "."), let z = s.lastIndex(where: { $0 == "Z" || $0 == "+" }) {
            let frac = s[s.index(after: dot)..<z].prefix(3)
            return f.date(from: s[..<dot] + "." + frac + s[z...])
        }
        f.formatOptions = [.withInternetDateTime]
        return f.date(from: s)
    }
}
