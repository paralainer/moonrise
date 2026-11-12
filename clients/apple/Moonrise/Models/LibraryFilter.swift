import Foundation

// The Library's filters, sorts and categories (Phase 7). The server applies them
// (`GET /v1/items` and `/v1/libraries/{id}/facets`); with no server the same filters run over the
// downloads (`LibraryFilter.matches`, `LibraryFacets.local`).

enum LibrarySort: String, CaseIterable, Identifiable, Codable {
    case title, added, year, rating, played, runtime
    var id: String { rawValue }
    var label: String {
        switch self {
        case .title: "Title"
        case .added: "Date Added"
        case .year: "Year"
        case .rating: "Rating"
        case .played: "Last Played"
        case .runtime: "Runtime"
        }
    }
    /// The direction a sort starts in (newest, best, longest first; titles A to Z).
    var descending: Bool { self != .title }
    var icon: String {
        switch self {
        case .title: "textformat"
        case .added: "calendar.badge.plus"
        case .year: "calendar"
        case .rating: "star"
        case .played: "clock.arrow.circlepath"
        case .runtime: "timer"
        }
    }
}

enum WatchedFilter: String, CaseIterable, Identifiable, Codable {
    case all, unwatched, inProgress, watched
    var id: String { rawValue }
    var label: String {
        switch self {
        case .all: "All"
        case .unwatched: "Unwatched"
        case .inProgress: "In Progress"
        case .watched: "Watched"
        }
    }
}

enum ResolutionFilter: String, CaseIterable, Identifiable, Codable, Comparable {
    case uhd = "4k", fhd = "1080", hd = "720", sd
    var id: String { rawValue }
    var label: String {
        switch self {
        case .uhd: "4K"
        case .fhd: "1080p"
        case .hd: "720p"
        case .sd: "SD"
        }
    }
    static func < (a: Self, b: Self) -> Bool { allCases.firstIndex(of: a)! < allCases.firstIndex(of: b)! }

    /// The server's buckets (`browse::res_bucket`): 576p and below are SD.
    init(width: Int, height: Int) {
        if width <= 1024 && height <= 576 { self = .sd }
        else if width <= 1280 && height <= 962 { self = .hd }
        else if width <= 2560 && height <= 1440 { self = .fhd }
        else { self = .uhd }
    }
}

/// A TMDB collection a movie belongs to.
struct MediaCollectionRef: Codable, Hashable {
    var id: Int
    var name: String?
}

/// How several chosen genres combine.
enum GenreMatch {
    /// A title with any of them.
    case any
    /// A title with all of them (the server's `genre_match=all`, from 0.7.5).
    case all
}

struct LibraryFilter: Codable, Hashable {
    /// Several genres: any of them (the one switch for both the server's and the downloads'
    /// filtering; every other filter narrows the list further).
    static let genreMatch: GenreMatch = .any

    var sort: LibrarySort = .title
    var descending = false
    var genres: Set<String> = []
    /// First year of a decade (1990 = 1990-1999).
    var decade: Int?
    /// Ratings as the facets name them ("none": not rated).
    var ratings: Set<String> = []
    var watched: WatchedFilter = .all
    var resolutions: Set<ResolutionFilter> = []
    var hdr = false
    var downloaded = false
    var recentlyAdded = false
    /// One library's titles (with several: libraries have no type, so Movies and Shows span
    /// them all), and its name to show.
    var library: String?
    var libraryName: String?
    /// Categories (set by a category page).
    var studio: String?
    var collection: MediaCollectionRef?
    /// A category tile's page ("Action", "HBO", "1990s"): the dimensions that make the page,
    /// which stay put (no tag, not in the filter menus, kept by Clear All).
    var locked: Set<LockedDimension>?

    enum LockedDimension: String, Codable, Hashable {
        case genre, studio, decade, collection
    }

    static let recentDays = 30

    func isLocked(_ d: LockedDimension) -> Bool { locked?.contains(d) == true }

    /// Any filter on (the sort isn't one, nor the page's own).
    var isFiltered: Bool { self.cleared != self }

    /// The same sort, no filters (but the page's own).
    var cleared: LibraryFilter {
        var c = LibraryFilter(sort: sort, descending: descending)
        guard let locked else { return c }
        c.locked = locked
        if locked.contains(.genre) { c.genres = genres }
        if locked.contains(.studio) { c.studio = studio }
        if locked.contains(.decade) { c.decade = decade }
        if locked.contains(.collection) { c.collection = collection }
        return c
    }

    /// The page's own dimensions locked: whichever category fields are set.
    var lockingCategory: LibraryFilter {
        var x = self
        var l: Set<LockedDimension> = []
        if !genres.isEmpty { l.insert(.genre) }
        if studio != nil { l.insert(.studio) }
        if decade != nil { l.insert(.decade) }
        if collection != nil { l.insert(.collection) }
        x.locked = l
        return x
    }

    var activeCount: Int {
        (isLocked(.genre) ? 0 : genres.count) + (decade == nil || isLocked(.decade) ? 0 : 1) + ratings.count + (watched == .all ? 0 : 1) + resolutions.count
            + (hdr ? 1 : 0) + (downloaded ? 1 : 0) + (recentlyAdded ? 1 : 0) + (studio == nil || isLocked(.studio) ? 0 : 1)
            + (collection == nil || isLocked(.collection) ? 0 : 1) + (library == nil ? 0 : 1)
    }

    mutating func setSort(_ s: LibrarySort) {
        if sort == s { descending.toggle() } else { sort = s; descending = s.descending }
    }

    /// The server's query parameters for these filters (not the sort, not `downloaded`).
    var serverQuery: [String: String?] {
        var q: [String: String?] = [:]
        if !genres.isEmpty { q["genre"] = genres.sorted().joined(separator: ",") }
        if genres.count > 1, Self.genreMatch == .all { q["genre_match"] = "all" }
        if let d = decade { q["year_from"] = "\(d)"; q["year_to"] = "\(d + 9)" }
        if !ratings.isEmpty { q["rating"] = ratings.sorted().joined(separator: ",") }
        switch watched {
        case .all: break
        case .unwatched: q["unplayed"] = "1"
        case .inProgress: q["in_progress"] = "1"
        case .watched: q["played"] = "1"
        }
        if !resolutions.isEmpty { q["resolution"] = resolutions.sorted().map(\.rawValue).joined(separator: ",") }
        if hdr { q["hdr"] = "1" }
        if recentlyAdded { q["added_days"] = "\(Self.recentDays)" }
        if let s = studio { q["studio"] = s }
        if let c = collection { q["collection"] = "\(c.id)" }
        if let l = library { q["library"] = l }
        return q
    }

    // MARK: Offline (downloads)

    /// Whether a title passes the filters (shows: `episodes` are its downloaded episodes).
    func matches(_ item: MediaItem, episodes: [MediaItem] = []) -> Bool {
        if !genres.isEmpty {
            let has = { (g: String) in (item.genres ?? []).contains { $0.caseInsensitiveCompare(g) == .orderedSame } }
            switch Self.genreMatch {
            case .any: if !genres.contains(where: has) { return false }
            case .all: if !genres.allSatisfy(has) { return false }
            }
        }
        if let d = decade { guard let y = item.productionYear, y >= d, y <= d + 9 else { return false } }
        if !ratings.isEmpty {
            let key = item.officialRating.map(Self.ratingKey).flatMap { $0.isEmpty ? nil : $0 } ?? "NONE"
            if !ratings.contains(where: { Self.ratingKey($0) == key }) { return false }
        }
        let playable = item.isSeries ? episodes : [item]
        switch watched {
        case .all: break
        case .unwatched: if !playable.contains(where: { $0.user?.played != true }) { return false }
        case .watched: if playable.isEmpty || playable.contains(where: { $0.user?.played != true }) { return false }
        case .inProgress:
            if item.isSeries {
                let started = episodes.contains { $0.user?.played == true || ($0.user?.position ?? 0) > 0 }
                if !started || !episodes.contains(where: { $0.user?.played != true }) { return false }
            } else if item.user?.played == true || (item.user?.position ?? 0) <= 0 { return false }
        }
        if !resolutions.isEmpty, !playable.contains(where: { $0.resolution.map(resolutions.contains) ?? false }) { return false }
        if hdr, !playable.contains(where: \.isHDR) { return false }
        if recentlyAdded {
            let cutoff = Date().addingTimeInterval(-Double(Self.recentDays) * 86_400)
            if !playable.contains(where: { ($0.addedAt ?? .distantPast) > cutoff }) { return false }
        }
        if let s = studio, !(item.studios ?? []).contains(s) { return false }
        if let c = collection, item.collection?.id != c.id { return false }
        if let l = library, let il = item.libraryId ?? episodes.first?.libraryId, il != l { return false }
        return true
    }

    /// Sorted as the server sorts (ties by title). `added`: a show's latest episode's date.
    func sorted(_ items: [MediaItem], lastPlayed: (MediaItem) -> Date? = { $0.lastPlayed },
                added: (MediaItem) -> Date? = { $0.addedAt }) -> [MediaItem] {
        func title(_ i: MediaItem) -> String { Self.sortTitle(i.displayTitle) }
        let byTitle: (MediaItem, MediaItem) -> Bool = {
            let o = title($0).localizedCaseInsensitiveCompare(title($1))
            return o == .orderedSame ? $0.id < $1.id : o == .orderedAscending
        }
        func cmp<T: Comparable>(_ a: T?, _ b: T?, _ x: MediaItem, _ y: MediaItem) -> Bool {
            switch (a, b) {
            case let (a?, b?) where a != b: return descending ? a > b : a < b
            case (nil, _?): return false // missing last
            case (_?, nil): return true
            default: return descending ? byTitle(y, x) : byTitle(x, y)
            }
        }
        return items.sorted { x, y in
            switch sort {
            case .title: return descending ? byTitle(y, x) : byTitle(x, y)
            case .added: return cmp(added(x), added(y), x, y)
            case .year: return cmp(x.productionYear, y.productionYear, x, y)
            case .rating: return cmp(x.communityRating, y.communityRating, x, y)
            case .played: return cmp(lastPlayed(x), lastPlayed(y), x, y)
            case .runtime: return cmp(x.runtime, y.runtime, x, y)
            }
        }
    }

    static func ratingKey(_ r: String) -> String { r.filter { !$0.isWhitespace }.uppercased() }

    /// "The Matrix" → "Matrix" (as the server's sort titles).
    static func sortTitle(_ t: String) -> String {
        for a in ["The ", "A ", "An "] where t.count > a.count && t.lowercased().hasPrefix(a.lowercased()) {
            return String(t.dropFirst(a.count))
        }
        return t
    }

    /// The alphabet index letter of a title: A-Z, else "#".
    static func letter(_ title: String) -> String {
        guard let c = sortTitle(title).first.map({ String($0).uppercased() }), c.count == 1,
              let s = c.unicodeScalars.first, ("A"..."Z").contains(Character(s)) else { return "#" }
        return c
    }

    // MARK: Remembered per library

    private static func key(_ library: String) -> String { "library.filter.\(library)" }

    static func saved(for library: String) -> LibraryFilter? {
        UserDefaults.standard.data(forKey: key(library)).flatMap { try? JSONDecoder().decode(LibraryFilter.self, from: $0) }
    }

    func save(for library: String) {
        if let d = try? JSONEncoder().encode(self) { UserDefaults.standard.set(d, forKey: Self.key(library)) }
    }

}

#if DEBUG
extension LibraryFilter {
    /// Test seam: "genre=Action,Comedy;decade=1990;rating=M;watched=unwatched;res=4k;hdr=1;downloaded=1;recent=1;sort=year;desc=1;studio=HBO;collection=1241".
    init(seam: String, base: LibraryFilter = LibraryFilter()) {
        self = base
        for part in seam.split(separator: ";") {
            let kv = part.split(separator: "=", maxSplits: 1).map(String.init)
            guard kv.count == 2 else { continue }
            let v = kv[1]
            switch kv[0] {
            case "genre": genres = Set(v.split(separator: ",").map(String.init))
            case "decade": decade = Int(v)
            case "rating": ratings = Set(v.split(separator: ",").map(String.init))
            case "watched": watched = WatchedFilter(rawValue: v) ?? .all
            case "res": resolutions = Set(v.split(separator: ",").compactMap { ResolutionFilter(rawValue: String($0)) })
            case "hdr": hdr = v == "1"
            case "downloaded": downloaded = v == "1"
            case "recent": recentlyAdded = v == "1"
            case "sort": if let s = LibrarySort(rawValue: v) { sort = s; descending = s.descending }
            case "desc": descending = v == "1"
            case "studio": studio = v
            case "collection": collection = Int(v).map { MediaCollectionRef(id: $0, name: nil) }
            case "library": library = v
            default: break
            }
        }
    }
}
#endif

/// What a library's filters can pick, with counts (the server's facets, or the downloads').
struct LibraryFacets: Equatable {
    struct Value: Hashable, Identifiable {
        var name: String
        var count: Int
        /// Artwork for a category tile.
        var backdrop: ImageRef?
        var poster: ImageRef?
        /// Collections: the TMDB id.
        var id: String { name }
    }
    struct Collection: Hashable, Identifiable {
        var id: Int
        var name: String
        var count: Int
        var poster: ImageRef?
        var backdrop: ImageRef?
    }
    /// A library and how many of the titles are in it.
    struct Library: Hashable, Identifiable {
        var id: String
        var name: String
        var count: Int
    }
    struct Decade: Hashable, Identifiable {
        var decade: Int
        var count: Int
        var backdrop: ImageRef?
        var id: Int { decade }
        var label: String { "\(decade)s" }
    }

    var total = 0
    var genres: [Value] = []
    var studios: [Value] = []
    var decades: [Decade] = []
    var ratings: [Value] = []
    var collections: [Collection] = []
    var resolutions: [Value] = []
    var hdr = 0
    var recentlyAdded = 0
    /// unplayed, in progress, played (nil: unknown).
    var watched: [WatchedFilter: Int]?
    /// The server's libraries with their counts (the Library filter, with more than one).
    var libraries: [Library] = []

    /// The libraries to offer: those with titles here, and the chosen one.
    func libraryChoices(_ chosen: String?) -> [Library] { libraries.filter { $0.count > 0 || $0.id == chosen } }

    /// The facets of downloaded titles (no server).
    private static func ratingKeyOf(_ r: String) -> String { LibraryFilter.ratingKey(r) }

    static func local(_ titles: [(item: MediaItem, episodes: [MediaItem])], filter: LibraryFilter) -> LibraryFacets {
        var f = LibraryFacets()
        // Each facet counted under the other filters (as the server does).
        func pass(_ change: (inout LibraryFilter) -> Void) -> [(item: MediaItem, episodes: [MediaItem])] {
            var x = filter
            change(&x)
            return titles.filter { x.matches($0.item, episodes: $0.episodes) }
        }
        func tally(_ list: [String]) -> [Value] {
            Dictionary(list.map { ($0, 1) }, uniquingKeysWith: +)
                .map { Value(name: $0.key, count: $0.value) }
                .sorted { $0.count != $1.count ? $0.count > $1.count : $0.name < $1.name }
        }
        f.total = pass { _ in }.count
        // A title counts once per value (its lists can repeat one).
        f.genres = tally(pass { $0.genres = [] }.flatMap { Set($0.item.genres ?? []) })
        f.studios = tally(pass { $0.studio = nil }.flatMap { Set($0.item.studios ?? []) })
        // Ratings grouped as the server groups them ("MA 15+" = "MA15+"), named by their most
        // common spelling.
        let rated = pass { $0.ratings = [] }.map { $0.item.officialRating?.trimmingCharacters(in: .whitespaces) ?? "" }
        f.ratings = Dictionary(grouping: rated) { $0.isEmpty ? "NONE" : Self.ratingKeyOf($0) }
            .map { key, spellings -> Value in
                let names = Dictionary(spellings.map { ($0, 1) }, uniquingKeysWith: +)
                let shown = names.max { $0.value != $1.value ? $0.value < $1.value : $0.key > $1.key }?.key ?? ""
                return Value(name: key == "NONE" ? "none" : shown, count: spellings.count)
            }
            .sorted { ($0.name == "none") != ($1.name == "none") ? $1.name == "none" : $0.count != $1.count ? $0.count > $1.count : $0.name < $1.name }
        f.decades = Dictionary(pass { $0.decade = nil }.compactMap { $0.item.productionYear.map { ($0 - $0 % 10, 1) } }, uniquingKeysWith: +)
            .map { Decade(decade: $0.key, count: $0.value) }.sorted { $0.decade > $1.decade }
        let res = pass { $0.resolutions = [] }.flatMap { t in Set((t.item.isSeries ? t.episodes : [t.item]).compactMap(\.resolution)) }
        f.resolutions = ResolutionFilter.allCases.compactMap { r in
            let n = res.filter { $0 == r }.count
            return n > 0 ? Value(name: r.rawValue, count: n) : nil
        }
        f.hdr = pass { $0.hdr = false }.filter { t in (t.item.isSeries ? t.episodes : [t.item]).contains(where: \.isHDR) }.count
        var colls: [Int: Collection] = [:]
        for t in pass({ $0.collection = nil }) {
            guard let c = t.item.collection else { continue }
            colls[c.id, default: Collection(id: c.id, name: c.name ?? "Collection", count: 0)].count += 1
        }
        f.collections = colls.values.sorted { $0.count != $1.count ? $0.count > $1.count : $0.name < $1.name }
        var w: [WatchedFilter: Int] = [:]
        let wl = pass { $0.watched = .all }
        for k in [WatchedFilter.unwatched, .inProgress, .watched] {
            var x = filter
            x.watched = k
            w[k] = wl.filter { x.matches($0.item, episodes: $0.episodes) }.count
        }
        f.watched = w
        f.recentlyAdded = pass { $0.recentlyAdded = true }.count
        return f
    }
}

extension MediaItem {
    /// The resolution bucket of the first video track (as the server's filter).
    var resolution: ResolutionFilter? {
        guard let v = tracks?.first(where: { $0.kind == .video }), let w = v.width else { return nil }
        return ResolutionFilter(width: w, height: v.height ?? 0)
    }

    var isHDR: Bool {
        guard let r = tracks?.first(where: { $0.kind == .video })?.videoRange else { return false }
        return r != "SDR" && r != "Unknown"
    }
}
