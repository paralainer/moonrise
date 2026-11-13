import Foundation

// Library models. The server client (`MoonriseClient`) answers with these, so the UI and the player
// don't depend on the server's wire format.
// Times are seconds, dates are already parsed, and images are references the client turns into
// URLs (`MediaServerClient.imageURL`). Codable: Home keeps its last result on disk.

struct MediaItem: Codable, Identifiable, Hashable {
    /// What an item is. Open-ended: a server can have kinds the app doesn't know.
    struct Kind: RawRepresentable, Codable, Hashable {
        let rawValue: String
        init(rawValue: String) { self.rawValue = rawValue }

        static let movie = Kind(rawValue: "movie")
        static let series = Kind(rawValue: "series")
        static let season = Kind(rawValue: "season")
        static let episode = Kind(rawValue: "episode")
        static let video = Kind(rawValue: "video")
        static let musicVideo = Kind(rawValue: "music_video")
        static let collection = Kind(rawValue: "collection")
        static let folder = Kind(rawValue: "folder")
        /// A library (Movies, Shows…), as `MediaServerClient.libraries()` lists them.
        static let library = Kind(rawValue: "library")
        static let other = Kind(rawValue: "other")
    }

    let id: String
    var kind: Kind
    /// Libraries: what they hold ("movies", "tvshows", "homevideos", "musicvideos", "mixed").
    var collectionType: String?
    /// Browsing it lists children (libraries, folders, collections).
    var isFolder: Bool?

    // Text
    var name: String?
    var overview: String?
    var genres: [String]?
    var officialRating: String?
    /// 0-10.
    var communityRating: Double?
    /// Rotten Tomatoes %, when the server has it.
    var criticRating: Double?
    /// IMDb, Rotten Tomatoes and Metacritic, as the server keeps them (servers from 0.26).
    var ratings: ExternalRatings? = nil
    var productionYear: Int?
    /// ISO 8601; the first 10 characters are the date.
    var premiereDate: String?
    /// Seconds.
    var runtime: Double?
    /// The file's container ("mkv", "mp4", "mkv,webm"…), when known.
    var container: String?
    /// Studios (movies) or networks (shows).
    var studios: [String]?
    /// The TMDB collection a movie belongs to.
    var collection: MediaCollectionRef?
    /// When the server first had it.
    var addedAt: Date?

    // Hierarchy
    var seriesId: String?
    var seriesName: String?
    var seasonId: String?
    var seasonName: String?
    /// A season's own number; an episode's season.
    var seasonNumber: Int?
    /// An episode's number in its season.
    var episodeNumber: Int?
    /// The library its files are in (a filter: libraries have no type).
    var libraryId: String?

    var images: ImageSet = ImageSet()
    /// The show's artwork as far as an episode or season knows it (`ItemDetailView.showStub`).
    var seriesImages: ImageSet?
    var people: [MediaPerson]?
    /// "Imdb", "Tmdb", "Tvdb"… (looked up case-insensitively).
    var providerIds: [String: String]?
    /// The signed-in user's watch state.
    var user: UserState?

    /// The files ("versions") of a playable title: cuts, qualities, dubs (the server's default
    /// first). Lists carry only how many there are (`versionCount`).
    var versions: [MediaVersion]?
    /// How many versions the title has (lists say it without the files).
    var versionCount: Int? = nil
    /// The title's files, by name, when the server lists files for it but none was found on disk
    /// at its last scan (moved, deleted, or on a share that's offline): nothing can play.
    var missingFiles: [String]? = nil
    /// The version the signed-in user chose for this title (remembered by the server).
    var chosenVersionId: String? = nil
    /// The streams of the item as listed (tech badges): the first version's when not separate.
    var tracks: [TrackInfo]?
    var chapters: [Chapter]?
    /// One file with two or more episodes: the last one's number (S3E12–13).
    var episodeEnd: Int? = nil
    /// One episode split over several files: which part this is (1…), of `parts`.
    var part: Int? = nil
    var parts: Int? = nil

    static func == (a: MediaItem, b: MediaItem) -> Bool { a.id == b.id && a.user == b.user }
    func hash(into h: inout Hasher) { h.combine(id) }

    func providerId(_ name: String) -> String? {
        providerIds?.first { $0.key.caseInsensitiveCompare(name) == .orderedSame }?.value
    }

    var isSeries: Bool { kind == .series }
    var isSeason: Bool { kind == .season }
    var isEpisode: Bool { kind == .episode }
    var isMovie: Bool { kind == .movie }
    var isPlayable: Bool { [.movie, .episode, .video, .musicVideo].contains(kind) }

    var displayTitle: String { name ?? "Untitled" }

    /// "S1 · E3" style label for episodes ("S3 · E12–13" for a file with two, "S5 · E23 · Part 2").
    var episodeLabel: String? {
        guard isEpisode else { return nil }
        let s = seasonNumber.map { "S\($0)" }
        let e = episodeNumberText.map { "E\($0)" }
        return [s, e, partLabel].compactMap { $0 }.joined(separator: " · ")
    }

    /// "3", or "12–13" for a file with two episodes.
    var episodeNumberText: String? {
        guard let n = episodeNumber else { return nil }
        if let end = episodeEnd, end > n { return "\(n)–\(end)" }
        return "\(n)"
    }

    /// Airing order of episodes (season, number, part), for sorting.
    static func order(_ item: MediaItem?) -> [Int] {
        [item?.seasonNumber ?? 0, item?.episodeNumber ?? 0, item?.part ?? 0]
    }

    /// "Part 2" for a part of a split episode.
    var partLabel: String? { part.map { "Part \($0)" } }

    /// An episode in a season's list: "4. Title", "12–13. Title", "23. Part 2 · Title".
    var episodeListTitle: String {
        let number = episodeNumberText.map { "\($0). " } ?? ""
        let part = partLabel.map { "\($0) · " } ?? ""
        return number + part + displayTitle
    }

    var resumeSeconds: Double? {
        guard let t = user?.position, t > 0 else { return nil }
        return t
    }

    var progress: Double? {
        guard let p = user?.playedPercentage, p > 0 else { return nil }
        return p / 100
    }

    /// The movie/show this item belongs to: the series for episodes, the item itself otherwise.
    var titleId: String { isEpisode ? (seriesId ?? id) : id }

    /// The Continue Watching thread it belongs to, as the server groups the row: its show, its
    /// film collection, or itself. A card keeps it when another device moves it on to the next
    /// episode or film.
    var continueThread: String {
        if isEpisode, let s = seriesId { return "series:\(s)" }
        if isSeries { return "series:\(id)" }
        if isMovie, let c = collection { return "collection:\(c.id)" }
        return id
    }

    var lastPlayed: Date? { user?.lastPlayed }

    /// Technical badges derived from the streams.
    var techBadges: [String] {
        var out: [String] = []
        let streams = tracks ?? []
        if let v = streams.first(where: { $0.kind == .video }) {
            if let r = resolution, v.width != nil { out.append(r.label) }
            if let range = v.videoRange, range != "SDR", range != "Unknown" {
                out.append(range.contains("DOVI") ? "Dolby Vision" : range.contains("HDR10Plus") ? "HDR10+" : "HDR")
            }
        }
        if let a = streams.first(where: { $0.kind == .audio && ($0.isDefault ?? false) }) ?? streams.first(where: { $0.kind == .audio }) {
            if let p = a.profile, p.contains("Atmos") { out.append("Atmos") }
            if let ch = a.channels { out.append(ch >= 8 ? "7.1" : ch >= 6 ? "5.1" : ch == 2 ? "Stereo" : "Mono") }
        }
        if streams.contains(where: { $0.kind == .subtitle }) { out.append("CC") }
        return out
    }
}

/// A server image. The client that made it knows how to turn it into a URL; `tag` changes when
/// the art does (image caches key on it).
struct ImageRef: Codable, Hashable {
    /// The image id.
    var id: String
    /// Unused by the Moonrise server (its ids name the image); kept so saved Home caches decode.
    var type: String?
    var index: Int?
    var tag: String?
}

/// The artwork an item is shown with, fallbacks already applied.
struct ImageSet: Codable, Hashable {
    /// Portrait 2:3 (an episode: its show's).
    var poster: ImageRef?
    /// Wide background.
    var backdrop: ImageRef?
    /// Landscape 16:9 card art: an episode's still, else a thumb, backdrop or poster.
    var thumb: ImageRef?
    var logo: ImageRef?
    /// The item's own primary image (an episode's still), without fallbacks.
    var primary: ImageRef?
}

struct MediaPerson: Codable, Hashable, Identifiable {
    let id: String
    var name: String?
    var role: String?
    /// "Actor", "Director", "Writer"…
    var kind: String?
    var image: ImageRef?
}

struct UserState: Codable, Hashable {
    /// Resume position, seconds.
    var position: Double?
    /// 0-100.
    var playedPercentage: Double?
    var played: Bool?
    var isFavorite: Bool?
    /// Series and seasons.
    var unplayedItemCount: Int?
    var lastPlayed: Date?
}

struct Chapter: Codable, Hashable {
    /// Seconds.
    var start: Double?
    var name: String?
}

/// One file of a title.
struct MediaVersion: Codable, Hashable, Identifiable {
    let id: String
    var name: String?
    var path: String?
    var container: String?
    var size: Int64?
    /// Bits/s.
    var bitrate: Int?
    /// Seconds.
    var runtime: Double?
    var defaultAudioIndex: Int?
    var defaultSubtitleIndex: Int?
    var tracks: [TrackInfo]?
    /// What tells it from the title's other versions ("4K HDR10 · Director's Cut") and everything
    /// about it ("2160p · HEVC · HDR10 · English 5.1 · BluRay · 18.2 GB"), as the server says.
    var label: String? = nil
    var details: String? = nil
    var edition: String? = nil
    /// "HDR10", "Dolby Vision"… (nil: SDR).
    var hdr: String? = nil
    /// ISO 639 codes of its audio tracks, the default first.
    var audioLanguages: [String]? = nil
    /// Where the signed-in user stopped in this version (seconds; 0: not started).
    var position: Double? = nil
    /// A Fix Match choice was made for this file.
    var manual: Bool? = nil
    /// The server found the file damaged or unreadable: it can't play. (Why, as the server says.)
    var broken: Bool? = nil
    var brokenReason: String? = nil

    var isBroken: Bool { broken == true }

    var audioTracks: [TrackInfo] { (tracks ?? []).filter { $0.kind == .audio } }
    var subtitleTracks: [TrackInfo] { (tracks ?? []).filter { $0.kind == .subtitle } }
    var videoHeight: Int? { tracks?.first { $0.kind == .video }?.height }
}

/// One stream of a file. `index` is the server's track index (sidecar files first, then
/// the file's streams): the player's track menus and subtitle URLs use it.
struct TrackInfo: Codable, Hashable {
    enum Kind: String, Codable { case video, audio, subtitle, other }

    var kind: Kind
    var index: Int
    var codec: String?
    /// ISO 639-2 ("eng").
    var language: String?
    var title: String?
    var displayTitle: String?
    var isDefault: Bool?
    var isForced: Bool?
    var isExternal: Bool?
    /// Subtitles: text rather than pictures (PGS, DVD, DVB: images the server sends in windows).
    /// The app draws both.
    var isText: Bool?
    var width: Int?
    var height: Int?
    var channels: Int?
    var bitRate: Int?
    var profile: String?
    /// "SDR", "HDR10", "HDR10Plus", "HLG", "DOVI…".
    var videoRange: String?

    var label: String {
        guard kind == .subtitle else { return displayTitle ?? title ?? language?.uppercased() ?? "Track \(index)" }
        let lang = language.flatMap { Locale.current.localizedString(forLanguageCode: $0) } ?? language?.uppercased()
        var parts: [String] = [lang ?? title ?? "Track \(index)"]
        let name = lang != nil ? cleanTitle(Self.subtitleTechnical) : nil
        if let name { parts.append(name) }
        if isForced == true, name?.range(of: "forced", options: .caseInsensitive) == nil { parts.append("Forced") }
        if isExternal == true { parts.append("External") }
        return parts.joined(separator: " · ")
    }

    private var languageName: String? {
        guard let language, language != "und" else { return nil }
        return Locale.current.localizedString(forLanguageCode: language) ?? language.uppercased()
    }

    /// The track's language, else its title.
    var audioName: String { languageName ?? title ?? displayTitle ?? "Track \(index)" }

    /// Audio menu title: the language, as in the TV app. A track with none is "Track 2": the
    /// detail line has its title and format (the server's display title only repeats the format).
    var audioMenuName: String { languageName ?? "Track \(index)" }

    /// Audio menu detail, TV-app style: track name (commentary, dub studio…), then "Dolby Digital 5.1".
    var audioDetail: String? {
        let sound = [audioFormat, channelLayout].compactMap { $0 }.joined(separator: " ")
        let parts = [audioTrackName, sound.isEmpty ? nil : sound].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private var channelLayout: String? {
        switch channels {
        case 1?: "Mono"
        case 2?: "Stereo"
        case let c? where c > 2: "\(c - 1).1"
        default: nil
        }
    }

    private static let audioTechnical = #"dd|ddp|dd\+|ac-?3|e-?ac-?3|dts(-hd)?|ma|truehd|atmos|aac|lc|flac|opus|mp3|pcm|lpcm|stereo|mono|surround|kbps|kb/s|\d+(\.\d)?(ch)?"#
    private static let subtitleTechnical = #"srt|subrip|ass|ssa|vtt|webvtt|pgs|pgssub|sup|vobsub|dvdsub|dvb_subtitle|mov_text|utf-?8|text"#

    private var audioTrackName: String? { cleanTitle(Self.audioTechnical) }

    /// The meaningful part of the track title. Titles are often technical ("DD 5.1 @ 448 kbps
    /// (AVO Goblin)", "SRT (English)"), which the menu already says, so keep only what's left.
    private func cleanTitle(_ words: String) -> String? {
        guard var t = title?.trimmingCharacters(in: .whitespaces), !t.isEmpty else { return nil }
        let technical = #"(?i)(\b("# + words + #")\b|[@+\-–,/.\s])+"#
        func isTechnical(_ x: Substring) -> Bool { x.range(of: "^" + technical + "$", options: .regularExpression) != nil }
        if let open = t.firstIndex(of: "("), let close = t.lastIndex(of: ")"), open < close,
           t[..<open].trimmingCharacters(in: .whitespaces).isEmpty || isTechnical(t[..<open]) {
            t = String(t[t.index(after: open)..<close]).trimmingCharacters(in: .whitespaces)
        }
        // "Russian Forced - Netflix": the menu already says the language.
        if let lang = languageName, t.lowercased().hasPrefix(lang.lowercased() + " ") {
            t = t.dropFirst(lang.count).trimmingCharacters(in: CharacterSet(charactersIn: "-–,:·|/").union(.whitespaces))
        }
        if t.isEmpty || isTechnical(Substring(t)) { return nil }
        if let lang = languageName, t.caseInsensitiveCompare(lang) == .orderedSame { return nil }
        if let code = language, t.caseInsensitiveCompare(code) == .orderedSame { return nil }
        return t
    }

    private var audioFormat: String? {
        switch codec?.lowercased() {
        case "ac3"?: "Dolby Digital"
        case "eac3"?: profile?.contains("Atmos") == true ? "Dolby Atmos" : "Dolby Digital Plus"
        case "truehd"?: profile?.contains("Atmos") == true ? "Dolby Atmos" : "Dolby TrueHD"
        case "dts"?: profile.map { $0.contains("MA") ? "DTS-HD MA" : "DTS" } ?? "DTS"
        case "aac"?: "AAC"
        case "flac"?: "FLAC"
        case "opus"?: "Opus"
        case let c?: c.uppercased()
        default: nil
        }
    }
}
