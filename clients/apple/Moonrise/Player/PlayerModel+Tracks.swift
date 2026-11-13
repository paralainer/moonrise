import Foundation

extension PlayerModel {
    // MARK: - Tracks

    /// The user picked an audio track (a menu): play it, and its language from now on.
    func chooseAudio(_ id: Int) {
        selectAudio(id)
        guard selectedAudio == id, let lang = audioLanguage(id) else { return }
        AppSettings.shared.preferredAudioLanguage = lang
        UserDefaults.standard.synchronize() // kept if the app is quit right after (seen on the simulator)
        #if DEBUG
        mark("audio preference \(lang)")
        #endif
    }

    /// The user picked subtitles (or Off): show them, and that language (and kind) from now on.
    /// Forced subtitles (foreign dialogue only) count as Off: those come on by themselves.
    func chooseSubtitle(_ option: SubtitleOption?) {
        selectSubtitle(option)
        rememberSubtitle(option)
    }

    private func rememberSubtitle(_ option: SubtitleOption?) {
        let s = AppSettings.shared
        if let option, !Self.isForced(option) {
            guard let lang = subtitleLanguage(option) else { return }
            s.preferredSubtitleLanguage = lang
            s.preferSDHSubtitles = Self.isSDH(option)
            s.autoSelectSubtitles = true
        } else {
            s.autoSelectSubtitles = false
        }
        UserDefaults.standard.synchronize()
        #if DEBUG
        mark("subtitle preference \(s.autoSelectSubtitles ? s.preferredSubtitleLanguage : "off")\(s.preferSDHSubtitles ? " SDH" : "")")
        #endif
    }

    func audioLanguage(_ id: Int) -> String? {
        if local == nil, let ms = mediaSource {
            return LanguageCode.normalized(ms.audioTracks.first { $0.index == id }?.language)
        }
        return LanguageCode.normalized(engine?.audioTracks.first { $0.id == id }?.language)
    }

    private func subtitleLanguage(_ o: SubtitleOption) -> String? { TrackRules.subtitleLanguage(o) }

    typealias AudioCandidate = TrackRules.AudioCandidate

    /// The track in the preferred audio language, or nil: the title's own default plays
    /// (`TrackRules.preferredAudio`).
    static func preferredAudio(_ tracks: [AudioCandidate]) -> Int? {
        TrackRules.preferredAudio(tracks, preferred: AppSettings.shared.preferredAudioLanguage)
    }

    static func isForced(_ o: SubtitleOption) -> Bool { TrackRules.isForced(o) }
    static func isSDH(_ o: SubtitleOption) -> Bool { TrackRules.isSDH(o) }

    func selectAudio(_ id: Int) {
        guard id != selectedAudio else { return }
        let previous = selectedAudio
        selectedAudio = id
        if client != nil, local == nil {
            // The server's stream carries the audio tracks: switch in place.
            if moonriseAudio.contains(id) {
                selectMoonriseAudio()
            } else {
                // Not in its stream: keep the track playing.
                notice = MoonriseClientError.audioUnavailable.localizedDescription
                selectedAudio = previous
            }
        } else {
            engine?.selectAudioTrack(id)
        }
    }

    func selectSubtitle(_ option: SubtitleOption?) {
        applySubtitle(option)
        restoreSubtitleDelay(option)
        updatePiPSubtitle()
        syncExternalSubtitles()
    }

    func applySubtitle(_ option: SubtitleOption?) {
        selectedSubtitleId = option?.id
        timingWatch?.cancel()
        subtitleDoc = nil
        subtitleText = nil
        pictureDoc = nil
        pictureCues = []
        windowLoader?.stop()
        windowLoader = nil
        engine?.selectSubtitleTrack(nil)

        guard let option else { return }
        switch option.kind {
        case let .pictures(index):
            // Images from the server, window by window around the playhead; the stream plays on.
            startPictureWindows(option, index: index)
        case let .engine(id):
            engine?.selectSubtitleTrack(id)
        case let .external(url):
            Task {
                // An embedded track the server hasn't extracted yet: windows at the playhead.
                if await self.startSubtitleWindows(option, url: url) { return }
                // A downloaded track: how its match stands first, so what is loaded is what the
                // answer says.
                let pending = await self.refreshTiming(option)
                guard self.selectedSubtitleId == option.id else { return }
                await self.loadWholeSubtitle(url)
                if pending { self.watchTiming(option, url: url) }
            }
        }
    }

    /// The rendition of the Moonrise server's stream for the selected audio track.
    private func selectMoonriseAudio() {
        guard let engine, let id = selectedAudio, let i = moonriseAudio.firstIndex(of: id),
              engine.audioTracks.indices.contains(i) else { return }
        if engine.currentAudioTrack != i { engine.selectAudioTrack(i) }
        playMethodLabel = moonriseLabel(audio: id)
        #if DEBUG
        fputs("MoonriseAudio: rendition \(i) for track \(id) (\(playMethodLabel))\n", stderr)
        #endif
    }

    /// The engine plays another audio rendition than the one Moonrise picked: the system player's
    /// Audio menu (Apple TV) — the user's choice, like Moonrise's own menu.
    func engineAudioSelected(_ rendition: Int) {
        // Until the item has settled the change is the item's own (its default rendition).
        guard itemReady, let readyAt, Date().timeIntervalSince(readyAt) > 2 else { reassertAudio(); return }
        if local == nil, !moonriseAudio.isEmpty {
            guard moonriseAudio.indices.contains(rendition), moonriseAudio[rendition] != selectedAudio else { return }
            #if DEBUG
            mark("system audio menu: rendition \(rendition)")
            #endif
            chooseAudio(moonriseAudio[rendition])
        } else if local != nil, localAudioApplied, rendition != selectedAudio {
            #if DEBUG
            mark("system audio menu: track \(rendition)")
            #endif
            chooseAudio(rendition)
        }
    }

    /// Plays the audio track Moonrise selected, if the engine moved off it by itself.
    func reassertAudio() {
        guard let engine else { return }
        if local == nil, !moonriseAudio.isEmpty {
            selectMoonriseAudio()
        } else if local != nil, localAudioApplied, let a = selectedAudio, engine.currentAudioTrack != a {
            engine.selectAudioTrack(a)
        }
        #if DEBUG
        mark("audio rendition \(engine.currentAudioTrack.map(String.init) ?? "-") (track \(selectedAudio.map(String.init) ?? "-"))")
        #endif
    }

    /// Embedded tracks for playback without a media server come from the engine itself.
    func engineTracksChanged() {
        defer { updatePiPSubtitle(); syncExternalSubtitles() }
        if !moonriseAudio.isEmpty { selectMoonriseAudio() }
        guard client == nil || local != nil, let engine else { return }
        let tracks = engine.audioTracks
        audioOptions = zip(tracks, Self.distinctDetails(tracks.map { ($0.title, nil) })).map { AudioOption(id: $0.id, title: $0.title, detail: $1) }
        selectedAudio = engine.currentAudioTrack
        if !localAudioApplied {
            // A download starting: the track played last in it, else the audio language picked
            // last, when it has it.
            localAudioApplied = true
            let current = engine.currentAudioTrack
            let candidates = engine.audioTracks.map {
                AudioCandidate(id: $0.id, language: $0.language, isDefault: $0.id == current, name: $0.title)
            }
            if let id = localKeepAudio ?? TrackRules.rememberedAudio(candidates, memory: trackMemory) ?? Self.preferredAudio(candidates),
               id != current {
                engine.selectAudioTrack(id)
                selectedAudio = id
            }
        }
        #if DEBUG
        fputs("MoonriseTracks: audio \(audioOptions.map(\.title)) selected \(selectedAudio.map(String.init) ?? "-"), subtitles \(subtitleOptions.count)\n", stderr)
        #endif
        // A segmented download's subtitle renditions are its .srt files too (listed already).
        let embedded = local?.isSegmented == true ? [] : engine.subtitleTracks.map {
            SubtitleOption(id: "engine:\($0.id)", title: $0.title, language: $0.language, kind: .engine($0.id))
        }
        subtitleOptions = subtitleOptions.filter { !$0.id.hasPrefix("engine:") } + embedded
        if localSubtitlePending, local != nil {
            localSubtitlePending = false
            autoSelectSubtitle()
        }
    }
}
