import Foundation

extension PlayerModel {
    // MARK: - Subtitle timing

    /// Where a track's timing is kept: on the server, for a track the server has.
    private func timingIndex(_ option: SubtitleOption) -> Int? {
        guard local == nil, client?.moonriseBaseURL != nil, let n = option.trackIndex ?? serverTrackIndex(option.id),
              moonriseSubtitles?.contains(where: { $0.index == n && $0.timing != nil }) == true else { return nil }
        return n
    }

    private func setTiming(_ timing: MoonriseServer.SubtitleTiming, index: Int) {
        guard let i = moonriseSubtitles?.firstIndex(where: { $0.index == index }) else { return }
        moonriseSubtitles?[i].timing = timing
        subtitleTimingRevision += 1
    }

    private var selectedTiming: (index: Int, timing: MoonriseServer.SubtitleTiming)? {
        guard let id = selectedSubtitleId, let o = subtitleOptions.first(where: { $0.id == id }), let n = timingIndex(o),
              let t = moonriseSubtitles?.first(where: { $0.index == n })?.timing else { return nil }
        return (n, t)
    }

    /// The selected track's automatic match can be switched off (it was applied).
    var subtitleSyncApplied: Bool { selectedTiming?.timing.synced == true }
    var subtitleOriginalTiming: Bool { selectedTiming?.timing.original == true }

    /// The menu's second line for a track: its details, and how its timing stands.
    func subtitleDetail(_ o: SubtitleOption) -> String? {
        _ = subtitleTimingRevision
        let timing = timingIndex(o).flatMap { n in moonriseSubtitles?.first { $0.index == n }?.timing }
        let twins = subtitleOptions.filter { $0.name == o.name }
        let detail = twins.firstIndex(of: o).map { Self.distinctDetails(twins.map { ($0.name, $0.detail) })[$0] } ?? o.detail
        #if os(tvOS)
        return Self.detailLine(detail, timing: timing?.shortLabel)
        #else
        return Self.detailLine(detail, timing: timing?.label)
        #endif
    }

    /// A subtitle row's second line: its details, then how its timing stands (added to the
    /// details, never instead of them: "Forced · In sync").
    static func detailLine(_ detail: String?, timing: String?) -> String? {
        let parts = [detail, timing].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Menu rows (title, detail) that would read alike, say two English tracks with no title,
    /// get their place among those twins ("#2"), so no two rows of a menu say the same.
    static func distinctDetails(_ rows: [(title: String, detail: String?)]) -> [String?] {
        rows.indices.map { i in
            let row = rows[i]
            let twins = rows.indices.filter { rows[$0].title == row.title && rows[$0].detail == row.detail }
            guard twins.count > 1, let n = twins.firstIndex(of: i) else { return row.detail }
            return [row.detail, "#\(n + 1)"].compactMap { $0 }.joined(separator: " · ")
        }
    }

    /// Changes whenever the menus should say something else about timing.
    var subtitleTimingKey: String { "\(subtitleTimingRevision)|\(subtitleDelay)" }

    /// Moves the selected track by `delta` seconds (later when positive), and keeps that for this
    /// file and track.
    func nudgeSubtitles(by delta: Double) { setSubtitleDelay(subtitleDelay + delta) }

    func resetSubtitleDelay() { setSubtitleDelay(0) }

    private func setSubtitleDelay(_ value: Double) {
        subtitleDelay = (value * 1000).rounded() / 1000
        updateSubtitle(at: currentTime)
        windowLoader?.advance(to: currentTime - subtitleDelay)
        guard let id = selectedSubtitleId, let o = subtitleOptions.first(where: { $0.id == id }) else { return }
        guard let n = timingIndex(o), let client, let base = client.moonriseBaseURL else {
            Self.storeLocalNudge(subtitleDelay, key: localNudgeKey(o))
            return
        }
        // Several taps in a row are one change.
        nudgeTask?.cancel()
        let (value, item, headers) = (subtitleDelay, mediaId, client.headers)
        nudgeTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(600))
            guard !Task.isCancelled else { return }
            if let t = try? await MoonriseServer.setSubtitleTiming(base: base, itemId: item, index: n, nudge: value, headers: headers) {
                self?.setTiming(t, index: n)
            }
        }
    }

    /// The selected track's own nudge, when it is picked.
    func restoreSubtitleDelay(_ option: SubtitleOption?) {
        nudgeTask?.cancel()
        guard let option else { return }
        if let n = timingIndex(option), let t = moonriseSubtitles?.first(where: { $0.index == n })?.timing {
            subtitleDelay = t.nudge
        } else {
            subtitleDelay = Self.localNudges[localNudgeKey(option)] ?? 0
        }
    }

    /// Tracks only this device knows (a download's, a file added here): the nudge is kept here.
    private func localNudgeKey(_ o: SubtitleOption) -> String { "\(local?.id.uuidString ?? mediaId)|\(o.title)" }
    private static let localNudgesKey = "subtitleNudges"
    private static var localNudges: [String: Double] {
        UserDefaults.standard.dictionary(forKey: localNudgesKey) as? [String: Double] ?? [:]
    }
    private static func storeLocalNudge(_ value: Double, key: String) {
        var all = localNudges
        all[key] = value == 0 ? nil : value
        UserDefaults.standard.set(all, forKey: localNudgesKey)
    }

    /// Asks how a downloaded track's match stands (asking starts it). True: it is running.
    func refreshTiming(_ option: SubtitleOption) async -> Bool {
        guard let n = timingIndex(option), isDownloadedServerTrack(n), let client, let base = client.moonriseBaseURL,
              let t = try? await MoonriseServer.subtitleTiming(base: base, itemId: mediaId, index: n, headers: client.headers) else { return false }
        setTiming(t, index: n)
        return t.pending
    }

    /// The selected track's match is running: when it is done, the track is loaded again.
    func watchTiming(_ option: SubtitleOption, url: URL) {
        timingWatch?.cancel()
        guard let n = timingIndex(option), let client, let base = client.moonriseBaseURL else { return }
        let (item, headers) = (mediaId, client.headers)
        timingWatch = Task { [weak self] in
            let started = Date()
            while !Task.isCancelled, Date().timeIntervalSince(started) < 1800 {
                try? await Task.sleep(for: .seconds(3))
                guard !Task.isCancelled, let self, self.selectedSubtitleId == option.id else { return }
                guard let t = try? await MoonriseServer.subtitleTiming(base: base, itemId: item, index: n, headers: headers) else { continue }
                self.setTiming(t, index: n)
                if t.pending { continue }
                #if DEBUG
                self.mark("subtitle timing \(t.state) \(t.offset)")
                #endif
                if t.synced, !t.original {
                    self.subtitlePrefetch[url] = nil
                    await self.loadWholeSubtitle(url)
                    if let label = t.label { self.notice = label }
                }
                return
            }
        }
    }

    /// The viewer's choice between the automatic match and the file's original timing.
    func useOriginalSubtitleTiming(_ original: Bool) {
        guard let id = selectedSubtitleId, let o = subtitleOptions.first(where: { $0.id == id }), case let .external(url) = o.kind,
              let n = timingIndex(o), let client, let base = client.moonriseBaseURL else { return }
        let (item, headers) = (mediaId, client.headers)
        Task {
            guard let t = try? await MoonriseServer.setSubtitleTiming(base: base, itemId: item, index: n, original: original, headers: headers) else {
                self.notice = "Couldn't change the subtitle timing"
                return
            }
            self.setTiming(t, index: n)
            guard self.selectedSubtitleId == id else { return }
            self.subtitlePrefetch[url] = nil
            await self.loadWholeSubtitle(url)
        }
    }
}
