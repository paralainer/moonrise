import Foundation

extension PlayerModel {
    // MARK: - Progress

    /// Debug seam: MOONRISE_NO_REPORT=1 keeps scripted test runs out of the user's watch history.
    var reportingEnabled: Bool {
        #if DEBUG
        return !DebugSeams.launch.noReport
        #else
        return true
        #endif
    }

    func report(paused: Bool) -> PlaybackReport {
        PlaybackReport(
            itemId: item?.id ?? "", versionId: mediaSource?.id, sessionId: playSessionId,
            position: currentTime, isPaused: paused, playMethod: playMethod,
            audioIndex: selectedAudio, subtitleIndex: burnInIndex, creditsStart: SegmentRules.creditsStart(segments),
            tracks: tracksPlaying())
    }

    /// The audio and subtitle tracks playing, as the server keeps them; nil until the
    /// tracks a title starts with are picked. A download's tracks have the player's own ids.
    func tracksPlaying() -> TrackRules.Choice? {
        if local != nil {
            guard localAudioApplied, !localSubtitlePending else { return nil }
        } else {
            guard tracksSettled else { return nil }
        }
        var audio: TrackRules.Pick?
        if let id = selectedAudio {
            if local == nil, let t = mediaSource?.audioTracks.first(where: { $0.index == id }) {
                audio = TrackRules.Pick(index: id, language: LanguageCode.normalized(t.language), title: t.title ?? t.displayTitle)
            } else if local != nil, let t = engine?.audioTracks.first(where: { $0.id == id }) {
                audio = TrackRules.Pick(index: id, language: LanguageCode.normalized(t.language), title: t.title)
            }
        }
        let option = selectedSubtitleId.flatMap { id in subtitleOptions.first { $0.id == id } }
        return TrackRules.Choice(audio: audio, subtitle: option.map { o in
            TrackRules.Pick(index: local == nil ? (o.trackIndex ?? serverTrackIndex(o.id)) : nil,
                            language: TrackRules.subtitleLanguage(o), title: TrackRules.subtitleDetail(o),
                            forced: TrackRules.isForced(o), sdh: TrackRules.isSDH(o))
        })
    }

    func saveProgress(final: Bool) {
        if let key = progressKey, duration > 0 {
            ProgressStore.save(final ? duration : currentTime, duration: duration, for: key)
        }
        if local != nil {
            if final { currentTime = duration }
            localReport(final ? "stop" : "progress")
            return
        }
        if let client, item != nil, reportingEnabled {
            let r = report(paused: !isPlaying)
            Task { await client.report(final ? .stopped : .progress, r) }
        }
    }

    func localReport(_ event: String) {
        guard let rec = local, let itemId = rec.itemId else { return }
        let d = duration > 0 ? duration : rec.runtimeTicks.map { Double($0) / 10_000_000 }
        let tracks = event == "start" ? nil : tracksPlaying()
        if let tracks { LocalTrackMemory.save(tracks, download: rec.id) }
        LocalReports.shared.add(source: rec.sourceId, viewing: viewing, item: itemId, event: event, position: currentTime, duration: d,
                                credits: SegmentRules.creditsStart(segments), tracks: tracks?.withoutIndexes, file: rec.mediaSourceId)
    }
}
