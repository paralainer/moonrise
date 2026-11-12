import Foundation

/// What the Apple TV player's Info tab shows (TVNativePlayer turns these into AVKit metadata;
/// the artwork comes on its own). Pure and built for every platform, so MoonriseTests checks it.
struct PlayerInfoFields: Equatable {
    var title: String
    /// AVKit's heading when there is one: an episode's "S1, E2 · Name" line. A movie's subtitle is
    /// only its year, which the details line has, so a movie has none and its heading is its title.
    var subtitle: String?
    var overview: String?
    var rating: String?
    /// At most two, joined.
    var genres: String?
    /// AVKit shows its year.
    var releaseDate: Date?

    /// `details` is where the age rating and genres come from: the movie, or an episode's show.
    /// Near the end (`nearEnd`), title and subtitle only: without the overview there's no Info tab,
    /// so Next Episode is the row's first tab.
    static func make(item: MediaItem?, title: String, subtitle: String?, details: MediaItem?, nearEnd: Bool) -> PlayerInfoFields {
        var f = PlayerInfoFields(title: title)
        if item?.isEpisode != false, let subtitle, !subtitle.isEmpty { f.subtitle = subtitle }
        guard !nearEnd else { return f }
        f.overview = item?.overview
        if let r = details?.officialRating, !r.isEmpty { f.rating = r }
        if let g = details?.genres, !g.isEmpty { f.genres = g.prefix(2).joined(separator: ", ") }
        f.releaseDate = releaseDate(item)
        return f
    }

    /// The premiere date (an episode: when it aired), else mid-year of the production year,
    /// in UTC at noon or mid-year so the year is the same in every time zone.
    static func releaseDate(_ item: MediaItem?) -> Date? {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        if let p = item?.premiereDate, p.count >= 10 {
            let parts = p.prefix(10).split(separator: "-").compactMap { Int($0) }
            if parts.count == 3, let d = utc.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2], hour: 12)) { return d }
        }
        return item?.productionYear.flatMap { utc.date(from: DateComponents(year: $0, month: 7, day: 1)) }
    }
}
