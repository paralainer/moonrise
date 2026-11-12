import Foundation

/// Watch progress of downloads played on the device, for the server's watch state (Continue
/// Watching, Next Up). Reports are kept on disk and sent (`POST /v1/sessions/local`) when the
/// server can be reached, so playback offline counts once the device is back. Only the latest
/// progress of a viewing is kept while it waits.
@MainActor
final class LocalReports {
    static let shared = LocalReports()

    struct Report: Codable, Equatable {
        var sourceId: UUID
        /// One playback (it counts as played once).
        var viewing: String
        var item: String
        /// start, progress or stop.
        var event: String
        var position: Double
        /// Unix seconds.
        var at: Int
        var duration: Double?
        /// Where the end credits start (`SegmentRules.creditsStart`).
        var credits: Double? = nil
        /// The tracks playing, without the device's own indexes.
        var tracks: TrackRules.Choice? = nil
        /// The file (version) the download was made from.
        var file: String? = nil

        /// The request's entry (`LocalReport` in api/openapi.yaml).
        var json: [String: Any] {
            var d: [String: Any] = ["viewing": viewing, "item": item, "event": event, "position": position, "at": at]
            if let duration { d["duration"] = duration }
            if let credits { d["credits"] = credits }
            if let tracks { d["tracks"] = tracks.json }
            if let file { d["file"] = file }
            return d
        }
    }

    private(set) var pending: [Report] = []
    private var sending = false
    private var again = false

    private let file: URL = {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("local-reports.json")
    }()

    private init() {
        if let data = try? Data(contentsOf: file), let r = try? JSONDecoder().decode([Report].self, from: data) { pending = r }
    }

    /// Debug seam: MOONRISE_NO_REPORT=1 keeps scripted test runs out of the account's watch state.
    private var enabled: Bool {
        #if DEBUG
        return !DebugSeams.launch.noReport
        #else
        return true
        #endif
    }

    func hasPending(item: String) -> Bool { pending.contains { $0.item == item } }

    func add(source: UUID, viewing: String, item: String, event: String, position: Double, duration: Double?, credits: Double? = nil,
             tracks: TrackRules.Choice? = nil, file: String? = nil) {
        guard enabled else { return }
        let r = Report(sourceId: source, viewing: viewing, item: item, event: event, position: position,
                       at: Int(Date().timeIntervalSince1970), duration: duration, credits: credits, tracks: tracks, file: file)
        // Progress of the same viewing replaces the previous progress still waiting.
        if r.event != "start", let i = pending.lastIndex(where: { $0.viewing == viewing && $0.event == "progress" }) {
            pending.remove(at: i)
        }
        pending.append(r)
        save()
        Task { await flush() }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(pending) { try? data.write(to: file, options: .atomic) }
    }

    /// Send what is waiting, when the server can be reached. Reports kept while signed out wait
    /// for the next sign-in; ones from another server than the one signed in to are dropped.
    func flush() async {
        guard !pending.isEmpty, let server = SourceStore.shared.server else { return }
        if sending { again = true; return }
        sending = true
        defer { sending = false }
        repeat {
            again = false
            if pending.contains(where: { $0.sourceId != server.id }) {
                pending.removeAll { $0.sourceId != server.id }
                save()
            }
            guard !pending.isEmpty, ConnectionRouter.shared.isReachable, let client = MoonriseClient.for(server) else { return }
            let batch = Array(pending.prefix(200))
            do {
                try await client.reportLocal(batch.map(\.json))
                pending.removeAll { batch.contains($0) }
                save()
                #if DEBUG
                fputs("MoonriseLocalReports: sent \(batch.count) (\(batch.map { "\($0.event)@\(Int($0.position))" }.joined(separator: ","))), \(pending.count) waiting\n", stderr)
                #endif
            } catch {
                #if DEBUG
                fputs("MoonriseLocalReports: \(batch.count) not sent (\(error.localizedDescription)), kept\n", stderr)
                #endif
                // A bad request won't get better: drop it rather than retry forever.
                if case let .http(code, _)? = error as? MoonriseClientError, (400..<500).contains(code), code != 401, code != 403 {
                    pending.removeAll { batch.contains($0) }
                    save()
                }
            }
        } while again
    }
}

/// The tracks last played in each download, on this device: a download starts with
/// them again, offline too. The server hears of them with the download's reports.
enum LocalTrackMemory {
    private static let key = "downloadTracks"

    static func load(download: UUID, defaults: UserDefaults = .standard) -> TrackRules.Choice? {
        guard let data = (defaults.dictionary(forKey: key) as? [String: Data])?[download.uuidString] else { return nil }
        return try? JSONDecoder().decode(TrackRules.Choice.self, from: data)
    }

    static func save(_ choice: TrackRules.Choice, download: UUID, defaults: UserDefaults = .standard) {
        var all = defaults.dictionary(forKey: key) as? [String: Data] ?? [:]
        guard let data = try? JSONEncoder().encode(choice), all[download.uuidString] != data else { return }
        all[download.uuidString] = data
        defaults.set(all, forKey: key)
    }

    static func forget(download: UUID, defaults: UserDefaults = .standard) {
        var all = defaults.dictionary(forKey: key) as? [String: Data] ?? [:]
        guard all.removeValue(forKey: download.uuidString) != nil else { return }
        defaults.set(all, forKey: key)
    }
}
