// A route change during playback, end to end: a real AVPlayer plays a real HLS stream from a fake
// server on the loopback (`RouteChangeHarness`), the route it came over dies the way a real one does,
// and playback has to go on by itself, on the other route, at the same place with the same tracks.
import Foundation
import Testing
@testable import Moonrise

@MainActor
@Suite(.serialized) struct RouteChangeTests {
    /// One scripted run: the title, started at `start`, the routes and the app on them.
    @MainActor
    final class Run {
        let server: FakeMoonrise
        let home: FakeRoute
        let tailnet: FakeRoute
        let model: PlayerModel
        let source: Source
        /// With `throughForwarder`: the real forwarder on the tailnet route, and the tunnel under it.
        let tunnel: FakeTunnel?
        let forwarder: PortForwarder?
        /// Since the run began: how much of it the process spent waiting for a CPU (in `line`).
        let watch = TestWatch()

        init(start: Double, startOnTailnet: Bool, throughForwarder: Bool = false) async throws {
            server = try FakeMoonrise()
            home = try FakeRoute(index: 0, server: server)
            tailnet = try FakeRoute(index: 1, server: server)
            if startOnTailnet { home.set(.hung) }
            source = Source(kind: .moonrise, name: "route-test", address: home.url.absoluteString, username: "viewer")
            let router = ConnectionRouter.shared
            if throughForwarder {
                let tunnel = FakeTunnel(upstream: tailnet.port)
                let forwarder = try PortForwarder(target: "100.64.0.1:8096", dial: { tunnel.dial($0) })
                self.tunnel = tunnel
                self.forwarder = forwarder
                router.debugTailnet = URL(string: "http://127.0.0.1:\(forwarder.port)")!
            } else {
                tunnel = nil
                forwarder = nil
                router.debugTailnet = tailnet.url
            }
            router.reset()
            SourceStore.shared.signIn(source)
            await router.refresh()
            let item = try MoonriseClient.decodeItem(server.answer(method: "GET", path: "/v1/items/\(FakeMoonrise.itemId)", query: [:], headers: [:], body: Data(), route: 0).body)
            var request = PlaybackRequest(target: .server(SourceStore.shared.server ?? source, item))
            request.startAt = start
            model = PlayerModel(request: request)
        }

        func finish() async {
            model.close()
            // Nothing may hold the sign-out up.
            home.set(.up)
            tailnet.set(.up)
            tunnel?.set(.up)
            await SourceStore.shared.signOut()
            ConnectionRouter.shared.debugTailnet = nil
            if let forwarder {
                forwarder.close()
                // The node goes (a live connection may sit idle in URLSession's pool until then):
                // everything the forwarder opened for this run has to go with it.
                tunnel?.closeAll()
                let began = TestWatch()
                while began.elapsed < 20, !forwarder.counts.isIdle {
                    try? await Task.sleep(for: .milliseconds(100))
                }
                #expect(forwarder.counts.isIdle, "still open: \(forwarder.counts)")
            }
            home.stop()
            tailnet.stop()
        }

        /// Plays until `after` seconds of the title have gone by.
        /// Only the start: how long the first stream takes to get going isn't what is tested.
        func playUntilMoving(past position: Double, timeout: Double = 60) async throws {
            let began = TestWatch()
            while began.elapsed < timeout {
                if model.currentTime >= position, model.isPlaying, !model.isBuffering { return }
                try await Task.sleep(for: .milliseconds(100))
            }
            Issue.record("didn't play to \(position): at \(model.currentTime), playing \(model.isPlaying), buffering \(model.isBuffering), recovery \(model.recovery)")
            throw CancellationError()
        }

        /// Waits until the title has played on, past `past`, with no spinner, and answers how long
        /// the viewer saw the spinner on the way (seconds on a `TestWatch`, sampled).
        func playsOn(past: Double, within: Double) async throws -> Double {
            let began = TestWatch()
            var spinner = 0.0
            var last = began.elapsed
            while began.elapsed < within {
                let now = began.elapsed
                if model.currentTime >= past, model.isPlaying, !model.isBuffering, model.recovery == .none {
                    return spinner
                }
                if model.isBuffering || model.recovery == .recovering { spinner += now - last }
                last = now
                try await Task.sleep(for: .milliseconds(100))
            }
            Issue.record("still not playing after \(within) s: at \(model.currentTime), playing \(model.isPlaying), buffering \(model.isBuffering), recovery \(model.recovery), sessions \(server.sessions.map(\.id)), route \(String(describing: ConnectionRouter.shared.route))")
            throw CancellationError()
        }

        func pickSecondAudio() async throws {
            let began = TestWatch()
            while began.elapsed < 30, !model.tracksSettled || model.audioOptions.count < 2 {
                try await Task.sleep(for: .milliseconds(100))
            }
            model.selectAudio(4)
            #expect(model.selectedAudio == 4)
        }

        var line: String {
            "at \(model.currentTime), sessions \(server.sessions.map { "\($0.id)@\($0.start ?? -1) on \($0.route)" }), "
                + String(format: "%.1f s waiting for a CPU", watch.lost)
        }
    }

    private func scenario(startOnTailnet: Bool, cut: FakeRoute.Mode, event: Bool, within: Double, staysOnRoute: Bool = false,
                          tailnetDownFor: Double = 0, movesOnRoute: Bool = false, maxSpinner: Double) async throws {
        let run = try await Run(start: 5, startOnTailnet: startOnTailnet)
        await run.model.start()
        do {
            try await run.pickSecondAudio()
            try await run.playUntilMoving(past: 8)
            let at = run.model.currentTime
            let versionBefore = run.model.wantedVersion
            // The route it is playing over dies; the other one works.
            let (old, new) = startOnTailnet ? (run.tailnet, run.home) : (run.home, run.tailnet)
            if tailnetDownFor > 0 {
                new.set(.hung)
                Task.detached { try? await Task.sleep(for: .seconds(tailnetDownFor)); new.set(.up) }
            }
            old.set(cut)
            if startOnTailnet { run.home.set(.up) }
            if event { ConnectionRouter.shared.debugNetworkChanged() }
            // Past everything fetched before the cut: only a working stream gets there.
            let took = try await run.playsOn(past: at + 10, within: within)
            print("RouteChange: start \(startOnTailnet ? "tailnet" : "home"), cut \(cut), event \(event): spinner \(took) s, \(run.line)")
            let sessions = run.server.sessions
            #expect(took <= maxSpinner, "spinner for \(took) s")
            // Moved because the route changed, not because the stream had already died.
            if movesOnRoute { #expect(run.model.routeMoves == 1) }
            if !staysOnRoute {
                #expect(sessions.count >= 2)
                #expect(sessions.last?.route == new.index)
            }
            let latest = try #require(sessions.last)
            let resumed = try #require(latest.start)
            if !staysOnRoute {
                #expect(resumed >= at - 1, "resumed at \(resumed), was at \(at)")
                #expect(resumed <= at + 15)
            }
            #expect(latest.version == nil || latest.version == versionBefore)
            #expect(run.model.selectedAudio == 4)
            #expect(run.model.wantedVersion == versionBefore)
            #expect(run.model.errorMessage == nil)
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    @Test func leavingHome_hung_withEvent() async throws {
        try await scenario(startOnTailnet: false, cut: .hung, event: true, within: 20, movesOnRoute: true, maxSpinner: 6)
    }

    @Test func leavingHome_hung_noEvent() async throws {
        try await scenario(startOnTailnet: false, cut: .hung, event: false, within: 30, maxSpinner: 12)
    }

    @Test func leavingHome_refused() async throws {
        try await scenario(startOnTailnet: false, cut: .refused, event: true, within: 20, maxSpinner: 6)
    }

    @Test func comingHome_hung() async throws {
        try await scenario(startOnTailnet: true, cut: .hung, event: true, within: 25, movesOnRoute: true, maxSpinner: 10)
    }

    /// The tunnel under the stream's connections dies; a new connection to the same address works.
    @Test func sameRoute_staleConnections() async throws {
        try await scenario(startOnTailnet: false, cut: .stale, event: false, within: 30, staysOnRoute: true, maxSpinner: 12)
    }

    /// Back home with the tailnet stream still working: it isn't interrupted.
    @Test func comingHome_healthyStreamIsLeftAlone() async throws {
        let run = try await Run(start: 5, startOnTailnet: true)
        await run.model.start()
        do {
            try await run.playUntilMoving(past: 8)
            run.home.set(.up)
            ConnectionRouter.shared.debugNetworkChanged()
            try await Task.sleep(for: .seconds(8))
            #expect(ConnectionRouter.shared.route == .lan)
            #expect(run.model.isPlaying)
            #expect(run.server.sessions.count == 1)
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    /// A new route that answers everything but the stream: recovery ends with its message at the
    /// limit, counting all its tries, instead of spinning through one full wait after another.
    @Test func aRouteThatNeverDeliversTheStreamEnds() async throws {
        let run = try await Run(start: 5, startOnTailnet: false)
        run.model.recoveryGiveUp = 20
        await run.model.start()
        do {
            try await run.playUntilMoving(past: 8)
            run.tailnet.set(.mediaHung)
            run.home.set(.hung)
            ConnectionRouter.shared.debugNetworkChanged()
            let began = TestWatch()
            while began.elapsed < 60, !(run.model.streamFailure != nil) {
                try await Task.sleep(for: .milliseconds(200))
            }
            let took = began.elapsed
            print("RouteChange neverDelivers: failed \((run.model.streamFailure != nil)) after \(took) s")
            #expect((run.model.streamFailure != nil))
            #expect(took < 50, "spun for \(took) s")
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    /// Paused and moved, then a pause longer than the give-up limit: that wait isn't recovery time,
    /// so a failure soon after resuming gets a new stream and not "Playback stopped". The buffer
    /// stays small (the moved stream serves only its first 6 s, and its route stops sending
    /// segments once it is ready), so the failure comes well inside the 15 s of playback that
    /// would clear the wait anyway, however long the move took. `kill` is what goes wrong during
    /// the pause.
    private func pausedMoveThenFailure(_ kill: (Run) -> Void, movesHome: Bool) async throws {
        let run = try await Run(start: 5, startOnTailnet: false)
        run.model.recoveryGiveUp = 6
        await run.model.start()
        do {
            try await run.playUntilMoving(past: 8)
            run.model.togglePlay()
            #expect(!run.model.isPlaying)
            let at = run.model.currentTime
            // Leaves home while paused: the stream moves, and stays paused.
            run.server.limitNextSession(to: 6)
            run.home.set(.hung)
            ConnectionRouter.shared.debugNetworkChanged()
            let began = TestWatch()
            while began.elapsed < 20, run.server.sessions.count < 2 || run.model.recovery != .none {
                try await Task.sleep(for: .milliseconds(100))
            }
            #expect(run.server.sessions.count == 2, "\(run.line)")
            #expect(!run.model.isPlaying)
            run.tailnet.set(.mediaHung)
            // Longer than the limit, still paused.
            try await Task.sleep(for: .seconds(8))
            kill(run)
            if movesHome {
                run.home.set(.up)
                ConnectionRouter.shared.debugNetworkChanged()
                let asked = TestWatch()
                while asked.elapsed < 10, ConnectionRouter.shared.route != .lan {
                    try await Task.sleep(for: .milliseconds(100))
                }
            }
            run.model.togglePlay()
            _ = try await run.playsOn(past: at + 10, within: 40)
            print("RouteChange pausedMove: \(run.line)")
            #expect(run.model.streamFailure == nil)
            #expect(run.server.sessions.count >= 3, "\(run.line)")
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    @Test func pausedMove_thenTailnetDies_movesHomeAfterResume() async throws {
        try await pausedMoveThenFailure({ $0.tailnet.set(.hung) }, movesHome: true)
    }

    @Test func pausedMove_thenServerRestarts_recoversAfterResume() async throws {
        try await pausedMoveThenFailure({ $0.server.forgetSessions(); $0.tailnet.set(.up) }, movesHome: false)
    }

    // The real forwarder between the player and a tunnel that dies (dials block, live connections
    // go silent). Only the node's C dial is replaced.

    @Test func throughForwarder_tunnelDies_movesHome() async throws {
        let run = try await Run(start: 5, startOnTailnet: true, throughForwarder: true)
        await run.model.start()
        do {
            try await run.playUntilMoving(past: 8)
            let at = run.model.currentTime
            run.tunnel?.set(.dead)
            run.home.set(.up)
            ConnectionRouter.shared.debugNetworkChanged()
            let spinner = try await run.playsOn(past: at + 10, within: 30)
            print("RouteChange forwarder moves home: spinner \(spinner) s, \(run.line), \(run.forwarder!.counts)")
            #expect(spinner <= 15)
            #expect(run.server.sessions.last?.route == run.home.index)
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    @Test func throughForwarder_tunnelComesBack() async throws {
        let run = try await Run(start: 5, startOnTailnet: true, throughForwarder: true)
        await run.model.start()
        do {
            try await run.playUntilMoving(past: 8)
            let at = run.model.currentTime
            let forwarderPort = run.forwarder!.port
            run.tunnel?.set(.dead)
            ConnectionRouter.shared.debugNetworkChanged()
            try await Task.sleep(for: .seconds(20))
            // Dead until a dial on it has been given up on: the first dial comes from the route
            // check after the event (behind a probe of the hung home address), and is given up
            // `dialTimeout` later. That is about 12 s in on an idle machine; on a busy one the
            // dial starts later, and a tunnel back before the timeout would complete it instead.
            let dead = TestWatch()
            while dead.elapsed < 30, run.forwarder!.counts.dialsTimedOut == 0 {
                try await Task.sleep(for: .milliseconds(100))
            }
            run.tunnel?.set(.up)
            let spinner = try await run.playsOn(past: at + 10, within: 45)
            print("RouteChange forwarder back: spinner \(spinner) s, \(run.line), \(run.forwarder!.counts)")
            #expect(run.forwarder!.port == forwarderPort)
            #expect(run.server.sessions.last?.route == run.tailnet.index)
            #expect(run.forwarder!.counts.dialsTimedOut > 0, "dials on the dead tunnel should have been given up on")
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    @Test func throughForwarder_noRouteEndsWithTheMessage() async throws {
        let run = try await Run(start: 5, startOnTailnet: true, throughForwarder: true)
        run.model.recoveryGiveUp = 20
        await run.model.start()
        do {
            try await run.playUntilMoving(past: 8)
            run.tunnel?.set(.dead)
            ConnectionRouter.shared.debugNetworkChanged()
            let began = TestWatch()
            while began.elapsed < 60, run.model.streamFailure == nil {
                try await Task.sleep(for: .milliseconds(200))
            }
            let took = began.elapsed
            // The player has given up: nothing is left on the dead tunnel.
            let released = TestWatch()
            while released.elapsed < 12, run.forwarder!.counts.bridges > 0 {
                try await Task.sleep(for: .milliseconds(100))
            }
            let counts = run.forwarder!.counts
            #expect(counts.bridges == 0 && counts.pumps == 0, "\(counts)")
            print("RouteChange forwarder no route: \(String(describing: run.model.streamFailure)) after \(took) s, \(counts), \(run.tunnel!.dials) dials")
            #expect(run.model.streamFailure?.title == "Can't reach the Moonrise server")
            #expect(took < 50, "spun for \(took) s")
            #expect(counts.dialsInFlight <= PortForwarder.Limits().maxDialsInFlight)
            await run.finish()
        } catch {
            await run.finish()
            throw error
        }
    }

    /// The tailnet path isn't back yet when home goes away (its node has to find a way out).
    @Test func tailnetLate() async throws {
        try await scenario(startOnTailnet: false, cut: .hung, event: true, within: 45, tailnetDownFor: 25, maxSpinner: 30)
    }
}
