import Foundation

extension PlayerModel {
    /// The server's tracks (text ones from the Moonrise server's files when it lists them), then the
    /// tracks the Moonrise server fetched from OpenSubtitles, then ones added on this device.
    func buildServerSubtitleOptions(_ ms: MediaVersion) {
        guard let client, let item else { return }
        let existing = subtitleOptions.filter { $0.trackIndex == nil && !$0.id.hasPrefix("moonrise:") }
        let moonriseBase = client.moonriseBaseURL
        let moonrise = Dictionary((moonriseSubtitles ?? []).map { ($0.index, $0) }, uniquingKeysWith: { a, _ in a })
        let moonriseURL = { (index: Int) -> URL? in
            guard let base = moonriseBase, let path = moonrise[index]?.url else { return nil }
            return MoonriseServer.resolve(path, base: base)
        }
        let known = Set(ms.subtitleTracks.map(\.index))
        let fetched = (moonriseSubtitles ?? []).filter { $0.text && !known.contains($0.index) }.compactMap { t -> SubtitleOption? in
            guard let url = moonriseURL(t.index) else { return nil }
            return SubtitleOption(id: "moonrise:\(t.index)", title: t.name.replacingOccurrences(of: " - ", with: " · "),
                                  language: t.language, kind: .external(url))
        }
        subtitleOptions = ms.subtitleTracks.compactMap { s -> SubtitleOption? in
            TrackRules.fileSubtitleOption(s) { moonriseURL(s.index) ?? client.subtitleURL(itemId: mediaId, versionId: ms.id, index: s.index) }
        } + fetched + existing
        subtitleTimingRevision += 1
        #if DEBUG
        fputs("MoonriseSubs: \(subtitleOptions.count) options, \(moonriseSubtitles == nil ? "no server list" : "Moonrise server files (\(fetched.count) fetched)")\n", stderr)
        #endif
    }

    /// The Moonrise server fetched a subtitle from OpenSubtitles: list the title's tracks again and
    /// select it.
    /// - byUser: fetched from the search sheet (it becomes the subtitle preference).
    func moonriseSubtitleFetched(_ track: MoonriseServer.SubtitleTrack, byUser: Bool = false) async {
        guard let client, let item, let ms = mediaSource, let base = client.moonriseBaseURL else { return }
        moonriseSubtitles = await MoonriseServer.subtitles(base: base, itemId: mediaId, headers: client.headers) ?? moonriseSubtitles
        buildServerSubtitleOptions(ms)
        if let o = subtitleOptions.first(where: { $0.id == "moonrise:\(track.index)" || $0.id == "t:\(track.index)" }) {
            if byUser { chooseSubtitle(o) } else { selectSubtitle(o) }
        }
    }

    /// Shows the cue for `t`. Time ticks come every 0.25 s, so a line would appear or clear up
    /// to that late; when a cue edge falls before the next tick, refresh exactly at the edge.
    func updateSubtitle(at t: Double) {
        subtitleEdgeTask?.cancel()
        if pictureDoc != nil {
            updatePictures(at: t)
            return
        }
        guard let doc = subtitleDoc else { return }
        let local = t - subtitleDelay
        let text = doc.text(at: local)
        if text != subtitleText {
            subtitleText = text
            #if DEBUG
            if DebugSeams.launch.logOverlay {
                fputs(String(format: "MoonriseOverlay %.3f %@\n", t, (text ?? "-").replacingOccurrences(of: "\n", with: " / ")), stderr)
            }
            #endif
        }
        guard isPlaying, let edge = doc.nextEdge(after: local), edge - local < 0.3 else { return }
        subtitleEdgeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(edge - local + 0.005))
            guard !Task.isCancelled, let self, let now = self.engine?.currentTime else { return }
            self.updateSubtitle(at: now)
        }
    }

    /// The picture cues for `t` (the subtitle delay applies as to text), refreshed exactly at
    /// the next edge when it falls before the next tick.
    private func updatePictures(at t: Double) {
        guard let doc = pictureDoc else { return }
        if let s = engine?.presentationSize, s != videoSize { videoSize = s }
        let local = t - subtitleDelay
        let showing = doc.showing(at: local)
        if showing != pictureCues {
            pictureCues = showing
            #if DEBUG
            if DebugSeams.launch.logOverlay {
                fputs(String(format: "MoonriseOverlay %.3f pictures %@\n", t,
                             showing.map { String(format: "%.0f,%.0f %.0fx%.0f%@", $0.rect.minX, $0.rect.minY, $0.rect.width, $0.rect.height,
                                                    $0.image == nil ? " (no image)" : "") }
                                 .joined(separator: " / ")), stderr)
            }
            #endif
        }
        guard isPlaying, let edge = doc.nextEdge(after: local), edge - local < 0.3 else { return }
        subtitleEdgeTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(edge - local + 0.005))
            guard !Task.isCancelled, let self, let now = self.engine?.currentTime else { return }
            self.updateSubtitle(at: now)
        }
    }

    /// A picture track of the server's (`SubtitleOption.Kind.pictures`): its images, fetched window
    /// by window around the playhead, drawn over the video by `PictureSubtitleOverlay`.
    func startPictureWindows(_ option: SubtitleOption, index n: Int) {
        guard local == nil, let client, let base = client.moonriseBaseURL else {
            notice = "These subtitles can't be shown here"
            return
        }
        let id = option.id
        pictureDoc = PictureDocument(canvas: .zero, cues: [])
        let loader = SubtitleWindowLoader(
            base: base, itemId: mediaId, index: n, headers: client.headers, fullURL: nil,
            fetchFull: { _ in throw CancellationError() },
            onCues: { _ in },
            onPictures: { [weak self] canvas, cues in
                guard let self, self.selectedSubtitleId == id else { return }
                self.pictureDoc = PictureDocument(canvas: canvas, cues: cues)
                self.updateSubtitle(at: self.currentTime)
            },
            onFallback: { [weak self] in
                guard let self, self.selectedSubtitleId == id else { return }
                self.windowLoader = nil
                self.notice = "This server can't send these subtitles"
            },
            log: { [weak self] line in
                #if DEBUG
                self?.mark(line)
                #else
                _ = (self, line)
                #endif
            })
        windowLoader = loader
        loader.advance(to: max(currentTime, seekTarget ?? 0) - subtitleDelay)
    }

    func loadWholeSubtitle(_ url: URL) async {
        await loadSubtitleFile {
            if let early = self.subtitlePrefetch[url], let data = try? await early.value { return data }
            return try await self.fetch(url)
        }
    }

    /// The server's embedded track `option`, when the server serves windows: fetched window by
    /// window around the playhead (from the first second, and right after a seek) until the
    /// whole track is ready there. False: load the whole file instead.
    func startSubtitleWindows(_ option: SubtitleOption, url: URL) async -> Bool {
        guard local == nil, !url.isFileURL, let client, let item, let base = client.moonriseBaseURL,
              let n = option.trackIndex ?? serverTrackIndex(option.id), isEmbeddedServerTrack(n),
              await MoonriseServer.servesSubtitleWindows(base: base) else { return false }
        // Another track was picked meanwhile.
        guard selectedSubtitleId == option.id, windowLoader == nil else { return true }
        let id = option.id
        let loader = SubtitleWindowLoader(
            base: base, itemId: mediaId, index: n, headers: client.headers, fullURL: url,
            fetchFull: { [weak self] u in
                guard let self else { throw CancellationError() }
                return try await self.fetch(u)
            },
            onCues: { [weak self] cues in
                guard let self, self.selectedSubtitleId == id else { return }
                self.subtitleDoc = SubtitleDocument(cues: cues)
                self.updateSubtitle(at: self.currentTime)
            },
            onFallback: { [weak self] in
                guard let self, self.selectedSubtitleId == id else { return }
                self.windowLoader = nil
                Task { await self.loadWholeSubtitle(url) }
            },
            log: { [weak self] line in
                #if DEBUG
                self?.mark(line)
                #else
                _ = (self, line)
                #endif
            })
        windowLoader = loader
        loader.advance(to: max(currentTime, seekTarget ?? 0) - subtitleDelay)
        return true
    }

    /// The server's track `index` is one it extracts from the file (not a sidecar or fetched file).
    private func isEmbeddedServerTrack(_ index: Int) -> Bool {
        guard let t = moonriseSubtitles?.first(where: { $0.index == index }) else { return false }
        return t.text && (t.source == "embedded" || (t.source == nil && t.external != true))
    }

    /// The server's track `index` was fetched or uploaded: the server matches it to this file. The
    /// file's own tracks, embedded or sidecar, are taken as they are.
    func isDownloadedServerTrack(_ index: Int) -> Bool {
        guard let t = moonriseSubtitles?.first(where: { $0.index == index }) else { return false }
        return t.text && (t.source == "opensubtitles" || t.source == "upload")
    }

    /// The server pulls an embedded track out of the video on first request, reading the whole
    /// file (seconds on a NAS). Start that as soon as an item plays, so picking a track is instant.
    /// A server that serves subtitle windows extracts the played title by itself: its embedded
    /// tracks aren't asked for whole (that would hold the extraction urgent for every track).
    func prefetchSubtitles() {
        let base = local == nil ? client?.moonriseBaseURL : nil
        Task {
            var windows = false
            var matched = false
            if let base {
                windows = await MoonriseServer.servesSubtitleWindows(base: base)
                matched = await MoonriseServer.serves("subtitle-sync", base: base) ?? false
            }
            for o in subtitleOptions {
                guard case let .external(url) = o.kind, !url.isFileURL, subtitlePrefetch[url] == nil else { continue }
                let n = o.trackIndex ?? serverTrackIndex(o.id)
                if windows, let n, isEmbeddedServerTrack(n) { continue }
                // Matched to the file when first asked for: not every track of every title played,
                // and what is kept here could be from before the match.
                if matched, let n, isDownloadedServerTrack(n) { continue }
                subtitlePrefetch[url] = Task { try await self.fetch(url) }
            }
        }
    }

    func fetch(_ url: URL) async throws -> Data {
        if url.isFileURL { return try Data(contentsOf: url) }
        if let client { return try await client.fetch(url) }
        return try await HTTP.data(from: url).0
    }

    private func loadSubtitleFile(_ load: @escaping () async throws -> Data) async {
        let requested = selectedSubtitleId
        do {
            let data = try await load()
            guard selectedSubtitleId == requested else { return } // user picked another track meanwhile
            let id = selectedSubtitleId
            let ext = id.flatMap { ($0 as NSString).pathExtension }
            subtitleDoc = SubtitleParser.parse(data, ext: ext)
            updateSubtitle(at: currentTime)
        } catch {
            if selectedSubtitleId == requested { notice = "Couldn't load subtitles: \(error.localizedDescription)" }
        }
    }

    /// Adds a subtitle downloaded from OpenSubtitles and selects it.
    func addDownloadedSubtitle(_ data: Data, title: String, language: String?) {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".srt")
        try? data.write(to: file)
        let option = SubtitleOption(id: "os:\(file.lastPathComponent)", title: title, language: language, kind: .external(file))
        subtitleOptions.append(option)
        chooseSubtitle(option)
        DownloadManager.shared.attachSubtitle(data, title: title, language: language, to: request)
    }

    /// A title starting: the subtitles picked last in it (or Off), or like those picked last in
    /// the show (`TrackRules.rememberedSubtitle`); else subtitles in the preferred language when
    /// they are on, else forced subtitles for foreign dialogue in the audio's language
    /// (`TrackRules.autoSubtitle`).
    func autoSelectSubtitle() {
        let s = AppSettings.shared
        var on = s.autoSelectSubtitles
        switch TrackRules.rememberedSubtitle(subtitleOptions, memory: trackMemory) {
        case let .track(o):
            selectSubtitle(o)
            return
        case .off:
            return
        case .usualOff:
            on = false
        case .usual:
            break
        }
        if let o = TrackRules.autoSubtitle(subtitleOptions, autoSelect: on, language: s.preferredSubtitleLanguage,
                                           preferSDH: s.preferSDHSubtitles, audioLanguage: selectedAudio.flatMap { audioLanguage($0) }) {
            selectSubtitle(o)
        }
    }

    // MARK: - Subtitle search context

    /// "Search Online…" is offered: the server searches (for a download, the server it came from,
    /// when it can be reached: with no server it isn't offered).
    var canSearchSubtitles: Bool {
        if let local, !serverReachable(local.sourceId) { return false }
        let ctx = searchContext
        return (ctx.server != nil || ctx.download != nil) && serverSubtitleSearch
    }

    var searchContext: SubtitleSearchContext {
        // A download playing: searched for through its server, and what's picked is kept with it.
        if let r = local {
            let download = r.itemId.flatMap { id in
                SourceStore.shared.source(r.sourceId).map { ($0, r.mediaSourceId.map { "\(id)~\($0)" } ?? id) }
            }
            return SubtitleSearchContext(title: r.title, download: download)
        }
        switch request.target {
        case let .server(source, item):
            return SubtitleSearchContext(title: item.seriesName ?? item.displayTitle, server: (source, item), imdbId: item.providerId("Imdb"),
                                         tmdbId: item.providerId("Tmdb"), season: item.seasonNumber, episode: item.episodeNumber)
        }
    }
}
