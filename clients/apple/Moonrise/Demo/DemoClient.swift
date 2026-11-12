import Foundation

/// "Try the Demo": the app without a Moonrise server (for App Review, and anyone curious before
/// setting one up). Four open movies of the Blender Foundation, with their metadata, artwork and
/// subtitles bundled in the app (`Moonrise/Demo/Resources`) and their video streamed from the public
/// hosts that publish them (no server of ours). Browsing, title pages, playback, audio and
/// subtitles work as with a server; watch progress lives in memory until the demo is left.
/// Nothing is written to any server, nothing is downloaded.
///
/// The films and their licences (credited on each title page):
/// - Big Buck Bunny (2008), CC BY 3.0, © Blender Foundation, peach.blender.org
/// - Elephants Dream (2006), CC BY 2.5, Blender Foundation / Netherlands Media Art Institute, orange.blender.org
/// - Sintel (2010), CC BY 3.0, © Blender Foundation, durian.blender.org
/// - Tears of Steel (2012), CC BY 3.0, © Blender Foundation, mango.blender.org
final class DemoClient: MediaServerClient, @unchecked Sendable {
    static let shared = DemoClient()

    /// The demo's source id (fixed: Recents and the Home cache of the demo are forgotten by it).
    static let sourceId = UUID(uuidString: "00000000-DE30-4EE1-8000-000000000001")!
    static var source: Source {
        Source(id: sourceId, kind: .demo, name: "Moonrise Demo", address: "demo", username: "Demo")
    }

    var headers: [String: String] { [:] }
    /// No Moonrise server: the player skips everything server-specific (renditions, windows, search).
    var moonriseBaseURL: URL? { nil }

    private let lock = NSLock()
    /// Watch state of this demo session (item id → state).
    private var state: [String: UserState] = [:]

    /// The library's one view of movies, as `LibraryList.views` names it.
    static let library = MediaItem(id: "@movie", kind: .library, collectionType: "movies", isFolder: true, name: "Movies")

    // MARK: The films

    struct Film {
        let key: String
        let name: String
        let year: Int
        let runtime: Double
        let overview: String
        let genres: [String]
        let director: String
        let license: String
        let site: String
        let width: Int
        let height: Int
        /// Streams to try in order (the first that answers plays).
        let streams: [URL]
        /// Bundled subtitles: ISO 639-2 code and file suffix (`demo-<key>-<suffix>.srt`).
        let subtitles: [(language: String, file: String)]
        /// When it was "added" (sorting by Date Added).
        let added: TimeInterval
    }

    static let films: [Film] = [
        Film(key: "sintel", name: "Sintel", year: 2010, runtime: 888,
             overview: "A lonely young woman, Sintel, searches for the baby dragon she rescued and raised, braving a harsh land of snow, deserts and swamps — and a truth she isn't ready for.",
             genres: ["Animation", "Fantasy"], director: "Colin Levy",
             license: "CC BY 3.0 · © Blender Foundation", site: "durian.blender.org", width: 2048, height: 872,
             streams: [URL(string: "https://archive.org/download/Sintel/sintel-2048-stereo.mp4")!],
             subtitles: [("fre", "fr"), ("dut", "nl"), ("rus", "ru")], added: 4),
        Film(key: "tos", name: "Tears of Steel", year: 2012, runtime: 734,
             overview: "In a future Amsterdam, a group of warriors and scientists make a last stand against destructive robots — by going back to the moment a young couple broke up.",
             genres: ["Science Fiction", "Action"], director: "Ian Hubert",
             license: "CC BY 3.0 · © Blender Foundation", site: "mango.blender.org", width: 1280, height: 534,
             streams: [URL(string: "https://download.blender.org/demo/movies/ToS/tears_of_steel_720p.mov")!,
                       URL(string: "https://archive.org/download/Tears-of-Steel/tears_of_steel_720p.mp4")!],
             subtitles: [("eng", "en"), ("ger", "de"), ("spa", "es"), ("fre", "fr"), ("rus", "ru")], added: 3),
        Film(key: "bbb", name: "Big Buck Bunny", year: 2008, runtime: 596,
             overview: "A giant, gentle rabbit's lazy day in the meadow is spoiled by three bullying rodents. He decides to get even — in style.",
             genres: ["Animation", "Comedy"], director: "Sacha Goedegebure",
             license: "CC BY 3.0 · © Blender Foundation", site: "peach.blender.org", width: 1920, height: 1080,
             streams: [URL(string: "https://test-streams.mux.dev/x36xhzz/x36xhzz.m3u8")!,
                       URL(string: "https://archive.org/download/BigBuckBunny_124/Content/big_buck_bunny_720p_surround.mp4")!],
             subtitles: [], added: 2),
        Film(key: "ed", name: "Elephants Dream", year: 2006, runtime: 654,
             overview: "Two men, the old Proog and the young Emo, wander through an endless, strange machine. Proog knows its rules; Emo sees it very differently.",
             genres: ["Animation", "Science Fiction"], director: "Bassam Kurdali",
             license: "CC BY 2.5 · Blender Foundation, Netherlands Media Art Institute", site: "orange.blender.org", width: 1024, height: 576,
             streams: [URL(string: "https://download.blender.org/ED/elephantsdream-1024-h264-st-aac.mov")!,
                       URL(string: "https://archive.org/download/ElephantsDream/ed_hd.mp4")!],
             subtitles: [], added: 1),
    ]

    private static func film(_ id: String) -> Film? { films.first { "demo-\($0.key)" == id } }

    /// The credit a demo title's page shows (nil for anything else).
    static func credit(_ id: String) -> String? {
        film(id).map { "Demo film · \($0.license) · \($0.site)" }
    }

    private func item(_ f: Film) -> MediaItem {
        var tracks: [TrackInfo] = [
            TrackInfo(kind: .video, index: 0, codec: "h264", width: f.width, height: f.height, videoRange: "SDR"),
            TrackInfo(kind: .audio, index: 1, codec: "aac", language: "eng", title: "English", isDefault: true, channels: 2),
        ]
        for (i, s) in f.subtitles.enumerated() {
            tracks.append(TrackInfo(kind: .subtitle, index: 2 + i, codec: "srt", language: s.language, isExternal: false, isText: true))
        }
        let poster = ImageRef(id: "demo-\(f.key)-poster", tag: "1")
        let backdrop = ImageRef(id: "demo-\(f.key)-backdrop", tag: "1")
        var m = MediaItem(id: "demo-\(f.key)", kind: .movie)
        m.name = f.name
        m.overview = f.overview
        m.genres = f.genres
        m.officialRating = "CC BY"
        m.productionYear = f.year
        m.runtime = f.runtime
        m.container = f.streams[0].pathExtension
        m.studios = ["Blender Foundation"]
        m.addedAt = Date(timeIntervalSince1970: 1_790_000_000 + f.added * 86_400)
        m.libraryId = "demo"
        m.images = ImageSet(poster: poster, backdrop: backdrop, thumb: backdrop, logo: nil, primary: poster)
        m.people = [MediaPerson(id: "demo-dir-\(f.key)", name: f.director, role: "Director", kind: "Director")]
        m.versions = [MediaVersion(id: "demo-\(f.key)-v", name: nil, container: m.container, bitrate: nil, runtime: f.runtime,
                                   defaultAudioIndex: 1, tracks: tracks)]
        m.tracks = tracks
        m.user = lock.withLock { state[m.id] } ?? UserState(played: false)
        return m
    }

    private var all: [MediaItem] { Self.films.map(item) }

    // MARK: Browsing

    func libraries() async throws -> [MediaItem] { [Self.library] }

    func items(_ q: ItemQuery) async throws -> ItemPage {
        guard q.parent.kind == .library, q.parent.id != "@series" else { return ItemPage(items: [], total: 0) }
        var list = all.filter { m in
            let f = q.filter
            if !f.genres.isEmpty, Set(m.genres ?? []).isDisjoint(with: f.genres) { return false }
            if let d = f.decade, (m.productionYear ?? 0) / 10 * 10 != d { return false }
            if let only = q.only, !only.contains(m.id) { return false }
            switch f.watched {
            case .all: return true
            case .watched: return m.user?.played == true
            case .unwatched: return m.user?.played != true
            case .inProgress: return (m.user?.position ?? 0) > 0
            }
        }
        list.sort { a, b in
            switch q.filter.sort {
            case .title, .rating: return a.displayTitle < b.displayTitle
            case .year: return (a.productionYear ?? 0) < (b.productionYear ?? 0)
            case .added: return (a.addedAt ?? .distantPast) < (b.addedAt ?? .distantPast)
            case .runtime: return (a.runtime ?? 0) < (b.runtime ?? 0)
            case .played: return (a.user?.lastPlayed ?? .distantPast) < (b.user?.lastPlayed ?? .distantPast)
            }
        }
        if q.filter.descending { list.reverse() }
        let page = Array(list.dropFirst(q.start).prefix(q.limit))
        var letters: [ItemPage.Letter] = []
        if q.filter.sort == .title {
            for (i, m) in list.enumerated() {
                let l = String(m.displayTitle.prefix(1)).uppercased()
                if letters.last?.letter != l { letters.append(ItemPage.Letter(letter: l, offset: i)) }
            }
        }
        return ItemPage(items: page, total: list.count, index: letters)
    }

    func items(ids: [String]) async throws -> [MediaItem] { all.filter { ids.contains($0.id) } }

    func item(_ id: String) async throws -> MediaItem {
        guard let f = Self.film(id) else { throw DemoError.notInDemo }
        return item(f)
    }

    func seasons(seriesId: String) async throws -> [MediaItem] { [] }
    func episodes(seriesId: String, seasonId: String?) async throws -> [MediaItem] { [] }

    func search(_ term: String) async throws -> [MediaItem] {
        let t = term.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return [] }
        return all.filter { $0.displayTitle.localizedCaseInsensitiveContains(t) || ($0.genres ?? []).contains { $0.localizedCaseInsensitiveContains(t) } }
    }

    func facets(libraryId: String, filter: LibraryFilter, only: [String]?) async throws -> LibraryFacets {
        let items = libraryId == "@series" ? [] : all.filter { only?.contains($0.id) ?? true }
        var f = LibraryFacets()
        f.total = items.count
        var genres: [String: Int] = [:]
        var decades: [Int: Int] = [:]
        for m in items {
            for g in m.genres ?? [] { genres[g, default: 0] += 1 }
            if let y = m.productionYear { decades[y / 10 * 10, default: 0] += 1 }
        }
        f.genres = genres.sorted { $0.key < $1.key }.map { LibraryFacets.Value(name: $0.key, count: $0.value, backdrop: nil, poster: nil) }
        f.studios = items.isEmpty ? [] : [LibraryFacets.Value(name: "Blender Foundation", count: items.count, backdrop: nil, poster: nil)]
        f.decades = decades.sorted { $0.key > $1.key }.map { LibraryFacets.Decade(decade: $0.key, count: $0.value, backdrop: nil) }
        f.watched = [.unwatched: items.filter { $0.user?.played != true }.count,
                     .inProgress: items.filter { ($0.user?.position ?? 0) > 0 }.count,
                     .watched: items.filter { $0.user?.played == true }.count]
        return f
    }

    func latest(libraryId: String, limit: Int) async throws -> [MediaItem] { Array(all.prefix(limit)) }

    func home() async throws -> HomeContent {
        let items = all
        let resume = items.filter { ($0.user?.position ?? 0) > 0 }.sorted { ($0.user?.lastPlayed ?? .distantPast) > ($1.user?.lastPlayed ?? .distantPast) }
        let view = MediaItem(id: "@movie", kind: .library, collectionType: "movies", isFolder: true, name: "Demo Movies")
        return HomeContent(resume: resume, nextUp: [], latest: [.init(library: view, items: items)])
    }

    func continueEpisode(seriesId: String) async throws -> MediaItem? { nil }

    func setPlayed(_ id: String, _ played: Bool) async throws {
        lock.withLock {
            var s = state[id] ?? UserState()
            s.played = played
            s.position = nil
            s.playedPercentage = played ? 100 : nil
            s.lastPlayed = played ? Date() : s.lastPlayed
            state[id] = s
        }
    }

    // MARK: Images

    /// Bundled artwork (`Moonrise/Demo/Resources`).
    func imageURL(_ ref: ImageRef, width: Int) -> URL {
        Bundle.main.url(forResource: ref.id, withExtension: "jpg") ?? URL(fileURLWithPath: "/dev/null")
    }

    // MARK: Playback

    func playback(_ options: PlaybackOptions) async throws -> PlaybackPlan {
        guard let f = Self.film(options.itemId) else { throw DemoError.notInDemo }
        let m = item(f)
        let url = await Self.firstAnswering(f.streams)
        let offer = MoonriseServer.Playback(
            supported: true, reasons: [], master: url.absoluteString, video: .init(codec: "h264", range: "SDR"),
            audio: [.init(index: 1, name: "English", language: "eng", converted: false)], subtitles: [],
            method: "copy", variant: nil, variants: nil, session: nil, pending: nil, mediaSourceId: m.versions?.first?.id,
            duration: f.runtime)
        return PlaybackPlan(master: url, offer: offer, version: m.versions![0], sessionId: nil)
    }

    /// The first stream whose host answers (a quick range request), else the first.
    private static func firstAnswering(_ urls: [URL]) async -> URL {
        guard urls.count > 1 else { return urls[0] }
        for u in urls {
            // The stream hosts are not a Moonrise server: no token.
            let req = HTTP.request(u, headers: ["Range": "bytes=0-1"], timeout: 6)
            if let (_, code) = try? await HTTP.data(for: req), code > 0, code < 400 {
                return u
            }
        }
        return urls[0]
    }

    func report(_ event: PlaybackEvent, _ report: PlaybackReport) async {
        guard let f = Self.film(report.itemId) else { return }
        lock.withLock {
            var s = state[report.itemId] ?? UserState()
            s.lastPlayed = Date()
            if report.position >= f.runtime * 0.9 {
                s.played = true
                s.position = nil
                s.playedPercentage = 100
            } else if report.position >= f.runtime * 0.05 {
                s.position = report.position
                s.playedPercentage = report.position / f.runtime * 100
            }
            state[report.itemId] = s
        }
    }

    func stop(sessionId: String) async {}

    func subtitleURL(itemId: String, versionId: String, index: Int) -> URL? {
        guard let f = Self.film(itemId), index >= 2, index - 2 < f.subtitles.count else { return nil }
        return Bundle.main.url(forResource: "demo-\(f.key)-\(f.subtitles[index - 2].file)", withExtension: "srt")
    }

    func segments(itemId: String, version: String?, provisional: Bool, community: Bool) async -> ServerSegments { ServerSegments() }

    func searchSubtitles(itemId: String, language: String, query: String?) async throws -> SubtitleSearch { throw DemoError.notInDemo }
    func fetchSubtitle(itemId: String, result: RemoteSubtitle) async throws -> SubtitleFetch { throw DemoError.notInDemo }

    func downloadURL(itemId: String) -> URL? { nil }

    func fetch(_ url: URL, timeout: TimeInterval) async throws -> Data {
        if url.isFileURL { return try Data(contentsOf: url) }
        return try await HTTP.data(for: HTTP.request(url, timeout: timeout, cachePolicy: .returnCacheDataElseLoad)).0
    }

    /// Leaving the demo forgets its watch state.
    func reset() { lock.withLock { state = [:] } }

    enum DemoError: LocalizedError {
        case notInDemo
        var errorDescription: String? { "Not available in the demo. Connect to your own Moonrise server for this." }
    }
}

/// Whether the demo is what the app shows now (readable off the main actor).
enum DemoMode {
    nonisolated(unsafe) static var isActive = false
}
