import Foundation

/// Finds the skippable segments to offer. The server merges the file's own (named chapters, end
/// credits found in the file) with the community databases' (TheIntroDB, IntroDB), which it looks
/// up and keeps itself (`segments?merged=1`, servers 0.28+): the app contacts no other service.
/// An older server answers with the file's own, and the app applies the same rules to
/// them alone (`SegmentRules`).
@MainActor
enum SegmentService {
    /// The final answer per title and version (`key`): once the server has said all it will (not
    /// still looking in the file or in the databases).
    private static var cache: [String: [MediaSegment]] = [:]
    /// Titles (and versions) whose last answer was still waiting for the community databases.
    private static var lookingUp: Set<String> = []

    private static func key(_ item: MediaItem, _ version: String?) -> String { version.map { "\(item.id)~\($0)" } ?? item.id }

    /// Whether the server's answer for a title (and version) isn't final yet (still looking for
    /// the end credits in the file or the title in the databases, not reached, or not asked yet):
    /// worth asking again later.
    static func awaitingServer(_ item: MediaItem, version: String? = nil) -> Bool { cache[key(item, version)] == nil }

    /// Whether the server was still looking the title up in the community databases: ask again
    /// in a few seconds (it never waits for them).
    static func awaitingCommunity(_ item: MediaItem, version: String? = nil) -> Bool { lookingUp.contains(key(item, version)) }

    /// - version: the version played, when one is chosen (its chapters and end credits).
    /// - provisional: while the server is still looking for the end credits in the file, use the
    ///   databases' that pass the checks anyway (a download, saved once), instead of holding
    ///   them back until the server's answer (playback, which asks again before the end).
    static func segments(for item: MediaItem, version: String? = nil, client: any MediaServerClient, provisional: Bool = false) async -> [MediaSegment] {
        let k = key(item, version)
        if let hit = cache[k] { return hit }
        let runtime = version.flatMap { v in item.versions?.first { $0.id == v }?.runtime } ?? item.runtime ?? 0
        let server = await client.segments(itemId: item.id, version: version, provisional: provisional, community: AppSettings.shared.onlineSegments)
        let result = offered(server, runtime: runtime, provisional: provisional, movie: item.isMovie)
        #if DEBUG
        let credits = result.first { $0.kind == .credits }
        fputs("MoonriseSegments: \(k) runtime \(Int(runtime)) credits \(credits.map { "\(Int($0.start))-\(Int($0.end))" } ?? "none")"
              + " (\(server.merged ? "merged" : "file only")\(server.pending ? " pending" : "")\(server.communityPending ? " looking up" : "")"
              + "\(provisional ? " provisional" : ""))\n", stderr)
        #endif
        if server.communityPending { lookingUp.insert(k) } else { lookingUp.remove(k) }
        if !server.pending, !server.communityPending, server.reached { cache[k] = result }
        return result
    }

    /// The segments to offer from the server's answer: as they are when it merged them, else the
    /// file's own by the same rules (no community databases: the app asks no one else).
    nonisolated static func offered(_ server: ServerSegments, runtime: Double, provisional: Bool, movie: Bool) -> [MediaSegment] {
        if server.merged { return server.items }
        return SegmentRules.merge(server: server.items, online: [], runtime: runtime, pending: server.pending && !provisional, movie: movie)
    }
}
