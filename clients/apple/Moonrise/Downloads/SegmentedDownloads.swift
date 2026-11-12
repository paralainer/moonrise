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

    /// Per download: complete segments (every track), the player's latest segment, the fetch task.
    private var done: [UUID: IndexSet] = [:]
    private var playhead: [UUID: (segment: Int, at: Date)] = [:]
    private var running: [UUID: Task<Void, Never>] = [:]
    private var runToken: [UUID: UUID] = [:]
    /// Files being fetched (download id + relative path), so the player and the loop share them.
    private var inflight: [String: Task<Failure?, Never>] = [:]
    private var bytes: [UUID: Int64] = [:]
    private var lastSave: [UUID: Date] = [:]

    init(root: URL) {
        self.root = root
        session = HTTP.downloadSetSession(cellular: AppSettings.shared.allowCellularDownloads)
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

    /// The local server served (or was asked for) segment `n`: download from there on.
    func playerAsked(_ id: UUID, segment n: Int) {
        playhead[id] = (n, Date())
    }

    func forget(_ id: UUID) {
        stop(id)
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
    }

    private func run(_ id: UUID) async {
        var backoff: Double = 2
        while !Task.isCancelled {
            guard let rec = record?(id), rec.state == .downloading || rec.state == .queued else { return }
            do {
                try await step(id)
                return
            } catch Failure.permanent(let msg) {
                log("\(rec.title): failed: \(msg)")
                onUpdate?(id) { $0.state = .failed; $0.error = msg }
                return
            } catch is CancellationError {
                return
            } catch {
                let msg: String = if case Failure.transient(let m) = error { m } else { error.localizedDescription }
                log("\(rec.title): waiting (\(msg)), again in \(Int(backoff)) s")
                onUpdate?(id) { $0.error = "Waiting for the server…" }
                try? await Task.sleep(for: .seconds(backoff))
                backoff = min(backoff * 2, 15)
                // The route may have gone (or come back): check it.
                await refreshRoutes()
            }
        }
    }

    /// Describe the set (once), then fetch what is missing. Returns when complete.
    private func step(_ id: UUID) async throws {
        guard var rec = record?(id) else { return }
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
            try await fetch(rec, "\(t)/init.mp4", client: client, base: base)
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
                        try await self.fetch(rec, "\(t)/\(n).m4s", client: client, base: base)
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

    /// The player asked for a segment of this download lately.
    private func playerActive(_ id: UUID) -> Bool {
        playhead[id].map { Date().timeIntervalSince($0.at) < 20 } ?? false
    }

    /// The next segment to fetch: from where the viewer is, then from the start.
    private func nextSegment(_ id: UUID, _ rec: DownloadRecord, count: Int, claimed: IndexSet) -> Int? {
        let have = completeSegments(rec).union(claimed)
        let from = min(playhead[id].map { $0.segment } ?? 0, count)
        for n in from..<count where !have.contains(n) { return n }
        for n in 0..<from where !have.contains(n) { return n }
        return nil
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
        let key = rec.id.uuidString + "/" + path
        let task: Task<Failure?, Never>
        if let t = inflight[key] {
            task = t
        } else {
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
                    let size = try await Self.download(req, to: dest, session: session)
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
        inflight[key] = nil
        if let failure { throw failure }
        return true
    }

    /// GET to a file (written whole: a partial file is never there).
    nonisolated private static func download(_ req: URLRequest, to dest: URL, session: URLSession) async throws -> Int {
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
        let deadline = Date().addingTimeInterval(wait)
        while Date() < deadline, !abandoned() {
            if exists(id, path) { return true }
            if let rec = record?(id), !reachable(rec) {
                // Watching for the server to come back.
                await refreshRoutes()
            }
            if let rec = record?(id), rec.hasPlaylist, reachable(rec), let client = client(for: rec), let base = client.moonriseBaseURL {
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
