import AVFoundation
import Foundation

extension PlayerModel {
    private var selectedTrackIndex: Int? {
        subtitleOptions.first { $0.id == selectedSubtitleId }?.trackIndex
    }

    /// Tell the native engine which rendition mirrors the overlay track, for PiP and AirPlay.
    func updatePiPSubtitle() {
        guard let av = engine as? AVPlayerEngine else { return }
        if let id = selectedSubtitleId, let o = subtitleOptions.first(where: { $0.id == id }), TrackRules.isPicture(o) {
            // Pictures are drawn by the overlay: the stream has no rendition of them.
            av.nativeSubtitle = nil
        } else if let id = selectedSubtitleId, id.hasPrefix("file:"), let rec = local,
           let s = rec.subtitles.first(where: { "file:\($0.file)" == id }), let name = s.rendition {
            // A download's subtitle: its rendition in the local playlist.
            av.nativeSubtitle = (name, s.language)
        } else if let id = selectedSubtitleId, id.hasPrefix("moonrise:"), let n = Int(id.dropFirst("moonrise:".count)),
           let t = moonriseSubtitles?.first(where: { $0.index == n }) {
            // A track fetched from OpenSubtitles: a rendition of the Moonrise server's stream.
            av.nativeSubtitle = (t.name, t.language)
        } else if let idx = selectedTrackIndex, let t = moonriseSubtitles?.first(where: { $0.index == idx }) {
            // The server names its renditions as it lists its tracks.
            av.nativeSubtitle = (t.name, t.language)
        } else if let idx = selectedTrackIndex, let stream = mediaSource?.subtitleTracks.first(where: { $0.index == idx }) {
            av.nativeSubtitle = (stream.displayTitle, stream.language)
        } else if let id = selectedSubtitleId, let o = subtitleOptions.first(where: { $0.id == id }), case .engine = o.kind {
            // A file's own track: the engine shows it natively everywhere.
            av.nativeSubtitle = (o.title, o.language)
        } else {
            av.nativeSubtitle = nil
        }
    }

    // MARK: - AirPlay

    #if DEBUG
    func runExternalPlaybackScript(_ script: String) {
        let began = Date()
        Task { [weak self] in
            for step in script.split(separator: ",") {
                let parts = step.split(separator: ":", maxSplits: 1).map(String.init)
                guard parts.count == 2, let at = Double(parts[0]) else { continue }
                let wait = at - Date().timeIntervalSince(began)
                if wait > 0 { try? await Task.sleep(for: .seconds(wait)) }
                guard let self, !self.closed else { return }
                let action = parts[1]
                self.mark("extplay step \(action)")
                if action == "on" || action == "off" {
                    // As AVPlayer's isExternalPlaybackActive would change.
                    if let av = self.engine as? AVPlayerEngine { av.debugSetExternal(action == "on") }
                } else if action.hasPrefix("devsub="), let o = self.subtitleOptions.first(where: { $0.id == String(action.dropFirst(7)) }),
                          case let .external(url) = o.kind, let data = try? await self.fetch(url) {
                    // A copy of a track as a file only this device has (as an OpenSubtitles download
                    // made with the app's own key).
                    let file = FileManager.default.temporaryDirectory.appendingPathComponent("extplay-\(UUID().uuidString.prefix(8)).srt")
                    try? data.write(to: file)
                    let copy = SubtitleOption(id: "os:\(file.lastPathComponent)", title: "Device copy", language: o.language, kind: .external(file))
                    self.subtitleOptions.append(copy)
                    self.selectSubtitle(copy)
                    self.mark("extplay subtitle \(copy.id) overlay \(self.subtitlesOnReceiver ? "hidden" : "shown")")
                } else if action.hasPrefix("sub=") {
                    // "sub=idx:4": the server's track 4, whichever option carries it (a download's file too).
                    let id = String(action.dropFirst(4))
                    let n = id.hasPrefix("idx:") ? Int(id.dropFirst(4)) : nil
                    self.selectSubtitle(self.subtitleOptions.first { o in
                        guard let n else { return o.id == id }
                        return self.serverTrackIndex(o.id) == n || (o.id.hasPrefix("file:") && SegmentedDownloads.trackIndex(fromFile: String(o.id.dropFirst(5))) == n)
                    })
                    self.mark("extplay subtitle \(self.selectedSubtitleId ?? "none") overlay \(self.subtitlesOnReceiver ? "hidden" : "shown")")
                }
            }
        }
    }
    #endif

    /// AirPlay started or ended. The receiver can't show the overlay, so the selected subtitle goes
    /// there as the stream's own rendition (a download plays the server's stream meanwhile).
    func externalPlaybackChanged(_ on: Bool) {
        guard on != isExternalPlayback else { return }
        isExternalPlayback = on
        airPlayFallbacks = []
        #if DEBUG
        mark("external playback \(on ? "on" : "off")")
        #endif
        if on, let rec = local {
            Task { await streamForAirPlay(rec) }
        } else if !on, let rec = airPlayDownload {
            Task { await backToDownload(rec) }
        }
        updatePiPSubtitle()
        syncExternalSubtitles()
    }

    /// A restarted stream on a new player: AirPlay normally carries over. If it didn't (the
    /// receiver went away meanwhile), say so after a while.
    func checkExternalAfterRestart() {
        externalCheck?.cancel()
        guard isExternalPlayback else { return }
        externalCheck = Task { [weak self] in
            try? await Task.sleep(for: .seconds(10))
            guard !Task.isCancelled, let self, self.isExternalPlayback,
                  let av = self.engine as? AVPlayerEngine, !av.isExternalPlaybackActive else { return }
            self.externalPlaybackChanged(false)
        }
    }

    /// While AirPlaying: hide the overlay when the receiver shows the subtitle, else make it able to
    /// (restart the stream so its playlist lists a track added since, or send a file only this
    /// device has to the server first).
    func syncExternalSubtitles() {
        guard let av = engine as? AVPlayerEngine else { subtitlesOnReceiver = false; return }
        av.externalActive = isExternalPlayback
        guard isExternalPlayback, let id = selectedSubtitleId,
              let option = subtitleOptions.first(where: { $0.id == id }) else {
            subtitlesOnReceiver = false
            return
        }
        switch option.kind {
        case .pictures:
            // Images drawn here: the receiver has no rendition of them, so they show on this
            // device's screen.
            subtitlesOnReceiver = false
            if !airPlayFallbacks.contains(option.id) {
                airPlayFallbacks.insert(option.id)
                notice = "Picture subtitles show on this device, not on the AirPlay screen"
            }
            return
        case .engine, .external:
            break
        }
        // The stream's renditions aren't known yet: the overlay stays until they are.
        guard let names = av.nativeSubtitleNames else { subtitlesOnReceiver = false; return }
        if let selected = av.nativeSelection, renditionMatches(option, selected: selected, names: names) {
            subtitlesOnReceiver = true
            #if DEBUG
            mark("airplay subtitle \(option.id) -> rendition \(selected)")
            #endif
            return
        }
        subtitlesOnReceiver = false
        airPlayFallback(option)
    }

    /// A server track must be its own rendition (a same-language one isn't the same subtitle).
    private func renditionMatches(_ option: SubtitleOption, selected: String, names: [String]) -> Bool {
        if case .engine = option.kind { return true }
        if let n = serverTrackIndex(option.id), let t = moonriseSubtitles?.first(where: { $0.index == n }), local == nil {
            return names.contains { $0.caseInsensitiveCompare(t.name) == .orderedSame }
        }
        return true
    }

    /// "t:12", "moonrise:10001" → the server's track index.
    func serverTrackIndex(_ id: String) -> Int? {
        if id.hasPrefix("t:") { return Int(id.dropFirst(2)) }
        if id.hasPrefix("moonrise:") { return Int(id.dropFirst("moonrise:".count)) }
        return nil
    }

    private func airPlayFallback(_ option: SubtitleOption) {
        guard !airPlayFallbacks.contains(option.id) else { return }
        airPlayFallbacks.insert(option.id)
        guard local == nil, let client, let item, let base = client.moonriseBaseURL else {
            notice = "These subtitles can't be shown on the AirPlay screen"
            return
        }
        if serverTrackIndex(option.id) != nil {
            // A server track the stream didn't list when it started (fetched since): restart it.
            #if DEBUG
            mark("airplay subtitle \(option.id): not in the stream, restarting it")
            #endif
            reloadKeepingPosition()
        } else if case let .external(url) = option.kind, url.isFileURL {
            // A file only this device has: the server keeps it as one of the title's tracks.
            #if DEBUG
            mark("airplay subtitle \(option.id): sending it to the server")
            #endif
            let headers = client.headers
            let itemId = item.id
            Task {
                do {
                    let data = try Data(contentsOf: url)
                    let track = try await MoonriseServer.uploadSubtitle(base: base, itemId: itemId, data: data, name: option.title,
                                                                    language: option.language, headers: headers)
                    guard self.selectedSubtitleId == option.id, self.isExternalPlayback else { return }
                    await self.moonriseSubtitleFetched(track)
                } catch {
                    self.notice = "These subtitles can't be shown on the AirPlay screen: \(error.localizedDescription)"
                }
            }
        } else {
            notice = "These subtitles can't be shown on the AirPlay screen"
        }
    }

    /// AirPlay while a download plays: the receiver can't reach this device's loopback server
    /// and the download's subtitle files aren't renditions it can show, so the server's stream
    /// plays for the AirPlay session (when the server can be reached).
    private func streamForAirPlay(_ rec: DownloadRecord) async {
        guard client != nil, let recItem = rec.item ?? item, serverReachable(rec.sourceId) else {
            notice = "The server can't be reached: subtitles stay on this device"
            syncExternalSubtitles()
            return
        }
        let t = max(currentTime, seekTarget ?? 0)
        let sub = selectedSubtitleId
        let localSub = sub.flatMap { id in rec.subtitles.first { "file:\($0.file)" == id } }
        #if DEBUG
        mark(String(format: "airplay: the server's stream instead of the download, at %.1f", t))
        #endif
        localReport("stop")
        stallTask?.cancel()
        airPlayDownload = rec
        local = nil
        localGrowing = false
        item = recItem
        audioOptions = []
        selectedAudio = nil
        subtitleOptions = []
        applySubtitle(nil)
        await loadServer(at: t)
        guard airPlayDownload?.id == rec.id else { return }
        // The same subtitle: the server's track the file came from, else the file itself.
        if let s = localSub, let n = SegmentedDownloads.trackIndex(fromFile: s.file),
           let o = subtitleOptions.first(where: { $0.id == "t:\(n)" || $0.id == "moonrise:\(n)" }) {
            selectSubtitle(o)
        } else if let s = localSub, let url = DownloadRecord.home(s.file) {
            let o = SubtitleOption(id: "file:\(s.file)", title: s.title, language: s.language, kind: .external(url))
            subtitleOptions.append(o)
            selectSubtitle(o)
        } else if sub == nil {
            selectSubtitle(nil)
        }
    }

    /// AirPlay ended: back to the download, where the server's stream got to.
    private func backToDownload(_ rec: DownloadRecord) async {
        airPlayDownload = nil
        guard let fresh = DownloadManager.shared.record(rec.id), fresh.isPlayable else { return }
        let t = max(currentTime, seekTarget ?? 0)
        let sub = selectedSubtitleId
        #if DEBUG
        mark(String(format: "airplay ended: the download again, at %.1f", t))
        #endif
        if let client, item != nil, reportingEnabled {
            let r = report(paused: true)
            await client.report(.stopped, r)
        }
        if let client, let id = playSessionId { await client.stop(sessionId: id) }
        playSessionId = nil
        if let master = moonriseMaster {
            moonriseMaster = nil
            await MoonriseServer.close(master: master)
        }
        guard !closed else { return }
        mediaSource = nil
        moonriseAudio = []
        moonriseConverted = []
        moonriseVariant = nil
        audioOptions = []
        selectedAudio = nil
        subtitleOptions = []
        applySubtitle(nil)
        await startLocal(fresh, source: SourceStore.shared.source(fresh.sourceId), at: t)
        localSubtitlePending = false // the subtitle from the stream, below
        if let n = sub.flatMap(serverTrackIndex),
           let s = fresh.subtitles.first(where: { SegmentedDownloads.trackIndex(fromFile: $0.file) == n }),
           let o = subtitleOptions.first(where: { $0.id == "file:\(s.file)" }) {
            selectSubtitle(o)
        } else if let sub, let o = subtitleOptions.first(where: { $0.id == sub }) {
            selectSubtitle(o)
        } else if sub == nil {
            selectSubtitle(nil)
        }
    }
}
