import AVFoundation
import Foundation

extension PlayerModel {
    /// Plays a download from the device: a file, or a segmented download through the loopback
    /// HLS server (also while it is still downloading).
    /// - restart: playing again where it was (a stall, a failure): keep the whole timeline
    ///   (never the growing playlist, which would move the viewer back to where it ends).
    func startLocal(_ record: DownloadRecord, source: Source?, at explicitStart: Double? = nil, restart: Bool = false) async {
        local = record
        // Again (a stall): the same tracks; else those picked last (once the tracks load).
        localAudioApplied = false
        localKeepAudio = restart ? selectedAudio : nil
        localSubtitlePending = !restart
        // The tracks played last in this download (this device keeps them).
        if !restart { trackMemory = LocalTrackMemory.load(download: record.id)?.memory }
        let keepSubtitle = restart ? selectedSubtitleId : nil
        title = record.item?.displayTitle ?? record.title
        subtitle = record.subtitle
        if let id = record.itemId { RecentsStore.record(source: record.sourceId, itemId: id, titleId: record.item?.titleId) }
        segments = record.segments ?? []
        // At launch the route may not be known yet: wait for it (it decides how to play).
        let route = ConnectionRouter.shared.route(to: record.sourceId)
        if record.isSegmented, record.state != .completed, route == nil || route == .checking { await ConnectionRouter.shared.ready() }
        let reachable = serverReachable(record.sourceId)
        var start = explicitStart ?? request.startAt ?? resumePosition(record)
        let url: URL
        if record.isSegmented {
            // Not complete and no server: the playlist ends where the download does, and grows.
            localGrowing = record.state != .completed && !reachable && !restart
            let edge = DownloadManager.shared.segmented.contiguousSeconds(record)
            if localGrowing, let s = start, s > edge - 5 {
                // Beyond what is on the device: start a little before its end.
                start = max(0, edge - 30)
                notice = "Offline: downloaded up to \(Fmt.clock(edge))"
            }
            guard let u = await LocalHLSServer.shared.url(for: record.id, growing: localGrowing) else {
                errorMessage = "Couldn't start local playback"; isBuffering = false; return
            }
            url = u
        } else {
            guard let u = record.localURL else { errorMessage = "File missing"; return }
            url = u
        }
        install(AVPlayerEngine(asset: AVURLAsset(url: url), start: start))
        playMethodLabel = "Offline · \(record.downloadLabel)" + (record.state == .completed ? "" : " · Downloading")
        #if DEBUG
        mark("local \(record.kind.rawValue) \(playMethodLabel) start \(Int(start ?? 0)) growing \(localGrowing) reachable \(reachable)")
        #endif
        subtitleOptions = record.subtitles.compactMap { s in
            DownloadRecord.home(s.file).map {
                SubtitleOption(id: "file:\(s.file)", title: s.title, language: s.language, kind: .external($0))
            }
        }
        // Subtitles are picked once its audio is (engineTracksChanged): forced ones follow it.
        if restart { selectSubtitle(subtitleOptions.first { $0.id == keepSubtitle }) }
        // The next episode: the server's list when it can be reached, else the downloaded ones.
        if let sid = record.item?.seriesId {
            let downloaded = DownloadManager.shared.episodes(ofSeries: sid).compactMap(\.item)
            seriesEpisodes = downloaded
            if reachable, let client {
                Task { if let all = try? await client.episodes(seriesId: sid, seasonId: nil), !all.isEmpty { seriesEpisodes = all } }
            }
        }
        // None saved, or saved while the server was still looking for the end credits.
        segmentsPending = record.segmentsPending == true
        segmentsAskedAt = -.infinity
        segmentsNearEndAsks = 0
        if reachable, segments.isEmpty || record.segmentsPending == true, let client, let item {
            Task { await loadSegments(item, client: client) }
        }
        if !restart { localReport("start") }
    }

    func watchStall(_ buffering: Bool) {
        stallTask?.cancel()
        guard buffering, local?.isSegmented == true else { return }
        stallTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(12))
                guard let self, !Task.isCancelled, self.isBuffering, let engine = self.engine, let rec = self.local else { return }
                let t = max(self.currentTime, self.seekTarget ?? 0)
                // Counted per place: a nudge toggles buffering, which restarts this task.
                if abs(t - self.stallAt) > 2 { self.stallAt = t; self.stallTries = 0 }
                self.stallTries += 1
                let tries = self.stallTries
                #if DEBUG
                self.mark(String(format: "local stall %d at %.1f", tries, t))
                #endif
                if tries < 4 {
                    engine.seek(to: t)
                } else if let fresh = DownloadManager.shared.record(rec.id) {
                    self.stallTries = 0
                    await self.startLocal(fresh, source: SourceStore.shared.source(fresh.sourceId), at: t, restart: true)
                    return
                }
            }
        }
    }

    /// Where to resume a download: this device's position when it has reports the server
    /// hasn't had yet, or with no server (the title's position is then as the download saved it),
    /// else the server's.
    private func resumePosition(_ record: DownloadRecord) -> Double? {
        let mine = ProgressStore.position(for: record.progressKey)
        if let id = record.itemId, LocalReports.shared.hasPending(item: id) { return mine }
        if !serverReachable(record.sourceId) { return mine ?? item?.resumeSeconds }
        if case let .server(_, item) = request.target { return item.resumeSeconds ?? mine }
        return mine
    }
}
