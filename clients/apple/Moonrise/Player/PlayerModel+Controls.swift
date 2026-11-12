import Foundation

extension PlayerModel {
    // MARK: - Controls

    func togglePlay() {
        switch recovery {
        case .recovering:
            // Resumes as asked once the new stream plays.
            recoverPlaying.toggle()
            isPlaying = recoverPlaying
            nowPlaying?.update()
            return
        case .failed:
            retryStream()
            return
        case .none:
            break
        }
        guard let engine else { return }
        if isFinished { isFinished = false; engine.seek(to: 0) }
        // The viewer's own choice (a stalled player counts as playing).
        let play = !(engine.isPlaying || (playIntent && isBuffering))
        playIntent = play
        intentChangedAt = Date()
        if play { engine.play() } else { engine.pause(); isPlaying = false }
    }

    func seek(to t: Double) {
        let target = min(max(0, t), max(duration - 1, 0))
        stuckSince = nil
        if recovery != .none {
            // The new stream starts there (or seeks there once it plays).
            recoveryPosition = target
            currentTime = target
            nowPlaying?.update()
            return
        }
        currentTime = target
        seekTarget = target
        seekStarted = Date()
        seekSlack = 1.5
        isFinished = false
        windowLoader?.advance(to: target - subtitleDelay)
        engine?.seek(to: target)
        nowPlaying?.update()
    }

    // MARK: Skips

    /// The run's total so far (e.g. +30), while one is pending.
    var skipTotal: Double? {
        guard let t = skipTarget, let o = skipOrigin else { return nil }
        return t - o
    }
    /// The time the controls show: the pending target, else where playback is.
    var shownTime: Double { skipTarget ?? currentTime }
    static let skipSettle: Duration = .milliseconds(450)

    func skip(_ delta: Double) {
        let base = skipTarget ?? currentTime
        let end = duration > 0 ? max(duration - 1, 0) : Double.greatestFiniteMagnitude
        if skipOrigin == nil { skipOrigin = currentTime }
        skipTarget = min(max(0, base + delta), end)
        skipTask?.cancel()
        skipTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: Self.skipSettle)
            guard let self, !Task.isCancelled, !self.closed, let target = self.skipTarget else { return }
            #if DEBUG
            fputs(String(format: "MoonriseSkip: seek to %.1f (total %+.0f)\n", target, self.skipTotal ?? 0), stderr)
            #endif
            self.skipTarget = nil
            self.skipOrigin = nil
            self.seek(to: target)
        }
    }
}
