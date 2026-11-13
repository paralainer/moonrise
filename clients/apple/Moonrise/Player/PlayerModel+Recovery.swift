import Foundation

extension PlayerModel {
    // MARK: - Stream recovery

    // A server stream dies when the server restarts (sessions live in its memory: their URLs
    // then answer 410), and stalls when the network goes (Wi-Fi drops, the tailnet route
    // changes). Signals: the item failing; a 410 (or a 404) for the session's URLs in its error
    // log; no progress while it should play (6 s: check the session; 20 s: recover anyway, 40 s
    // before the stream has ever been ready). Ordinary buffering (a seek, a slow segment) passes
    // the check: the session answers. Recovery waits (backoff) until ConnectionRouter reaches the
    // server on some route, opens a new session at the same position with the same audio,
    // subtitle and quality, and plays it in the same AVPlayer (PiP and AirPlay carry on),
    // paused or playing as before. It gives up after a minute without the server, or when the
    // stream keeps failing, and offers Retry.
    //
    // A stream holds the address of the route it was opened on, and a change of route doesn't
    // reach it by itself: the watchdog compares the two (`StreamRouteRules`) and moves it, with
    // the same recovery, as soon as its address can't work, or once a stall says it doesn't.

    /// Playing the server's stream (not a download from the device).
    var streamsFromServer: Bool { local == nil && client != nil && item != nil && !closed }

    /// Seconds without the server before recovery gives up.
    static let giveUpAfter: TimeInterval = {
        #if DEBUG
        if let s = DebugSeams.launch.recoveryGiveUp { return s }
        #endif
        return 60
    }()

    func startWatchdog() {
        guard watchdog == nil else { return }
        watchdog = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard let self, !self.closed else { return }
                self.watchdogTick()
            }
        }
    }

    /// Once a second: is the stream moving while it should?
    private func watchdogTick() {
        guard streamsFromServer, recovery == .none, let engine else { stuckSince = nil; return }
        let router = ConnectionRouter.shared
        let now = Date()
        let stalled = stuckSince.map { now.timeIntervalSince($0) } ?? 0
        if !isFinished, StreamRouteRules.shouldMove(from: streamRoute, to: router.route, stalledFor: stalled,
                                                    routeSettledFor: now.timeIntervalSince(router.routeChangedAt),
                                                    sinceLastMove: now.timeIntervalSince(routeMovedAt)) {
            routeMovedAt = now
            routeMoves += 1
            recover("the route changed to \(router.route?.label ?? "-")", counts: false)
            return
        }
        // Stalled with no word of a change: the router only looks when told something happened.
        if StreamRouteRules.shouldAskForRoute(stalledFor: stalled, sinceLastAsked: now.timeIntervalSince(routeAskedAt)) {
            routeAskedAt = now
            Task { await router.refresh() }
        }
        let t = engine.currentTime
        let moving = abs(t - lastTick) > 0.05
        lastTick = t
        // Should be playing: the viewer's intent (AVPlayer may have paused itself on a stall).
        let shouldPlay = playIntent && !isFinished
        // Asked not to play: whatever was lost is over; a pause isn't time spent trying to recover.
        if !shouldPlay { streamLostAt = nil }
        guard shouldPlay, !moving else {
            stuckSince = nil
            if moving, engine.isPlaying {
                playedSinceRecovery += 1
                // Played a while: a later loss starts the count of tries afresh.
                if playedSinceRecovery >= 15 { recoveryAttempts = 0; streamLostAt = nil }
            }
            return
        }
        let since = stuckSince ?? Date()
        stuckSince = since
        let stuck = Date().timeIntervalSince(since)
        if stuck >= (itemReady ? 20 : 40) {
            recover(String(format: "no progress for %.0f s", stuck))
        } else if stuck >= 6 {
            checkSession(String(format: "no progress for %.0f s", stuck), stuck: true)
        }
    }

    /// AVPlayer logged a failed request (it retries some by itself).
    func streamError(status: Int, uri: String?) {
        guard streamsFromServer, recovery == .none, let uri, uri.contains("/v1/s/") else { return }
        // (AVPlayer reports a refused segment as a CoreMedia code, below 0, as often as an HTTP one.)
        if status >= 400 && status != 410 || status < 0 { lastStreamError = (status, uri) }
        if status == 410 {
            // The server doesn't have the session (it restarted, or the session expired).
            recover("the session is gone (410)")
        } else if status == 404 || status < 0 || status >= 500 {
            // A lost session on an older server, or the network: ask the session itself.
            checkSession("request failed (\(status))", stuck: stuckSince != nil)
        }
    }

    private enum SessionState: String { case alive, gone, unreachable }

    /// Asks the session's master playlist whether the session is still there. A gone session
    /// recovers at once (even while the buffer still plays); an unreachable server only once
    /// playback is stuck (the buffer may outlast a short drop).
    func checkSession(_ why: String, stuck: Bool) {
        guard !probing, Date().timeIntervalSince(lastProbe) >= 3, let master = moonriseMaster else { return }
        probing = true
        lastProbe = Date()
        Task { [weak self] in
            let state = await Self.probe(master)
            guard let self else { return }
            self.probing = false
            #if DEBUG
            self.mark("stream check (\(why)): \(state.rawValue)")
            #endif
            guard self.streamsFromServer, self.recovery == .none, self.moonriseMaster == master else { return }
            switch state {
            case .gone: self.recover("the session is gone (\(why))")
            case .unreachable where stuck || self.stuckSince != nil: self.recover("the server can't be reached (\(why))")
            case .unreachable, .alive: break
            }
        }
    }

    private static func probe(_ master: URL) async -> SessionState {
        let req = HTTP.request(master, timeout: 4, cachePolicy: .reloadIgnoringLocalCacheData)
        do {
            let (_, code) = try await HTTP.data(for: req)
            return code == 410 || code == 404 ? .gone : .alive
        } catch {
            return .unreachable
        }
    }

    /// Starts recovery (once at a time).
    /// - failure: the item failed with this message (AVPlayer paused it, so a pause just before
    ///   doesn't mean the viewer paused).
    /// - counts: false for a move to another route, which says nothing about the stream failing.
    func recover(_ why: String, failure: String? = nil, counts: Bool = true) {
        guard streamsFromServer, recovery != .recovering else { return }
        if counts { recoveryAttempts += 1 }
        let position = seekTarget ?? (currentTime > 0.5 ? currentTime : lastPosition)
        let playing = playIntent || (failure != nil && Date().timeIntervalSince(intentChangedAt) < 2)
        #if DEBUG
        mark(String(format: "recovery: %@ at %.1f, %@, try %d", why, position, playing ? "playing" : "paused", recoveryAttempts))
        #endif
        // A stream that fails again and again (never got going, or only briefly): stop trying.
        if recoveryAttempts > (everReady ? 4 : 2) {
            giveUpFailing(failure)
            return
        }
        let lostAt = streamLostAt ?? Date()
        streamLostAt = lostAt
        // Tried for as long as the viewer should wait, across every try (a route that answers
        // but never delivers the stream looks reachable to each one).
        if Date().timeIntervalSince(lostAt) >= recoveryGiveUp {
            giveUpFailing(failure)
            return
        }
        recoveryPosition = position
        recoverPlaying = playing
        recovery = .recovering
        awaitingNewItem = false
        stuckSince = nil
        seekTarget = nil
        currentTime = position
        isPlaying = playing
        isBuffering = true
        errorMessage = nil
        engine?.pause()
        nowPlaying?.update()
        recoveryTask?.cancel()
        recoveryTask = Task { [weak self] in await self?.runRecovery() }
    }

    private func runRecovery() async {
        let began = streamLostAt ?? Date()
        var delay: Double = 1
        let router = ConnectionRouter.shared
        while !Task.isCancelled, !closed, recovery == .recovering {
            await router.refresh()
            guard !Task.isCancelled, !closed, recovery == .recovering else { return }
            if router.isReachable {
                do {
                    #if DEBUG
                    mark(String(format: "recovery: server reachable (%@), new session at %.1f", router.route?.label ?? "-", recoveryPosition))
                    #endif
                    try await openSession(at: recoveryPosition, keepSubtitle: true, reuse: true)
                    guard !Task.isCancelled, !closed else { return }
                    // The new item: ready (finishRecovery) or failed (statusChanged) within 20 s
                    // (AVPlayer can sit on a connection that died with the network: try again).
                    for _ in 0..<40 where recovery == .recovering && awaitingNewItem {
                        try? await Task.sleep(for: .milliseconds(500))
                    }
                    if recovery == .recovering, awaitingNewItem, !Task.isCancelled {
                        awaitingNewItem = false
                        recovery = .none
                        recover("the new stream didn't start")
                    }
                    return
                } catch where !Self.isTransient(error) {
                    giveUp(StreamFailure(PlaybackProblem(error)))
                    return
                } catch {
                    #if DEBUG
                    mark("recovery: no session yet: \(error.localizedDescription)")
                    #endif
                }
            }
            if Date().timeIntervalSince(began) >= recoveryGiveUp {
                let offline = !router.isOnline
                giveUp(StreamFailure(
                    title: offline ? "You're offline" : "Can't reach the Moonrise server",
                    detail: "Playback stopped at \(Fmt.duration(recoveryPosition)). "
                        + (offline ? "Connect to the internet." : "Check your connection.")))
                return
            }
            try? await Task.sleep(for: .seconds(delay))
            delay = min(delay * 1.5, 5)
        }
    }

    /// The new session's stream is ready.
    func finishRecovery() {
        awaitingNewItem = false
        recovery = .none
        playedSinceRecovery = 0
        stuckSince = nil
        guard let engine else { return }
        // Moved meanwhile (a seek during recovery): go there now.
        if abs(recoveryPosition - recoveryOpenedAt) > 1 { seek(to: recoveryPosition) }
        if recoverPlaying { engine.play() } else { engine.pause() }
        isPlaying = recoverPlaying
        playIntent = recoverPlaying
        #if DEBUG
        mark(String(format: "recovered at %.1f, %@ (audio %@, subtitle %@, quality %@)", recoveryPosition, recoverPlaying ? "playing" : "paused",
                    selectedAudio.map(String.init) ?? "-", selectedSubtitleId ?? "none", quality.label))
        #endif
        nowPlaying?.update()
    }

    /// The stream keeps failing: say so, then (when the server refused a request of it) ask that
    /// request again for the server's own words, and say what they mean (a file gone from disk,
    /// a share that can't be read…).
    private func giveUpFailing(_ failure: String?) {
        let first = StreamFailure(title: "Playback stopped", detail: "The server's stream keeps failing. Try again.", more: failure)
        giveUp(first)
        guard let last = lastStreamError, let url = URL(string: last.uri) else { return }
        Task { [weak self] in
            let req = HTTP.request(url, timeout: 5, cachePolicy: .reloadIgnoringLocalCacheData)
            guard let (data, code) = try? await HTTP.data(for: req), code >= 400 else { return }
            let body = String(decoding: data.prefix(600), as: UTF8.self)
            let p = PlaybackProblem.classify(status: code, text: body)
            guard let self, self.recovery == .failed(first) else { return }
            #if DEBUG
            self.mark("recovery: the server says \(code) (was \(last.status)): \(p.kind.rawValue)")
            #endif
            self.recovery = .failed(StreamFailure(p))
        }
    }

    private func giveUp(_ failure: StreamFailure) {
        recoveryTask = nil
        awaitingNewItem = false
        recovery = .failed(failure)
        isBuffering = false
        isPlaying = false
        engine?.pause()
        nowPlaying?.update()
        #if DEBUG
        mark("recovery: gave up: \(failure.title). \(failure.detail)")
        #endif
    }

    /// The give-up message's Retry: try again for another minute, from where it stopped.
    func retryStream() {
        guard case .failed = recovery else { return }
        recovery = .none
        recoveryAttempts = 0
        streamLostAt = nil
        // As it was when the stream was lost.
        playIntent = recoverPlaying
        intentChangedAt = .distantPast
        recover("retry")
    }

    /// Worth trying again (the server or the network may come back), as opposed to an answer.
    private static func isTransient(_ error: Error) -> Bool {
        if error is URLError { return true }
        if let e = error as? MoonriseServer.OfferError {
            switch e {
            case .unreachable: return true
            case let .refused(code, _): return code == 0 || code == 408 || code == 429 || code >= 500
            case let .unsupported(_, pending): return pending
            case .notFound, .notAsAsked: return false
            }
        }
        if let e = error as? MoonriseClientError, case let .http(code, _) = e { return code == 0 || code >= 500 }
        return false
    }
}
