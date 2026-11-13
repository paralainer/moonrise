import AVFoundation
import Foundation

extension PlayerModel {
    // MARK: - Media server

    /// `inPlace`: a restart the viewer asked for (quality, version): on the Apple TV the new stream
    /// goes into the same AVPlayer (a new player set on the system player while its menu was up
    /// showed no picture).
    func loadServer(at start: Double, keepSubtitle: Bool = false, inPlace: Bool = false) async {
        do {
            try await openSession(at: start, keepSubtitle: keepSubtitle, inPlace: inPlace)
        } catch {
            var p = PlaybackProblem(error)
            // The title page knew why (a damaged file, or none found): its words, with its details.
            if p.kind == .missing || p.kind == .damaged, let known = item?.unplayableProblem { p = known }
            show(p)
            isBuffering = false
        }
    }

    /// Playback can't start: the problem's title and line, its server text behind Details.
    func show(_ p: PlaybackProblem) {
        errorMessage = p.message
        errorTitle = p.title
        errorDetail = p.detail
    }

    /// Asks the server for a session at `start` and plays it.
    /// - reuse: recovery: the new stream goes into the same player (PiP and AirPlay carry on),
    ///   paused or playing as before; the lost session is ended without waiting on its old address.
    func openSession(at start: Double, keepSubtitle: Bool = false, reuse: Bool = false, inPlace: Bool = false) async throws {
        guard let client, let item else { return }
        #if os(tvOS)
        let swap = inPlace && !reuse && engine is AVPlayerEngine
        #else
        let swap = false
        #endif
        // Resume or restart (language/quality/subtitle change): ignore the 0:00 the new stream
        // reports before it reaches the position (it would flash "Skip Intro" and the scrubber).
        if start > 1 { seekTarget = start; seekStarted = Date(); seekSlack = 1.5; currentTime = start }
        windowLoader?.advance(to: start - subtitleDelay)
        // A restart: stop the previous server jobs before asking for new ones (a Moonrise server
        // transcode holds one of its few GPU slots).
        if reuse {
            // Its address may be the one that went away: end it on the current route.
            moonriseMaster = nil
            engine?.pause()
            if let previous = playSessionId { await client.stop(sessionId: previous) }
            playSessionId = nil
        }
        if let master = moonriseMaster {
            moonriseMaster = nil
            if swap { engine?.pause() } else { engine?.stop() }
            await MoonriseServer.close(master: master)
        }
        if let previous = playSessionId {
            if swap { engine?.pause() } else { engine?.stop() }
            await client.stop(sessionId: previous)
        }
        isBuffering = true
        errorMessage = nil
        lastStreamError = nil
        // The title's subtitle files, as the server lists them.
        let subsBase = keepSubtitle ? nil : client.moonriseBaseURL
        let headers = client.headers
        let subsId = wantedVersion.map { "\(item.id)~\($0)" } ?? mediaId
        async let moonriseSubs: [MoonriseServer.SubtitleTrack]? = {
            guard let subsBase else { return nil }
            return await MoonriseServer.subtitles(base: subsBase, itemId: subsId, headers: headers)
        }()
        do {
            let plan = try await client.playback(PlaybackOptions(
                itemId: item.id, versionId: wantedVersion ?? mediaSource?.id, start: start, maxBitrate: quality.bitrate,
                audioIndex: selectedAudio, isRetry: reuse))
            #if DEBUG
            mark("playbackInfo")
            #endif
            // Closed while the server answered (the title page was left): end the new session.
            if closed {
                if let sid = plan.sessionId { await client.stop(sessionId: sid) }
                await MoonriseServer.close(master: plan.master)
                return
            }
            let ms = plan.version
            mediaSource = ms
            wantedVersion = ms.id
            if !plan.versions.isEmpty { versions = plan.versions }
            playSessionId = plan.sessionId
            #if !os(tvOS)
            loadThumbnails(itemId: wantedVersion.map { "\(item.id)~\($0)" } ?? item.id, sessionId: plan.sessionId)
            #endif

            moonriseAudio = plan.offer.audio.map(\.index)
            if !keepSubtitle || audioOptions.isEmpty { trackMemory = plan.offer.tracks }
            if audioOptions.isEmpty {
                let details = Self.distinctDetails(ms.audioTracks.map { ($0.audioMenuName, $0.audioDetail) })
                audioOptions = zip(ms.audioTracks, details).map { AudioOption(id: $0.index, title: $0.audioMenuName, detail: $1) }
                // A new title: the track picked last in it (or in the show), else the language
                // picked last, when its stream has it.
                let offered = ms.audioTracks.filter { moonriseAudio.isEmpty || moonriseAudio.contains($0.index) }
                let candidates = offered.map {
                    AudioCandidate(id: $0.index, language: $0.language, isDefault: $0.index == ms.defaultAudioIndex || $0.isDefault == true,
                                   name: [$0.title, $0.displayTitle].compactMap { $0 }.joined(separator: " "))
                }
                selectedAudio = selectedAudio ?? TrackRules.rememberedAudio(candidates, memory: trackMemory)
                    ?? Self.preferredAudio(candidates) ?? ms.defaultAudioIndex
            }
            let wantedAudio = selectedAudio ?? ms.defaultAudioIndex
            moonriseConverted = Set(plan.offer.audio.filter { $0.converted == true }.map(\.index))
            moonriseMaster = plan.master
            streamRoute = ConnectionRouter.shared.route
            moonriseVariant = plan.offer.served
            #if DEBUG
            mark("moonrise server \(moonriseVariant?.id ?? "copy")")
            #endif
            playMethod = moonriseVariant?.transcode == true ? .transcode : .directStream
            playMethodLabel = moonriseLabel(audio: wantedAudio)
            // Its playlist already starts at `start` (EXT-X-START), exactly.
            itemReady = false
            if reuse, let av = engine as? AVPlayerEngine {
                awaitingNewItem = true
                recoveryOpenedAt = start
                av.replace(asset: client.streamAsset(plan.master), start: nil)
                if recoverPlaying { av.play() }
            } else if swap, let av = engine as? AVPlayerEngine {
                av.replace(asset: client.streamAsset(plan.master), start: nil)
                av.play()
            } else {
                install(AVPlayerEngine(asset: client.streamAsset(plan.master), start: nil))
            }
            updatePiPSubtitle()

            if !keepSubtitle {
                moonriseSubtitles = await moonriseSubs
                buildServerSubtitleOptions(ms)
                prefetchSubtitles()
                autoSelectSubtitle()
            }
            tracksSettled = true
            if reportingEnabled { await client.report(.start, report(paused: false)) }
        }
    }

    #if !os(tvOS)
    /// The thumbnails of the file the new session plays (kept when it is the same file).
    private func loadThumbnails(itemId: String, sessionId: String?) {
        guard let base = client?.moonriseBaseURL, let headers = client?.headers else { return }
        Task { [weak self] in
            let t = await ScrubThumbnails.fetch(base: base, itemId: itemId, sessionId: sessionId, headers: headers)
            guard let self, !self.closed else { return }
            if t?.sheets != self.thumbnails?.sheets { self.thumbnails = t }
        }
    }
    #endif

    /// The Moonrise server copies the video (the audio is copied too, or converted), or transcodes
    /// it: a lower quality, a codec Apple devices can't play, HDR tone-mapped.
    func moonriseLabel(audio: Int?) -> String {
        if client is DemoClient { return "Direct · Demo stream" }
        if let v = moonriseVariant, v.transcode {
            var parts = ["Transcode"]
            if v.label != "Original" { parts.append(v.label) }
            return (parts + ["Moonrise Server"]).joined(separator: " · ")
        }
        return audio.map { moonriseConverted.contains($0) } == true ? "Remux + Audio · Moonrise Server" : "Remux · Moonrise Server"
    }

    func setQuality(_ q: QualityOption) {
        guard q != quality, local == nil else { return }
        quality = q
        AppSettings.shared.streamingQuality = q.label
        // Recovering: the new session is asked for at this quality.
        guard recovery == .none else { return }
        let t = currentTime
        Task { await loadServer(at: t, keepSubtitle: true, inPlace: true) }
    }

    /// The user picked another version of the title (a menu): it plays from the same position,
    /// with its own tracks (languages and subtitles as preferred), and is this title's version
    /// from now on (remembered by the server, on every device).
    func chooseVersion(_ id: String) {
        guard id != mediaSource?.id, local == nil, let item, versions.contains(where: { $0.id == id }) else { return }
        wantedVersion = id
        VersionChoices.shared.set(id, for: item.id)
        if let c = client as? MoonriseClient { Task { await c.chooseVersion(itemId: item.id, fileId: id) } }
        audioOptions = []
        selectedAudio = nil
        guard recovery == .none else { return }
        let t = currentTime
        Task { await loadServer(at: t, inPlace: true) }
    }

    /// Restart the stream where it is (a new session's playlist also lists tracks added since).
    func reloadKeepingPosition() {
        // Recovering: the new session carries the change (quality, tracks added).
        guard recovery == .none else { return }
        let t = currentTime
        Task { await loadServer(at: t, keepSubtitle: true, inPlace: true) }
    }
}
