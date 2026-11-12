import Foundation
import Network
import Observation
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// Picks how to reach the Moonrise server: its LAN address when that answers, otherwise the
/// server's tailnet address through the embedded Tailscale node. Re-evaluated on network changes,
/// when the app comes back to the foreground, and every so often while the server can't be reached.
@MainActor
@Observable
final class ConnectionRouter {
    static let shared = ConnectionRouter()

    enum Route: Equatable {
        case checking, lan, tailnet(host: String), unreachable
        var label: String {
            switch self {
            case .checking: "Checking…"
            case .lan: "Local network"
            case let .tailnet(host): "Tailscale · \(host)"
            case .unreachable: "Unreachable"
            }
        }
    }

    /// Why a server can't be reached, with what the user can do about it.
    enum Problem: Equatable {
        case offline
        case tailscaleOff
        case tailscaleNeedsLogin(URL?)
        case tailscaleStarting
        case tailscaleFailed(String)
        case serverDown
    }

    /// How the server is reached; nil before the first check (or with no server).
    private(set) var route: Route? {
        didSet { if route != oldValue { routeChangedAt = Date() } }
    }
    /// When `route` last changed (the player waits for a route to settle before moving to it).
    @ObservationIgnored private(set) var routeChangedAt = Date.distantPast
    /// Device has any network at all.
    private(set) var isOnline = true
    private(set) var onCellular = false
    @ObservationIgnored private let monitor = NWPathMonitor()
    @ObservationIgnored private var refreshTask: Task<Void, Never>?
    @ObservationIgnored private var inflight: Task<Void, Never>?
    @ObservationIgnored private var rerun = false
    @ObservationIgnored private var started = false
    @ObservationIgnored private var retryTask: Task<Void, Never>?
    @ObservationIgnored private var lastPath: TailnetNudge.Path?

    #if DEBUG
    /// Test seam: stands in for the embedded node's tailnet route (`RouteChangeTests`).
    @ObservationIgnored var debugTailnet: URL?
    /// Test seam: what a path update does.
    func debugNetworkChanged() { scheduleRefresh() }
    #endif

    func start() {
        guard !started else { return }
        started = true
        monitor.pathUpdateHandler = { path in
            let online = path.status == .satisfied
            let cell = path.usesInterfaceType(.cellular) && !path.usesInterfaceType(.wifi)
            let seen = TailnetNudge.Path(online: online, interfaces: path.availableInterfaces.map(\.name),
                                         gateways: path.gateways.map(\.debugDescription))
            Task { @MainActor in
                let r = ConnectionRouter.shared
                r.isOnline = online
                r.onCellular = cell
                if TailnetNudge.shouldNudge(from: r.lastPath, to: seen) { TailscaleService.shared.networkChanged() }
                r.lastPath = seen
                r.scheduleRefresh()
            }
        }
        monitor.start(queue: DispatchQueue(label: "moonrise.router"))
        // A device waking from sleep (an Apple TV especially) can come back before its network
        // does, with no path change afterwards: check again whenever the app becomes active.
        #if canImport(UIKit)
        let active = UIApplication.didBecomeActiveNotification
        #else
        let active = NSApplication.didBecomeActiveNotification
        #endif
        NotificationCenter.default.addObserver(forName: active, object: nil, queue: .main) { _ in
            Task { @MainActor in ConnectionRouter.shared.scheduleRefresh() }
        }
        scheduleRefresh()
    }

    /// While unreachable, try again on our own: 3 s, then backing off to every 30 s.
    private func scheduleRetry() {
        guard route == .unreachable, SourceStore.shared.server != nil else {
            retryTask?.cancel()
            retryTask = nil
            return
        }
        guard retryTask == nil else { return }
        retryTask = Task {
            var delay = 3.0
            while !Task.isCancelled, route == .unreachable, SourceStore.shared.server != nil {
                try? await Task.sleep(for: .seconds(delay))
                guard !Task.isCancelled else { return }
                delay = min(delay * 2, 30)
                await refresh()
            }
            if !Task.isCancelled { retryTask = nil }
        }
    }

    /// Debounced: path updates arrive in bursts when switching networks.
    private func scheduleRefresh() {
        refreshTask?.cancel()
        refreshTask = Task {
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            await refresh()
        }
    }

    /// Await the current routing pass so screens don't load against a dead address.
    func ready() async {
        start()
        await refreshTask?.value
        if let t = inflight { await t.value }
        if SourceStore.shared.server != nil, route == nil { await refresh() }
    }

    /// The route to the server a download or report came from (nil when that isn't the server).
    func route(to sourceId: UUID) -> Route? {
        SourceStore.shared.server?.id == sourceId ? route : nil
    }

    /// The server can be answered on some route now.
    var isReachable: Bool { route != nil && route != .unreachable && route != .checking }

    /// Signed out: nothing to route.
    func reset() { route = nil }

    /// One pass at a time. A call made during a pass joins it and makes it run once more at the end,
    /// so it sees fresh results. The pass clears `inflight` itself, so waiters never spin on a
    /// finished task (that busy-loop froze the app when offline).
    func refresh() async {
        if let t = inflight {
            rerun = true
            await t.value
            return
        }
        let t = Task { @MainActor in
            repeat {
                rerun = false
                if let s = SourceStore.shared.server { await probe(s) } else { route = nil }
            } while rerun
            inflight = nil
            scheduleRetry()
        }
        inflight = t
        await t.value
    }

    /// Why the server can't be reached (nil: it can, or it is still being checked).
    var problem: Problem? {
        guard route == .unreachable else { return nil }
        if !isOnline { return .offline }
        guard TailscaleService.isSupported else { return .serverDown }
        let ts = TailscaleService.shared
        switch ts.status {
        case .off: return .tailscaleOff
        case .starting: return .tailscaleStarting
        case let .needsLogin(url): return .tailscaleNeedsLogin(url)
        case let .failed(msg): return .tailscaleFailed(msg)
        case .running: return .serverDown
        }
    }

    private func probe(_ source: Source) async {
        // The demo streams from public hosts: nothing to route.
        if source.kind == .demo { route = .lan; return }
        guard let client = MoonriseClient.for(source) else { return }
        let lan = client.lanBaseURL
        #if DEBUG
        let t0 = Date()
        defer { fputs(String(format: "MoonriseRouter: %@ -> %@ (%.1f s)\n", source.name, route?.label ?? "-", Date().timeIntervalSince(t0)), stderr) }
        #endif
        if route == nil { route = .checking }
        // Signed out (or into another server) while probing: the answer is stale.
        func settle(_ r: Route) { if SourceStore.shared.server?.id == source.id { route = r } }

        // 1. Home network. (Debug seam: MOONRISE_FORCE_TAILNET=1 pretends we're away.)
        var forceTailnet = false
        #if DEBUG
        forceTailnet = DebugSeams.launch.forceTailnet
        #endif
        func unreachable() {
            client.baseURL = lan
            #if DEBUG
            if forceTailnet { client.baseURL = URL(string: "http://127.0.0.1:9")! } // simulate being away
            #endif
            settle(.unreachable)
        }
        // No network at all: nothing to probe, and probing would only add timeouts.
        guard isOnline else {
            settle(.unreachable)
            return
        }
        if !forceTailnet, let id = await Self.serverId(at: lan) {
            client.baseURL = lan
            settle(.lan)
            remember(source, serverId: id, tailnetHost: nil)
            return
        }

        #if DEBUG
        if let fake = debugTailnet {
            guard let id = await Self.serverId(at: fake), source.serverId == nil || id == source.serverId else { return unreachable() }
            client.baseURL = fake
            settle(.tailnet(host: "fake"))
            remember(source, serverId: id, tailnetHost: nil)
            return
        }
        #endif

        // 2. Tailnet, when the embedded node is up (give a starting node a moment to come up).
        let ts = TailscaleService.shared
        for _ in 0..<40 where ts.status == .starting { try? await Task.sleep(for: .milliseconds(300)) }
        guard ts.isRunning else { return unreachable() }
        let port = lan.port ?? MoonriseServer.port
        for host in await tailnetCandidates(for: source) {
            guard let local = try? ts.forward(host: host, port: port),
                  let url = URL(string: "http://127.0.0.1:\(local)"),
                  let id = await Self.serverId(at: url),
                  source.serverId == nil || id == source.serverId else { continue }
            client.baseURL = url
            settle(.tailnet(host: host))
            remember(source, serverId: id, tailnetHost: host)
            return
        }
        unreachable()
    }

    /// Known host first, then peers whose name looks like the server, then every other online peer.
    private func tailnetCandidates(for source: Source) async -> [String] {
        var out: [String] = []
        if let h = source.tailnetHost { out.append(h) }
        let name = source.name.lowercased()
        let peers = await TailscaleService.shared.peers().filter(\.online)
        let named = peers.filter { $0.hostName.lowercased() == name || $0.hostName.lowercased().hasPrefix(name + "-")
            || $0.dnsName.lowercased().hasPrefix(name + ".") || $0.dnsName.lowercased().hasPrefix(name + "-") }
        out += named.map(\.ip)
        // Only sweep the whole tailnet when we've never found the server there and can verify it by id.
        if out.isEmpty, source.serverId != nil { out += peers.prefix(8).map(\.ip) }
        var seen = Set<String>()
        return out.filter { seen.insert($0).inserted }
    }

    private func remember(_ source: Source, serverId: String?, tailnetHost: String?) {
        guard var s = SourceStore.shared.source(source.id) else { return }
        var changed = false
        if let serverId, s.serverId != serverId { s.serverId = serverId; changed = true }
        if let tailnetHost, s.tailnetHost != tailnetHost { s.tailnetHost = tailnetHost; changed = true }
        if changed { SourceStore.shared.update(s) }
    }

    /// The id of the Moonrise server answering at `base` (`GET /v1/info`), nil when none does.
    private static func serverId(at base: URL) async -> String? {
        let c = MoonriseClient(sourceId: nil, baseURL: base, token: nil)
        return try? await c.info(timeout: 3).serverId
    }
}
