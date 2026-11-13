import AVFoundation
import Foundation

extension PlayerModel {
    // MARK: - Engine wiring

    func install(_ new: PlayerEngine) {
        // Closed while the server was still answering (the page was left): don't start playing.
        guard !closed else { new.stop(); return }
        engine?.stop()
        engine = new
        engineGeneration += 1
        new.setFill(fill)
        new.setRate(rate)
        new.setVolume(playerVolume)
        new.onTime = { [weak self] t in self?.timeUpdated(t) }
        new.onPlayingChanged = { [weak self, weak new] p in
            guard let self else { return }
            // Given up, and played from outside our controls (the Apple TV's system player takes
            // Select/Play, PiP's play button): that's Try Again.
            if p, case .failed = self.recovery {
                new?.pause()
                self.retryStream()
                return
            }
            // Recovering: the old item is paused on purpose; the controls show what resumes.
            guard self.recovery == .none else { return }
            let av = new as? AVPlayerEngine
            // Paused by AVPlayer itself, out of data: still "playing" (a spinner shows; it goes on
            // when data comes, or recovery takes over).
            if !p, self.playIntent, !self.isFinished,
               Date().timeIntervalSince(self.stalledAt) < 2 || av?.isBufferEmpty == true || av?.hasFailed == true {
                self.isBuffering = true
                return
            }
            self.isPlaying = p
            if p != self.playIntent {
                self.playIntent = p
                self.intentChangedAt = Date()
            }
            self.nowPlaying?.update()
        }
        new.onBuffering = { [weak self] b in
            guard let self else { return }
            self.isBuffering = b || self.recovery == .recovering
            self.watchStall(b)
        }
        new.onStatus = { [weak self] s in self?.statusChanged(s) }
        new.onTracksChanged = { [weak self] in self?.engineTracksChanged() }
        if let av = new as? AVPlayerEngine {
            av.onPictureInPicture = { [weak self] on in
                self?.isInPictureInPicture = on
                if !on { self?.onPictureInPictureEnded?() }
            }
            av.onExternalPlayback = { [weak self] on in self?.externalPlaybackChanged(on) }
            av.onErrorLogEntry = { [weak self] status, uri in self?.streamError(status: status, uri: uri) }
            av.onAudioSelectionChanged = { [weak self] i in self?.engineAudioSelected(i) }
            av.onStalled = { [weak self, weak av] in
                guard let self else { return }
                self.stalledAt = Date()
                #if DEBUG
                self.mark(String(format: "stalled at %.1f", self.currentTime))
                #endif
                // Out of data: is the session still there? (A restarted server's isn't.)
                if self.streamsFromServer, self.recovery == .none { self.checkSession("stalled", stuck: true) }
                // Go on by itself once data comes again (else it stays paused).
                guard self.playIntent, self.recovery == .none else { return }
                self.isBuffering = true
                Task { @MainActor [weak self, weak av] in
                    try? await Task.sleep(for: .milliseconds(300))
                    guard let self, let av, self.engine === av, self.playIntent, self.recovery == .none else { return }
                    av.resumeWhenBuffered()
                }
            }
            // A restarted stream (quality, AirPlay's own fallbacks) keeps AirPlay's state.
            av.externalActive = isExternalPlayback
        }
        if nowPlaying == nil { nowPlaying = NowPlayingController(model: self) }
        itemReady = false
        stuckSince = nil
        startWatchdog()
        new.play()
    }

    func statusChanged(_ s: EngineStatus) {
        #if DEBUG
        mark("engine \(s)")
        // Debug seams, a few seconds after the first start, as the menus would:
        // MOONRISE_AUTOPLAY_AUDIO=<track index> switches the audio track,
        // MOONRISE_AUTOPLAY_QUALITY=<bits/s> picks that quality preset (0: Original),
        // MOONRISE_AUTOPLAY_SEEK=<seconds> then seeks there (once the restart, if any, is playing).
        if case .ready = s, !debugAudioApplied {
            debugAudioApplied = true
            let seams = DebugSeams.launch
            let audio = seams.autoplayAudio
            let q = seams.autoplayQuality
            let seekTo = seams.autoplaySeek
            // MOONRISE_AUTOPLAY_SUB=<option id> ("t:4", "moonrise:10000", "none") picks that subtitle as the
            // menu does (MOONRISE_AUTOPLAY_AUDIO too: they set the preferences);
            // MOONRISE_AUTOPLAY_SUBSEARCH=<language> searches OpenSubtitles through the Moonrise server
            // and fetches the first result (as the search sheet does).
            // MOONRISE_EXTPLAY="8:on,16:sub=t:3,24:sub=none,30:off": AirPlay as the simulator can't,
            // through the engine's external-playback path, and subtitle picks meanwhile (seconds after
            // this first start).
            if let script = seams.extPlay { runExternalPlaybackScript(script) }
            // MOONRISE_AUTOPLAY_VERSION=<file id>: switches to that version 6 s after the first start,
            // as the Version menu does (it is remembered as the title's version).
            if let v = seams.autoplayVersion {
                Task { [weak self] in
                    try? await Task.sleep(for: .seconds(6))
                    guard let self else { return }
                    self.mark("select version \(v) at \(Int(self.currentTime))")
                    self.chooseVersion(v)
                }
            }
            let sub = seams.autoplaySub
            let subSearch = seams.autoplaySubSearch
            if sub != nil || subSearch != nil {
                Task { [weak self] in
                    try? await Task.sleep(for: .seconds(3))
                    guard let self else { return }
                    if sub == "none" {
                        self.mark("select subtitle off")
                        self.chooseSubtitle(nil)
                    } else if let sub, let o = self.subtitleOptions.first(where: { $0.id == sub }) {
                        self.mark("select subtitle \(o.id) \(o.title)")
                        self.chooseSubtitle(o)
                    }
                    if let lang = subSearch, let client = self.client, let item = self.item, let base = client.moonriseBaseURL {
                        do {
                            let a = try await MoonriseServer.searchSubtitles(base: base, itemId: mediaId, language: lang, query: nil, headers: client.headers)
                            self.mark("subtitle search \(a.results.count) results, first \(a.results.first?.name ?? "-")")
                            if let r = a.results.first {
                                let t = try await MoonriseServer.fetchSubtitle(base: base, itemId: mediaId, result: r, headers: client.headers)
                                self.mark("subtitle fetched: track \(t.index) \(t.name)")
                                await self.moonriseSubtitleFetched(t)
                                self.mark("selected \(self.selectedSubtitleId ?? "-")")
                            }
                        } catch {
                            self.mark("subtitle search failed: \(error.localizedDescription)")
                        }
                    }
                }
            }
            if audio != nil || q != nil || seekTo != nil {
                Task { [weak self] in
                    try? await Task.sleep(for: .seconds(4))
                    guard let self else { return }
                    if let audio { self.mark("select audio \(audio)"); self.chooseAudio(audio) }
                    if let q, let preset = q == 0 ? QualityOption.original : QualityOption.presets.first(where: { $0.bitrate == q }) {
                        self.mark("select quality \(preset.label)")
                        self.setQuality(preset)
                    }
                    if let seekTo {
                        try? await Task.sleep(for: .seconds(q != nil ? 10 : 4))
                        self.mark("seek to \(Int(seekTo))")
                        self.debugSeekTarget = seekTo
                        self.seek(to: seekTo)
                    }
                }
            }
        }
        #endif
        // A download playing from the device failed (its server went away while a segment
        // was missing): start again where it was, from what is on the device.
        if case .failed = s, let rec = local, rec.isSegmented, !localRetried, let fresh = DownloadManager.shared.record(rec.id) {
            localRetried = true
            let t = max(currentTime, seekTarget ?? 0)
            #if DEBUG
            mark("local playback failed; again at \(Int(t))")
            #endif
            Task { await startLocal(fresh, source: SourceStore.shared.source(fresh.sourceId), at: t, restart: true) }
            return
        }
        // The server's stream failed: a new session, where it was (see "Stream recovery").
        if case let .failed(msg) = s, streamsFromServer {
            switch recovery {
            case .recovering:
                // The new session's stream failed too (the old item's failure doesn't count).
                guard awaitingNewItem else { return }
                awaitingNewItem = false
                recovery = .none
                recover("the new stream failed: \(msg)", failure: msg)
            case .failed:
                break
            case .none:
                recover("the stream failed: \(msg)", failure: msg)
            }
            return
        }
        switch s {
        case .ready:
            itemReady = true
            readyAt = Date()
            // An item getting ready can fall back to its default audio rendition: Moonrise's again.
            reassertAudio()
            everReady = true
            if recovery == .recovering, awaitingNewItem { finishRecovery() }
            isBuffering = false
            duration = engine?.duration ?? duration
            nowPlaying?.update()
            checkExternalAfterRestart()
        case .ended:
            isFinished = true
            isPlaying = false
            isBuffering = false
            saveProgress(final: true)
            // Watched to the end (credits not skipped): go on to the next episode.
            if AppSettings.shared.autoplayNext, let next = nextEpisode { playEpisode(next) }
        case let .failed(msg):
            // A download from the device (server streams recover above).
            errorMessage = msg
            isBuffering = false
        case .loading: isBuffering = true
        }
    }

    private func timeUpdated(_ t: Double) {
        #if DEBUG
        // Past the target: playing there (the engine reports the target while it buffers).
        if let target = debugSeekTarget, t > target + 0.5, t < target + 5, isPlaying {
            debugSeekTarget = nil
            mark(String(format: "playing at %.1f", t))
        }
        #endif
        // Recovering: the old item's clock means nothing (the scrubber shows where it resumes).
        if recovery != .none { return }
        if t > 0.5 { lastPosition = t }
        if let target = seekTarget {
            if (t > target - seekSlack && t < target + 1.5) || Date().timeIntervalSince(seekStarted) > 30 {
                seekTarget = nil
                seekSlack = 1.5
            } else {
                return
            }
        }
        currentTime = t
        if let d = engine?.duration, d > 0 { duration = d }
        #if DEBUG
        // Local playback: where it is, every 5 s (watch-while-downloading checks).
        if local != nil, Date().timeIntervalSince(lastLocalLog) >= 5 {
            lastLocalLog = Date()
            mark(String(format: "local at %.1f %@", t, isPlaying ? (isBuffering ? "buffering" : "playing") : "paused"))
        }
        #endif
        #if DEBUG
        // MOONRISE_AUTOPLAY_PAUSE=<seconds>: pauses once there (screenshots of what shows then).
        if !debugPaused, let p = DebugSeams.launch.autoplayPause, t >= p, isPlaying {
            debugPaused = true
            mark(String(format: "pause at %.1f", t))
            togglePlay()
        }
        #endif
        updateSubtitle(at: t)
        windowLoader?.advance(to: t - subtitleDelay)
        updateSegment(t)
        refetchSegmentsNearEnd(t)
        #if DEBUG
        if nextEpisodeOffered != loggedNextOffer {
            loggedNextOffer = nextEpisodeOffered
            fputs(String(format: "MoonriseNext: offered=%@ at %.1f of %.1f (credits %@)\n", loggedNextOffer ? "yes" : "no", t, duration,
                         segments.filter { $0.kind == .credits }.map { String(format: "%.1f-%.1f%@", $0.start, $0.end, $0.source.map { " " + $0 } ?? "") }.joined(separator: ", ")), stderr)
        }
        #endif
        if Date().timeIntervalSince(lastReport) > 10 {
            lastReport = Date()
            saveProgress(final: false)
        }
    }
}
