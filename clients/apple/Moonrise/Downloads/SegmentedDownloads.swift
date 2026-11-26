import AVFoundation
import Foundation

/// Segmented downloads (`DownloadRecord.Kind.segments`): the Moonrise server's HLS segments of one
/// video variant and the title's audio tracks and subtitles ("offline set", `GET
/// /v1/items/{id}/offline`), saved one by one under `<download>/hls/` with the set's playlists:
///
///     hls/master.m3u8
///     hls/video/index.m3u8, init.mp4, 0.m4s, 1.m4s …
///     hls/audio/<index>/index.m3u8, init.mp4, 0.m4s …
///     hls/subs/<index>/index.m3u8, cues.vtt
///
/// `LocalHLSServer` serves that directory to AVPlayer, so a download plays from the moment its
/// first segments are in. Segments are fetched in order from where the viewer is (the local
/// server reports the segments the player asks for), then from the start; one the player asks
/// for that isn't here yet is fetched at once (`fetchNow`). A segment counts once every track
/// has it. One download runs at a time; on iOS a continued-processing task keeps the app
/// running in the background meanwhile (`DownloadManager`).
@MainActor
final class SegmentedDownloads {
    /// Segment requests in flight at once (each fetches every track of its segment).
    private static let parallel = 3

    private let root: URL
    private let session: URLSession
    /// Called on progress and state changes (the manager updates the record).
    var onUpdate: ((UUID, (inout DownloadRecord) -> Void) -> Void)?
    var record: ((UUID) -> DownloadRecord?)?

    /// Per download: complete segments (every track), where the player is, the fetch task.
    private var done: [UUID: IndexSet] = [:]
    private var playhead: [UUID: PlayerPosition] = [:]
    private var running: [UUID: Task<Void, Never>] = [:]
    private var runToken: [UUID: UUID] = [:]
    /// Files being fetched (download id + relative path), so the player and the loop share them.
    private var inflight: [String: Task<Failure?, Never>] = [:]
    private var bytes: [UUID: Int64] = [:]
    private var lastSave: [UUID: Date] = [:]
    /// Downloads deleted in this run (`forget`): nothing more is written for them.
    private var deleted: Set<UUID> = []

    /// Whether the device is on mobile data now, and whether "Download over Cellular" is on, read
    /// at every fetch (a test sets them).
    var onCellular: () -> Bool = { NetworkType.hasCellular && NetworkType.shared.isCellular }
    var allowCellular: () -> Bool = { AppSettings.shared.allowCellularDownloads }

    init(root: URL, session: URLSession? = nil) {
        self.root = root
        self.session = session ?? HTTP.downloadSetSession()
    }

    /// "Download over Cellular": whether a download may fetch now. On mobile data (no Wi-Fi or
    /// wired link) with the setting off it waits, whatever the route: through the Tailscale
    /// forwarder on 127.0.0.1 the request never counts as cellular to the system, so the
    /// session's `allowsCellularAccess` can't decide it. Read again before every fetch, so the
    /// download goes on as soon as Wi-Fi is back or the setting is turned on.
    nonisolated static func mayFetch(onCellular: Bool, allowCellular: Bool) -> Bool {
        !onCellular || allowCellular
    }

    private func mayFetchNow() -> Bool {
        Self.mayFetch(onCellular: onCellular(), allowCellular: allowCellular())
    }

    func hlsDirectory(_ id: UUID) -> URL {
        root.appendingPathComponent(id.uuidString, isDirectory: true).appendingPathComponent("hls", isDirectory: true)
    }

    var isRunning: Bool { !running.isEmpty }
    func isRunning(_ id: UUID) -> Bool { running[id] != nil }

    // MARK: - State

    /// Complete segments of a download, read from its directory the first time.
    func completeSegments(_ rec: DownloadRecord) -> IndexSet {
        if let d = done[rec.id] { return d }
        guard let count = rec.segmentCount else { return IndexSet() }
        let dir = hlsDirectory(rec.id)
        let fm = FileManager.default
        func present(_ sub: String) -> Set<Int> {
            let names = (try? fm.contentsOfDirectory(atPath: dir.appendingPathComponent(sub).path)) ?? []
            return Set(names.compactMap { $0.hasSuffix(".m4s") ? Int($0.dropLast(4)) : nil })
        }
        var all = present("video")
        for a in rec.audioTracks ?? [] { all.formIntersection(present("audio/\(a)")) }
        let set = IndexSet(all.filter { $0 < count })
        done[rec.id] = set
        return set
    }

    /// Complete segments from the first one on, without a gap.
    func contiguous(_ rec: DownloadRecord) -> Int {
        let d = completeSegments(rec)
        var n = 0
        while d.contains(n) { n += 1 }
        return n
    }

    /// Seconds of the title the contiguous downloaded part covers (from the start).
    func contiguousSeconds(_ rec: DownloadRecord) -> Double {
        let n = contiguous(rec)
        guard n > 0, let text = try? String(contentsOf: hlsDirectory(rec.id).appendingPathComponent("video/index.m3u8"), encoding: .utf8) else { return 0 }
        var total = 0.0, seen = 0
        for line in text.split(separator: "\n") where line.hasPrefix("#EXTINF:") && seen < n {
            total += Double(line.dropFirst(8).split(separator: ",").first ?? "") ?? 0
            seen += 1
        }
        return total
    }

    /// The segments playback at `seconds` needs next are on the device (every track): the one
    /// there and, near its end, the one after.
    func hasSegments(_ rec: DownloadRecord, at seconds: Double) -> Bool {
        guard let text = try? String(contentsOf: hlsDirectory(rec.id).appendingPathComponent("video/index.m3u8"), encoding: .utf8) else { return false }
        let have = completeSegments(rec)
        return Self.segmentsNeeded(playlist: text, at: seconds).allSatisfy { have.contains($0) }
    }

    /// The segments of a media playlist that playback at `seconds` needs next: the one there, and
    /// the next one too within a second of its end (where a player stalls waiting for it).
    nonisolated static func segmentsNeeded(playlist: String, at seconds: Double) -> [Int] {
        let durations = playlist.split(separator: "\n").filter { $0.hasPrefix("#EXTINF:") }
            .map { Double($0.dropFirst(8).split(separator: ",").first ?? "") ?? 0 }
        var start = 0.0
        for (n, d) in durations.enumerated() {
            if seconds < start + d {
                return seconds > start + d - 1 && n + 1 < durations.count ? [n, n + 1] : [n]
            }
            start += d
        }
        return []
    }

    /// The local server served (or was asked for) a segment file ("video/12.m4s"): download from
    /// there on.
    func playerAsked(_ id: UUID, path: String) {
        playhead[id, default: PlayerPosition()].asked(path)
    }

    /// The player waits for a file that isn't on the device yet (`fetchNow`).
    private func playerWaits(_ id: UUID, _ waiting: Bool) {
        playhead[id, default: PlayerPosition()].waits(waiting)
    }

    /// The download is deleted: its run and its files in flight stop, and a file that arrives
    /// anyway is thrown away rather than making the download's folder again.
    func forget(_ id: UUID) {
        stop(id)
        deleted.insert(id)
        let prefix = id.uuidString + "/"
        for (key, task) in inflight where key.hasPrefix(prefix) {
            task.cancel()
            inflight[key] = nil
        }
        done[id] = nil
        playhead[id] = nil
        bytes[id] = nil
    }

    // MARK: - Running

    func start(_ id: UUID) {
        guard running[id] == nil else { return }
        let token = UUID()
        runToken[id] = token
        running[id] = Task { [weak self] in
            await self?.run(id)
            if self?.runToken[id] == token {
                self?.running[id] = nil
                self?.runToken[id] = nil
            }
        }
    }

    func stop(_ id: UUID) {
        running.removeValue(forKey: id)?.cancel()
        runToken[id] = nil
    }

    private func client(for rec: DownloadRecord) -> (any MediaServerClient)? {
        SourceStore.shared.source(rec.sourceId).flatMap { MediaServers.client(for: $0) }
    }

    private func reachable(_ rec: DownloadRecord) -> Bool {
        let r = ConnectionRouter.shared.route(to: rec.sourceId)
        return r != .unreachable && r != nil
    }

    enum Failure: Error {
        /// Try again later (the server can't be reached, is busy, or errored).
        case transient(String)
        /// Give up (the file changed, the title is gone, the sign-in expired).
        case permanent(String)
        /// On mobile data with "Download over Cellular" off (`mayFetch`): wait for Wi-Fi.
        case waitingForWiFi
    }

    /// How often a download waiting for Wi-Fi looks again.
    static let wifiCheck: Double = 2

    /// How a download rides out a bad link. A file that fails is tried again at once, a couple of
    /// times, while the other segments go on (a dropped connection costs a moment, not the run).
    /// Only when that fails too does the whole run wait, and the wait grows only while nothing gets
    /// through: after segments arrived it starts again from the shortest, and the row says
    /// "Waiting for the server…" only from a second failure in a row with nothing in between.
    struct Retry {
        /// Tries of one file, and the pauses before the second and the third.
        static let fileTries = 3
        static let filePauses: [Double] = [0.5, 1.5]
        static let shortest: Double = 2
        static let longest: Double = 15

        private var wait = Self.shortest
        private var failedInARow = 0

        /// The run failed; `progressed`: segments arrived since the run started or last failed.
        /// How long to wait, and whether to tell the viewer it waits for the server.
        mutating func failed(progressed: Bool) -> (seconds: Double, waiting: Bool) {
            if progressed {
                wait = Self.shortest
                failedInARow = 0
            }
            failedInARow += 1
            let seconds = wait
            wait = min(wait * 2, Self.longest)
            return (seconds, failedInARow > 1)
        }

        /// Runs `fetch`, trying again after a transient failure (`fileTries` in all).
        static func file<T>(pause: (Double) async throws -> Void = { try await Task.sleep(for: .seconds($0)) },
                            _ fetch: () async throws -> T) async throws -> T {
            var tries = 1
            while true {
                do {
                    return try await fetch()
                } catch Failure.transient where tries < fileTries {
                    try Task.checkCancellation()
                    try await pause(filePauses[min(tries - 1, filePauses.count - 1)])
                    tries += 1
                }
            }
        }
    }

    private func run(_ id: UUID) async {
        var retry = Retry()
        while !Task.isCancelled {
            guard let rec = record?(id), rec.state == .downloading || rec.state == .queued else { return }
            let before = done[id]?.count ?? 0
            do {
                try await step(id)
                return
            } catch Failure.permanent(let msg) {
                log("\(rec.title): failed: \(msg)")
                onUpdate?(id) { $0.state = .failed; $0.error = msg }
                return
            } catch is CancellationError {
                return
            } catch Failure.waitingForWiFi {
                // Not the server's fault: no backoff, and back within seconds of Wi-Fi.
                if rec.error != Self.waitingForWiFiText {
                    log("\(rec.title): waiting for Wi-Fi (Download over Cellular is off)")
                    onUpdate?(id) { $0.error = Self.waitingForWiFiText }
                }
                try? await Task.sleep(for: .seconds(Self.wifiCheck))
            } catch {
                let msg: String = if case Failure.transient(let m) = error { m } else { error.localizedDescription }
                let (seconds, waiting) = retry.failed(progressed: (done[id]?.count ?? 0) > before)
                log("\(rec.title): waiting (\(msg)), again in \(Int(seconds)) s")
                if waiting { onUpdate?(id) { $0.error = "Waiting for the server…" } }
                try? await Task.sleep(for: .seconds(seconds))
                // The route may have gone (or come back): check it.
                await refreshRoutes()
            }
        }
    }

    nonisolated static let waitingForWiFiText = "Waiting for Wi-Fi…"

    /// Describe the set (once), then fetch what is missing. Returns when complete.
    private func step(_ id: UUID) async throws {
        guard var rec = record?(id) else { return }
        guard mayFetchNow() else { throw Failure.waitingForWiFi }
        if rec.error == Self.waitingForWiFiText { onUpdate?(id) { $0.error = nil } }
        guard let client = client(for: rec), let base = client.moonriseBaseURL else { throw Failure.transient("no server") }
        guard reachable(rec) else { throw Failure.transient("unreachable") }
        if !rec.hasPlaylist {
            try await describe(rec, client: client, base: base)
            guard let r = record?(id) else { return }
            rec = r
        }
        onUpdate?(id) { $0.state = .downloading; $0.error = nil }
        guard let count = rec.segmentCount else { return }
        let tracks = Self.tracks(rec)
        // Init sections first (the player needs them before any segment).
        for t in tracks where !exists(id, "\(t)/init.mp4") {
            try await Retry.file { try await self.fetch(rec, "\(t)/init.mp4", client: client, base: base) }
        }
        // Subtitles: one file per track, made by the server from the whole title.
        let subs = Task { [weak self] in
            for s in rec.subtitleTracks ?? [] {
                guard let self, !Task.isCancelled else { return }
                if !self.exists(id, "subs/\(s)/cues.vtt") { _ = try? await self.fetch(rec, "subs/\(s)/cues.vtt", client: client, base: base, timeout: 900) }
            }
        }
        defer { subs.cancel() }

        try await withThrowingTaskGroup(of: Void.self) { group in
            var claimed = IndexSet()
            var active = 0
            func spawn() async -> Bool {
                // While someone watches it, one at a time: the player's own requests come first.
                guard active < (await self.playerActive(id) ? 1 : Self.parallel) else { return false }
                guard let n = await self.nextSegment(id, rec, count: count, claimed: claimed) else { return false }
                claimed.insert(n)
                active += 1
                group.addTask { @MainActor [weak self] in
                    guard let self else { return }
                    for t in tracks where !self.exists(id, "\(t)/\(n).m4s") {
                        try await Retry.file { try await self.fetch(rec, "\(t)/\(n).m4s", client: client, base: base) }
                    }
                    self.segmentComplete(id, n, count: count)
                }
                return true
            }
            for _ in 0..<Self.parallel {
                guard await spawn() else { break }
            }
            while try await group.next() != nil {
                active -= 1
                try Task.checkCancellation()
                guard await self.record?(id)?.state == .downloading else { group.cancelAll(); throw CancellationError() }
                while await spawn() {}
            }
        }
        // Wait for the subtitles (they are small; extraction can take a while the first time).
        await subs.value
        let missingSubs = (rec.subtitleTracks ?? []).filter { !exists(id, "subs/\($0)/cues.vtt") }
        // Stopped by mobile data, not missing on the server: fetched once Wi-Fi is back.
        if !missingSubs.isEmpty, !mayFetchNow() { throw Failure.waitingForWiFi }
        if !missingSubs.isEmpty { log("\(rec.title): \(missingSubs.count) subtitle renditions missing") }
        guard completeSegments(rec).count >= count else { throw Failure.transient("segments missing") }
        let size = Self.directorySize(hlsDirectory(id))
        log("\(rec.title): complete, \(count) segments, \(size >> 20) MB")
        onUpdate?(id) {
            $0.state = .completed
            $0.progress = 1
            $0.segmentsDone = count
            $0.bytesWritten = size
            $0.bytesExpected = size
            $0.error = nil
            $0.localPath = DownloadRecord.relative(self.hlsDirectory(id))
        }
    }

    /// Someone watches this download: the player asked for a segment lately, or waits for one.
    private func playerActive(_ id: UUID) -> Bool {
        playhead[id]?.active() ?? false
    }

    private func nextSegment(_ id: UUID, _ rec: DownloadRecord, count: Int, claimed: IndexSet) -> Int? {
        let have = completeSegments(rec).union(claimed)
        return Self.nextSegment(have: have, count: count, from: playhead[id]?.from(have: have) ?? 0)
    }

    /// The next segment to fetch: the first one missing from `from` (where the viewer is) on,
    /// then from the start.
    nonisolated static func nextSegment(have: IndexSet, count: Int, from: Int) -> Int? {
        let from = max(0, min(from, count))
        for n in from..<count where !have.contains(n) { return n }
        for n in 0..<from where !have.contains(n) { return n }
        return nil
    }

    /// Where the player is in a download, from the segment files the local server is asked for.
    ///
    /// AVPlayer asks for each track's segments on its own, and the audio runs a few segments ahead
    /// of the video. Where it plays is the track furthest behind: the latest segment of each track,
    /// the lowest of those. (Taking the newest request of any track instead made the download
    /// start past the video segment the player was waiting for, and fetch that one last.)
    /// A track not asked for within `trackGone` of the newest request is one no longer played
    /// (another audio track was picked).
    ///
    /// A segment it asked for lately that still isn't here comes first of all: AVPlayer can give up
    /// on a segment (the link went while it waited), go on asking for the next ones, and then
    /// stall at the one it skipped. Only one a few segments behind where it plays, though: one
    /// left behind by a seek ahead isn't wanted any more.
    ///
    /// The player counts as watching while a request waits for a file (`fetchNow`), however long:
    /// a player waiting for a segment asks for nothing else, and the download must not take the
    /// link from that request then (AVPlayer gives up on a request after several seconds, and
    /// asks for the segment again only when playback restarts).
    struct PlayerPosition {
        static let recent: TimeInterval = 20
        static let trackGone: TimeInterval = 30
        static let wanted: TimeInterval = 90
        static let behind = 5

        private var latest: [String: (segment: Int, at: Date)] = [:]
        /// The segments asked for within `wanted`, and when.
        private var asks: [Int: Date] = [:]
        private var waiting = 0

        /// "video/12.m4s" or "audio/1/12.m4s" was asked for.
        mutating func asked(_ path: String, at now: Date = Date()) {
            guard let n = SegmentedDownloads.segment(path) else { return }
            latest[(path as NSString).deletingLastPathComponent] = (n, now)
            asks = asks.filter { now.timeIntervalSince($0.value) <= Self.wanted }
            asks[n] = now
        }

        /// Where the download goes on from: the first segment asked for lately, near where the
        /// player plays, that isn't in `have`; else where the player plays (`segment`).
        func from(have: IndexSet, at now: Date = Date()) -> Int? {
            guard let at = segment() else { return nil }
            let missing = asks.filter { $0.key >= at - Self.behind && now.timeIntervalSince($0.value) <= Self.wanted && !have.contains($0.key) }.keys
            return missing.min() ?? at
        }

        mutating func waits(_ start: Bool) {
            waiting = max(0, waiting + (start ? 1 : -1))
        }

        /// The segment the player plays (the track furthest behind), nil before any request.
        func segment() -> Int? {
            guard let newest = latest.values.map(\.at).max() else { return nil }
            return latest.values.filter { newest.timeIntervalSince($0.at) <= Self.trackGone }.map(\.segment).min()
        }

        func active(at now: Date = Date()) -> Bool {
            if waiting > 0 { return true }
            guard let newest = latest.values.map(\.at).max() else { return false }
            return now.timeIntervalSince(newest) < Self.recent
        }
    }

    /// Video, then each audio track: the directories of the set's tracks.
    static func tracks(_ rec: DownloadRecord) -> [String] {
        ["video"] + (rec.audioTracks ?? []).map { "audio/\($0)" }
    }

    struct Set_: Decodable {
        struct Variant: Decodable { let id: String; let label: String; let bitrate: Int; let transcode: Bool }
        struct Audio: Decodable { let index: Int; let name: String; let language: String? }
        struct Subtitle: Decodable { let index: Int; let name: String; let language: String?; let forced: Bool }
        let version: String
        let variant: Variant
        let duration: Double
        let segments: Int
        let audio: [Audio]
        let subtitles: [Subtitle]
        let master: String
        let playlist: String
        let subsPlaylist: String
        let expected: Int64
        let base: String
    }

    private func describe(_ rec: DownloadRecord, client: any MediaServerClient, base: URL) async throws {
        guard let itemId = rec.itemId else { throw Failure.permanent("No title") }
        let quality = QualityOption.find(rec.qualityLabel).bitrate
        var comps = URLComponents(url: base.appendingPathComponent("v1/items/\(itemId)/offline"), resolvingAgainstBaseURL: false)!
        var q = [URLQueryItem(name: "hdr", value: AVPlayer.eligibleForHDRPlayback ? "1" : "0")]
        if let quality { q.append(URLQueryItem(name: "quality", value: String(quality))) }
        // A version picked for the download (a title with several files).
        if let v = rec.mediaSourceId { q.append(URLQueryItem(name: "version", value: v)) }
        comps.queryItems = q
        let req = HTTP.request(comps.url!, headers: client.headers, timeout: 60)
        let (data, status) = try await HTTP.data(for: req, on: session)
        // Deleted meanwhile: no folder for it.
        try Task.checkCancellation()
        if deleted.contains(rec.id) { throw CancellationError() }
        guard status == 200 else {
            let msg = String(data: data, encoding: .utf8) ?? "HTTP \(status)"
            throw status == 503 || status >= 500 ? Failure.transient(msg) : Failure.permanent(msg.isEmpty ? "The server answered \(status)." : msg)
        }
        let dec = JSONDecoder()
        dec.keyDecodingStrategy = .convertFromSnakeCase
        let set = try dec.decode(Set_.self, from: data)
        let dir = hlsDirectory(rec.id)
        let fm = FileManager.default
        try fm.createDirectory(at: dir.appendingPathComponent("video"), withIntermediateDirectories: true)
        try Data(set.master.utf8).write(to: dir.appendingPathComponent("master.m3u8"), options: .atomic)
        try Data(set.playlist.utf8).write(to: dir.appendingPathComponent("video/index.m3u8"), options: .atomic)
        for a in set.audio {
            let d = dir.appendingPathComponent("audio/\(a.index)")
            try fm.createDirectory(at: d, withIntermediateDirectories: true)
            try Data(set.playlist.utf8).write(to: d.appendingPathComponent("index.m3u8"), options: .atomic)
        }
        for s in set.subtitles {
            let d = dir.appendingPathComponent("subs/\(s.index)")
            try fm.createDirectory(at: d, withIntermediateDirectories: true)
            try Data(set.subsPlaylist.utf8).write(to: d.appendingPathComponent("index.m3u8"), options: .atomic)
        }
        log("\(rec.title): set \(set.variant.id) (\(set.variant.label)), \(set.segments) segments, audio \(set.audio.map(\.index)), \(set.subtitles.count) subtitles, ~\(set.expected >> 20) MB")
        let names = Dictionary(set.subtitles.map { ($0.index, $0.name) }, uniquingKeysWith: { a, _ in a })
        onUpdate?(rec.id) {
            $0.setBase = set.base
            $0.setVersion = set.version
            $0.segmentCount = set.segments
            $0.segmentsDone = 0
            $0.audioTracks = set.audio.map(\.index)
            $0.subtitleTracks = set.subtitles.map(\.index)
            $0.bytesExpected = set.expected
            $0.qualityLabel = set.variant.transcode ? set.variant.label : "Original"
            $0.runtimeTicks = $0.runtimeTicks ?? Int64(set.duration * 10_000_000)
            // The overlay's SRT files name their rendition, for PiP.
            for i in $0.subtitles.indices {
                if let n = Self.trackIndex(fromFile: $0.subtitles[i].file), let name = names[n] { $0.subtitles[i].rendition = name }
            }
        }
        done[rec.id] = IndexSet()
    }

    /// "…/sub-4.srt" → 4.
    nonisolated static func trackIndex(fromFile file: String) -> Int? {
        let name = (file as NSString).lastPathComponent
        guard name.hasPrefix("sub-"), name.hasSuffix(".srt") else { return nil }
        return Int(name.dropFirst(4).dropLast(4))
    }

    // MARK: - Files

    func exists(_ id: UUID, _ path: String) -> Bool {
        FileManager.default.fileExists(atPath: hlsDirectory(id).appendingPathComponent(path).path)
    }

    /// Fetch one file of the set to the device (shared with a request for the same file).
    @discardableResult
    func fetch(_ rec: DownloadRecord, _ path: String, client: any MediaServerClient, base: URL, timeout: TimeInterval = 90) async throws -> Bool {
        if deleted.contains(rec.id) { throw CancellationError() }
        let key = rec.id.uuidString + "/" + path
        let task: Task<Failure?, Never>
        if let t = inflight[key] {
            task = t
        } else {
            guard mayFetchNow() else { throw Failure.waitingForWiFi }
            guard let setBase = rec.setBase, let url = MoonriseServer.resolve(setBase + "/" + path, base: base) else {
                throw Failure.permanent("No download set")
            }
            var comps = URLComponents(url: url, resolvingAgainstBaseURL: false)!
            if let v = rec.setVersion { comps.queryItems = [URLQueryItem(name: "v", value: v)] }
            let req = HTTP.request(comps.url!, headers: client.headers, timeout: timeout)
            let dest = hlsDirectory(rec.id).appendingPathComponent(path)
            let session = self.session
            let id = rec.id
            task = Task<Failure?, Never> {
                do {
                    let tmp = try await Self.download(req, session: session)
                    // Deleted while it came (`forget`): thrown away, the folder stays gone.
                    if Task.isCancelled || self.deleted.contains(id) {
                        try? FileManager.default.removeItem(at: tmp)
                        return .permanent("Deleted")
                    }
                    let size = try Self.place(tmp, at: dest)
                    self.bytes[id, default: 0] += Int64(size)
                    return nil
                } catch let f as Failure {
                    return f
                } catch {
                    return .transient(error.localizedDescription)
                }
            }
            inflight[key] = task
        }
        let failure = await task.value
        if inflight[key] == task { inflight[key] = nil }
        if deleted.contains(rec.id) { throw CancellationError() }
        if let failure { throw failure }
        return true
    }

    /// GET to a temporary file.
    nonisolated private static func download(_ req: URLRequest, session: URLSession) async throws -> URL {
        let (tmp, status) = try await HTTP.download(for: req, on: session)
        guard status == 200 else {
            let body = (try? String(contentsOf: tmp, encoding: .utf8)) ?? ""
            try? FileManager.default.removeItem(at: tmp)
            switch status {
            case 409: throw Failure.permanent("The file changed on the server. Delete the download and download it again.")
            case 401, 403: throw Failure.permanent("Your sign-in has expired. Sign in again in Settings.")
            case 404: throw Failure.permanent("The server doesn't have this title any more.")
            default: throw Failure.transient("HTTP \(status) \(body.prefix(120))")
            }
        }
        return tmp
    }

    /// A fetched file into the download (moved whole: a partial file is never there). Its size.
    nonisolated private static func place(_ tmp: URL, at dest: URL) throws -> Int {
        let fm = FileManager.default
        try? fm.createDirectory(at: dest.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? fm.removeItem(at: dest)
        try fm.moveItem(at: tmp, to: dest)
        return (try? fm.attributesOfItem(atPath: dest.path)[.size] as? Int) ?? 0
    }

    private func segmentComplete(_ id: UUID, _ n: Int, count: Int) {
        var d = done[id] ?? IndexSet()
        d.insert(n)
        done[id] = d
        let total = d.count
        let written = bytes[id] ?? 0
        // The record (and its index.json) at most every second, and at the end.
        let now = Date()
        guard total == count || now.timeIntervalSince(lastSave[id] ?? .distantPast) > 1 else { return }
        lastSave[id] = now
        onUpdate?(id) { r in
            r.segmentsDone = total
            r.progress = Double(total) / Double(max(count, 1))
            r.bytesWritten = max(r.bytesWritten, written)
            if r.progress > 0.02, r.bytesWritten > 0 { r.bytesExpected = max(r.bytesExpected, Int64(Double(r.bytesWritten) / r.progress)) }
        }
    }

    /// A file the player asked for that isn't on the device: fetch it now when the server can be
    /// reached; otherwise wait (up to `wait`) for it to arrive. True when it is there.
    func fetchNow(_ id: UUID, _ path: String, wait: TimeInterval, abandoned: @Sendable () -> Bool = { false }) async -> Bool {
        playerWaits(id, true)
        defer { playerWaits(id, false) }
        let deadline = Date().addingTimeInterval(wait)
        while Date() < deadline, !abandoned() {
            if exists(id, path) { return true }
            if let rec = record?(id), !reachable(rec) {
                // Watching for the server to come back.
                await refreshRoutes()
            }
            if mayFetchNow(), let rec = record?(id), rec.hasPlaylist, reachable(rec), let client = client(for: rec), let base = client.moonriseBaseURL {
                let ok: Bool
                do {
                    ok = try await fetch(rec, path, client: client, base: base, timeout: max(5, deadline.timeIntervalSinceNow))
                } catch {
                    // The server went away (or is busy): let the router find out.
                    ok = false
                    await refreshRoutes()
                }
                if ok {
                    if let n = Self.segment(path), let count = rec.segmentCount,
                       Self.tracks(rec).allSatisfy({ exists(id, "\($0)/\(n).m4s") }) {
                        segmentComplete(id, n, count: count)
                    }
                    return true
                }
            }
            try? await Task.sleep(for: .milliseconds(700))
        }
        return exists(id, path)
    }

    private var lastRouteCheck = Date.distantPast

    /// Re-check how the server can be reached, at most every 5 s.
    private func refreshRoutes() async {
        guard Date().timeIntervalSince(lastRouteCheck) > 5 else { return }
        lastRouteCheck = Date()
        await ConnectionRouter.shared.refresh()
    }

    /// "video/12.m4s" → 12.
    nonisolated static func segment(_ path: String) -> Int? {
        let name = (path as NSString).lastPathComponent
        guard name.hasSuffix(".m4s") else { return nil }
        return Int(name.dropLast(4))
    }

    nonisolated static func directorySize(_ url: URL) -> Int64 {
        var total: Int64 = 0
        if let e = FileManager.default.enumerator(at: url, includingPropertiesForKeys: [.fileSizeKey]) {
            for case let f as URL in e { total += Int64((try? f.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) }
        }
        return total
    }

    private func log(_ s: String) {
        #if DEBUG
        fputs("MoonriseDownload: \(s)\n", stderr)
        #endif
    }
}
