// Small builders for the tests' items: generic titles and ids only.
import Foundation
@testable import Moonrise

enum Fixture {
    static func movie(_ id: String, collection: Int? = nil, runtime: Double? = 6000, user: UserState? = nil,
                      versions: [MediaVersion]? = nil) -> MediaItem {
        MediaItem(id: id, kind: .movie, name: "Film \(id)", runtime: runtime,
                  collection: collection.map { MediaCollectionRef(id: $0, name: "Collection \($0)") },
                  user: user, versions: versions)
    }

    static func episode(_ id: String, series: String, season: Int = 1, number: Int = 1, runtime: Double? = 2400,
                        user: UserState? = nil) -> MediaItem {
        MediaItem(id: id, kind: .episode, name: "Episode \(number)", runtime: runtime,
                  seriesId: series, seriesName: "Show \(series)", seasonNumber: season, episodeNumber: number,
                  user: user)
    }

    static func series(_ id: String) -> MediaItem {
        MediaItem(id: id, kind: .series, name: "Show \(id)")
    }

    static func version(_ id: String, height: Int = 1080, hdr: String? = nil, audio: [String]? = nil,
                        broken: Bool? = nil) -> MediaVersion {
        var v = MediaVersion(id: id)
        v.tracks = [TrackInfo(kind: .video, index: 0, height: height)]
        v.hdr = hdr
        v.audioLanguages = audio
        v.broken = broken
        return v
    }
}
