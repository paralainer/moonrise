import Foundation

struct LocalSubtitle: Codable, Hashable {
    var title: String
    var language: String?
    /// Path relative to the app's home directory.
    var file: String
    /// The name of the matching subtitle rendition in a segmented download's playlist (PiP).
    var rendition: String?
}

struct DownloadRecord: Codable, Identifiable, Hashable {
    enum Kind: String, Codable {
        /// Original file over HTTP (background URLSession).
        case file
        /// A progressive MP4 the Moonrise server prepares (remuxed original or a transcode), then
        /// downloaded like `.file`. (Made by Phase 5 builds; they still play.)
        case prepared
        /// The Moonrise server's HLS segments (an "offline set"), fetched one by one into a local
        /// playlist the loopback `LocalHLSServer` serves: it plays while it downloads.
        case segments
        /// Kinds older versions made; they still decode, and DownloadManager drops them on load.
        case hls, smb
    }

    enum State: String, Codable {
        case queued, downloading, paused, completed, failed
    }

    var id = UUID()
    var kind: Kind
    var state: State = .queued
    var title: String
    var subtitle: String?
    var overview: String?
    var sourceId: UUID
    var itemId: String?
    var mediaSourceId: String?
    /// Moonrise server downloads: the path of the file on the server ("/v1/…"; the server's address
    /// can change with the route), and the prepared job's id.
    var moonrisePath: String?
    var moonriseJobId: String?
    /// The Moonrise server is still preparing the file (`progress` is its progress).
    var preparing: Bool?
    var qualityLabel: String
    /// Which version of the title was downloaded (a title with several files): its label.
    var versionLabel: String?
    /// "4K HDR10 · Director's Cut · 1080p", or the quality alone.
    var downloadLabel: String { [versionLabel, qualityLabel].compactMap { $0 }.joined(separator: " · ") }
    var fileExtension: String
    var bytesWritten: Int64 = 0
    var bytesExpected: Int64 = 0
    var progress: Double = 0
    /// Relative to NSHomeDirectory() (the container path changes between installs).
    var localPath: String?
    var posterFile: String?
    var backdropFile: String?
    var subtitles: [LocalSubtitle] = []
    var error: String?
    var createdAt = Date()
    var runtimeTicks: Int64?
    var year: Int?
    /// Intro/credits markers captured at download time so skipping works offline.
    var segments: [MediaSegment]?
    /// The server was still looking for the end credits in the file when `segments` were saved
    /// (they have the community databases' then): asked again later (`DownloadManager`).
    var segmentsPending: Bool?
    /// The title as the server described it when the download started (offline pages, badges,
    /// next episode, reports).
    var item: MediaItem?
    /// An episode's show as the server described it (the offline Library's genres, ratings…).
    var series: MediaItem?

    // Segmented downloads (`.segments`)
    /// Where the set's files are on the server ("/v1/items/<id>/offline/<variant>") and the
    /// file's version they were cut from.
    var setBase: String?
    var setVersion: String?
    /// Segments per track, and how many of them are complete on the device (every track).
    var segmentCount: Int?
    var segmentsDone: Int?
    /// The audio tracks in the set (server track indices), the subtitle renditions.
    var audioTracks: [Int]?
    var subtitleTracks: [Int]?

    static func home(_ relative: String?) -> URL? {
        relative.map { URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent($0) }
    }

    static func relative(_ url: URL) -> String {
        let home = URL(fileURLWithPath: NSHomeDirectory()).standardizedFileURL.path
        let p = url.standardizedFileURL.path
        // Account for /private/var vs /var on device.
        if let r = p.range(of: home) { return String(p[r.upperBound...]).trimmingCharacters(in: CharacterSet(charactersIn: "/")) }
        if let r = p.range(of: "/Library/") { return "Library/" + p[r.upperBound...] }
        return p
    }

    var localURL: URL? { Self.home(localPath) }
    var posterURL: URL? { Self.home(posterFile) }
    var backdropURL: URL? { Self.home(backdropFile) }

    var progressKey: String { itemId.map(ProgressStore.key(itemId:)) ?? "dl:\(id.uuidString)" }

    var isSegmented: Bool { kind == .segments }
    /// The set's playlists are on the device and segments are coming in (or all there).
    var hasPlaylist: Bool { isSegmented && setBase != nil && segmentCount != nil }
    /// Plays from the device now: finished, or a segmented download that has started.
    var isPlayable: Bool { state == .completed || (hasPlaylist && (segmentsDone ?? 0) > 0) }
    var seriesId: String? { item?.seriesId }
}
