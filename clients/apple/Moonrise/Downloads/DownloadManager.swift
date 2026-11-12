import AVFoundation
import Foundation
import Observation

/// Owns all offline copies, all from the Moonrise server. New downloads are segmented
/// (`SegmentedDownloads`): the server's HLS segments at the chosen quality, saved one by one and
/// played through `LocalHLSServer` while they come in; one runs at a time, and on iOS a
/// `BGContinuedProcessingTask` (iOS 26) keeps the app running in the background meanwhile.
/// Downloads made by earlier builds (an original MP4, or a progressive MP4 the server prepared,
/// over a background `URLSession`) still play, resume and delete. Text subtitles (also those it
/// fetched from OpenSubtitles) are saved as .srt next to the download. A Live Activity mirrors
/// progress in the Dynamic Island.
///
/// `badges` (and the series/season counts) is what the library screens observe: it changes only
/// when a title's downloaded state or whole percent does.
@MainActor
@Observable
final class DownloadManager: NSObject {
    static let shared = DownloadManager()

    private(set) var records: [DownloadRecord] = []

    enum Badge: Equatable {
        case downloaded
        /// Whole percent.
        case downloading(Int)
        case queued, paused, failed
    }

    /// By item id: what the cards and pages show.
    private(set) var badges: [String: Badge] = [:]
    /// Downloaded (or downloading) episodes by show and by season id.
    private(set) var seriesCounts: [String: Int] = [:]
    private(set) var seasonCounts: [String: Int] = [:]

    /// Not on the Apple TV: it streams only.
    static var isSupported: Bool {
        #if os(tvOS)
        return false
        #else
        // The demo's films aren't downloaded.
        return !DemoMode.isActive
        #endif
    }

    @ObservationIgnored private(set) lazy var segmented: SegmentedDownloads = {
        let s = SegmentedDownloads(root: root)
        s.record = { [weak self] in self?.record($0) }
        s.onUpdate = { [weak self] id, change in self?.segmentedUpdate(id, change) }
        return s
    }()

    @ObservationIgnored private var fileSession: URLSession!
    @ObservationIgnored private var tasks: [UUID: URLSessionTask] = [:]
    @ObservationIgnored private var speed: [UUID: (bytes: Int64, at: Date, bps: Double)] = [:]
    @ObservationIgnored private var lastSave = Date.distantPast
    @ObservationIgnored var backgroundCompletionHandlers: [String: () -> Void] = [:]
    @ObservationIgnored private let activity = DownloadActivityController()

    static let fileSessionId = AppInfo.bundleId + ".bg.files"

    private let root: URL = {
        #if os(tvOS)
        let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        #else
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        #endif
        let dir = base.appendingPathComponent("Downloads", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        var d = dir
        try? d.setResourceValues(values)
        return dir
    }()

    private var indexURL: URL { root.appendingPathComponent("index.json") }

    override init() {
        super.init()
        #if os(tvOS)
        // A build briefly allowed downloads on the Apple TV: drop what it left.
        try? FileManager.default.removeItem(at: root)
        return
        #endif
        if let data = try? Data(contentsOf: indexURL),
           let decoded = try? JSONDecoder().decode([DownloadRecord].self, from: data) {
            records = decoded
        }
        // Kinds only older versions made (HLS packages, file-share copies): gone, with
        // their files.
        let legacy = records.filter { $0.kind == .hls || $0.kind == .smb }
        if !legacy.isEmpty {
            for r in legacy { removeFiles(of: r) }
            records.removeAll { $0.kind == .hls || $0.kind == .smb }
            save()
        }

        fileSession = HTTP.backgroundFileSession(identifier: Self.fileSessionId, cellular: AppSettings.shared.allowCellularDownloads,
                                                 delegate: self)

        refreshBadges()
        restoreTasks()
    }

    // MARK: - Queries

    func record(_ id: UUID) -> DownloadRecord? { records.first { $0.id == id } }
    func record(forItem itemId: String) -> DownloadRecord? { records.first { $0.itemId == itemId } }
    /// The download of a title that plays from the device now (finished, or segmented and started).
    func playable(forItem itemId: String) -> DownloadRecord? { records.first { $0.itemId == itemId && $0.isPlayable } }
    /// Downloaded or downloading episodes of a show, in airing order.
    func episodes(ofSeries seriesId: String) -> [DownloadRecord] {
        records.filter { $0.seriesId == seriesId }.sorted {
            MediaItem.order($0.item).lexicographicallyPrecedes(MediaItem.order($1.item))
        }
    }

    var active: [DownloadRecord] { records.filter { $0.state == .downloading || $0.state == .queued } }

    /// Downloads made before shows were kept with their episodes, and titles saved before the
    /// Library's filters knew studios and collections: described again (once, while online).
    func refreshSnapshots(client: any MediaServerClient) async {
        let stale = records.filter { $0.itemId != nil && ($0.item?.addedAt == nil || ($0.item?.isEpisode == true && $0.series == nil)) }
        guard !stale.isEmpty else { return }
        let ids = Array(Set(stale.compactMap(\.itemId)))
        let fresh = (try? await client.items(ids: ids)) ?? []
        var shows: [String: MediaItem] = [:]
        for sid in Set(stale.compactMap { $0.item?.isEpisode == true && $0.series == nil ? $0.seriesId : nil }) {
            if let show = try? await client.item(sid) { shows[sid] = Self.snapshot(show) }
        }
        for r in stale {
            update(r.id) { rec in
                if let f = fresh.first(where: { $0.id == rec.itemId }) {
                    // Keep the file's tracks (the list answer has a summary only when the item has none).
                    var i = Self.snapshot(f)
                    if i.tracks == nil { i.tracks = rec.item?.tracks }
                    if i.versions == nil { i.versions = rec.item?.versions }
                    rec.item = i
                }
                if let sid = rec.seriesId, let show = shows[sid] { rec.series = show }
            }
        }
    }

    // MARK: - Enqueue

    /// `version`: which of the title's files (several versions: the one picked).
    func download(item: MediaItem, from source: Source, quality: QualityOption, version: MediaVersion? = nil) {
        guard MediaServers.client(for: source) != nil, record(forItem: item.id) == nil else { return }
        var rec = DownloadRecord(
            kind: .segments,
            title: item.displayTitle,
            subtitle: item.isEpisode ? [item.seriesName, item.episodeLabel].compactMap { $0 }.joined(separator: " · ") : item.productionYear.map(String.init),
            overview: item.overview, sourceId: source.id, itemId: item.id,
            qualityLabel: quality.label, fileExtension: "m3u8")
        // Several versions: the one picked (its label goes with the quality's).
        if let version, item.hasVersions {
            rec.mediaSourceId = version.id
            rec.versionLabel = PlayerModel.versionTitle(version)
        }
        rec.runtimeTicks = (version?.runtime ?? item.runtime).map { Int64($0 * 10_000_000) }
        rec.year = item.productionYear
        rec.item = Self.snapshot(item)
        rec.state = .queued
        if let b = quality.bitrate, let t = version?.runtime ?? item.runtime { rec.bytesExpected = Int64(Double(b) / 8 * t) }
        records.insert(rec, at: 0)
        save()
        let id = rec.id
        Task {
            if let client = MediaServers.client(for: source) {
                await fetchArtwork(id: id, item: item, client: client)
                await fetchSubtitles(id: id, item: item, client: client)
            }
        }
        startNextSegmented()
    }

    /// What a download keeps of its title: enough for offline pages, badges and reports.
    private static func snapshot(_ item: MediaItem) -> MediaItem {
        var i = item
        i.people = nil
        i.chapters = nil
        return i
    }

    /// One segmented download at a time: the oldest one waiting.
    private func startNextSegmented() {
        let waiting = records.filter { $0.isSegmented && ($0.state == .downloading || $0.state == .queued) }
        guard !waiting.contains(where: { segmented.isRunning($0.id) }) else { return }
        guard let next = waiting.min(by: { $0.createdAt < $1.createdAt }) else { workChanged(); return }
        update(next.id) { $0.state = .downloading; $0.error = nil }
        for r in waiting where r.id != next.id && r.state == .downloading { update(r.id) { $0.state = .queued } }
        segmented.start(next.id)
        didStartWork(title: next.title, keepAppAlive: true)
        // Its subtitle files, if the app stopped before they were saved.
        if next.subtitles.isEmpty, let item = next.item, let source = SourceStore.shared.source(next.sourceId),
           let client = MediaServers.client(for: source) {
            Task { await fetchSubtitles(id: next.id, item: item, client: client) }
        }
    }

    /// A change from the segment fetcher.
    private func segmentedUpdate(_ id: UUID, _ change: (inout DownloadRecord) -> Void) {
        guard let i = records.firstIndex(where: { $0.id == id }) else { return }
        let before = records[i].state
        change(&records[i])
        let after = records[i].state
        save(force: before != after || records[i].progress >= 1)
        refreshBadges()
        reportProgress()
        if before != after, after == .completed || after == .failed {
            workChanged()
            startNextSegmented()
        }
    }

    /// Ask the Moonrise server for the MP4 (or find the job again), wait until it is made, then
    /// fetch it.
    private func startPrepared(id: UUID, itemId: String, quality: QualityOption, client: any MediaServerClient) async {
        guard let moonrise = client.moonriseBaseURL else {
            update(id) { $0.state = .failed; $0.error = "The Moonrise server isn't reachable." }
            return
        }
        update(id) { $0.state = .downloading; $0.preparing = true; $0.error = nil }
        var job: MoonriseServer.DownloadJob
        do {
            if let jid = record(id)?.moonriseJobId, let j = try? await MoonriseServer.downloadJob(base: moonrise, id: jid, headers: client.headers), j.state != "failed" {
                job = j
            } else {
                job = try await MoonriseServer.createDownloadJob(base: moonrise, itemId: itemId, quality: quality.bitrate, audio: nil, headers: client.headers)
                update(id) { $0.moonriseJobId = job.id }
            }
        } catch {
            #if DEBUG
            fputs("MoonriseDownload: job refused (\(error.localizedDescription))\n", stderr)
            #endif
            update(id) { $0.state = .failed; $0.preparing = false; $0.error = error.localizedDescription }
            return
        }
        #if DEBUG
        fputs("MoonriseDownload: job \(job.id) \(job.method) \(job.label) \(job.state)\n", stderr)
        #endif
        if record(id)?.bytesExpected == 0 { update(id) { $0.bytesExpected = job.expected } }
        while job.state == "queued" || job.state == "running" {
            try? await Task.sleep(for: .seconds(2))
            // Paused or deleted meanwhile: the server keeps making it; polling resumes later.
            guard let r = record(id), r.state == .downloading, r.preparing == true else { return }
            guard let j = try? await MoonriseServer.downloadJob(base: moonrise, id: job.id, headers: client.headers) else { continue }
            job = j
            progressed(id, written: 0, total: max(job.expected, 1), fraction: job.progress)
        }
        guard job.state == "done", let path = job.file, let url = MoonriseServer.resolve(path, base: moonrise) else {
            // Gone (the server restarted or expired it) or failed: make it again once.
            if job.state != "done", record(id)?.moonriseJobId == job.id {
                update(id) { $0.moonriseJobId = nil }
                if job.state == "failed" {
                    update(id) { $0.state = .failed; $0.preparing = false; $0.error = job.error ?? "The server couldn't prepare it." }
                }
            }
            return
        }
        #if DEBUG
        fputs("MoonriseDownload: job \(job.id) ready (\(job.size >> 20) MB), fetching\n", stderr)
        #endif
        update(id) {
            $0.preparing = false
            $0.moonrisePath = path
            $0.bytesExpected = job.size
            $0.progress = 0
        }
        startFile(id: id, url: url, headers: client.headers)
    }

    // MARK: - Workers

    private func directory(for id: UUID) -> URL {
        let d = root.appendingPathComponent(id.uuidString, isDirectory: true)
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        return d
    }

    private func startFile(id: UUID, url: URL, headers: [String: String], resumeData: Data? = nil) {
        let task: URLSessionDownloadTask
        if let resumeData {
            task = fileSession.downloadTask(withResumeData: resumeData)
        } else {
            var req = HTTP.request(url, headers: headers)
            req.allowsCellularAccess = AppSettings.shared.allowCellularDownloads
            task = fileSession.downloadTask(with: req)
        }
        task.taskDescription = id.uuidString
        tasks[id] = task
        update(id) { $0.state = .downloading; $0.error = nil }
        task.resume()
    }

    private func fetchArtwork(id: UUID, item: MediaItem, client: any MediaServerClient) async {
        let dir = directory(for: id)
        if item.isEpisode, let sid = item.seriesId, let show = try? await client.item(sid) {
            update(id) { $0.series = Self.snapshot(show) }
        }
        await saveSegments(id: id, item: item, client: client)
        if record(id)?.segmentsPending == true { refreshSegments(id: id, item: item, client: client) }
        if let url = client.posterURL(item, maxWidth: 500), let data = try? await client.fetch(url) {
            let f = dir.appendingPathComponent("poster.jpg")
            try? data.write(to: f)
            update(id) { $0.posterFile = DownloadRecord.relative(f) }
        }
        // Wide artwork: an episode's own still, else the title's backdrop.
        if let url = (item.isEpisode ? client.thumbURL(item, maxWidth: 1000) : nil) ?? client.backdropURL(item, maxWidth: 1400),
           let data = try? await client.fetch(url) {
            let f = dir.appendingPathComponent("backdrop.jpg")
            try? data.write(to: f)
            update(id) { $0.backdropFile = DownloadRecord.relative(f) }
        }
    }

    /// The skippable segments, kept with the download for offline playback. While the server is
    /// still looking for the end credits in the file, the community databases' (checked against
    /// the file) are kept meanwhile, and the download is marked to ask again; so is one saved
    /// while the server still waits for the databases (no credits yet).
    func saveSegments(id: UUID, item: MediaItem, client: any MediaServerClient) async {
        let version = record(id)?.mediaSourceId
        let segments = await SegmentService.segments(for: item, version: version, client: client, provisional: true)
        // A film too while the server waits for the community databases: its credits wait for them.
        let pending = (item.isEpisode || SegmentService.awaitingCommunity(item, version: version))
            && SegmentService.awaitingServer(item, version: version)
        guard record(id) != nil else { return }
        update(id) {
            if !segments.isEmpty { $0.segments = segments }
            $0.segmentsPending = pending ? true : nil
        }
        #if DEBUG
        fputs("MoonriseDownload: segments saved, credits \(segments.filter { $0.kind == .credits }.map { "\(Int($0.start))\($0.source.map { " " + $0 } ?? "")" })\(pending ? ", asked again later" : "")\n", stderr)
        #endif
    }

    /// Downloads whose segments are being asked for again (`refreshSegments`).
    @ObservationIgnored private var refreshingSegments: Set<UUID> = []

    /// Asks for a download's segments again each minute until the server's answer is final (it
    /// looks at one file at a time, so a season can take a while), for up to an hour.
    func refreshSegments(id: UUID, item: MediaItem, client: any MediaServerClient) {
        guard refreshingSegments.insert(id).inserted else { return }
        Task {
            defer { refreshingSegments.remove(id) }
            for _ in 0..<60 {
                try? await Task.sleep(for: .seconds(60))
                guard record(id)?.segmentsPending == true else { return }
                await saveSegments(id: id, item: item, client: client)
            }
        }
    }

    /// Text subtitles as .srt so they work offline with the overlay renderer: the Moonrise server's
    /// tracks (it extracts embedded ones, which can take minutes for a big file the first time,
    /// and lists those fetched from OpenSubtitles).
    private func fetchSubtitles(id: UUID, item: MediaItem, client: any MediaServerClient) async {
        let dir = directory(for: id)
        // The version downloaded: its own subtitle tracks.
        let mediaId = record(id)?.mediaSourceId.map { "\(item.id)~\($0)" } ?? item.id
        if let moonrise = client.moonriseBaseURL, let tracks = await MoonriseServer.subtitles(base: moonrise, itemId: mediaId, headers: client.headers) {
            var subs: [LocalSubtitle] = []
            for t in tracks where t.text {
                guard let path = t.url, let url = MoonriseServer.resolve(path, base: moonrise),
                      let data = try? await client.fetch(url, timeout: 900) else { continue }
                let f = dir.appendingPathComponent("sub-\(t.index).srt")
                try? data.write(to: f)
                subs.append(LocalSubtitle(title: t.name.replacingOccurrences(of: " - ", with: " · "), language: t.language,
                                          file: DownloadRecord.relative(f), rendition: t.name))
            }
            #if DEBUG
            fputs("MoonriseDownload: \(subs.count) of \(tracks.filter(\.text).count) subtitle tracks saved\n", stderr)
            #endif
            if record(id) != nil { update(id) { $0.subtitles = subs } }
        }
    }

    // MARK: - Controls

    func pause(_ id: UUID) {
        guard let rec = record(id) else { return }
        switch rec.kind {
        case .file, .prepared:
            (tasks[id] as? URLSessionDownloadTask)?.cancel { data in
                Task { @MainActor in
                    if let data { try? data.write(to: self.directory(for: id).appendingPathComponent("resume.data")) }
                }
            }
            tasks[id] = nil
        case .segments:
            segmented.stop(id)
        case .hls, .smb:
            break
        }
        update(id) { $0.state = .paused }
        workChanged()
        startNextSegmented()
    }

    func resume(_ id: UUID) {
        guard let rec = record(id), let source = SourceStore.shared.source(rec.sourceId) else { return }
        switch rec.kind {
        case .segments:
            update(id) { $0.state = .queued; $0.error = nil }
            startNextSegmented()
            return
        case .file:
            guard let client = MediaServers.client(for: source), let itemId = rec.itemId else { return }
            let resumeFile = directory(for: id).appendingPathComponent("resume.data")
            let data = try? Data(contentsOf: resumeFile)
            try? FileManager.default.removeItem(at: resumeFile)
            guard let url = rec.moonrisePath.flatMap({ p in client.moonriseBaseURL.flatMap { MoonriseServer.resolve(p, base: $0) } })
                ?? client.downloadURL(itemId: itemId) else { return }
            startFile(id: id, url: url, headers: client.headers, resumeData: data)
        case .prepared:
            guard let client = MediaServers.client(for: source), let itemId = rec.itemId else { return }
            let resumeFile = directory(for: id).appendingPathComponent("resume.data")
            if rec.preparing == false, let path = rec.moonrisePath, let base = client.moonriseBaseURL, let url = MoonriseServer.resolve(path, base: base) {
                let data = try? Data(contentsOf: resumeFile)
                try? FileManager.default.removeItem(at: resumeFile)
                startFile(id: id, url: url, headers: client.headers, resumeData: data)
            } else {
                Task { await startPrepared(id: id, itemId: itemId, quality: QualityOption.find(rec.qualityLabel), client: client) }
            }
        case .hls, .smb:
            return
        }
        didStartWork(title: rec.title, keepAppAlive: isTailnet(rec.sourceId))
    }

    func delete(_ id: UUID) {
        if let rec = record(id), let job = rec.moonriseJobId, rec.state != .completed,
           let source = SourceStore.shared.source(rec.sourceId), let client = MediaServers.client(for: source), let base = client.moonriseBaseURL {
            // Not needed any more: the server stops making it and frees the space.
            Task { await MoonriseServer.deleteDownloadJob(base: base, id: job, headers: client.headers) }
        }
        tasks.removeValue(forKey: id)?.cancel()
        segmented.forget(id)
        if let rec = record(id) { removeFiles(of: rec) }
        LocalTrackMemory.forget(download: id)
        records.removeAll { $0.id == id }
        save()
        workChanged()
        startNextSegmented()
    }

    /// Everything downloaded from these servers (dropped ones), with the files.
    func removeAll(fromSources ids: Set<UUID>) {
        for r in records where ids.contains(r.sourceId) {
            tasks.removeValue(forKey: r.id)?.cancel()
            segmented.forget(r.id)
            removeFiles(of: r)
            LocalTrackMemory.forget(download: r.id)
        }
        records.removeAll { ids.contains($0.sourceId) }
        save()
        workChanged()
    }

    private func removeFiles(of rec: DownloadRecord) {
        if let url = rec.localURL { try? FileManager.default.removeItem(at: url) }
        try? FileManager.default.removeItem(at: root.appendingPathComponent(rec.id.uuidString))
    }

    /// Persist a subtitle fetched during playback alongside an offline copy of the same media.
    func attachSubtitle(_ data: Data, title: String, language: String?, to request: PlaybackRequest) {
        let rec: DownloadRecord? = switch request.target {
        case let .server(_, item): record(forItem: item.id)
        }
        guard let rec else { return }
        let f = directory(for: rec.id).appendingPathComponent("os-\(UUID().uuidString.prefix(8)).srt")
        try? data.write(to: f)
        update(rec.id) { $0.subtitles.append(LocalSubtitle(title: title, language: language, file: DownloadRecord.relative(f))) }
    }

    // MARK: - State

    private func update(_ id: UUID, _ change: (inout DownloadRecord) -> Void) {
        guard let i = records.firstIndex(where: { $0.id == id }) else { return }
        change(&records[i])
        save()
    }

    private func save(force: Bool = true) {
        refreshBadges()
        guard force || Date().timeIntervalSince(lastSave) > 5 else { return }
        lastSave = Date()
        if let data = try? JSONEncoder().encode(records) {
            try? data.write(to: indexURL, options: .atomic)
        }
    }

    private func refreshBadges() {
        var b: [String: Badge] = [:]
        var series: [String: Int] = [:]
        var seasons: [String: Int] = [:]
        for r in records {
            guard let id = r.itemId else { continue }
            let badge: Badge = switch r.state {
            case .completed: .downloaded
            case .downloading: .downloading(Int(r.progress * 100))
            case .queued: .queued
            case .paused: .paused
            case .failed: .failed
            }
            b[id] = badge
            if let s = r.item?.seriesId { series[s, default: 0] += 1 }
            if let s = r.item?.seasonId { seasons[s, default: 0] += 1 }
        }
        if b != badges { badges = b }
        if series != seriesCounts { seriesCounts = series }
        if seasons != seasonCounts { seasonCounts = seasons }
    }

    fileprivate func progressed(_ id: UUID, written: Int64, total: Int64, fraction: Double? = nil) {
        guard let i = records.firstIndex(where: { $0.id == id }) else { return }
        records[i].bytesWritten = written
        if total > 0 { records[i].bytesExpected = total }
        records[i].progress = fraction ?? (total > 0 ? Double(written) / Double(total) : 0)
        if records[i].state != .paused { records[i].state = .downloading }

        let now = Date()
        if let s = speed[id] {
            let dt = now.timeIntervalSince(s.at)
            if dt >= 1 {
                let inst = Double(written - s.bytes) / dt
                speed[id] = (written, now, s.bps == 0 ? inst : s.bps * 0.7 + inst * 0.3)
            }
        } else {
            speed[id] = (written, now, 0)
        }
        save(force: false)
        reportProgress()
    }

    private func finished(_ id: UUID, location: URL?, error: String?) {
        tasks[id] = nil
        speed[id] = nil
        update(id) { rec in
            if let error {
                rec.state = .failed
                rec.error = error
            } else if let location {
                rec.localPath = DownloadRecord.relative(location)
                rec.state = .completed
                rec.progress = 1
            }
        }
        workChanged()
    }

    private func restoreTasks() {
        fileSession.getAllTasks { tasks in
            MainActor.assumeIsolated { self.adopt(tasks) }
        }
        // Segmented downloads pick up where they were once the routes are known.
        if records.contains(where: { $0.isSegmented && ($0.state == .downloading || $0.state == .queued) }) {
            Task {
                await ConnectionRouter.shared.ready()
                startNextSegmented()
            }
        }
        // Segments saved while the server was still looking for the end credits: ask again.
        let segmentsPending = records.filter { $0.segmentsPending == true && $0.item != nil }
        if !segmentsPending.isEmpty {
            Task {
                await ConnectionRouter.shared.ready()
                for rec in segmentsPending {
                    guard let source = SourceStore.shared.source(rec.sourceId), let client = MediaServers.client(for: source), let item = rec.item else { continue }
                    refreshSegments(id: rec.id, item: item, client: client)
                }
            }
        }
        // Prepared downloads still being made on the server: poll again once the routes are known.
        let preparing = records.filter { $0.kind == .prepared && $0.state == .downloading && $0.preparing == true }
        if !preparing.isEmpty {
            Task {
                await ConnectionRouter.shared.ready()
                for r in preparing {
                    guard let source = SourceStore.shared.source(r.sourceId), let client = MediaServers.client(for: source), let itemId = r.itemId else { continue }
                    await startPrepared(id: r.id, itemId: itemId, quality: QualityOption.find(r.qualityLabel), client: client)
                }
            }
        }
    }

    private func adopt(_ found: [URLSessionTask]) {
        for t in found {
            guard let s = t.taskDescription, let id = UUID(uuidString: s), record(id) != nil else {
                t.cancel()
                continue
            }
            tasks[id] = t
        }
        workChanged()
    }

    // MARK: - Live Activity + continued processing

    /// - keepAppAlive: only downloads over the tailnet need the app (the tunnel is in-process).
    ///   Others live in the system's background session, which iOS 26 already shows in the Dynamic
    ///   Island; a continued-processing task on top of those just shows the same download twice.
    private func isTailnet(_ sourceId: UUID) -> Bool {
        if case .tailnet = ConnectionRouter.shared.route(to: sourceId) { return true }
        return false
    }

    private func didStartWork(title: String, keepAppAlive: Bool) {
        activity.begin(title: title)
        #if os(iOS)
        if keepAppAlive, #available(iOS 26.0, *) {
            ContinuedDownloadTask.shared.begin(title: "Downloading \(title)")
        }
        #endif
        reportProgress()
    }

    private func workChanged() {
        reportProgress()
        if active.isEmpty {
            activity.end(completedTitle: records.first(where: { $0.state == .completed })?.title)
            #if os(iOS)
            if #available(iOS 26.0, *) { ContinuedDownloadTask.shared.finish(success: !records.contains { $0.state == .failed }) }
            #endif
        } else if !active.contains(where: { isTailnet($0.sourceId) || $0.isSegmented }) {
            #if os(iOS)
            if #available(iOS 26.0, *) { ContinuedDownloadTask.shared.finish(success: true) }
            #endif
        }
    }

    private func reportProgress() {
        let running = records.filter { $0.state == .downloading }
        guard let current = running.first else { return }
        let total = running.reduce(Int64(0)) { $0 + max($1.bytesExpected, 1) }
        let done = running.reduce(Int64(0)) { $0 + $1.bytesWritten }
        let overall = running.count == 1 ? current.progress : (total > 0 ? Double(done) / Double(total) : 0)
        let bps = running.reduce(0.0) { $0 + (speed[$1.id]?.bps ?? 0) }
        let detail = current.bytesExpected > 0
            ? "\(Fmt.bytes(current.bytesWritten)) of \(Fmt.bytes(current.bytesExpected))"
            : current.qualityLabel
        activity.update(title: current.title, detail: detail, progress: overall, bytesPerSecond: bps,
                        queued: max(0, active.count - 1))
        #if os(iOS)
        if #available(iOS 26.0, *), let r = running.first(where: { isTailnet($0.sourceId) || $0.isSegmented }) {
            ContinuedDownloadTask.shared.update(
                fraction: r.progress, title: r.title,
                subtitle: "\(Fmt.bytes(r.bytesWritten)) of \(Fmt.bytes(r.bytesExpected))")
        }
        #endif
    }
}

// MARK: - URLSession delegates (delegate queue is .main)

extension DownloadManager: URLSessionDownloadDelegate {
    nonisolated func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64,
                                totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        guard let s = downloadTask.taskDescription, let id = UUID(uuidString: s) else { return }
        MainActor.assumeIsolated { progressed(id, written: totalBytesWritten, total: totalBytesExpectedToWrite) }
    }

    nonisolated func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        guard let s = downloadTask.taskDescription, let id = UUID(uuidString: s) else { return }
        let status = (downloadTask.response as? HTTPURLResponse)?.statusCode ?? 200
        // The temp file vanishes when this method returns, so move it synchronously.
        MainActor.assumeIsolated {
            guard (200..<300).contains(status) else {
                finished(id, location: nil, error: "Server returned \(status)")
                return
            }
            let ext = record(id)?.fileExtension ?? "mp4"
            let dest = directory(for: id).appendingPathComponent("media.\(ext)")
            try? FileManager.default.removeItem(at: dest)
            do {
                try FileManager.default.moveItem(at: location, to: dest)
                finished(id, location: dest, error: nil)
            } catch {
                finished(id, location: nil, error: error.localizedDescription)
            }
        }
    }

    nonisolated func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        guard let s = task.taskDescription, let id = UUID(uuidString: s) else { return }
        // Success is handled in didFinishDownloadingTo.
        guard let error else { return }
        let ns = error as NSError
        MainActor.assumeIsolated {
            if ns.code == NSURLErrorCancelled { return } // paused/deleted
            if let data = ns.userInfo[NSURLSessionDownloadTaskResumeData] as? Data {
                try? data.write(to: directory(for: id).appendingPathComponent("resume.data"))
            }
            finished(id, location: nil, error: error.localizedDescription)
        }
    }

    nonisolated func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        let identifier = session.configuration.identifier ?? ""
        MainActor.assumeIsolated {
            backgroundCompletionHandlers.removeValue(forKey: identifier)?()
        }
    }
}
