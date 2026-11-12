import Foundation

/// Scores beside TMDB's: IMDb (0-10), Rotten Tomatoes critics (%) and Metacritic (0-100), as the
/// server gives them (IMDb's dataset, OMDb with the admin's key). The apps look nothing up
/// themselves.
struct ExternalRatings: Codable, Hashable {
    var imdb: Double?
    var rottenTomatoes: Int?
    var metacritic: Int?
    var imdbVotes: Int? = nil

    var isEmpty: Bool { imdb == nil && rottenTomatoes == nil && metacritic == nil }

    /// The credit IMDb asks for with its data, and OMDb's.
    static let attribution = "Information courtesy of IMDb (https://www.imdb.com). Used with permission. Rotten Tomatoes and Metacritic scores through OMDb (omdbapi.com)."

    /// A title page's scores: the title's own (an episode's, or the film's), the critics' scores
    /// topped up with the show's (Rotten Tomatoes and Metacritic rate shows, rarely episodes);
    /// IMDb stays the title's own.
    static func page(own: ExternalRatings?, show: ExternalRatings?) -> ExternalRatings {
        let o = own ?? ExternalRatings()
        return ExternalRatings(imdb: o.imdb, rottenTomatoes: o.rottenTomatoes ?? show?.rottenTomatoes,
                               metacritic: o.metacritic ?? show?.metacritic, imdbVotes: o.imdbVotes)
    }

    /// The Apple TV credits line: "★ 7.9   IMDb 8.1   RT 94%   MC 78" (TMDB's star first, as
    /// before; what's missing left out). Nil: nothing to show.
    static func line(tmdb: Double?, _ r: ExternalRatings) -> String? {
        var parts: [String] = []
        if let t = tmdb { parts.append(String(format: "★ %.1f", t)) }
        if let v = r.imdb { parts.append(String(format: "IMDb %.1f", v)) }
        if let v = r.rottenTomatoes { parts.append("RT \(v)%") }
        if let v = r.metacritic { parts.append("MC \(v)") }
        return parts.isEmpty ? nil : parts.joined(separator: "   ")
    }
}
