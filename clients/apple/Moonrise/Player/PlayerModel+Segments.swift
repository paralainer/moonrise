import Foundation

extension PlayerModel {
    // MARK: - Segments

    func updateSegment(_ t: Double) {
        let settings = AppSettings.shared
        let seg = segments.first { t >= $0.start && t < $0.end - 1 && settings.skipMode($0.kind) != .off }
        if seg == activeSegment {
            if let p = autoSkipProgress {
                let delay = max(settings.autoSkipDelay, 0.1)
                let progress = min(1, max(p, (t - autoSkipFrom) / delay))
                if progress >= 1 { skipSegment() } else { autoSkipProgress = progress }
            }
            return
        }
        activeSegment = seg
        autoSkipProgress = nil
        promptTask?.cancel()
        guard let seg else { segmentPromptVisible = false; return }
        // Only the first entry into a segment auto-skips. Seeking back into a segment you
        // skipped (or chose to watch) just offers the button.
        let firstTime = handledSegments.insert(seg.id).inserted
        if firstTime, settings.skipMode(seg.kind) == .auto {
            if settings.autoSkipDelay <= 0 { skipSegment(); return }
            autoSkipFrom = t
            autoSkipProgress = 0
        }
        segmentPromptVisible = true
        promptTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(8))
            guard !Task.isCancelled, let self, self.autoSkipProgress == nil else { return }
            self.segmentPromptVisible = false
        }
    }

    // MARK: - Next episode

    /// The episode after this one in airing order, crossing into the next season (a part of a
    /// split episode counts: Part 1 → Part 2). The specials (listed last) only follow a special.
    var nextEpisode: MediaItem? {
        guard let item, let i = seriesEpisodes.firstIndex(where: { $0.id == item.id }),
              i + 1 < seriesEpisodes.count else { return nil }
        let next = seriesEpisodes[i + 1]
        if next.seasonNumber == 0, (seriesEpisodes[i].seasonNumber ?? 0) != 0 { return nil }
        return next
    }

    /// Near the end, Next Episode is offered with the controls: in the end credits (from where
    /// they start, if they run to the end), or in the last `nextEpisodeLead` seconds. Not for
    /// movies or a show's last episode. The rule: `SegmentRules.nextEpisodeOffered`.
    var nextEpisodeOffered: Bool {
        SegmentRules.nextEpisodeOffered(duration: duration, time: currentTime, shownTime: shownTime, active: activeSegment,
                                        segments: segments, hasNext: nextEpisode != nil)
    }

    func refetchSegmentsNearEnd(_ t: Double) {
        // Not final yet (the community databases or the file still looked at, or the server not
        // reached): ask again on entering the last 10 minutes and 2 minutes before the end
        // (a film's credits wait for the databases' answer).
        let due = SegmentRules.nearEndAsksDue(duration: duration, time: t)
        if due > segmentsNearEndAsks, let client, let item, local == nil || segmentsPending,
           SegmentService.awaitingServer(item, version: segmentsVersion) {
            segmentsNearEndAsks = due
            Task { await loadSegments(item, client: client) }
        }
        guard segmentsPending, duration > 0, duration - t < 300, duration - t > 60, abs(t - segmentsAskedAt) >= 60,
              let client, let item else { return }
        segmentsAskedAt = t
        Task { await loadSegments(item, client: client) }
    }

    /// Asks for the title's segments (the version playing), and keeps them unless the player has
    /// moved on to another title meanwhile. A download's are asked for as it saves them (the
    /// databases' credits while the server is still looking), and saved again once final.
    func loadSegments(_ item: MediaItem, client: any MediaServerClient) async {
        let download = local
        let version = segmentsVersion
        let s = await SegmentService.segments(for: item, version: version, client: client, provisional: download != nil)
        guard self.item?.id == item.id, local?.id == download?.id else { return }
        let final = !SegmentService.awaitingServer(item, version: version)
        if !s.isEmpty { segments = s }
        segmentsPending = item.isEpisode && !final
        // The server was still looking the title up in the community databases (it never waits
        // for them): ask again shortly, a few times, so Skip Intro is there early on.
        if SegmentService.awaitingCommunity(item, version: version), segmentsLookupRetries < SegmentRules.lookupRetries {
            segmentsLookupRetries += 1
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(SegmentRules.lookupRetryDelay))
                guard let self, self.item?.id == item.id else { return }
                await self.loadSegments(item, client: client)
            }
        }
        if let download, final {
            await DownloadManager.shared.saveSegments(id: download.id, item: item, client: client)
        }
    }

    /// The version whose segments are asked for: a download's, else the one playing when the title
    /// has several.
    var segmentsVersion: String? {
        local?.mediaSourceId ?? (versions.count > 1 ? mediaSource?.id ?? wantedVersion : nil)
    }

    /// Skipping the end credits that run to the end moves on to the next episode (the Skip
    /// button says Next Episode then).
    var creditsSkipPlaysNext: Bool {
        guard let seg = activeSegment, seg.kind == .credits, nextEpisode != nil else { return false }
        return seg.end >= duration - 5
    }

    func playNextEpisode() {
        #if DEBUG
        fputs("MoonriseNext: next episode \(nextEpisode?.id ?? "-")\n", stderr)
        #endif
        if let next = nextEpisode { playEpisode(next) }
    }

    func playEpisode(_ episode: MediaItem) {
        onPlayEpisode?(episode)
    }

    func skipSegment() {
        guard let seg = activeSegment else { return }
        if creditsSkipPlaysNext, let next = nextEpisode {
            playEpisode(next) // "skipping" end credits means moving on
            return
        }
        activeSegment = nil
        autoSkipProgress = nil
        segmentPromptVisible = false
        seek(to: seg.end)
    }

    /// Stop the countdown and watch the segment; the button stays available.
    func cancelAutoSkip() {
        autoSkipProgress = nil
        promptTask?.cancel()
        promptTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(5))
            if !Task.isCancelled { self?.segmentPromptVisible = false }
        }
    }

    /// Apple TV: the remote did something else while Skip had it: no countdown, no button (the
    /// remote is the system player's again, which can't move focus back to it).
    func dismissSegmentPrompt() {
        autoSkipProgress = nil
        promptTask?.cancel()
        segmentPromptVisible = false
    }
}
